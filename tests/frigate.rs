use home_automation_kit::frigate::{
    Error, FrigateClient, Recording, RecordingFreshness, recording_freshness,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

/// One synthetic response per connection; returns the captured request for assertions.
fn server(response: Vec<u8>) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|s| s == b"\r\n\r\n") {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..n]);
        }
        tx.send(String::from_utf8(request).unwrap()).unwrap();
        // A bounded client may close the socket before a large body finishes.
        let _ = stream.write_all(&response);
    });
    (address, rx, handle)
}

fn json_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn client(url: &str) -> FrigateClient {
    FrigateClient::new(url, None, Duration::from_secs(2)).unwrap()
}

#[test]
fn freshness_distinguishes_absence_staleness_and_clock_skew() {
    let records = [
        Recording {
            start_time: 70.0,
            end_time: 90.0,
        },
        Recording {
            start_time: 30.0,
            end_time: 40.0,
        },
    ];
    assert_eq!(
        recording_freshness(&records, 100.0, 10.0).unwrap(),
        RecordingFreshness::Recent { age_seconds: 10.0 }
    );
    assert_eq!(
        recording_freshness(&records, 100.0, 9.0).unwrap(),
        RecordingFreshness::Stale { age_seconds: 10.0 }
    );
    assert_eq!(
        recording_freshness(&[], 100.0, 10.0).unwrap(),
        RecordingFreshness::NoSegmentsInWindow
    );
    for record in [
        Recording {
            start_time: 90.0,
            end_time: 101.0,
        },
        Recording {
            start_time: 90.0,
            end_time: 80.0,
        },
        Recording {
            start_time: f64::NAN,
            end_time: 90.0,
        },
        Recording {
            start_time: -1.0,
            end_time: 90.0,
        },
    ] {
        assert_eq!(
            recording_freshness(&[record], 100.0, 10.0).unwrap(),
            RecordingFreshness::InvalidTimestamps
        );
    }
    assert!(recording_freshness(&[], f64::INFINITY, 10.0).is_err());
    assert!(recording_freshness(&[], 100.0, -1.0).is_err());
}

#[test]
fn recording_request_is_get_with_both_bounds_and_preserves_base_path() {
    let (url, requests, handle) = server(json_response(
        r#"[{"start_time":70,"end_time":90,"extra":"ignored"}]"#,
    ));
    let records = client(&format!("{url}proxy/frigate"))
        .recordings("sample_camera", 50.0, 100.0)
        .unwrap();
    assert_eq!(records.len(), 1);
    assert!(requests.recv().unwrap().starts_with(
        "GET /proxy/frigate/api/sample_camera/recordings?after=50&before=100 HTTP/1.1"
    ));
    handle.join().unwrap();
}

#[test]
fn stats_accepts_additive_fields_and_keeps_missing_rates_unknown() {
    let (url, requests, handle) = server(json_response(
        r#"{"cameras":{"sample_camera":{"camera_fps":5,"future_field":1}},"new_section":{}}"#,
    ));
    let stats = client(&url).stats().unwrap();
    assert_eq!(stats.cameras["sample_camera"].camera_fps, Some(5.0));
    assert_eq!(stats.cameras["sample_camera"].process_fps, None);
    requests.recv().unwrap();
    handle.join().unwrap();
}

#[test]
fn malformed_and_truncated_replies_are_errors() {
    for (response, expected) in [
        (json_response("not json"), Error::InvalidResponse),
        (
            json_response(r#"{"cameras":{"sample_camera":{"camera_fps":"five"}}}"#),
            Error::InvalidResponse,
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec(),
            Error::Transport,
        ),
    ] {
        let (url, requests, handle) = server(response);
        assert_eq!(client(&url).stats().unwrap_err(), expected);
        requests.recv().unwrap();
        handle.join().unwrap();
    }
}

#[test]
fn redirects_are_rejected_and_errors_do_not_echo_bodies_or_tokens() {
    let (url, requests, handle) = server(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret".to_vec());
    let client = FrigateClient::new(&url, Some("synthetic-token"), Duration::from_secs(2)).unwrap();
    let error = client.stats().unwrap_err();
    assert_eq!(error, Error::Http(302));
    assert_eq!(error.to_string(), "Frigate returned HTTP 302");
    assert!(
        requests
            .recv()
            .unwrap()
            .to_lowercase()
            .contains("authorization: bearer synthetic-token")
    );
    handle.join().unwrap();
}

#[test]
fn response_limit_applies_with_and_without_content_length() {
    let huge = " ".repeat(1_048_577);
    for response in [
        json_response(&huge),
        format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{huge}").into_bytes(),
    ] {
        let (url, requests, handle) = server(response);
        assert_eq!(client(&url).stats().unwrap_err(), Error::ResponseTooLarge);
        requests.recv().unwrap();
        handle.join().unwrap();
    }
}

#[test]
fn stalled_response_has_a_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (_connection, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(500));
    });
    let client = FrigateClient::new(&url, None, Duration::from_millis(100)).unwrap();
    assert_eq!(client.stats().unwrap_err(), Error::Transport);
    handle.join().unwrap();
}

#[test]
fn invalid_inputs_are_rejected_before_network_access() {
    for url in [
        "file:///etc/passwd",
        "http://user:password@localhost",
        "http://localhost?token=secret",
        "http://localhost#fragment",
    ] {
        assert!(FrigateClient::new(url, None, Duration::from_secs(1)).is_err());
    }
    let client = client("http://127.0.0.1:1");
    for camera in ["", "../stats", "camera?x=1", "a/b"] {
        assert!(matches!(
            client.recordings(camera, 1.0, 2.0),
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        client.recordings("sample", 2.0, 1.0),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        client.observe("sample", 100.0, 50.0, f64::NAN),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn cli_fetches_both_sources_and_reports_unknown_fields() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for body in [
            "[]",
            r#"{"cameras":{"sample_camera":{"camera_fps":2,"process_fps":-1}}}"#,
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            stream.write_all(&json_response(body)).unwrap();
        }
    });
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_home-automation-kit"))
        .args(["check-frigate", &url, "sample_camera"])
        .env_remove("FRIGATE_TOKEN")
        .output()
        .unwrap();
    handle.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["present_in_stats"], true);
    assert_eq!(result["camera_fps"], 2.0);
    assert!(result["process_fps"].is_null());
    assert!(result["skipped_fps"].is_null());
    assert_eq!(
        result["recording_freshness"]["status"],
        "no_segments_in_window"
    );
}
