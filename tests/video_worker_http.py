"""Synthetic-only Linux worker qualification; never reads configured home media."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    binary = Path("target/debug/video-worker").resolve()
    ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")
    assert ffmpeg and ffprobe, "real FFmpeg is required, not a skipped test"
    with tempfile.TemporaryDirectory(prefix="clip-http-") as tmp:
        root = Path(tmp)
        source = root / "source.mp4"
        subprocess.run([ffmpeg, "-nostdin", "-v", "error", "-f", "lavfi", "-i",
                        "testsrc2=size=320x180:rate=30", "-f", "lavfi", "-i",
                        "sine=frequency=440:sample_rate=48000", "-t", "8",
                        "-c:v", "libx264", "-g", "90", "-c:a", "aac", str(source)], check=True)
        original = source.read_bytes()
        slow, marker = root / "slow", root / "owned-child"
        shim = root / "ffmpeg-test-shim"
        shim.write_text("#!/usr/bin/env python3\nimport os,sys,time\nfrom pathlib import Path\n"
            f"if Path({str(slow)!r}).exists():\n"
            f" Path({str(marker)!r}).write_text(str(os.getpid()))\n time.sleep(60)\n"
            f"os.execv({ffmpeg!r},[{ffmpeg!r},*sys.argv[1:]])\n")
        shim.chmod(0o700)
        config = dict(root=str(root / "cache"), saved_root=str(root / "saved"),
                      ffmpeg=str(shim), ffprobe=ffprobe, backend="software", deadline_seconds=15,
                      sources=[dict(id="synthetic", camera="fixture", start_ms=10000, end_ms=18000,
                                    path=str(source), sha256=hashlib.sha256(source.read_bytes()).hexdigest())])
        config_path = root / "config.json"
        config_path.write_text(json.dumps(config))
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        token = "synthetic-test-token-not-a-secret-000000"
        base = f"http://127.0.0.1:{port}"
        proc = subprocess.Popen([str(binary), str(config_path), str(port)],
                                env={**os.environ, "VIDEO_WORKER_TOKEN": token},
                                start_new_session=True, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)

        def api(path, body=None, auth=True, headers=None):
            req = urllib.request.Request(base + path,
                data=None if body is None else json.dumps(body).encode(),
                headers={**({"Authorization": f"Bearer {token}"} if auth else {}),
                         "Content-Type": "application/json", **(headers or {})})
            return urllib.request.urlopen(req, timeout=10)

        def data(path, body=None):
            with api(path, body) as response:
                return json.load(response)

        def terminal(job):
            until = time.monotonic() + 65
            while time.monotonic() < until:
                current = data("/jobs/" + job["id"])
                if current["state"] in ("ready", "failed", "cancelled"):
                    return current
                time.sleep(.05)
            raise AssertionError("worker exceeded test deadline")

        def owned_pid():
            until = time.monotonic() + 10
            while time.monotonic() < until:
                if marker.exists():
                    return int(marker.read_text())
                time.sleep(.02)
            raise AssertionError("owned process did not start")

        def gone(pid):
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            raise AssertionError("owned conversion survived cancellation/deadline")

        try:
            for _ in range(100):
                try:
                    data("/health")
                    break
                except (OSError, urllib.error.URLError):
                    assert proc.poll() is None, "worker exited"
                    time.sleep(.05)
            try:
                api("/jobs", auth=False)
                raise AssertionError("unauthenticated access succeeded")
            except urllib.error.HTTPError as e:
                assert e.code == 401
            selection = dict(source="synthetic", camera="fixture", start_ms=11100,
                             end_ms=13200, profile="browser-1080p-v1")
            assert data("/sources/resolve",{k:selection[k] for k in ("camera","start_ms","end_ms")}) == {"source":"synthetic"}
            assert data("/jobs/lookup", selection) is None
            assert data("/jobs") == []
            job = data("/jobs", selection)
            assert data("/jobs/lookup", selection)["id"] == job["id"]
            assert data("/jobs", selection)["id"] == job["id"]
            ready = terminal(job)
            assert ready["state"] == "ready", ready
            assert abs(ready["media"]["duration_ms"] - 2100) <= 150
            assert ready["media"]["audio"]
            assert data("/clips") == [], "preview must not create a permanent saved clip"
            clip = data(f"/jobs/{job['id']}/save", {"name": "A synthetic moment"})
            assert data(f"/jobs/{job['id']}/save", {"name": "Renamed"})["id"] == clip["id"]
            assert len(data("/clips")) == 1
            with api(f"/clips/{clip['id']}/media", headers={"Range":"bytes=0-99"}) as response:
                assert response.status == 206 and len(response.read()) == 100
                assert response.headers["Content-Type"] == "video/mp4"
            with api(f"/clips/{clip['id']}/media") as response:
                downloaded = response.read()
            with api(f"/clips/{clip['id']}/media", headers={"Range":"bytes=0-999999999"}) as response:
                assert response.status == 206 and response.read() == downloaded
                assert response.headers["Content-Range"] == f"bytes 0-{len(downloaded)-1}/{len(downloaded)}"
            assert hashlib.sha256(downloaded).hexdigest() == ready["media"]["sha256"]
            # Precise trim at a non-keyframe boundary: compare output frame count
            # and source-reference frames, allowing lossy encoding pixel error.
            output = root / "reopened.mp4"
            output.write_bytes(downloaded)
            frames = subprocess.check_output([ffprobe, "-v","error","-count_frames",
                "-select_streams","v:0","-show_entries","stream=nb_read_frames",
                "-of","csv=p=0",str(output)]).decode().strip()
            assert int(frames) == 63, frames
            def pixels(path, extra):
                return subprocess.check_output([ffmpeg,"-v","error","-i",str(path),
                    *extra,"-an","-vf","scale=32:18","-pix_fmt","gray","-f","rawvideo","-"])
            actual = pixels(output,[])
            expected = pixels(source,["-ss","1.1","-t","2.1"])
            assert len(actual) == len(expected)
            assert sum(abs(a-b) for a,b in zip(actual,expected))/len(actual) < 5
            # Audio spans the selected duration; video/audio start together.
            info = json.loads(subprocess.check_output([ffprobe,"-v","error","-show_streams","-of","json",str(output)]))
            starts = [float(s["start_time"]) for s in info["streams"]]
            assert max(starts)-min(starts) < .050
            for s in info["streams"]:
                assert abs(float(s["duration"])-2.1) < .150
            if os.environ.get("VIDEO_TEST_BROWSER") == "1":
                subprocess.run(["node", "tests/video_worker_browser.cjs"], check=True, timeout=90,
                    env={**os.environ,"VIDEO_TEST_BASE":base,"VIDEO_TEST_TOKEN":token})
            # Changed source identity is rejected, never recycled into another clip.
            source.write_bytes(b"changed source")
            changed = data("/jobs",{**selection,"start_ms":14000,"end_ms":16000})
            assert terminal(changed)["state"] == "failed"
            source.write_bytes(original)
            # Exercise an actually running child, not a race against a fast failure.
            slow.touch()
            cancelled = data("/jobs",{**selection,"start_ms":15000,"end_ms":17000})
            pid = owned_pid()
            data(f"/jobs/{cancelled['id']}/cancel",{})
            assert terminal(cancelled)["state"] == "cancelled"
            gone(pid)
            slow.unlink()
            marker.unlink()
            assert terminal(data(f"/jobs/{cancelled['id']}/retry",{}))["state"] == "ready"
            # Deadline also kills/reaps the child and never publishes its output.
            slow.touch()
            expired = data("/jobs",{**selection,"start_ms":14500,"end_ms":16500})
            pid = owned_pid()
            assert terminal(expired)["state"] == "failed"
            gone(pid)
            try:
                api(f"/jobs/{expired['id']}/media")
                raise AssertionError("failed output became playable")
            except urllib.error.HTTPError as e:
                assert e.code == 404
            assert data(f"/clips/{clip['id']}")["name"] == "Renamed"
            print("HTTP auth, dedupe, real encode/decode, exact frame trim, audio sync, byte ranges, save/reopen, source identity, running cancellation/retry and deadline reaping passed")
        finally:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=10)


if __name__ == "__main__":
    main()
