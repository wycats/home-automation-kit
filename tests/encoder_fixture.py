"""Independent FFmpeg decode of synthetic I_PCM vectors; no devices or network."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def command(args, success=True):
    result = subprocess.run(args, capture_output=True, timeout=30)
    if (result.returncode == 0) != success:
        raise AssertionError(result.stderr.decode(errors="replace"))
    return result


def verify(binary, ffmpeg):
    with tempfile.TemporaryDirectory(prefix="encoder-fixture-") as temporary:
        root = Path(temporary)
        for width, height in [(1920, 1080), (642, 482), (640, 480)]:
            path = root / f"{width}x{height}"
            args = [binary, "fixture", str(path), "2", str(width), str(height)]
            command(args)
            manifest_bytes = (path / "manifest.json").read_bytes()
            plan = json.loads(manifest_bytes)
            assert not plan["hardware_backend_enabled"]
            source = (path / "synthetic.nv12").read_bytes()
            assert len(source) == plan["fixture_bytes"] == plan["frame_bytes"] * 2
            # No -frames limit: extra decoded pictures must fail our exact length check.
            decoded = command([ffmpeg, "-nostdin", "-v", "error", "-xerror", "-i",
                str(path / "crop-test.h264"), "-f", "rawvideo", "-pix_fmt", "nv12", "pipe:1"]).stdout
            stride = plan["stride"]
            expected = b"".join(source[y * stride:y * stride + width] for y in range(height))
            base = plan["luma_bytes"]
            expected += b"".join(source[base + y * stride:base + y * stride + width]
                for y in range(height // 2))
            assert len(decoded) == width * height * 3 // 2
            assert decoded == expected, f"visible pixels/cropping differ at {width}x{height}"
            command(args, success=False)
            assert (path / "manifest.json").read_bytes() == manifest_bytes
            print(f"{width}x{height}: exact visible NV12 pixels, padding/cropping and no-overwrite passed")
        invalid_path = root / "invalid"
        command([binary, "fixture", str(invalid_path), "121"], success=False)
        assert not invalid_path.exists()
        command([binary, "encode", "/dev/video0"], success=False)


if __name__ == "__main__":
    verify(sys.argv[1], sys.argv[2])
