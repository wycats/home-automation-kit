"""Synthetic tests for bridge policy; no Home Assistant or worker required."""
import sys
import importlib.util
import types
import unittest
from pathlib import Path
import asyncio

from aiohttp import ClientSession, web
from aiohttp.test_utils import TestServer

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "integrations" / "ha_video_worker"))
import core


def load_component_with_home_assistant_stubs():
    """Load handlers with HA import stubs; route/auth behavior gets HTTP tests."""
    def module(name):
        item = types.ModuleType(name)
        sys.modules[name] = item
        return item
    for name in ("homeassistant", "homeassistant.auth", "homeassistant.auth.permissions",
                 "homeassistant.components", "homeassistant.components.http", "homeassistant.helpers",
                 "homeassistant.helpers.aiohttp_client"):
        module(name)
    permissions = module("homeassistant.auth.permissions.const")
    permissions.POLICY_READ = "read"
    http = sys.modules["homeassistant.components.http"]
    http.KEY_HASS = "hass"
    http.KEY_HASS_REFRESH_TOKEN_ID = "refresh_token_id"
    auth = module("homeassistant.components.http.auth")
    async_sign_calls = []
    def sign_path(hass, path, expiration, *, refresh_token_id=None):
        async_sign_calls.append((path, expiration.total_seconds(), refresh_token_id))
        return path + "?authSig=synthetic-signature"
    auth.async_sign_path = sign_path
    helpers_http = module("homeassistant.helpers.http")
    class HomeAssistantView:
        @staticmethod
        def json(value, status_code=200, headers=None):
            return web.json_response(value, status=status_code, headers=headers)
    helpers_http.HomeAssistantView = HomeAssistantView
    aiohttp_client = sys.modules["homeassistant.helpers.aiohttp_client"]
    aiohttp_client.async_get_clientsession = lambda hass: hass.session
    package_path = Path(__file__).resolve().parents[1] / "integrations" / "ha_video_worker"
    spec = importlib.util.spec_from_file_location("ha_video_worker_test", package_path / "__init__.py",
                                                   submodule_search_locations=[str(package_path)])
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = loaded
    spec.loader.exec_module(loaded)
    return loaded, async_sign_calls


class BridgePolicyTests(unittest.TestCase):
    def test_worker_url_requires_https_or_ip_loopback_http(self):
        self.assertEqual(core.validate_worker_url("https://worker.example:8787/base/"), "https://worker.example:8787/base")
        self.assertEqual(core.validate_worker_url("http://127.0.0.1:8787"), "http://127.0.0.1:8787")
        self.assertEqual(core.validate_worker_url("http://[::1]:8787"), "http://[::1]:8787")
        for value in ("http://worker.example", "https://user:pass@worker.example", "https://worker.example/?x=1", "file:///etc/passwd"):
            with self.subTest(value=value), self.assertRaises(ValueError): core.validate_worker_url(value)

    def test_component_config_defaults_to_admins_and_rejects_unknown_camera_grants(self):
        component, _ = load_component_with_home_assistant_stubs()
        config = {"ha_video_worker": {"url": "https://worker.example", "token": "t" * 40,
                  "cameras": {"room_a": "camera.room_a"}}}
        self.assertEqual(component._setup_config(config)["allowed_users"], {})
        config["ha_video_worker"]["allowed_users"] = {"user-id": ["other_camera"]}
        with self.assertRaises(ValueError): component._setup_config(config)

    def test_non_admin_requires_explicit_user_camera_and_ha_read_permission(self):
        cameras = {"room_a": "camera.room_a", "room_b": "camera.room_b"}
        users = {"viewer-id": {"room_a"}}
        read = lambda entity: entity == "camera.room_a"
        self.assertTrue(core.authorize_camera("viewer-id", False, "room_a", cameras, users, read))
        self.assertFalse(core.authorize_camera("other-user", False, "room_a", cameras, users, read))
        self.assertFalse(core.authorize_camera("viewer-id", False, "room_b", cameras, users, read))
        self.assertFalse(core.authorize_camera("viewer-id", False, "room_a", cameras, users, lambda _: False))
        self.assertTrue(core.authorize_camera("admin-id", True, "room_a", cameras, {}, read))
        self.assertFalse(core.authorize_camera("admin-id", True, "room_a", cameras, {}, lambda _: False))

    def test_selection_is_exactly_shaped_bounded_and_integer_typed(self):
        self.assertEqual(core.validate_selection({"camera": "room_a", "start_ms": 100, "end_ms": 1200}),
                         {"camera": "room_a", "start_ms": 100, "end_ms": 1200})
        for body in (
            {"camera": "room_a", "start_ms": True, "end_ms": 1200},
            {"camera": "room_a", "start_ms": 0, "end_ms": 999},
            {"camera": "room_a", "start_ms": 0, "end_ms": 120001},
            {"camera": "../secret", "start_ms": 0, "end_ms": 1000},
            {"camera": "room_a", "start_ms": 0, "end_ms": 1000, "path": "/etc/passwd"},
        ):
            with self.subTest(body=body), self.assertRaises(ValueError): core.validate_selection(body)

    def test_worker_routes_are_fixed_and_ids_cannot_traverse(self):
        ident = "a" * 64
        self.assertEqual(core.worker_path("GET", f"jobs/{ident}"), f"/jobs/{ident}")
        self.assertEqual(core.worker_path("POST", f"jobs/{ident}/cancel"), f"/jobs/{ident}/cancel")
        for method, path in (("GET", "../config"), ("GET", "jobs/not-an-id"), ("POST", "health"), ("POST", "jobs/x/save/extra")):
            with self.subTest(method=method, path=path), self.assertRaises(ValueError): core.worker_path(method, path)
        with self.assertRaises(ValueError): core.worker_path("GET", f"jobs/{'a' * 64}/media")
        with self.assertRaises(ValueError): core.validate_identifier(None)

    def test_responses_drop_worker_paths_and_raw_errors(self):
        job = {"id": "a" * 64, "state": "failed", "backend": "software", "error": "token=secret path=/private/x",
               "request": {"source": "private-source", "camera": "room_a", "start_ms": 1, "end_ms": 1001,
                           "profile": "browser-1080p-v1", "path": "/private/source"}, "extra": "discard"}
        safe = core.safe_job(job)
        # The bridge uses a fixed generic error instead of worker diagnostics.
        job["state"] = "failed"
        safe = core.safe_job(job)
        self.assertEqual(safe["error"], "video preparation failed")
        self.assertNotIn("secret", repr(safe))
        self.assertNotIn("source", safe["request"])
        self.assertNotIn("path", safe["request"])
        self.assertNotIn("extra", safe)
        clip = core.safe_clip({"id": "b" * 64, "name": "Moment", "request": {"camera": "room_a", "start_ms": 0, "end_ms": 1000, "source": "private"}})
        self.assertNotIn("source", clip["request"])


class WorkerHttpBoundaryTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        app = web.Application()
        self.mode = "ok"
        async def ok(request):
            assert request.headers.get("Authorization") == "Bearer " + "t" * 40
            if self.mode == "redirect": raise web.HTTPFound("/health")
            if self.mode == "large": return web.json_response({"payload": "x" * 1_048_600})
            if self.mode == "slow": await asyncio.sleep(0.1)
            return web.json_response({"service": "synthetic", "version": 1})
        app.router.add_get("/health", ok)
        self.server = TestServer(app)
        await self.server.start_server()
        self.session = ClientSession()
        self.base = str(self.server.make_url("")).rstrip("/")

    async def asyncTearDown(self):
        await self.session.close()
        await self.server.close()

    async def test_synthetic_http_auth_and_fixed_route(self):
        result = await core.async_worker_json(self.session, self.base, "t" * 40, "GET", "health", None, 1)
        self.assertEqual(result, {"service": "synthetic", "version": 1})
        with self.assertRaises(ValueError):
            await core.async_worker_json(self.session, self.base, "t" * 40, "GET", "../secrets", None, 1)

    async def test_redirect_oversize_and_timeout_fail_closed_without_details(self):
        for mode, timeout in (("redirect", 1), ("large", 1), ("slow", 0.01)):
            self.mode = mode
            with self.subTest(mode=mode), self.assertRaises(core.WorkerFailure) as caught:
                await core.async_worker_json(self.session, self.base, "t" * 40, "GET", "health", None, timeout)
            self.assertEqual(str(caught.exception), "")


class ComponentHandlerHttpTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.component, self.sign_calls = load_component_with_home_assistant_stubs()
        self.job_id = "a" * 64
        self.request_bodies = []
        worker = web.Application()
        self.worker_job = {"id": self.job_id, "state": "ready", "backend": "software", "created_ms": 1,
            "request": {"source": "private-source", "camera": "room_a", "start_ms": 1000, "end_ms": 3000, "profile": "browser-1080p-v1"},
            "media": {"width": 1280, "height": 720, "duration_ms": 2000, "audio": False, "sha256": "b" * 64}}
        async def resolve(request): return web.json_response({"source": "private-source"})
        async def create_job(request):
            self.request_bodies.append(await request.json())
            return web.json_response(self.worker_job)
        async def get_job(request): return web.json_response(self.worker_job)
        async def cancel(request): return web.json_response({"job": self.worker_job, "cancel_requested": True})
        self.worker_ranges = []
        async def media(request):
            value = request.headers.get("Range")
            self.worker_ranges.append(value)
            headers = {"Accept-Ranges": "bytes"}
            if value == "bytes=0-3":
                headers["Content-Range"] = "bytes 0-3/15"
                return web.Response(body=b"synt", status=206, content_type="video/mp4", headers=headers)
            return web.Response(body=b"synthetic-video", content_type="video/mp4", headers=headers)
        worker.router.add_post("/sources/resolve", resolve)
        worker.router.add_post("/jobs", create_job)
        worker.router.add_get("/jobs/{id}", get_job)
        worker.router.add_post("/jobs/{id}/cancel", cancel)
        worker.router.add_get("/jobs/{id}/media", media)
        self.worker_server = TestServer(worker)
        await self.worker_server.start_server()
        self.worker_session = ClientSession()
        # Middleware closes over user factory and stands in for HA only in this test.
        @web.middleware
        async def auth_middleware(request, handler):
            user_id = request.headers.get("X-Test-User")
            if user_id:
                request["hass_user"] = self.make_user(user_id)
                request["refresh_token_id"] = "refresh-" + user_id
            return await handler(request)
        app = web.Application(middlewares=[auth_middleware])
        class FakeHass:
            pass
        hass = FakeHass()
        hass.session = self.worker_session
        hass.data = {"ha_video_worker": {"settings": {
            "url": str(self.worker_server.make_url("")).rstrip("/"), "token": "t" * 40,
            "cameras": {"room_a": "camera.room_a", "room_b": "camera.room_b"},
            "allowed_users": {"viewer": {"room_a", "room_b"}},
        }}}
        app["hass"] = hass
        control = self.component.ControlView()
        media_view = self.component.MediaView()
        media_url = self.component.MediaUrlView()
        async def control_get(request): return await control.get(request, path=request.match_info["path"])
        async def control_post(request): return await control.post(request, path=request.match_info["path"])
        async def media_get(request):
            return await media_view.get(request, **{key: request.match_info[key] for key in ("kind", "object_id", "camera", "user_id")})
        app.router.add_route("GET", "/api/ha_video_worker/control/{path:.*}", control_get)
        app.router.add_route("POST", "/api/ha_video_worker/control/{path:.*}", control_post)
        app.router.add_post("/api/ha_video_worker/media-url", media_url.post)
        app.router.add_get("/api/ha_video_worker/media/{kind}/{object_id}/{camera}/{user_id}", media_get)
        self.ha_server = TestServer(app)
        await self.ha_server.start_server()
        self.client = ClientSession()

    def make_user(self, user_id):
        class Permissions:
            def check_entity(self, entity_id, key): return key == "read" and entity_id.startswith("camera.")
        return types.SimpleNamespace(id=user_id, is_active=True, is_admin=False, permissions=Permissions())

    async def asyncTearDown(self):
        await self.client.close()
        await self.ha_server.close()
        await self.worker_session.close()
        await self.worker_server.close()

    async def test_authenticated_job_contract_filters_paths_and_cancel_envelope(self):
        headers = {"X-Test-User": "viewer"}
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/control/jobs"), headers=headers,
                                          json={"camera": "room_a", "start_ms": 1000, "end_ms": 3000})
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers.get("Cache-Control"), "private, no-store")
        ready = await response.json()
        self.assertNotIn("source", ready["request"])
        self.assertEqual(self.request_bodies[0], {"camera": "room_a", "start_ms": 1000, "end_ms": 3000,
            "source": "private-source", "profile": "browser-1080p-v1"})
        response = await self.client.post(self.ha_server.make_url(f"/api/ha_video_worker/control/jobs/{self.job_id}/cancel"),
                                          headers=headers, json={})
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers.get("Cache-Control"), "private, no-store")
        result = await response.json()
        self.assertTrue(result["cancel_requested"])
        self.assertEqual(result["job"]["id"], self.job_id)
        self.assertNotIn("source", result["job"]["request"])

    async def test_media_url_binds_user_and_camera_and_uses_five_minute_ha_signature(self):
        headers = {"X-Test-User": "viewer"}
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/media-url"), headers=headers,
                                          json={"kind": "jobs", "id": self.job_id})
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers.get("Cache-Control"), "private, no-store")
        signed = await response.json()
        self.assertIn(f"/media/jobs/{self.job_id}/room_a/viewer?authSig=", signed["url"])
        self.assertEqual(self.sign_calls[-1][1:], (300.0, "refresh-viewer"))
        response = await self.client.get(self.ha_server.make_url(f"/api/ha_video_worker/media/jobs/{self.job_id}/room_b/viewer?authSig=synthetic-signature"), headers=headers)
        self.assertEqual(response.status, 404)
        response = await self.client.get(self.ha_server.make_url(f"/api/ha_video_worker/media/jobs/{self.job_id}/room_a/forged?authSig=synthetic-signature"), headers=headers)
        self.assertEqual(response.status, 404)
        response = await self.client.get(self.ha_server.make_url(f"/api/ha_video_worker/media/jobs/{self.job_id}/room_a/viewer?authSig=synthetic-signature"), headers={**headers, "Range": "bytes=0-3"})
        self.assertEqual(response.status, 206)
        self.assertEqual(await response.read(), b"synt")
        self.assertEqual(self.worker_ranges[-1], "bytes=0-3")
        response = await self.client.get(self.ha_server.make_url(f"/api/ha_video_worker/media/jobs/{self.job_id}/room_a/viewer?authSig=synthetic-signature"), headers={**headers, "Range": "bytes=0-1,4-5"})
        self.assertEqual(response.status, 400)
        self.assertIsNone(self.component.MEDIA_TIMEOUT.total)

    async def test_denied_user_and_oversized_body(self):
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/media-url"), headers={"X-Test-User": "other-user"},
                                          json={"kind": "jobs", "id": self.job_id})
        self.assertEqual(response.status, 404)
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/media-url"), headers={"X-Test-User": "viewer"},
                                          json={"kind": [], "id": self.job_id})
        self.assertEqual(response.status, 400)
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/media-url"), headers={"X-Test-User": "viewer"},
                                          json={"kind": "jobs", "id": None})
        self.assertEqual(response.status, 400)
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/control/sources/resolve"),
                                          headers={"X-Test-User": "viewer", "Content-Type": "application/json"}, data=b"{" + b" " * 5000 + b"}")
        self.assertEqual(response.status, 413)

    async def test_cross_origin_control_and_worker_camera_spoof_fail_closed(self):
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/control/jobs"),
            headers={"X-Test-User": "viewer", "Origin": "https://attacker.example"},
            json={"camera": "room_a", "start_ms": 1000, "end_ms": 3000})
        self.assertEqual(response.status, 403)
        self.worker_job["request"]["camera"] = "room_b"
        response = await self.client.post(self.ha_server.make_url("/api/ha_video_worker/control/jobs"), headers={"X-Test-User": "viewer"},
            json={"camera": "room_a", "start_ms": 1000, "end_ms": 3000})
        self.assertEqual(response.status, 502)


if __name__ == "__main__":
    unittest.main()
