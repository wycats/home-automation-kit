//! Loopback-only experimental worker. The isolated authenticated HA adapter resolves
//! recording selections to staged source IDs; this server never opens LAN access.
use home_automation_kit::video_worker::{ClipRequest, Config, Result, Selection, State, Store};
use serde::Deserialize;
use std::{
    env,
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("constant HTTP header")
}

fn json(request: Request, status: u16, value: serde_json::Value) {
    let _ = request.respond(
        Response::from_string(value.to_string())
            .with_status_code(StatusCode(status))
            .with_header(header("Content-Type", "application/json"))
            .with_header(header("Cache-Control", "no-store")),
    );
}

fn body<T: serde::de::DeserializeOwned>(request: &mut Request) -> Result<T> {
    let mut bytes = Vec::new();
    request.as_reader().take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err("request too large".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn media(request: Request, store: &Store, id: &str, saved: bool) -> Result<()> {
    let path = store.media_path(id, saved)?;
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let range = request.headers().iter().find(|h| h.field.equiv("Range"));
    let (start, end, partial) = if let Some(range) = range {
        let parsed = range
            .value
            .as_str()
            .strip_prefix("bytes=")
            .and_then(|s| s.split_once('-'))
            .and_then(|(a, b)| {
                Some((
                    a.parse::<u64>().ok()?,
                    if b.is_empty() {
                        len.checked_sub(1)?
                    } else {
                        b.parse().ok()?
                    },
                ))
            });
        match parsed {
            Some((a, b)) if a <= b && a < len => (a, b.min(len - 1), true),
            _ => {
                request.respond(
                    Response::empty(StatusCode(416))
                        .with_header(header("Content-Range", &format!("bytes */{len}"))),
                )?;
                return Ok(());
            }
        }
    } else {
        (0, len.checked_sub(1).ok_or("empty media")?, false)
    };
    file.seek(SeekFrom::Start(start))?;
    let count = end - start + 1;
    let mut response = Response::new(
        StatusCode(if partial { 206 } else { 200 }),
        vec![
            header("Content-Type", "video/mp4"),
            header("Cache-Control", "private, no-store"),
            header("Accept-Ranges", "bytes"),
            header("X-Content-Type-Options", "nosniff"),
        ],
        file.take(count),
        Some(count.try_into()?),
        None,
    );
    if partial {
        response.add_header(header(
            "Content-Range",
            &format!("bytes {start}-{end}/{len}"),
        ));
    }
    request.respond(response)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    name: String,
}

fn handle(mut request: Request, store: &Store, token: &str, streams: &Arc<AtomicUsize>) {
    let authorized = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
    if !authorized {
        json(
            request,
            401,
            serde_json::json!({"error":"authentication required"}),
        );
        return;
    }
    let path = request.url().to_string();
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if request.method() == &Method::Get
        && parts.len() == 3
        && parts[2] == "media"
        && matches!(parts[0], "jobs" | "clips")
    {
        if store.media_path(parts[1], parts[0] == "clips").is_err() {
            json(request, 404, serde_json::json!({"error":"media not ready"}));
        } else if streams.load(Ordering::SeqCst) >= 2 {
            json(
                request,
                503,
                serde_json::json!({"error":"media streams busy; retry shortly"}),
            );
        } else {
            // Only this control thread increments; stream threads decrement.
            streams.fetch_add(1, Ordering::SeqCst);
            // A slow player must not block cancellation or status requests.
            let owner = store.clone();
            let streams = Arc::clone(streams);
            let id = parts[1].to_string();
            let saved = parts[0] == "clips";
            thread::spawn(move || {
                let _ = media(request, &owner, &id, saved);
                streams.fetch_sub(1, Ordering::SeqCst);
            });
        }
        return;
    }
    let result: Result<serde_json::Value> = (|| match (request.method(), parts.as_slice()) {
        (&Method::Get, ["health"]) => Ok(
            serde_json::json!({"service":"video-worker","version":1,"backend":store.config.backend}),
        ),
        (&Method::Get, ["jobs"]) => Ok(serde_json::to_value(store.jobs()?)?),
        (&Method::Post, ["sources", "resolve"]) => Ok(serde_json::json!({
            "source":store.resolve(&body::<Selection>(&mut request)?)?
        })),
        (&Method::Post, ["jobs"]) => Ok(serde_json::to_value(
            store.submit(body::<ClipRequest>(&mut request)?)?,
        )?),
        (&Method::Post, ["jobs", "lookup"]) => {
            Ok(serde_json::to_value(
                store.lookup(&body::<ClipRequest>(&mut request)?)?,
            )?)
        }
        (&Method::Get, ["jobs", id]) => Ok(serde_json::to_value(store.get(id)?)?),
        (&Method::Post, ["jobs", id, "retry"]) => Ok(serde_json::to_value(store.retry(id)?)?),
        (&Method::Post, ["jobs", id, "cancel"]) => {
            let job = store.cancel(id)?;
            Ok(
                serde_json::json!({"job":job,"cancel_requested":matches!(job.state,State::Queued|State::Running)}),
            )
        }
        (&Method::Post, ["jobs", id, "save"]) => {
            let save: Save = body(&mut request)?;
            Ok(serde_json::to_value(store.save(id, &save.name)?)?)
        }
        (&Method::Get, ["clips"]) => {
            let clips: Vec<_> = std::fs::read_dir(&store.config.saved_root)?
                .filter_map(|e| e.ok())
                .filter_map(|e| store.saved(&e.file_name().to_string_lossy()).ok())
                .collect();
            Ok(serde_json::to_value(clips)?)
        }
        (&Method::Get, ["clips", id]) => Ok(serde_json::to_value(store.saved(id)?)?),
        _ => Err("unknown route".into()),
    })();
    match result {
        Ok(value) => json(request, 200, value),
        Err(_) => json(
            request,
            400,
            serde_json::json!({"error":"request rejected or resource unavailable"}),
        ),
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let config_path = args
        .next()
        .ok_or("usage: video-worker CONFIG.json [PORT]")?;
    let port: u16 = args.next().unwrap_or("8787".into()).parse()?;
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let token = env::var("VIDEO_WORKER_TOKEN")?;
    if token.len() < 32 || !token.is_ascii() || token.bytes().any(|c| c.is_ascii_control()) {
        return Err("VIDEO_WORKER_TOKEN must have at least 32 printable ASCII characters".into());
    }
    let config: Config = serde_json::from_reader(File::open(config_path)?)?;
    let store = Store::open(config)?;
    let _lock = store.lock()?;
    store.recover()?;
    let server = Server::http(("127.0.0.1", port))?;
    eprintln!("video-worker listening on loopback port {port}");
    let mut worker: Option<thread::JoinHandle<Result<()>>> = None;
    let streams = Arc::new(AtomicUsize::new(0));
    loop {
        if worker.as_ref().is_some_and(|w| w.is_finished()) {
            worker
                .take()
                .expect("worker exists")
                .join()
                .map_err(|_| "worker panicked")??;
        }
        if worker.is_none()
            && let Some(job) = store.jobs()?.into_iter().find(|j| j.state == State::Queued)
        {
            let owner = store.clone();
            worker = Some(thread::spawn(move || {
                owner.run(&job.id)?;
                Ok(())
            }));
        }
        if let Some(request) = server.recv_timeout(Duration::from_millis(100))? {
            handle(request, &store, &token, &streams);
        }
    }
}

fn main() {
    if run().is_err() {
        eprintln!("video-worker stopped; check configuration or private job diagnostics");
        std::process::exit(1);
    }
}
