"""Stage audited executables and their ELF dependency closure on native arm64 CI."""
from pathlib import Path
import json
import shutil
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
bins = {"/bin/busybox": "/bin/busybox", "/usr/bin/kmod": "/bin/kmod",
        "/usr/bin/v4l2-ctl": "/usr/bin/v4l2-ctl", "/usr/bin/media-ctl": "/usr/bin/media-ctl"}
for source, dest in bins.items():
    target = root / dest.lstrip("/")
    shutil.copyfile(source, target)
    target.chmod(0o755)

records = []
for binary in [root / "usr/bin/ffmpeg", root / "usr/bin/ffprobe", *[root / p.lstrip("/") for p in bins.values()]]:
    header = subprocess.check_output(["readelf", "-h", str(binary)], text=True)
    if "AArch64" not in header:
        raise ValueError(f"Not an arm64 ELF: {binary.name}")
    result = subprocess.run(["lddtree", "-l", str(binary)], capture_output=True, text=True, check=True)
    if result.stderr.strip():
        raise ValueError(result.stderr)
    deps = []
    for line in result.stdout.splitlines():
        path = Path(line)
        if path == binary:
            continue
        if not path.is_absolute() or not path.is_file():
            raise ValueError(f"Unresolved ELF dependency: {line}")
        dest = root / path.as_posix().lstrip("/")
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, dest)  # Materialize symlinks; no links to CI host.
        dest.chmod(0o755)
        deps.append(line)
    records.append({"executable": binary.relative_to(root).as_posix(), "libraries": deps})

for applet in ["sh", "mount", "umount", "ls", "cat", "dmesg", "sleep", "setsid",
               "cttyhack", "mkdir", "echo", "uname", "head", "tail", "grep", "sed",
               "awk", "sha256sum", "time", "sync", "poweroff", "reboot", "find", "cp"]:
    (root / "bin" / applet).symlink_to("busybox")
for applet in ["modprobe", "modinfo", "lsmod", "rmmod", "depmod", "insmod"]:
    (root / "bin" / applet).symlink_to("kmod")
(root / "runtime-closure.json").write_text(json.dumps(records, indent=2) + "\n")
# Preserve distro copyright notices and exact package versions in the artifact.
copyrights = root / "usr/share/doc/lab-packages"
copyrights.mkdir(parents=True)
for package in ["busybox-static", "kmod", "v4l-utils", "libc6", "libdrm2", "libudev1", "libx264-dev", "libx265-dev"]:
    source = Path("/usr/share/doc") / package / "copyright"
    if source.is_file():
        shutil.copyfile(source, copyrights / (package + "-copyright"))
