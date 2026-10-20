use home_automation_kit::{video_staging, video_worker};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use video_staging::{Segment, stage_selection};
use video_worker::Selection;

fn temp() -> PathBuf {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "video-staging-{}-{id}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn segment(id: &str, path: PathBuf, closed: bool, origin: u64, duration: u64) -> Segment {
    Segment {
        id: id.into(),
        camera: "living_room".into(),
        path,
        closed,
        media_origin_ms: origin,
        media_duration_ms: duration,
    }
}
fn selection(start_ms: u64, end_ms: u64) -> Selection {
    Selection {
        camera: "living_room".into(),
        start_ms,
        end_ms,
    }
}

#[test]
fn stages_immutable_copy_with_exact_origin_and_integrity() {
    let root = temp();
    let approved = root.join("approved");
    let staging = root.join("staging");
    fs::create_dir_all(&approved).unwrap();
    fs::write(approved.join("segment.mp4"), b"synthetic-media").unwrap();
    let plan = stage_selection(
        &approved,
        &staging,
        &[segment(
            "seg1",
            approved.join("segment.mp4"),
            true,
            1_000,
            5_000,
        )],
        &selection(2_000, 4_000),
    )
    .unwrap();
    assert_eq!(plan.source.start_ms, 1_000);
    assert_eq!(plan.source.end_ms, 6_000);
    assert_eq!(plan.requested_start_ms, 2_000);
    assert_eq!(plan.requested_end_ms, 4_000);
    assert_eq!(plan.bytes, 15);
    assert_eq!(plan.sha256, plan.source.sha256);
    assert_eq!(fs::read(&plan.source.path).unwrap(), b"synthetic-media");
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[segment(
                "seg1",
                approved.join("segment.mp4"),
                true,
                1_000,
                5_000
            )],
            &selection(2_000, 4_000)
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_open_invalid_or_ambiguous_segments_and_oversized_selection() {
    let root = temp();
    let approved = root.join("approved");
    let staging = root.join("staging");
    fs::create_dir_all(&approved).unwrap();
    fs::write(approved.join("s"), b"x").unwrap();
    let closed = segment("s1", approved.join("s"), true, 1_000, 5_000);
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[segment("s1", approved.join("s"), false, 1_000, 5_000)],
            &selection(2_000, 4_000)
        )
        .is_err()
    );
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[segment("s1", approved.join("s"), true, u64::MAX - 2, 5)],
            &selection(2_000, 4_000)
        )
        .is_err()
    );
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[closed.clone(), closed],
            &selection(2_000, 4_000)
        )
        .is_err()
    );
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[
                segment("s1", approved.join("s"), true, 0, 5_000),
                segment("s2", approved.join("s"), true, 4_000, 5_000)
            ],
            &selection(1_000, 3_000)
        )
        .is_err()
    );
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[segment("s1", approved.join("s"), true, 1_000, 200_000)],
            &selection(1_000, 122_000)
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_path_traversal_and_symlink_escape() {
    let root = temp();
    let approved = root.join("approved");
    let outside = root.join("outside");
    let staging = root.join("staging");
    fs::create_dir_all(&approved).unwrap();
    fs::write(&outside, b"outside").unwrap();
    assert!(
        stage_selection(
            &approved,
            &staging,
            &[segment(
                "s1",
                approved.join("..").join("outside"),
                true,
                0,
                5_000
            )],
            &selection(0, 1_000)
        )
        .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, approved.join("link")).unwrap();
        assert!(
            stage_selection(
                &approved,
                &staging,
                &[segment("s2", approved.join("link"), true, 0, 5_000)],
                &selection(0, 1_000)
            )
            .is_err()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
