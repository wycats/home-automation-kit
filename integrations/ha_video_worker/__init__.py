"""Authenticated Home Assistant bridge to the private video-worker service."""
from __future__ import annotations

import asyncio
import json
import re
from datetime import timedelta
from urllib.parse import quote

from aiohttp import ClientError, ClientTimeout, hdrs, web
from homeassistant.auth.permissions.const import POLICY_READ
from homeassistant.components.http import KEY_HASS, KEY_HASS_REFRESH_TOKEN_ID
from homeassistant.components.http.auth import async_sign_path
from homeassistant.helpers.http import HomeAssistantView
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from yarl import URL

from .core import (MAX_BODY, WorkerFailure, async_worker_json, authorize_camera, safe_clip, safe_job, validate_camera_map,
                   validate_identifier, validate_selection, validate_worker_url, worker_path)

DOMAIN = "ha_video_worker"
TIMEOUT = ClientTimeout(total=8, connect=2, sock_read=6)
MEDIA_TIMEOUT = ClientTimeout(total=None, connect=2, sock_read=30)
RANGE_RE = re.compile(r"^bytes=(\d+)-(\d*)$")


def _setup_config(config: dict) -> dict:
    raw = config.get(DOMAIN)
    if not isinstance(raw, dict):
        return {}
    base = validate_worker_url(raw.get("url", ""))
    token = raw.get("token")
    if not isinstance(token, str) or len(token) < 32 or not token.isascii() or any(ord(c) < 32 or ord(c) > 126 for c in token):
        raise ValueError("worker token must be at least 32 printable ASCII characters")
    camera_map = validate_camera_map(raw.get("cameras", {}))
    users_raw = raw.get("allowed_users", {})
    if not isinstance(users_raw, dict):
        raise ValueError("allowed_users must map HA user IDs to worker camera IDs")
    users: dict[str, set[str]] = {}
    for user_id, cameras in users_raw.items():
        if not isinstance(user_id, str) or not user_id or not isinstance(cameras, list) or any(c not in camera_map for c in cameras):
            raise ValueError("invalid allowed_users mapping")
        users[user_id] = set(cameras)
    return {"url": base, "token": token, "cameras": camera_map, "allowed_users": users}


async def async_setup(hass, config):
    try:
        settings = _setup_config(config)
    except (ValueError, TypeError):
        return False
    if not settings:
        return True
    hass.data.setdefault(DOMAIN, {})["settings"] = settings
    hass.http.register_view(ControlView())
    hass.http.register_view(MediaView())
    hass.http.register_view(MediaUrlView())
    return True


class Bridge:
    def __init__(self, hass):
        self.hass = hass
        self.settings = hass.data[DOMAIN]["settings"]
        self.session = async_get_clientsession(hass)

    def authorize(self, request, camera: str) -> bool:
        user = request.get("hass_user")
        if user is None or not getattr(user, "is_active", False):
            return False
        return authorize_camera(
            user.id, bool(user.is_admin), camera, self.settings["cameras"],
            self.settings["allowed_users"],
            lambda entity_id: user.permissions.check_entity(entity_id, POLICY_READ),
        )

    async def json(self, method: str, path: str, body: dict | None = None):
        try:
            return await async_worker_json(self.session, self.settings["url"], self.settings["token"], method, path, body, TIMEOUT)
        except (WorkerFailure, ValueError):
            raise web.HTTPBadGateway(text="video worker unavailable") from None

    async def object(self, kind: str, object_id: str):
        validate_identifier(object_id)
        path = f"{kind}/{object_id}"
        value = await self.json("GET", path)
        if not isinstance(value, dict) or value.get("id") != object_id or not isinstance(value.get("request"), dict):
            raise web.HTTPBadGateway(text="invalid worker response")
        camera = value["request"].get("camera")
        safe_camera(camera)
        return value, camera

    def signed_url(self, request, kind: str, object_id: str, camera: str) -> str:
        user = request.get("hass_user")
        path = f"/api/{DOMAIN}/media/{kind}/{object_id}/{quote(camera, safe='')}/{user.id}"
        refresh_token_id = request.get(KEY_HASS_REFRESH_TOKEN_ID)
        if not refresh_token_id:
            raise web.HTTPUnauthorized(text="authenticated Home Assistant session required")
        return async_sign_path(self.hass, path, timedelta(minutes=5), refresh_token_id=refresh_token_id)


def safe_camera(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", value):
        raise web.HTTPBadGateway(text="invalid worker response")


def safe_job_response(value):
    try: return safe_job(value)
    except (TypeError, ValueError, KeyError): raise web.HTTPBadGateway(text="invalid worker response") from None


def safe_clip_response(value):
    try: return safe_clip(value)
    except (TypeError, ValueError, KeyError): raise web.HTTPBadGateway(text="invalid worker response") from None


class NoStoreView(HomeAssistantView):
    @staticmethod
    def json(result, status_code=200, headers=None):
        response_headers = dict(headers or {})
        response_headers["Cache-Control"] = "private, no-store"
        return HomeAssistantView.json(result, status_code, headers=response_headers)


def check_origin(request):
    origin = request.headers.get(hdrs.ORIGIN)
    if origin:
        try:
            parsed_origin = URL(origin).origin()
            request_origin = URL(f"{request.scheme}://{request.host}").origin()
        except Exception:
            raise web.HTTPForbidden(text="cross-origin request rejected") from None
        if parsed_origin != request_origin:
            raise web.HTTPForbidden(text="cross-origin request rejected")
    if request.headers.get("Sec-Fetch-Site", "").lower() in {"cross-site", "cross-origin"}:
        raise web.HTTPForbidden(text="cross-origin request rejected")


async def request_json(request):
    if request.content_length is not None and request.content_length > MAX_BODY:
        raise web.HTTPRequestEntityTooLarge(max_size=MAX_BODY, actual_size=request.content_length)
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="JSON required")
    raw = bytearray()
    try:
        async with asyncio.timeout(2):
            while len(raw) <= MAX_BODY:
                chunk = await request.content.read(min(1024, MAX_BODY + 1 - len(raw)))
                if not chunk: break
                raw.extend(chunk)
    except asyncio.TimeoutError:
        raise web.HTTPRequestTimeout(text="request body timed out") from None
    if len(raw) > MAX_BODY:
        raise web.HTTPRequestEntityTooLarge(max_size=MAX_BODY, actual_size=len(raw))
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeDecodeError):
        raise web.HTTPBadRequest(text="invalid JSON") from None
    if not isinstance(value, dict):
        raise web.HTTPBadRequest(text="JSON object required")
    return value


class ControlView(NoStoreView):
    url = f"/api/{DOMAIN}/control/{{path:.*}}"
    name = "api:ha_video_worker:control"
    requires_auth = True
    cors_allowed = False

    async def get(self, request, path):
        check_origin(request)
        bridge = Bridge(request.app[KEY_HASS])
        try:
            route = worker_path("GET", path)
        except ValueError:
            raise web.HTTPNotFound from None
        if route == "/health":
            value = await bridge.json("GET", route)
            return self.json({"service": "video-worker", "version": value.get("version") if isinstance(value, dict) else None})
        if route == "/jobs":
            jobs = await bridge.json("GET", route)
            if not isinstance(jobs, list): raise web.HTTPBadGateway(text="invalid worker response")
            visible = []
            for job in jobs:
                try:
                    safe = safe_job(job); camera = safe["request"]["camera"]
                    if bridge.authorize(request, camera): visible.append(safe)
                except (KeyError, ValueError, TypeError):
                    continue
            return self.json(visible)
        if route == "/clips":
            clips = await bridge.json("GET", route)
            if not isinstance(clips, list): raise web.HTTPBadGateway(text="invalid worker response")
            visible = []
            for clip in clips:
                try:
                    safe = safe_clip(clip)
                    if bridge.authorize(request, safe["request"]["camera"]): visible.append(safe)
                except (KeyError, ValueError, TypeError):
                    continue
            return self.json(visible)
        kind, object_id = route.strip("/").split("/")
        value, camera = await bridge.object(kind, object_id)
        if not bridge.authorize(request, camera): raise web.HTTPNotFound
        return self.json(safe_job_response(value) if kind == "jobs" else safe_clip_response(value))

    async def post(self, request, path):
        check_origin(request)
        bridge = Bridge(request.app[KEY_HASS])
        try: route = worker_path("POST", path)
        except ValueError: raise web.HTTPNotFound from None
        body = await request_json(request)
        if route in {"/sources/resolve", "/jobs", "/jobs/lookup"}:
            try: selection = validate_selection(body)
            except ValueError: raise web.HTTPBadRequest(text="invalid selection") from None
            if not bridge.authorize(request, selection["camera"]): raise web.HTTPNotFound
            resolved = await bridge.json("POST", "/sources/resolve", selection)
            source = resolved.get("source") if isinstance(resolved, dict) else None
            if not isinstance(source, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", source):
                raise web.HTTPBadGateway(text="no validated recording coverage")
            if route == "/sources/resolve": return self.json({"source": source})
            worker_body = {**selection, "source": source, "profile": "browser-1080p-v1"}
            result = await bridge.json("POST", route, worker_body)
            if result is None and route == "/jobs/lookup": return self.json(None)
            if not isinstance(result, dict) or not isinstance(result.get("request"), dict) or result["request"].get("camera") != selection["camera"]:
                raise web.HTTPBadGateway(text="invalid worker response")
            return self.json(safe_job_response(result))
        parts = route.strip("/").split("/")
        kind, object_id, action = parts
        if kind != "jobs": raise web.HTTPNotFound
        value, camera = await bridge.object("jobs", object_id)
        if not bridge.authorize(request, camera): raise web.HTTPNotFound
        if action == "save":
            if set(body) != {"name"} or not isinstance(body["name"], str): raise web.HTTPBadRequest(text="invalid save request")
        elif body:
            raise web.HTTPBadRequest(text="request body must be empty")
        result = await bridge.json("POST", route, body if action == "save" else None)
        if action == "cancel":
            if not isinstance(result, dict) or not isinstance(result.get("job"), dict) or not isinstance(result["job"].get("request"), dict) or result["job"]["request"].get("camera") != camera:
                raise web.HTTPBadGateway(text="invalid worker response")
            return self.json({"job": safe_job_response(result["job"]), "cancel_requested": bool(result.get("cancel_requested"))})
        if not isinstance(result, dict) or not isinstance(result.get("request"), dict) or result["request"].get("camera") != camera:
            raise web.HTTPBadGateway(text="invalid worker response")
        return self.json(safe_clip_response(result) if action == "save" else safe_job_response(result))


class MediaView(NoStoreView):
    url = f"/api/{DOMAIN}/media/{{kind}}/{{object_id}}/{{camera}}/{{user_id}}"
    name = "api:ha_video_worker:media"
    requires_auth = True  # HA's signed-request middleware restores the issuing user.
    cors_allowed = False

    async def get(self, request, kind, object_id, camera, user_id):
        check_origin(request)
        if kind not in {"jobs", "clips"}: raise web.HTTPNotFound
        try: validate_identifier(object_id)
        except ValueError: raise web.HTTPNotFound from None
        bridge = Bridge(request.app[KEY_HASS])
        user = request.get("hass_user")
        if user is None or user.id != user_id or not bridge.authorize(request, camera): raise web.HTTPNotFound
        _, actual_camera = await bridge.object(kind, object_id)
        if actual_camera != camera or not bridge.authorize(request, actual_camera): raise web.HTTPNotFound
        range_value = request.headers.get(hdrs.RANGE)
        if range_value and not RANGE_RE.fullmatch(range_value): raise web.HTTPBadRequest(text="unsupported byte range")
        upstream_path = f"/{kind}/{object_id}/media"
        headers = {hdrs.AUTHORIZATION: "Bearer " + bridge.settings["token"]}
        if range_value: headers[hdrs.RANGE] = range_value
        try:
            async with bridge.session.get(bridge.settings["url"] + upstream_path, headers=headers,
                    timeout=MEDIA_TIMEOUT, allow_redirects=False) as upstream:
                if upstream.status == 416:
                    content_range = upstream.headers.get(hdrs.CONTENT_RANGE)
                    headers = {"Cache-Control": "private, no-store"}
                    if content_range: headers[hdrs.CONTENT_RANGE] = content_range
                    return web.Response(status=416, headers=headers)
                if upstream.status not in {200, 206} or upstream.headers.get(hdrs.CONTENT_TYPE, "").split(";")[0] != "video/mp4":
                    raise web.HTTPBadGateway(text="video worker unavailable")
                response = web.StreamResponse(status=upstream.status, headers={"Content-Type": "video/mp4", "Cache-Control": "private, no-store", "Accept-Ranges": "bytes", "X-Content-Type-Options": "nosniff"})
                for header in (hdrs.CONTENT_LENGTH, hdrs.CONTENT_RANGE):
                    if header in upstream.headers: response.headers[header] = upstream.headers[header]
                await response.prepare(request)
                async for chunk in upstream.content.iter_chunked(64 * 1024): await response.write(chunk)
                await response.write_eof()
                return response
        except (ClientError, asyncio.TimeoutError):
            raise web.HTTPBadGateway(text="video worker unavailable") from None


class MediaUrlView(NoStoreView):
    url = f"/api/{DOMAIN}/media-url"
    name = "api:ha_video_worker:media_url"
    requires_auth = True
    cors_allowed = False

    async def post(self, request):
        check_origin(request)
        body = await request_json(request)
        if set(body) != {"kind", "id"} or not isinstance(body.get("kind"), str) or body["kind"] not in {"jobs", "clips"} or not isinstance(body.get("id"), str):
            raise web.HTTPBadRequest(text="invalid media request")
        bridge = Bridge(request.app[KEY_HASS])
        try: validate_identifier(body["id"])
        except (TypeError, ValueError): raise web.HTTPBadRequest(text="invalid media request") from None
        value, camera = await bridge.object(body["kind"], body["id"])
        if not bridge.authorize(request, camera): raise web.HTTPNotFound
        if body["kind"] == "jobs" and value.get("state") != "ready": raise web.HTTPConflict(text="preview is not ready")
        signed = bridge.signed_url(request, body["kind"], body["id"], camera)
        return self.json({"url": signed})
