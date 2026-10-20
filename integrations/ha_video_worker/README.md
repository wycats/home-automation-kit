# Authenticated Home Assistant video-worker bridge

This custom integration is an isolated adapter for `video-worker`. It is not
enabled or deployed by this repository. Configure the worker URL as HTTPS, or
HTTP to a literal loopback IP when a managed SSH tunnel terminates there. The
worker bearer token stays in server-side Home Assistant YAML/`!secret` config.

```yaml
ha_video_worker:
  url: http://127.0.0.1:8787
  token: !secret video_worker_token
  cameras:
    room_a: camera.room_a
  # Omit this mapping to allow administrators only.
  allowed_users:
    "<Home Assistant user id>": [room_a]
```

Every view requires Home Assistant authentication. Administrators may use any
configured camera mapping; other users need an explicit user-to-camera mapping.
Every operation also checks that user's Home Assistant entity `read` permission
for the mapped camera. Job and saved-clip lists are filtered, and object
operations re-fetch the object and check its camera on every request. Browser
control requests must be same-origin and use the Home Assistant frontend API;
cross-origin CORS is disabled.

The review component transport should use authenticated `hass.callApi` for
control, for example `hass.callApi("POST", "ha_video_worker/control/jobs", …)` with only
`camera`, `start_ms`, and `end_ms`. The bridge resolves that selection against
the worker's already-staged catalog, then supplies the fixed profile and source
ID itself. Save, cancel, retry, status, and library paths are under
`/api/ha_video_worker/control/` and accept only the worker's documented routes.
There is no browser-side worker token or caller-identity header.

An HTML video element cannot attach the usual HA bearer header. First call
`hass.callApi("POST", "ha_video_worker/media-url", { kind, id })` with
`kind: "jobs"` or `kind: "clips"`, then set the element source to the
returned relative URL. Its five-minute lifetime supports longer clips and
seeking; a copied URL is a bearer capability during that period. The bridge
checks the current user's camera access and object ownership before minting it.
It uses HA's `async_sign_path` for a five-minute URL bound to the object ID,
camera and issuing user. HA's HTTP auth
middleware validates the signature, expiration, and issuing refresh token and
restores that HA user before the `requires_auth = True` media view runs. Media
requests re-check user, camera, and object ownership, allow only one explicit
byte range, and stream chunks without buffering the file.

Control responses expose only reviewed fields; private source paths and raw
worker errors are omitted. Worker requests use fixed route allowlists, an
8-second total timeout for control calls, a 2-second connect timeout, no redirects, a 4 KiB request
body cap, and a 1 MiB JSON response cap. This does not establish deployed HA
behavior: run the separate synthetic policy tests here, then qualify the actual
integration in an isolated HA instance before production adoption.

## Home Assistant source reviewed

The auth behavior was checked against Home Assistant Core's `dev` sources on
2026-10-19; these branch links move, so repeat the review against the exact Core
version before deployment:

- [`homeassistant/components/http/auth.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/components/http/auth.py):
  `async_sign_path` binds path, query parameters, expiry and issuing refresh
  token; signed-request middleware verifies those claims, looks up the refresh
  token, restores `hass_user`, and marks the request authenticated.
- [`homeassistant/helpers/http.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/helpers/http.py):
  `request_handler_factory` permits valid signed requests through views with
  `requires_auth = True`; the default view does not enable CORS for every origin.
- [`homeassistant/auth/permissions/const.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/auth/permissions/const.py)
  defines the `read` entity policy, and
  [`homeassistant/auth/models.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/auth/models.py)
  exposes `user.permissions.check_entity` used for the per-camera check.
- The frontend's [`HomeAssistantApi` type](https://github.com/home-assistant/frontend/blob/dev/src/types.ts)
  defines `hass.callApi(method, path, parameters, headers)`; callers pass the
  API path without the `/api/` prefix. The signed media URL itself is returned
  as a relative URL including `/api/` for the browser's video element.
