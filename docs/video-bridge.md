# HA review bridge and local source publisher

This is an isolated integration candidate. Nothing in this repository installs
the component, opens a firewall, changes recorder retention, or activates QSV.
The existing recorder and original-quality export remain independent.

## Authentication and playback

`integrations/ha_video_worker` is a Home Assistant custom component. Its
configuration maps worker camera IDs to HA camera entities. Administrators still
need entity read permission; other users also need an explicit camera grant.
The browser uses `homeAssistantTransport(hass)` and HA's `callApi` authentication.
It never receives the worker token, recording paths, or a general worker proxy.

An isolated host can opt in once it has the current HA `hass` object:

```js
import {attachWorkerReview, homeAssistantTransport} from "./review-worker.mjs";
attachWorkerReview(reviewCard, homeAssistantTransport(hass));
```

Attach once per card instance, before its next selection update. This is an
explicit host hook; the module does not patch an installed dashboard by itself.

Control routes resolve selections server-side. Object operations and lists check
camera authorization. Playback uses HA's expiring signed paths, bound to the
issuing user and one job/clip; the media handler checks current permissions again.
The signed URL is a temporary bearer capability: sharing it temporarily shares
that clip. Reload playback obtains a fresh URL after expiry or interruption.
See the component README for the exact configuration and security boundaries.

The worker continues to listen on loopback. Commissioning must supply either a
managed authenticated tunnel ending on HA's loopback, or a TLS endpoint with a
private server-side bearer token. Do not expose port 8787 directly on the LAN.

## Publishing a source without restarting the worker

Build `cargo build --locked --release --bin video-stage --bin video-worker`.
The trusted **local** publisher accepts a private JSON input:

```json
{
  "approved_root": "/mnt/recordings",
  "worker_root": "/var/lib/video-worker/cache",
  "segments": [{
    "id": "closed-segment-001",
    "camera": "room",
    "path": "/mnt/recordings/closed-segment.mp4",
    "closed": true,
    "media_origin_ms": 1792440000000,
    "media_duration_ms": 10000
  }],
  "selection": {
    "camera": "room",
    "start_ms": 1792440001000,
    "end_ms": 1792440005000
  }
}
```

Run `video-stage /private/input.json`. The publisher locks concurrent publishers,
copies and hashes the source, rechecks source integrity, and publishes immutable
media before `source.json`. The running worker discovers the manifest under
`cache/sources/<id>/` during resolution. No HTTP route can publish a source.
Use globally unique IDs, a private local filesystem supporting hard links, and
an unprivileged service account with umask 0077. NAS recordings are read-only;
staged copies are local. Never modify a published source. Failed partial staging
is unavailable to the worker and requires offline reconciliation.

This currently accepts **one closed, inspected source covering the selection**.
It rejects ambiguous overlap, gaps, open files, path escapes, selections outside
1–120 seconds, sources over ten minutes or 512 MiB, and catalog/path drift.
It does not infer wall time from filenames or equate Frigate's API interval with
the first decoded frame. The trusted catalog must establish the UTC origin
relative to the media timeline FFmpeg uses.

Automatic Frigate segment discovery, proving closure and timestamp alignment,
and assembling selections across segments are still required. In particular, a
30-second selection spanning three recorder files is not supported by pretending
the files form one continuous clip. No claim of end-to-end live recorder staging
follows from the synthetic publisher tests.

## Commissioning and acceptance

Before enabling the review transport on the real dashboard:

1. Run the HA integration tests against the installed HA version and verify two
   real users with different camera grants, signed-path expiry/revocation,
   byte-range seeking, and loss of worker connectivity.
2. Mount NAS originals read-only and a separate durable saved-clips directory.
   Configure retention for the cache separately; saved clips must survive cache
   removal. Verify a save can be reopened after worker restart.
3. Run the worker under cgroup supervision (`KillMode=control-group`), private
   files, resource limits, and a pinned FFmpeg installation. Kill/restart the
   service during conversion and verify no media child survives.
4. On the purchased Intel machine, qualify QSV decode, scale and H.264 encode
   with synthetic H.264 and HEVC, then authorized camera samples. Measure
   throughput, CPU, audio alignment, and browser playback. A software CI pass
   does not establish hardware performance.
5. Keep production opt-in until recorder staging proves exact selection timing
   and gaps. Preserve original recordings and the current export path.
