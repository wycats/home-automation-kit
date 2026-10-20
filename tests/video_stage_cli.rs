use home_automation_kit::video_worker::{Backend, Config, Selection, Store};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn local_publisher_is_discovered_without_worker_restart() {
    let root = std::env::temp_dir().join(format!(
        "stage-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let approved = root.join("recordings");
    fs::create_dir_all(&approved).unwrap();
    fs::write(approved.join("synthetic"), b"synthetic closed source").unwrap();
    let store = Store::open(Config {
        root: root.join("worker"),
        saved_root: root.join("saved"),
        ffmpeg: root.join("ffmpeg"),
        ffprobe: root.join("ffprobe"),
        backend: Backend::Software,
        deadline_seconds: 5,
        sources: vec![],
    })
    .unwrap();
    let selection = Selection {
        camera: "room".into(),
        start_ms: 11000,
        end_ms: 13000,
    };
    assert!(store.resolve(&selection).is_err());
    let input = serde_json::json!({
        "approved_root":approved,"worker_root":store.config.root,
        "segments":[{"id":"segment1","camera":"room","path":approved.join("synthetic"),
            "closed":true,"media_origin_ms":10000,"media_duration_ms":10000}],
        "selection":{"camera":"room","start_ms":11000,"end_ms":13000}
    });
    let path = root.join("input.json");
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    for _ in 0..2 {
        let result = Command::new(env!("CARGO_BIN_EXE_video-stage"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?}", result.stderr);
        assert_eq!(store.resolve(&selection).unwrap(), "segment1");
    }
    // A locally corrupted path cannot make the worker open an unrelated file.
    let manifest = store.config.root.join("sources/segment1/source.json");
    let mut source: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    source["path"] = serde_json::to_value(approved.join("synthetic")).unwrap();
    fs::write(&manifest, serde_json::to_vec(&source).unwrap()).unwrap();
    assert!(store.resolve(&selection).is_err());
    fs::remove_dir_all(root).unwrap();
}
