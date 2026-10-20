#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0
"""Actual candidate job callbacks with host workqueue/MMIO/resource doubles.

Native host threads race IRQ/timeout/abort producers under a real host mutex.
This is not a kernel scheduler, IRQ, workqueue, DMA or hardware qualification.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
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
    driver = args.kernel / "drivers/media/platform/verisilicon"
    source = (driver / "hantro_drv.c").read_text()
    start = source.index("enum hantro_h264_job_owner {")
    end = source.index("void hantro_irq_done(", start)
    subject = source[:source.index("#include")] + source[start:end]
    subject += function(source, "device_run_h264") + "\n"
    subject += function(source, "device_run") + "\n"
    (args.output / "job_subject.inc").write_text(subject, newline="\n")
    codec = (driver / "rockchip_vpu_hw_h264_enc.c").read_text()
    tail = function(codec, "rk3399_vpu_h264_enc_run").split(
        "\tspin_lock_irqsave(&vpu->irqlock, flags);")[-1]
    assert tail == """
	ret = hantro_h264_enc_arm_locked(ctx);
	if (!ret)
		vepu_write(vpu, reg, VEPU_REG_ENCODE_START);
	spin_unlock_irqrestore(&vpu->irqlock, flags);
	/* An immediate completion can release ctx now; touch no job data. */
	return ret;
}""", "Re-review start/return lifetime boundary"
    irq = (driver / "rockchip_vpu_hw.c").read_text()
    assert "rk3399_vepu_thread" not in irq and "ctx->result" not in irq
    v4l2 = (driver / "hantro_v4l2.c").read_text()
    streamon = function(v4l2, "hantro_start_streaming")
    assert streamon.count("ctx->h264_abort = false;") == 1
    assert streamon.index("spin_lock_irqsave") < streamon.index("ctx->h264_abort = false;")
    assert streamon.index("ctx->h264_abort = false;") < streamon.index("spin_unlock_irqrestore")
    assert streamon.index("spin_unlock_irqrestore") < streamon.index("v4l2_m2m_update_start_streaming_state")
    assert "h264_abort = false" not in source, "device_run must preserve pre-entry abort"
    cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("clang")
    if not cc and os.name == "nt":
        cc = "C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/ARM64/bin/clang.exe"
    binary = args.output / ("job-lifetime-test.exe" if os.name == "nt" else "job-lifetime-test")
    command = [cc, "-std=c11", "-O1", "-Wall", "-Wextra", "-Werror", "-I", str(args.output)]
    if os.name == "nt":
        command += ["-Xlinker", "/MANIFEST:EMBED", "-Xlinker",
                    "/MANIFESTUAC:level='asInvoker' uiAccess='false'"]
    else:
        command += ["-pthread"]
    if args.sanitize:
        command += ["-fsanitize=address,undefined", "-g"]
    command += [str(Path(__file__).with_suffix(".c")), "-o", str(binary)]
    subprocess.run(command, check=True, timeout=60)
    result = subprocess.run([str(binary.resolve())], check=True, text=True,
                            capture_output=True, timeout=60)
    print(result.stdout, end="")
    (args.output / "result.json").write_text(json.dumps({
        "scope": __doc__, "command": command, "output": result.stdout,
        "sanitize": args.sanitize,
        "source_sha256": hashlib.sha256((driver / "hantro_drv.c").read_bytes()).hexdigest(),
        "subject_sha256": hashlib.sha256(subject.encode()).hexdigest(),
        "test_sha256": hashlib.sha256(Path(__file__).with_suffix(".c").read_bytes()).hexdigest(),
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
