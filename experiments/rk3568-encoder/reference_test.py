#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0
"""Exact candidate buffer/lookup/ioctl paths with host vb2/DMA API doubles.

STREAMON/OFF call sites are checked as source invariants; epoch arithmetic uses
the actual shared header. This is not kernel vb2 locking or DMA validation.
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
    sources = {name: (driver / name).read_text() for name in
               ["hantro_v4l2.c", "hantro_h264.c", "rockchip_vpu_hw_h264_enc.c"]}
    parts = []
    for name, functions in [
        ("hantro_h264.c", ["hantro_h264_enc_rec_luma_size", "hantro_h264_enc_rec_image_size"]),
        ("rockchip_vpu_hw_h264_enc.c", ["rk3399_vpu_h264_enc_prepare_buffers"]),
        ("hantro_v4l2.c", ["hantro_h264_capture_allocation", "hantro_reqbufs",
                           "hantro_create_bufs", "hantro_remove_bufs",
                           "hantro_h264_invalidate_buffer", "hantro_buf_init",
                           "hantro_buf_prepare", "hantro_buf_queue", "hantro_buf_cleanup"]),
    ]:
        source = sources[name]
        parts.append(source[:source.index("#include")])
        parts.extend(function(source, f) for f in functions)
    subject = "\n\n".join(parts) + "\n"
    (args.output / "reference_subject.inc").write_text(subject, newline="\n")
    v4l2 = sources["hantro_v4l2.c"]
    for f in ["hantro_start_streaming", "hantro_stop_streaming"]:
        body = function(v4l2, f)
        assert body.count("hantro_h264_ref_advance(&ctx->h264_refs)") == 1
        assert body.index("spin_lock_irqsave") < body.index("hantro_h264_ref_advance") < body.index("spin_unlock_irqrestore")
        assert "ctx->is_encoder && ctx->vpu_dst_fmt->codec_mode == HANTRO_MODE_H264_ENC" in body
    for op in ["reqbufs", "create_bufs", "remove_bufs"]:
        assert f".vidioc_{op} = hantro_{op}," in v4l2
    fmt = function(v4l2, "hantro_set_fmt_out")
    assert "vb2_is_busy(vq)" in fmt and "v4l2_m2m_get_dst_vq" in fmt and "V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE" in fmt
    shim = args.output / "host-shims/linux"
    shim.mkdir(parents=True, exist_ok=True)
    (shim / "types.h").write_text("#include <stdint.h>\ntypedef uint64_t __u64;\n")
    cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("clang")
    if not cc and os.name == "nt":
        cc = "C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/ARM64/bin/clang.exe"
    binary = args.output / ("reference-test.exe" if os.name == "nt" else "reference-test")
    command = [cc, "-std=gnu11", "-O1", "-Wall", "-Wextra", "-Werror",
               "-I", str(shim.parent), "-I", str(driver), "-I", str(args.output)]
    if os.name == "nt":
        command += ["-Xlinker", "/MANIFEST:EMBED", "-Xlinker", "/MANIFESTUAC:level='asInvoker' uiAccess='false'"]
    if args.sanitize:
        command += ["-fsanitize=address,undefined", "-g"]
    command += [str(Path(__file__).with_suffix(".c")), "-o", str(binary)]
    subprocess.run(command, check=True, timeout=60)
    result = subprocess.run([str(binary.resolve())], check=True, text=True, capture_output=True, timeout=30)
    print(result.stdout, end="")
    (args.output / "result.json").write_text(json.dumps({
        "scope": __doc__, "command": command, "output": result.stdout, "sanitize": args.sanitize,
        "source_sha256": {n: hashlib.sha256((driver / n).read_bytes()).hexdigest() for n in sources},
        "subject_sha256": hashlib.sha256(subject.encode()).hexdigest(),
        "header_sha256": hashlib.sha256((driver / "hantro_h264_enc_ref.h").read_bytes()).hexdigest(),
        "test_sha256": hashlib.sha256(Path(__file__).with_suffix(".c").read_bytes()).hexdigest(),
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
