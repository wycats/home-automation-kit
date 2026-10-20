# Browser clip worker: isolated pilot

This worker prepares a bounded H.264/AAC MP4 preview from an immutable staged
recording, then saves a separate named copy only when requested. The recorder,
its original recordings, and retention policy remain independent.

The implementation is a pilot, not a deployed HA integration. It includes a
Rust service, persisted job state, a browser component and an opt-in seam for the
current review card. **Live recording resolution/staging and the authenticated
HA proxy are still integration work.** The Intel backend needs qualification on
the actual machine. No measured throughput or hardware support claim follows
from the software tests.

## Run in an isolated Linux service

Build `cargo build --release --locked --bin video-worker`. Supply a private JSON
config with absolute paths:

```json
{
  "root": "/var/lib/video-worker/cache",
  "saved_root": "/var/lib/video-worker/saved",
  "ffmpeg": "/usr/bin/ffmpeg",
  "ffprobe": "/usr/bin/ffprobe",
  "backend": "software",
  "deadline_seconds": 120,
  "sources": [{
    "id": "fixture-v1",
    "camera": "fixture",
    "start_ms": 10000,
    "end_ms": 18000,
    "sha256": "<64 hexadecimal characters>",
    "path": "/var/lib/video-worker/sources/fixture.mp4"
  }]
}
```

Source timestamps are UTC milliseconds. Catalog entries must describe **closed,
immutable, contiguous media**, at most ten minutes and 512 MiB each. Requests
select 1–120 seconds fully covered by one entry. The trusted staging adapter
must preserve the timestamp origin, reject recording gaps, hash the staged
file, and prevent modification while it is read. Arbitrary URLs, paths, codecs
or FFmpeg arguments are never accepted from the browser.

Set `VIDEO_WORKER_TOKEN` to a random private token of at least 32 printable ASCII
characters through the service environment; do not put it in a URL or browser.
Run `video-worker /private/config.json 8787`. It binds only `127.0.0.1`.
Every route requires `Authorization: Bearer ...`.

Start with the software backend on synthetic footage. `intel_qsv` explicitly
requests QSV decoding, scaling and H.264 encoding and fails rather than silently
switching to software. FFmpeg must be built with QSV support and the Intel media
driver must support the device. Device discovery, render permissions and actual
camera-format validation are machine commissioning tasks.
The pilot explicitly selects `hevc_qsv` or `h264_qsv` for input and a hardware
QSV device; other input codecs are rejected on this backend. See FFmpeg's
[device selection](https://ffmpeg.org/ffmpeg.html#Advanced-Video-options) and
[QSV encoder options](https://ffmpeg.org/ffmpeg-codecs.html#QSV-Encoders).

Run under a supervisor that kills **the entire process group/cgroup** on stop
or crash; for systemd use `KillMode=control-group`, `TimeoutStopSec=10`,
`Restart=on-failure`, `UMask=0077`, a dedicated unprivileged account, and private
read/write paths. The server holds an OS lock to exclude another instance on the
same cache. Restart marks interrupted jobs failed, never ready. A bare process
restart cannot guarantee that a child from an abruptly killed server is gone;
the process-group requirement is part of the deployment contract.

Do not mount production storage or enable the service merely to test this PR.
For adoption, stage input onto local SSD; give the service read-only source
access and a dedicated saved-clips share, not general NAS write access.

## API and lifecycle

| Route | Meaning |
| --- | --- |
| `GET /health` | Backend/service identity |
| `POST /jobs` | Submit `{source,camera,start_ms,end_ms,profile}` |
| `POST /jobs/lookup` | Same selection: find an existing job without submitting |
| `GET /jobs`, `GET /jobs/{id}` | Persisted status |
| `POST /jobs/{id}/cancel` | Request cancellation; poll for acknowledgement |
| `POST /jobs/{id}/retry` | Retry a terminal failed/cancelled job |
| `GET /jobs/{id}/media` | Verified preview, only when ready |
| `POST /jobs/{id}/save` | Save/rename `{name}` with an idempotent identity |
| `GET /clips`, `GET /clips/{id}` | Named saved library |
| `GET /clips/{id}/media` | Reopen saved MP4, with byte-range support |

The only profile is `browser-1080p-v1`: preserve aspect, no upscaling, at most
1920×1080, H.264 8-bit 4:2:0 and AAC when source audio exists. Output-side
trimming avoids starting at an earlier keyframe. A complete decode plus codec,
dimension, audio-presence and duration checks precede publication.

Job identity includes selection, source hash/origin, profile and backend.
Repeat submission reuses the existing job. One conversion runs at a time;
the queue allows eight active jobs and the pilot cache allows 128 total jobs.
Only two media responses stream concurrently so slow playback cannot block the
control loop. The trusted proxy must also enforce HTTP body and connection
timeouts: this small loopback service is not a public-facing web server.

Cancellation/processing deadlines kill and reap the owned media child before a
terminal status is persisted. Deadlines bound child processing; staging/hash
I/O and a stalled filesystem are not a hard real-time guarantee. Partial files
are not exposed. Metadata publication is atomic on Linux, with file and
directory synchronization; Windows tests use a remove/rename fallback that can
leave missing metadata after a crash, but cannot claim a partial file is ready.
Storage-server durability still depends on the share's own guarantees.

Saving copies and verifies the preview into the separate saved root; preview
eviction cannot remove that saved file. A retry of a save or rename uses the
same clip identity. There is no automatic eviction in this pilot: clean closed
cache jobs offline, and never apply cache cleanup to the saved root.

## Review integration

`web/review-worker.mjs` provides `ReviewSession`, `<katz-worker-review>` and
`attachWorkerReview(card, transport)`. The host supplies:

* `resolve(selection)`: resolve verified recording coverage to a staged source ID.
* `request(path, body?)`: authenticated job/library requests.
* `mediaUrl(id, saved)`: authenticated, range-capable playback URL.

The same-origin helper assumes a **real authenticated host proxy**; its custom
header is a cross-origin-request guard, not an authentication or CSRF-token
implementation. The proxy must verify the user's existing HA session and camera
access on every operation, reject untrusted origins, retain worker credentials
server-side, enforce timeouts and forward Range responses. A remote worker can
stay loopback-only behind a managed tunnel. No token belongs in browser storage.

The component separates Prepare preview from Save clip, restores existing jobs
without submitting new work after navigation, preserves a draft name in
session storage, and ignores stale responses after the selection changes.
The opt-in host seam replaces the old compatible-export action while preserving
original-quality export. It is not automatically loaded or deployed.

## Qualification

`cargo test --locked --all-targets` covers request scope, identity, interrupted
state, lock exclusion and cancellation state. The real-media Rust test is
explicitly ignored unless invoked with installed FFmpeg:

```sh
VIDEO_TEST_FFMPEG=/usr/bin/ffmpeg VIDEO_TEST_FFPROBE=/usr/bin/ffprobe \
  cargo test --locked --test video_worker -- --include-ignored
python3 tests/video_worker_http.py
node web/review-worker.test.mjs
```

The Linux CI media job requires real FFmpeg. It checks a non-keyframe trim using
decoded frame comparison and frame count, audio timing, save/reopen hash,
authenticated byte ranges, running-process cancellation/retry and deadline
reaping. Fixtures are generated colors/patterns and a sine tone.
`web/review-worker.browser.cjs` uses an existing Playwright installation to test
the UI with mocked API/media. `REVIEW_CARD_SOURCE` optionally points at the
private current review card to exercise the real integration seam.

On the purchased machine, compare software and QSV on the same synthetic source,
then an authorized isolated camera sample: confirm Intel engine use, HEVC→H.264
decoding/encoding, frame boundaries, audio sync, browser playback, cancellation,
latency, temperatures and saved-share reopening. Measure one job before raising
concurrency. Only after the source adapter and HA authentication path are tested
should the production UI opt in.
