//! Closed-source staging for the isolated video worker.
//!
//! The Frigate recordings API reports segment intervals, but does not establish
//! that a file's first decoded frame is exactly at the reported start time. This
//! module therefore requires an explicit media origin and duration supplied by
//! a trusted local catalog after media inspection. It deliberately accepts one
//! source segment per selection; joining segments requires verified mux timing.
use crate::video_worker::{Selection, Source};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const MAX_SELECTION_MS: u64 = 120_000;
pub const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;

/// `closed` must come from a recorder/catalog state that establishes this file
/// will no longer grow. `media_origin_ms` and `media_duration_ms` must come from
/// inspected media timing; recording API bounds alone are insufficient.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub id: String,
    pub camera: String,
    pub path: PathBuf,
    pub closed: bool,
    pub media_origin_ms: u64,
    pub media_duration_ms: u64,
}

#[derive(Clone)]
pub struct StagedPlan {
    pub source: Source,
    pub requested_start_ms: u64,
    pub requested_end_ms: u64,
    pub sha256: String,
    pub bytes: u64,
}

pub fn stage_selection(
    approved_root: &Path,
    staging_root: &Path,
    segments: &[Segment],
    selection: &Selection,
) -> Result<StagedPlan> {
    if !approved_root.is_absolute()
        || !staging_root.is_absolute()
        || !valid_id(&selection.camera)
        || selection.end_ms <= selection.start_ms
        || !(1000..=MAX_SELECTION_MS).contains(&(selection.end_ms - selection.start_ms))
    {
        return Err("invalid root, camera, or selection bounds".into());
    }
    let approved = fs::canonicalize(approved_root)?;
    fs::create_dir_all(staging_root)?;
    let staging = fs::canonicalize(staging_root)?;
    let mut catalog: Vec<(u64, u64)> = Vec::new();
    for item in segments.iter().filter(|s| s.camera == selection.camera) {
        let end = item
            .media_origin_ms
            .checked_add(item.media_duration_ms)
            .ok_or("invalid segment timestamp range")?;
        if !valid_id(&item.id) || !(1..=600_000).contains(&item.media_duration_ms) {
            return Err("invalid segment identity or timing".into());
        }
        catalog.push((item.media_origin_ms, end));
    }
    catalog.sort_unstable();
    if catalog.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err("overlapping segment timing is ambiguous".into());
    }
    let candidates: Vec<_> = segments
        .iter()
        .filter(|s| {
            s.camera == selection.camera
                && s.media_origin_ms <= selection.start_ms
                && s.media_origin_ms
                    .checked_add(s.media_duration_ms)
                    .is_some_and(|end| selection.end_ms <= end)
        })
        .collect();
    if candidates.len() != 1 {
        return Err("selection must fit exactly one catalogued segment; gaps or ambiguous coverage are rejected".into());
    }
    let segment = candidates[0];
    if !valid_id(&segment.id)
        || !valid_id(&segment.camera)
        || !segment.closed
        || segment.media_duration_ms == 0
    {
        return Err("segment is open or has invalid identity/timing".into());
    }
    // Reject traversal syntax even when normalization would land back inside root.
    if segment
        .path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("source path traversal is not allowed".into());
    }
    let input = fs::canonicalize(&segment.path)?;
    if !input.starts_with(&approved) || !input.is_file() {
        return Err("source must be a regular file inside the approved root".into());
    }
    let before = fs::metadata(&input)?;
    if before.len() == 0 || before.len() > MAX_SOURCE_BYTES {
        return Err("source size is empty or exceeds 512 MiB".into());
    }
    let modified = before
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos());
    let file_name = format!("{}.source", segment.id);
    let dest = staging.join(&file_name);
    if dest.exists() {
        return Err("staged source already exists".into());
    }
    let part = staging.join(format!(".{}.{}.part", segment.id, std::process::id()));
    // Only remove a partial file after acquiring ownership of that file.
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)?;
    let result = (|| -> Result<(u64, String)> {
        let mut src = File::open(&input)?;
        let mut hash = Sha256::new();
        let mut buf = [0u8; 65536];
        let mut bytes = 0u64;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            bytes = bytes.checked_add(n as u64).ok_or("source size overflow")?;
            if bytes > MAX_SOURCE_BYTES {
                return Err("source exceeds 512 MiB".into());
            }
            out.write_all(&buf[..n])?;
            hash.update(&buf[..n]);
        }
        out.sync_all()?;
        let after = fs::metadata(&input)?;
        let modified_after = after
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos());
        if before.len() != after.len() || bytes != after.len() || modified != modified_after {
            return Err("source changed while staging".into());
        }
        let hash = format!("{:x}", hash.finalize());
        // Modification times can have coarse resolution. Rehash the source so
        // same-size changes during the copy cannot silently produce a mixed file.
        if hash_file(&input)? != hash {
            return Err("source content changed while staging".into());
        }
        drop(out);
        // Publish without replacing a concurrent writer's immutable destination.
        fs::hard_link(&part, &dest)?;
        fs::remove_file(&part)?;
        sync_dir(&staging)?;
        Ok((bytes, hash))
    })();
    let (bytes, hash) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
    };
    let source = Source {
        id: segment.id.clone(),
        camera: segment.camera.clone(),
        start_ms: segment.media_origin_ms,
        end_ms: segment.media_origin_ms + segment.media_duration_ms,
        sha256: hash.clone(),
        path: dest,
    };
    Ok(StagedPlan {
        source,
        requested_start_ms: selection.start_ms,
        requested_end_ms: selection.end_ms,
        sha256: hash,
        bytes,
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
