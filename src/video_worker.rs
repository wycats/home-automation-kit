//! Isolated clip preparation. Inputs are immutable, operator-staged sources;
//! the public request never selects a path, URL, codec or command.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const PROFILE: &str = "browser-1080p-v1";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub camera: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sha256: String,
    pub path: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub root: PathBuf,
    pub saved_root: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub backend: Backend,
    pub deadline_seconds: u64,
    pub sources: Vec<Source>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Software,
    IntelQsv,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipRequest {
    pub source: String,
    pub camera: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    Ready,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Media {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub audio: bool,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub request: ClipRequest,
    pub state: State,
    pub backend: Backend,
    pub media: Option<Media>,
    pub error: Option<String>,
    pub created_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedClip {
    pub id: String,
    pub name: String,
    pub request: ClipRequest,
    pub media: Media,
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hash.update(&chunk[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

// Same-directory rename publishes complete metadata. On Windows replacing a file
// requires removing the prior name; a crash in that interval leaves an unusable
// job, never a falsely ready one. Linux rename replaces atomically.
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(tmp, path)?;
    sync_directory(path.parent().ok_or("metadata parent missing")?)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    pub config: Config,
}

impl Store {
    pub fn open(config: Config) -> Result<Self> {
        if !(1..=900).contains(&config.deadline_seconds)
            || !config.root.is_absolute()
            || !config.saved_root.is_absolute()
            || !config.ffmpeg.is_absolute()
            || !config.ffprobe.is_absolute()
        {
            return Err("absolute paths and a 1..900 second deadline required".into());
        }
        let mut ids = std::collections::HashSet::new();
        for s in &config.sources {
            if !valid_id(&s.id)
                || !valid_id(&s.camera)
                || !ids.insert(&s.id)
                || s.end_ms <= s.start_ms
                || s.end_ms - s.start_ms > 600_000
                || !s.path.is_absolute()
                || s.sha256.len() != 64
                || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("invalid source catalog".into());
            }
        }
        fs::create_dir_all(config.root.join("jobs"))?;
        fs::create_dir_all(&config.saved_root)?;
        if fs::canonicalize(&config.root)? == fs::canonicalize(&config.saved_root)? {
            return Err("preview and saved storage must be separate".into());
        }
        Ok(Self { config })
    }

    pub fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.config.root.join("worker.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)?;
        Ok(file)
    }

    fn source(&self, request: &ClipRequest) -> Result<&Source> {
        let source = self
            .config
            .sources
            .iter()
            .find(|s| s.id == request.source)
            .ok_or("unknown source")?;
        if request.profile != PROFILE
            || request.camera != source.camera
            || request.start_ms < source.start_ms
            || request.end_ms > source.end_ms
            || request.end_ms <= request.start_ms
            || !(1000..=120_000).contains(&(request.end_ms - request.start_ms))
        {
            return Err(
                "selection must be covered by one staged source and last 1..120 seconds".into(),
            );
        }
        Ok(source)
    }

    pub fn dir(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("invalid job identity".into());
        }
        Ok(self.config.root.join("jobs").join(id))
    }

    pub fn submit(&self, request: ClipRequest) -> Result<Job> {
        let source = self.source(&request)?;
        let id = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &request,
                source.sha256.to_lowercase(),
                source.start_ms,
                self.config.backend
            ))?)
        );
        let dir = self.dir(&id)?;
        if dir.exists() {
            return self.get(&id);
        }
        if self.jobs()?.len() >= 128 {
            return Err(
                "laboratory cache limit reached; archive or remove closed jobs offline".into(),
            );
        }
        if self
            .jobs()?
            .iter()
            .filter(|j| matches!(j.state, State::Queued | State::Running))
            .count()
            >= 8
        {
            return Err("queue full".into());
        }
        fs::create_dir(&dir)?;
        let job = Job {
            id,
            request,
            state: State::Queued,
            backend: self.config.backend,
            media: None,
            error: None,
            created_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        };
        write_json(&dir.join("job.json"), &job)?;
        Ok(job)
    }

    pub fn get(&self, id: &str) -> Result<Job> {
        Ok(serde_json::from_reader(File::open(
            self.dir(id)?.join("job.json"),
        )?)?)
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut jobs = Vec::new();
        for entry in fs::read_dir(self.config.root.join("jobs"))? {
            let id = entry?.file_name().to_string_lossy().to_string();
            if let Ok(job) = self.get(&id) {
                jobs.push(job);
            }
        }
        jobs.sort_by_key(|j| j.created_ms);
        Ok(jobs)
    }

    pub fn recover(&self) -> Result<()> {
        for mut job in self.jobs()? {
            if job.state == State::Running {
                job.state = State::Failed;
                job.error = Some("interrupted; preparation did not finish".into());
                write_json(&self.dir(&job.id)?.join("job.json"), &job)?;
            }
        }
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<Job> {
        let job = self.get(id)?;
        if matches!(job.state, State::Queued | State::Running) {
            File::create(self.dir(id)?.join("cancel"))?.sync_all()?;
        }
        // Cancellation is requested, not acknowledged until the child has exited.
        Ok(job)
    }

    pub fn retry(&self, id: &str) -> Result<Job> {
        let mut job = self.get(id)?;
        if !matches!(job.state, State::Failed | State::Cancelled) {
            return Err("only a stopped failed or cancelled job can be retried".into());
        }
        if self
            .jobs()?
            .iter()
            .filter(|j| matches!(j.state, State::Queued | State::Running))
            .count()
            >= 8
        {
            return Err("queue full".into());
        }
        for name in ["cancel", "preview.mp4", "preview.part.mp4"] {
            let path = self.dir(id)?.join(name);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        job.state = State::Queued;
        job.error = None;
        job.media = None;
        write_json(&self.dir(id)?.join("job.json"), &job)?;
        Ok(job)
    }

    pub fn media_path(&self, id: &str, saved: bool) -> Result<PathBuf> {
        self.dir(id)?;
        if saved {
            self.saved(id)?;
            Ok(self.config.saved_root.join(id).join("clip.mp4"))
        } else {
            if self.get(id)?.state != State::Ready {
                return Err("preview is not ready".into());
            }
            Ok(self.dir(id)?.join("preview.mp4"))
        }
    }

    pub fn saved(&self, id: &str) -> Result<SavedClip> {
        self.dir(id)?;
        Ok(serde_json::from_reader(File::open(
            self.config.saved_root.join(id).join("clip.json"),
        )?)?)
    }

    pub fn save(&self, id: &str, name: &str) -> Result<SavedClip> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return Err("choose a name of 1..120 characters".into());
        }
        let job = self.get(id)?;
        if job.state != State::Ready {
            return Err("wait for a verified preview before saving".into());
        }
        let media = job.media.ok_or("missing media verification")?;
        let dest = self.config.saved_root.join(id);
        fs::create_dir_all(&dest)?;
        let final_path = dest.join("clip.mp4");
        if !final_path.exists() || sha256(&final_path)? != media.sha256 {
            let part = dest.join("clip.part");
            fs::copy(self.dir(id)?.join("preview.mp4"), &part)?;
            OpenOptions::new().write(true).open(&part)?.sync_all()?;
            if sha256(&part)? != media.sha256 {
                return Err("saved copy verification failed".into());
            }
            if final_path.exists() {
                fs::remove_file(&final_path)?;
            }
            fs::rename(part, final_path)?;
            sync_directory(&dest)?;
        }
        let clip = SavedClip {
            id: id.into(),
            name: name.into(),
            request: job.request,
            media,
        };
        write_json(&dest.join("clip.json"), &clip)?;
        Ok(clip)
    }

    /// Run exactly one owned conversion. No shell, network inputs or fallback.
    pub fn run(&self, id: &str) -> Result<Job> {
        let mut job = self.get(id)?;
        if job.state != State::Queued {
            return Err("job is not queued".into());
        }
        let dir = self.dir(id)?;
        job.state = State::Running;
        write_json(&dir.join("job.json"), &job)?;
        let deadline = Instant::now() + Duration::from_secs(self.config.deadline_seconds);
        let outcome = self.convert(&job, &dir, deadline);
        if dir.join("cancel").exists() {
            job.state = State::Cancelled;
            job.media = None;
        } else {
            match outcome {
                Ok(media) => {
                    job.state = State::Ready;
                    job.media = Some(media);
                }
                Err(_) => {
                    job.state = State::Failed;
                    job.error = Some(
                        "preparation or media verification failed; inspect local diagnostics"
                            .into(),
                    );
                }
            }
        }
        write_json(&dir.join("job.json"), &job)?;
        Ok(job)
    }

    fn convert(&self, job: &Job, dir: &Path, deadline: Instant) -> Result<Media> {
        let source = self.source(&job.request)?;
        let expected_id = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &job.request,
                source.sha256.to_lowercase(),
                source.start_ms,
                job.backend
            ))?)
        );
        if expected_id != job.id || fs::metadata(&source.path)?.len() > 512 * 1024 * 1024 {
            return Err("source catalog changed or staged source exceeds 512 MiB".into());
        }
        if sha256(&source.path)? != source.sha256.to_lowercase() {
            return Err("staged source identity changed".into());
        }
        let source_probe = self.probe(&source.path, dir, "source", deadline)?;
        let streams = source_probe["streams"]
            .as_array()
            .ok_or("missing streams")?;
        let video = streams
            .iter()
            .find(|s| s["codec_type"] == "video")
            .ok_or("no video")?;
        let source_has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
        let width = video["width"].as_u64().ok_or("missing width")? as u32;
        let height = video["height"].as_u64().ok_or("missing height")? as u32;
        let (width, height) = dimensions(width, height)?;
        let end_offset = (job.request.end_ms - source.start_ms) as f64 / 1000.0;
        if duration(&source_probe)? + 0.050 < end_offset {
            return Err("source media does not cover selection".into());
        }
        let part = dir.join("preview.part.mp4");
        let mut args: Vec<String> =
            vec!["-nostdin".into(), "-y".into(), "-v".into(), "error".into()];
        if job.backend == Backend::IntelQsv {
            let decoder = match video["codec_name"].as_str() {
                Some("hevc") => "hevc_qsv",
                Some("h264") => "h264_qsv",
                _ => return Err("Intel pilot requires H.264 or HEVC input".into()),
            };
            args.extend(
                [
                    "-init_hw_device",
                    "qsv=hw:hw",
                    "-filter_hw_device",
                    "hw",
                    "-hwaccel",
                    "qsv",
                    "-hwaccel_device",
                    "hw",
                    "-hwaccel_output_format",
                    "qsv",
                    "-c:v",
                    decoder,
                ]
                .map(String::from),
            );
        }
        args.extend(["-protocol_whitelist", "file,pipe", "-i"].map(String::from));
        args.push(source.path.to_str().ok_or("non-UTF8 source path")?.into());
        // Output-side accurate trimming decodes through the boundary rather than
        // stream-copying from an earlier keyframe.
        args.extend([
            "-ss".into(),
            format!(
                "{:.3}",
                (job.request.start_ms - source.start_ms) as f64 / 1000.0
            ),
            "-t".into(),
            format!(
                "{:.3}",
                (job.request.end_ms - job.request.start_ms) as f64 / 1000.0
            ),
        ]);
        args.extend(
            [
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-map_metadata",
                "-1",
                "-vf",
            ]
            .map(String::from),
        );
        if job.backend == Backend::IntelQsv {
            args.extend([
                format!("vpp_qsv=w={width}:h={height}:format=nv12"),
                "-c:v".into(),
                "h264_qsv".into(),
                "-global_quality".into(),
                "23".into(),
            ]);
        } else {
            args.extend([
                format!("scale={width}:{height}"),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                "-crf".into(),
                "23".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-threads".into(),
                "2".into(),
            ]);
        }
        args.extend(
            [
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "-progress",
            ]
            .map(String::from),
        );
        args.push(dir.join("progress.txt").to_string_lossy().into());
        args.push(part.to_string_lossy().into());
        run_process(&self.config.ffmpeg, &args, dir, "encode", deadline)?;
        let probe = self.probe(&part, dir, "result", deadline)?;
        let result = probe["streams"]
            .as_array()
            .ok_or("missing output streams")?;
        let v = result
            .iter()
            .find(|s| s["codec_type"] == "video")
            .ok_or("no output video")?;
        let actual_duration = duration(&probe)?;
        let expected = (job.request.end_ms - job.request.start_ms) as f64 / 1000.0;
        if v["codec_name"] != "h264"
            || v["width"] != width
            || v["height"] != height
            || v["pix_fmt"] != "yuv420p"
            || (actual_duration - expected).abs() > 0.150
            || source_has_audio != result.iter().any(|s| s["codec_type"] == "audio")
            || result
                .iter()
                .any(|s| s["codec_type"] == "audio" && s["codec_name"] != "aac")
        {
            return Err("output does not match browser profile".into());
        }
        run_process(
            &self.config.ffmpeg,
            &[
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-xerror".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-i".into(),
                part.to_string_lossy().into(),
                "-f".into(),
                "null".into(),
                "-".into(),
            ],
            dir,
            "decode",
            deadline,
        )?;
        let media = Media {
            width,
            height,
            duration_ms: (actual_duration * 1000.0).round() as u64,
            audio: result.iter().any(|s| s["codec_type"] == "audio"),
            sha256: sha256(&part)?,
        };
        OpenOptions::new().write(true).open(&part)?.sync_all()?;
        fs::rename(part, dir.join("preview.mp4"))?;
        sync_directory(dir)?;
        Ok(media)
    }

    fn probe(
        &self,
        path: &Path,
        dir: &Path,
        label: &str,
        deadline: Instant,
    ) -> Result<serde_json::Value> {
        run_process(
            &self.config.ffprobe,
            &[
                "-v".into(),
                "error".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-show_streams".into(),
                "-show_format".into(),
                "-of".into(),
                "json".into(),
                path.to_string_lossy().into(),
            ],
            dir,
            label,
            deadline,
        )?;
        Ok(serde_json::from_reader(File::open(
            dir.join(format!("{label}.stdout")),
        )?)?)
    }
}

fn duration(probe: &serde_json::Value) -> Result<f64> {
    let value: f64 = probe["format"]["duration"]
        .as_str()
        .ok_or("missing duration")?
        .parse()?;
    if !value.is_finite() || value <= 0.0 {
        return Err("invalid duration".into());
    }
    Ok(value)
}

pub fn dimensions(width: u32, height: u32) -> Result<(u32, u32)> {
    if !(2..=8192).contains(&width) || !(2..=8192).contains(&height) {
        return Err("unsupported source dimensions".into());
    }
    let scale = (1920.0 / width as f64).min(1080.0 / height as f64).min(1.0);
    Ok((
        ((width as f64 * scale) as u32 / 2 * 2).max(2),
        ((height as f64 * scale) as u32 / 2 * 2).max(2),
    ))
}

fn run_process(
    exe: &Path,
    args: &[String],
    dir: &Path,
    label: &str,
    deadline: Instant,
) -> Result<()> {
    if dir.join("cancel").exists() || Instant::now() >= deadline {
        return Err("cancelled or deadline exceeded".into());
    }
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(File::create(dir.join(format!("{label}.stdout")))?)
        .stderr(File::create(dir.join(format!("{label}.stderr")))?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, never elevation.
    }
    let mut child = command.spawn()?;
    loop {
        if let Some(exit) = child.try_wait()? {
            return if exit.success() {
                Ok(())
            } else {
                Err("media process failed".into())
            };
        }
        if dir.join("cancel").exists() || Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err("cancelled or deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(40));
    }
}
