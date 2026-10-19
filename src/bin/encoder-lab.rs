//! Offline only: never discovers, opens or changes a video device.
use home_automation_kit::encoder_lab::Plan;
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::ExitCode,
    time::Instant,
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args == ["plan"] {
        println!(
            "{}",
            serde_json::to_string_pretty(&Plan::new(1920, 1080, 3, 30)?)?
        );
        return Ok(());
    }
    if ![3, 5].contains(&args.len()) || args[0] != "fixture" {
        return Err("usage: encoder-lab plan | encoder-lab fixture NEW_DIRECTORY FRAMES [WIDTH HEIGHT]; device execution is not enabled".into());
    }
    let (width, height) = if args.len() == 5 {
        (args[3].parse()?, args[4].parse()?)
    } else {
        (1920, 1080)
    };
    let plan = Plan::new(width, height, args[2].parse()?, 30)?;
    let path = Path::new(&args[1]);
    // Exclusive directory/file creation: never replace an earlier capture/fixture.
    fs::create_dir(path)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("synthetic.nv12"))?;
    let started = Instant::now();
    plan.write_fixture(&mut output, || started.elapsed())?;
    output.sync_all()?;
    drop(output);
    let mut headers = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("headers.h264"))?;
    headers.write_all(&plan.annex_b_headers())?;
    headers.sync_all()?;
    let mut vector = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("crop-test.h264"))?;
    vector.write_all(&plan.pcm_test_vector(0)?)?;
    vector.sync_all()?;
    if started.elapsed().as_secs() >= 30 {
        return Err("fixture deadline".into());
    }
    // A manifest is the completion marker. On error the partial fixture remains
    // without a manifest, for inspection; it must never be fed to a device runner.
    let mut manifest = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("manifest.pending"))?;
    manifest.write_all(serde_json::to_string_pretty(&plan)?.as_bytes())?;
    manifest.sync_all()?;
    drop(manifest);
    fs::rename(path.join("manifest.pending"), path.join("manifest.json"))?;
    println!(
        "Prepared {} synthetic frames ({} bytes). No hardware encoding or device access.",
        plan.frames(),
        plan.fixture_bytes()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
