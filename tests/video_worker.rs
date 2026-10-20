use home_automation_kit::video_worker::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture() -> Store {
    // SystemTime can return the same value to parallel tests on Windows.
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "clip-test-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    Store::open(Config {
        root: root.join("preview"),
        saved_root: root.join("saved"),
        ffmpeg: root.join("missing-ffmpeg"),
        ffprobe: root.join("missing-ffprobe"),
        backend: Backend::Software,
        deadline_seconds: 5,
        sources: vec![Source {
            id: "fixture".into(),
            camera: "room".into(),
            start_ms: 10000,
            end_ms: 20000,
            sha256: "a".repeat(64),
            path: root.join("source.mp4"),
        }],
    })
    .unwrap()
}
fn request() -> ClipRequest {
    ClipRequest {
        source: "fixture".into(),
        camera: "room".into(),
        start_ms: 11000,
        end_ms: 13000,
        profile: PROFILE.into(),
    }
}
#[test]
fn selection_scope_and_identity() {
    let store = fixture();
    assert!(store.lookup(&request()).unwrap().is_none());
    let job = store.submit(request()).unwrap();
    assert_eq!(store.lookup(&request()).unwrap().unwrap().id, job.id);
    assert_eq!(job.id, store.submit(request()).unwrap().id);
    let mut other = request();
    other.end_ms += 1000;
    assert_ne!(job.id, store.submit(other).unwrap().id);
    let mut invalid = request();
    invalid.camera = "elsewhere".into();
    assert!(store.submit(invalid).is_err());
    let mut invalid = request();
    invalid.end_ms = 20001;
    assert!(store.submit(invalid).is_err());
    let mut invalid = request();
    invalid.start_ms = invalid.end_ms;
    assert!(store.submit(invalid).is_err());
    assert!(store.get("../source").is_err());
    assert!(store.save(&job.id, "test").is_err());
    let mut config = store.config.clone();
    config.sources[0].sha256 = "b".repeat(64);
    let changed = Store::open(config).unwrap();
    assert!(changed.lookup(&request()).unwrap().is_none());
    assert_ne!(changed.submit(request()).unwrap().id, job.id);
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}
#[test]
fn cancel_is_acknowledged_only_after_runner_finishes() {
    let store = fixture();
    let job = store.submit(request()).unwrap();
    assert_eq!(store.cancel(&job.id).unwrap().state, State::Queued);
    assert_eq!(store.run(&job.id).unwrap().state, State::Cancelled);
    assert!(store.media_path(&job.id, false).is_err());
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}
#[test]
fn failed_process_never_publishes_media() {
    let store = fixture();
    let job = store.submit(request()).unwrap();
    assert_eq!(store.run(&job.id).unwrap().state, State::Failed);
    assert!(store.media_path(&job.id, false).is_err());
    assert!(store.save(&job.id, "Family moment").is_err());
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}
#[test]
fn interrupted_job_is_not_ready_and_lock_excludes_second_worker() {
    let store = fixture();
    let mut job = store.submit(request()).unwrap();
    job.state = State::Running;
    fs::write(
        store.dir(&job.id).unwrap().join("job.json"),
        serde_json::to_vec(&job).unwrap(),
    )
    .unwrap();
    store.recover().unwrap();
    assert_eq!(store.get(&job.id).unwrap().state, State::Failed);
    let lock = store.lock().unwrap();
    assert!(store.lock().is_err());
    drop(lock);
    assert!(store.lock().is_ok());
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}
#[test]
fn preserves_aspect_without_upscaling() {
    assert_eq!(dimensions(3840, 2160).unwrap(), (1920, 1080));
    assert_eq!(dimensions(1080, 1920).unwrap(), (606, 1080));
    assert_eq!(dimensions(640, 480).unwrap(), (640, 480));
    assert!(dimensions(0, 1080).is_err());
}

#[test]
fn storage_roots_cannot_contain_each_other() {
    let store = fixture();
    for saved_inside in [true, false] {
        let mut config = store.config.clone();
        if saved_inside {
            config.saved_root = config.root.join("saved");
        } else {
            config.root = config.saved_root.join("cache");
        }
        assert!(Store::open(config).is_err());
    }
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}

#[test]
fn interrupted_submission_can_be_recreated_without_publishing_remnants() {
    let store = fixture();
    let job = store.submit(request()).unwrap();
    let dir = store.dir(&job.id).unwrap();
    for metadata in [None, Some(b"broken".as_slice())] {
        if let Some(bytes) = metadata {
            fs::write(dir.join("job.json"), bytes).unwrap();
        } else {
            fs::remove_file(dir.join("job.json")).unwrap();
        }
        fs::write(dir.join("cancel"), b"").unwrap();
        fs::write(dir.join("preview.mp4"), b"partial").unwrap();
        assert!(store.lookup(&request()).unwrap().is_none());
        let restored = store.submit(request()).unwrap();
        assert_eq!(restored.id, job.id);
        assert_eq!(restored.state, State::Queued);
        assert!(!dir.join("cancel").exists());
        assert!(!dir.join("preview.mp4").exists());
        assert!(store.media_path(&job.id, false).is_err());
    }
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}

// Opt-in only: use locally installed binaries, no download or household media.
#[test]
#[ignore = "requires real FFmpeg/ffprobe; mandatory in synthetic-media CI"]
fn synthetic_real_conversion_and_saved_copy() {
    let (Ok(ffmpeg), Ok(ffprobe)) = (
        std::env::var("VIDEO_TEST_FFMPEG"),
        std::env::var("VIDEO_TEST_FFPROBE"),
    ) else {
        panic!("VIDEO_TEST_FFMPEG and VIDEO_TEST_FFPROBE are required");
    };
    let store = fixture();
    let mut config = store.config.clone();
    config.ffmpeg = PathBuf::from(ffmpeg);
    config.ffprobe = PathBuf::from(ffprobe);
    config.deadline_seconds = 90;
    let source = &mut config.sources[0];
    assert!(
        std::process::Command::new(&config.ffmpeg)
            .args([
                "-nostdin",
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "10",
                "-c:v",
                "libx264",
                "-g",
                "90",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac"
            ])
            .arg(&source.path)
            .status()
            .unwrap()
            .success()
    );
    source.sha256 = sha256(&source.path).unwrap();
    let store = Store::open(config).unwrap();
    let job = store.submit(request()).unwrap();
    let ready = store.run(&job.id).unwrap();
    assert_eq!(ready.state, State::Ready, "{:?}", ready.error);
    assert!(ready.media.as_ref().unwrap().audio);
    assert!((ready.media.as_ref().unwrap().duration_ms as i64 - 2000).abs() <= 150);
    assert!(
        fs::read_dir(&store.config.saved_root)
            .unwrap()
            .next()
            .is_none()
    );
    let clip = store.save(&job.id, "A moment").unwrap();
    assert_eq!(
        clip.media.sha256,
        sha256(&store.media_path(&job.id, true).unwrap()).unwrap()
    );
    assert_eq!(store.save(&job.id, "A new name").unwrap().id, clip.id);
    assert_eq!(fs::read_dir(&store.config.saved_root).unwrap().count(), 1);
    fs::remove_file(store.media_path(&job.id, false).unwrap()).unwrap();
    assert!(store.media_path(&job.id, true).unwrap().exists());
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "requires real FFmpeg/ffprobe; mandatory in synthetic-media CI"]
fn synthetic_4k_hevc_becomes_1080p_h264_without_inventing_audio() {
    let mut config = fixture().config;
    config.ffmpeg = PathBuf::from(std::env::var("VIDEO_TEST_FFMPEG").unwrap());
    config.ffprobe = PathBuf::from(std::env::var("VIDEO_TEST_FFPROBE").unwrap());
    config.deadline_seconds = 90;
    let source = &mut config.sources[0];
    assert!(
        std::process::Command::new(&config.ffmpeg)
            .args([
                "-nostdin",
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=3840x2160:rate=5",
                "-t",
                "3",
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-crf",
                "28",
                "-x265-params",
                "pools=1:frame-threads=1:log-level=error",
                "-an"
            ])
            .arg(&source.path)
            .status()
            .unwrap()
            .success()
    );
    source.sha256 = sha256(&source.path).unwrap();
    let store = Store::open(config).unwrap();
    let job = store.submit(request()).unwrap();
    let ready = store.run(&job.id).unwrap();
    assert_eq!(ready.state, State::Ready, "{:?}", ready.error);
    let media = ready.media.unwrap();
    assert_eq!((media.width, media.height), (1920, 1080));
    assert!(!media.audio);
    assert!((media.duration_ms as i64 - 2000).abs() <= 150);
    fs::remove_dir_all(store.config.root.parent().unwrap()).unwrap();
}
