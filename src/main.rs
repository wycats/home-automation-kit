use home_automation_kit::frigate::FrigateClient;
use std::{
    env,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "check-frigate" {
        return Err("usage: home-automation-kit check-frigate BASE_URL CAMERA".into());
    }
    let token = env::var("FRIGATE_TOKEN").ok();
    let client = FrigateClient::new(&args[1], token.as_deref(), Duration::from_secs(15))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    let observation = client.observe(&args[2], now, 1800.0, 120.0)?;
    println!("{}", serde_json::to_string_pretty(&observation)?);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
