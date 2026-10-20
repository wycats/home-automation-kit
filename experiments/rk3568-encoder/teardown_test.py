#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0
"""Execute extracted Hantro teardown paths with synchronous runtime-PM doubles.

No device access. Checks PM/reset ordering, not DMA quiescence or genpd behavior.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
from prestart_test import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kernel", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sanitize", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = (args.kernel / "drivers/media/platform/verisilicon/hantro_drv.c").read_text()
    probe = function(source, "hantro_probe")
    # Entry labels come from the actual failure branches, not a parallel model.
    operations = [
        "reset_control_deassert", "clk_bulk_prepare", "v4l2_device_register",
        "hantro_get_v4l2_m2m_dev", "hantro_add_enc_func",
        "hantro_add_dec_func", "media_device_register",
    ]
    targets = []
    for operation in operations:
        tail = probe[probe.index(operation + "("):]
        targets.append(re.search(r"goto (\w+);", tail).group(1))
    dispatch = "\n".join(f"case {i}: goto {target};" for i, target in enumerate(targets))
    subject = source[:source.index("#include")]
    subject += "static int probe_failure(struct hantro_dev *vpu, int stage, int ret) {\n"
    subject += "switch (stage) {\n" + dispatch + "\ndefault: abort();\n}\n"
    subject += probe[probe.index("\nerr_rm_dec_func:"):]
    subject += "\n" + function(source, "hantro_remove") + "\n"
    (args.output / "teardown_subject.inc").write_text(subject, newline="\n")
    cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("clang")
    if not cc and os.name == "nt":
        cc = "C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/ARM64/bin/clang.exe"
    assert cc
    binary = args.output / ("teardown-test.exe" if os.name == "nt" else "teardown-test")
    test = Path(__file__).with_suffix(".c")
    command = [cc, "-std=c11", "-O1", "-Wall", "-Wextra", "-Werror",
               "-Wno-unused-parameter", "-I", str(args.output)]
    if args.sanitize:
        command += ["-fsanitize=address,undefined", "-g"]
    if os.name == "nt":
        command += ["-Xlinker", "/MANIFEST:EMBED", "-Xlinker",
                    "/MANIFESTUAC:level='asInvoker' uiAccess='false'"]
    command += [str(test), "-o", str(binary)]
    subprocess.run(command, check=True, timeout=60)
    result = subprocess.run([str(binary.resolve())], check=True, capture_output=True,
                            text=True, timeout=30)
    evidence = {"scope": __doc__, "command": command, "output": result.stdout,
                "sanitize": args.sanitize, "failure_targets": dict(zip(operations, targets)),
                "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                "subject_sha256": hashlib.sha256(subject.encode()).hexdigest(),
                "test_sha256": hashlib.sha256(test.read_bytes()).hexdigest()}
    (args.output / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(result.stdout, end="")


if __name__ == "__main__":
    main()
