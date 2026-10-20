"""Policy and bounded HTTP helpers for the HA video-worker bridge."""
from __future__ import annotations

import ipaddress
import asyncio
import json
import re
from urllib.parse import urlsplit

from aiohttp import ClientError, hdrs

MAX_BODY = 4096
MAX_SELECTION_MS = 120_000
WORKER_ROUTES = {
    ("GET", ("health",)), ("GET", ("jobs",)),
    ("POST", ("sources", "resolve")), ("POST", ("jobs",)),
    ("POST", ("jobs", "lookup")), ("GET", ("jobs", "{id}")),
    ("POST", ("jobs", "{id}", "cancel")), ("POST", ("jobs", "{id}", "retry")),
    ("POST", ("jobs", "{id}", "save")), ("GET", ("clips",)),
    ("GET", ("clips", "{id}")),
}
ID_RE = re.compile(r"^[A-Fa-f0-9]{64}$")
CAMERA_RE = re.compile(r"^[A-Za-z0-9_-]{1,80}$")


class WorkerFailure(Exception):
    """Safe outward error; upstream status/body/transport details are omitted."""


async def async_worker_json(session, base: str, token: str, method: str, path: str, body, timeout):
    route = worker_path(method, path)
    headers = {hdrs.AUTHORIZATION: "Bearer " + token, "Accept": "application/json"}
    data = json.dumps(body, separators=(",", ":")).encode() if body is not None else None
    if data is not None:
        headers[hdrs.CONTENT_TYPE] = "application/json"
    try:
        async with session.request(method, base + route, headers=headers, data=data,
                                   timeout=timeout, allow_redirects=False) as response:
            if response.status < 200 or response.status >= 300 or (response.content_length is not None and response.content_length > 1_048_576):
                raise WorkerFailure
            raw = bytearray()
            while len(raw) <= 1_048_576:
                chunk = await response.content.read(min(65536, 1_048_577 - len(raw)))
                if not chunk:
                    break
                raw.extend(chunk)
            if len(raw) > 1_048_576:
                raise WorkerFailure
            return json.loads(raw)
    except (ClientError, asyncio.TimeoutError, ValueError, WorkerFailure):
        raise WorkerFailure from None


def validate_worker_url(value: str) -> str:
    """Allow HTTPS endpoints or explicit loopback HTTP (e.g. SSH tunnel)."""
    parsed = urlsplit(value)
    if parsed.scheme not in {"https", "http"} or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("worker URL must be HTTPS or loopback HTTP")
    if parsed.scheme == "http":
        try:
            if not ipaddress.ip_address(parsed.hostname).is_loopback:
                raise ValueError("plain HTTP is restricted to loopback")
        except ValueError as exc:
            if "restricted" in str(exc):
                raise
            raise ValueError("plain HTTP is restricted to loopback") from None
    return value.rstrip("/")


def validate_camera_map(camera_map: dict[str, str]) -> dict[str, str]:
    if not isinstance(camera_map, dict) or not camera_map:
        raise ValueError("camera mapping is required")
    for camera, entity in camera_map.items():
        if not isinstance(camera, str) or not CAMERA_RE.fullmatch(camera):
            raise ValueError("invalid worker camera id")
        if not isinstance(entity, str) or not re.fullmatch(r"camera\.[a-z0-9_]+", entity):
            raise ValueError("invalid Home Assistant camera entity")
    return dict(camera_map)


def authorize_camera(user_id: str, is_admin: bool, camera: str, camera_map: dict[str, str], allowed_users: dict[str, set[str]], can_read) -> bool:
    """Admins are default-allowed; others need explicit user/camera mapping.

    HA entity read permission is checked for every caller, including admins.
    """
    entity = camera_map.get(camera)
    if entity is None or not can_read(entity):
        return False
    return bool(is_admin or camera in allowed_users.get(user_id, set()))


def validate_selection(body: object) -> dict:
    if not isinstance(body, dict) or set(body) != {"camera", "start_ms", "end_ms"}:
        raise ValueError("invalid selection")
    camera, start, end = body["camera"], body["start_ms"], body["end_ms"]
    if not isinstance(camera, str) or not CAMERA_RE.fullmatch(camera):
        raise ValueError("invalid selection")
    if type(start) is not int or type(end) is not int or start < 0 or end > (2**64 - 1) or end - start < 1000 or end <= start or end - start > MAX_SELECTION_MS:
        raise ValueError("invalid selection")
    return {"camera": camera, "start_ms": start, "end_ms": end}


def validate_identifier(value: str) -> str:
    if not isinstance(value, str) or not ID_RE.fullmatch(value):
        raise ValueError("invalid object identity")
    return value


def worker_path(method: str, path: str) -> str:
    """Map the public operation vocabulary to a fixed worker route allowlist."""
    parts = tuple(part for part in path.split("/") if part)
    if any(part in {".", ".."} for part in parts) or not parts:
        raise ValueError("unknown operation")
    for route_method, pattern in WORKER_ROUTES:
        if route_method != method or len(pattern) != len(parts):
            continue
        if all(want == got or (want == "{id}" and ID_RE.fullmatch(got)) for want, got in zip(pattern, parts)):
            return "/" + "/".join(parts)
    raise ValueError("unknown operation")


def safe_job(job: object) -> dict:
    if not isinstance(job, dict) or not isinstance(job.get("request"), dict) or not isinstance(job.get("id"), str) or not ID_RE.fullmatch(job["id"]):
        raise ValueError("invalid worker response")
    request = job["request"]
    if not isinstance(request.get("camera"), str) or not isinstance(request.get("source"), str):
        raise ValueError("invalid worker response")
    if not CAMERA_RE.fullmatch(request["camera"]) or type(request.get("start_ms")) is not int or type(request.get("end_ms")) is not int or request["end_ms"] <= request["start_ms"]:
        raise ValueError("invalid worker response")
    if job.get("state") not in {"queued", "running", "ready", "cancelled", "failed"}:
        raise ValueError("invalid worker response")
    result = {key: job[key] for key in ("id", "state", "backend", "created_ms") if key in job}
    result["request"] = {key: request[key] for key in ("camera", "start_ms", "end_ms", "profile") if key in request}
    if isinstance(job.get("media"), dict):
        media = job["media"]
        result["media"] = {key: media[key] for key in ("width", "height", "duration_ms", "audio", "sha256") if key in media}
    if job.get("state") == "failed":
        result["error"] = "video preparation failed"
    return result


def safe_clip(clip: object) -> dict:
    if not isinstance(clip, dict) or not isinstance(clip.get("request"), dict) or not isinstance(clip.get("id"), str) or not ID_RE.fullmatch(clip["id"]) or not isinstance(clip.get("name"), str):
        raise ValueError("invalid worker response")
    if not isinstance(clip["request"].get("camera"), str) or not CAMERA_RE.fullmatch(clip["request"]["camera"]):
        raise ValueError("invalid worker response")
    if type(clip["request"].get("start_ms")) is not int or type(clip["request"].get("end_ms")) is not int or clip["request"]["end_ms"] <= clip["request"]["start_ms"]:
        raise ValueError("invalid worker response")
    result = {key: clip[key] for key in ("id", "name") if key in clip}
    result["request"] = {key: clip["request"][key] for key in ("camera", "start_ms", "end_ms", "profile") if key in clip["request"]}
    if isinstance(clip.get("media"), dict):
        result["media"] = {key: clip["media"][key] for key in ("width", "height", "duration_ms", "audio", "sha256") if key in clip["media"]}
    return result
