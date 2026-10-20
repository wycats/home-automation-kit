"""Validate explicit build staging areas. No device or network access."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


def inventory(root):
    records = []
    for p in sorted(root.rglob("*")):
        name = p.relative_to(root).as_posix()
        if p.is_symlink():
            raise ValueError(f"Unexpected symlink in payload: {name}")
        if name == "manifest.json" or p.is_dir():
            continue
        if p.is_symlink() or not p.is_file():
            raise ValueError(f"Unexpected non-regular payload file: {name}")
        if p.suffix.lower() in {".pem", ".key", ".p12"}:
            raise ValueError(f"Private-key-capable file forbidden: {name}")
        records.append({"path": name, "bytes": p.stat().st_size,
                        "sha256": hashlib.sha256(p.read_bytes()).hexdigest()})
    return records


def kernel(root):
    release = (root / "kernelrelease").read_text().strip()
    if not re.fullmatch(r"[a-zA-Z0-9._+-]+", release):
        raise ValueError("Invalid kernel release")
    records = inventory(root)
    modules = []
    for record in records:
        name = record["path"]
        allowed = name in {"boot/Image", "boot/odroid-m1.dtb", "kernelrelease"}
        allowed |= name.startswith("uapi/include/")
        prefix = f"rootfs/lib/modules/{release}/"
        if name.startswith(prefix):
            leaf = name[len(prefix):]
            allowed |= leaf.startswith("modules.") and "/" not in leaf
            if leaf.endswith((".ko", ".ko.xz", ".ko.zst", ".ko.gz")):
                allowed = True
                vermagic = subprocess.check_output(
                    ["modinfo", "-F", "vermagic", str(root / name)], text=True).strip()
                if not vermagic or vermagic.split()[0] != release:
                    raise ValueError(f"Mismatched module release: {name}")
                modules.append(name)
        if not allowed:
            raise ValueError(f"File outside payload allowlist: {name}")
    if not any(p.endswith("/rockchip-vdec.ko") for p in modules):
        raise ValueError("Missing decoder module")
    for path in ["boot/Image", "boot/odroid-m1.dtb",
                 f"rootfs/lib/modules/{release}/modules.dep", "uapi/include/linux/videodev2.h"]:
        if not (root / path).is_file():
            raise ValueError(f"Missing required file: {path}")
    (root / "manifest.json").write_text(json.dumps(
        {"kind": "kernel-diagnostic-payload", "kernelrelease": release,
         "module_count": len(modules), "files": records}, indent=2) + "\n")


def verify(root):
    manifest = json.loads((root / "manifest.json").read_text())
    if inventory(root) != manifest["files"]:
        raise ValueError("Payload inventory/hash mismatch")


if __name__ == "__main__":
    {"kernel": kernel, "verify": verify}[sys.argv[1]](Path(sys.argv[2]).resolve())
