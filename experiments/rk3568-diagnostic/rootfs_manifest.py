"""Inventory staged RAM root, rejecting external links and persistent services."""
import hashlib
import json
import os
from pathlib import Path
import sys


def inspect(root):
    records = []
    for p in sorted(root.rglob("*")):
        name = p.relative_to(root).as_posix()
        if p.is_symlink():
            link = os.readlink(p)
            if Path(link).is_absolute() or not p.resolve().is_relative_to(root) or not p.resolve().is_file():
                raise ValueError(f"External/dangling rootfs link: {name}")
            records.append({"path": name, "link": link})
        elif p.is_dir():
            continue
        elif p.is_file():
            if p.suffix.lower() in {".key", ".pem", ".p12"}:
                raise ValueError(f"Key-capable file: {name}")
            if name.startswith(("etc/systemd/", "etc/init.d/", "etc/network/")) or name == "etc/fstab":
                raise ValueError(f"Persistent service/mount configuration: {name}")
            records.append({"path": name, "bytes": p.stat().st_size,
                            "mode": oct(p.stat().st_mode & 0o777),
                            "sha256": hashlib.sha256(p.read_bytes()).hexdigest()})
        else:
            raise ValueError(f"Unexpected special file: {name}")
    return {"files": records, "additional_cpio_nodes": [{"path": "dev/console", "type": "char", "major": 5, "minor": 1, "mode": "0600"}]}


if __name__ == "__main__":
    print(json.dumps(inspect(Path(sys.argv[1]).resolve()), indent=2))
