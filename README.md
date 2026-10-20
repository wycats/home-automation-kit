# Home Automation Kit

A Rust toolkit for inspecting and maintaining home automation systems.

The goal is a reusable, tested core for protocol handling and analysis, with small
command-line tools around it. Observations retain their source and time: a camera
being reachable, a recent recording, and a populated search index are different
claims.

The package is being developed through pull requests. Household configuration,
credentials, recordings, and device inventories belong outside this repository.

Licensed under MIT.

## Browser clip worker pilot

The [isolated video worker](docs/video-worker.md) prepares verified browser
previews and saves named clips separately. It includes a persisted Rust job
service, a review UI component and synthetic-media qualification. It is not
deployed automatically; hardware acceleration and the authenticated recorder
adapter require commissioning before production use.

## First adapter: Frigate observations

```sh
cargo run --locked -- check-frigate http://localhost:5000 sample_camera
```

The CLI reads `/api/stats` and a recording metadata window with explicit `after`
and `before` timestamps. It prints JSON with camera/process/skipped frame rates
and recording freshness (a 30-minute window and 120-second freshness threshold).
It sends no control commands. For an authenticated endpoint, supply a bearer
token through `FRIGATE_TOKEN`; never put credentials in the URL.

Each request has a 15-second deadline and a 1 MiB response limit. Redirects are
rejected and TLS certificates are verified. Errors omit raw bodies and URLs.
HTTP is available for trusted local Frigate endpoints; use HTTPS when credentials
cross an untrusted network. Output contains the supplied camera name and belongs
in your private operational logs.

Exit 0 means the observations were fetched, even if recordings are stale. Exit 1
means usage, transport, or protocol failure. Missing data remains unknown.
Recent metadata does **not** establish playable video, uninterrupted coverage,
correct storage placement, or semantic-search index readiness. Those require
separate checks. This first adapter intentionally does not invoke AI providers or
make changes to the running system.

The wire shape is based on Frigate 0.18.2. Tests use synthetic local HTTP servers,
including malformed replies, timeouts, redirects, and oversized bodies. See
[CONTRIBUTING.md](CONTRIBUTING.md) for validation and contribution expectations.
