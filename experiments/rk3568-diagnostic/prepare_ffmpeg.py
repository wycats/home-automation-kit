"""Fetch only pinned public sources and hash-checked patches into a new directory."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

packet = Path(__file__).resolve().parent
manifest = json.loads((packet / "sources.json").read_text())
target = Path(sys.argv[1]).resolve()
target.mkdir(exist_ok=False)
subprocess.run(["git", "init", str(target)], check=True)
subprocess.run(["git", "-C", str(target), "fetch", "--depth=1",
                manifest["ffmpeg"]["repository"], manifest["ffmpeg"]["commit"]], check=True)
subprocess.run(["git", "-C", str(target), "checkout", "--detach", "FETCH_HEAD"], check=True)
assert subprocess.check_output(["git", "-C", str(target), "rev-parse", "HEAD"], text=True).strip() == manifest["ffmpeg"]["commit"]
patch_dir = target / "lab-patches"
patch_dir.mkdir()
source = manifest["patch_source"]
for patch in manifest["patches"]:
    url = source["repository"].replace("github.com", "raw.githubusercontent.com") + "/" + source["commit"] + "/" + patch["path"]
    with urllib.request.urlopen(url, timeout=90) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != patch["sha256"]:
        raise ValueError(f"Patch hash mismatch: {patch['path']}")
    path = patch_dir / Path(patch["path"]).name
    path.write_bytes(data)
    subprocess.run(["git", "-C", str(target), "apply", "--check", str(path)], check=True)
    subprocess.run(["git", "-C", str(target), "apply", str(path)], check=True)
