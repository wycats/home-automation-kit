//! Trusted local publisher, intentionally not exposed by the worker HTTP API.
use home_automation_kit::{
    video_staging::{Segment, stage_selection},
    video_worker::{Result, Selection, Source, sha256},
};
use serde::Deserialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    approved_root: PathBuf,
    worker_root: PathBuf,
    segments: Vec<Segment>,
    selection: Selection,
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: video-stage TRUSTED_INPUT.json")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("catalog too large".into());
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    if !input.worker_root.is_absolute() || input.segments.len() > 128 {
        return Err("invalid worker root or catalog bound".into());
    }
    let segment = input
        .segments
        .iter()
        .find(|s| {
            s.camera == input.selection.camera
                && s.media_origin_ms <= input.selection.start_ms
                && s.media_origin_ms
                    .checked_add(s.media_duration_ms)
                    .is_some_and(|e| e >= input.selection.end_ms)
        })
        .ok_or("selection has no closed segment")?;
    if segment.id.is_empty()
        || segment.id.len() > 80
        || !segment
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("invalid segment identity".into());
    }
    let root = input.worker_root.join("sources");
    fs::create_dir_all(&root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("staging.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    let dest = root.join(&segment.id);
    fs::create_dir_all(&dest)?;
    let manifest = dest.join("source.json");
    if manifest.exists() {
        let source: Source = serde_json::from_reader(File::open(&manifest)?)?;
        if source.id != segment.id
            || source.camera != segment.camera
            || source.start_ms != segment.media_origin_ms
            || source.end_ms
                != segment
                    .media_origin_ms
                    .checked_add(segment.media_duration_ms)
                    .ok_or("overflow")?
            || sha256(&source.path)? != source.sha256
        {
            return Err("existing immutable source does not match".into());
        }
        println!(
            "{}",
            serde_json::json!({"source":source.id,"already_staged":true})
        );
        return Ok(());
    }
    let plan = stage_selection(
        &input.approved_root,
        &dest,
        &input.segments,
        &input.selection,
    )?;
    let tmp = dest.join("source.json.part");
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(&plan.source)?)?;
    file.sync_all()?;
    drop(file);
    fs::hard_link(&tmp, &manifest)?;
    fs::remove_file(tmp)?;
    #[cfg(unix)]
    {
        File::open(&dest)?.sync_all()?;
        File::open(&root)?.sync_all()?;
        File::open(&input.worker_root)?.sync_all()?;
    }
    println!(
        "{}",
        serde_json::json!({"source":plan.source.id,"bytes":plan.bytes,"already_staged":false})
    );
    Ok(())
}

fn main() {
    if run().is_err() {
        // Paths and catalog contents are private; do not print upstream errors.
        eprintln!("Source staging rejected; check the trusted catalog and local files.");
        std::process::exit(1);
    }
}
