#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0
"""Execute actual candidate pre-start C paths with bounded host fault injection.

The extracted run prefix ends before register programming. Its replacement tail
only counts synthetic watchdog/start events. Real resource APIs are mocked. This
does not qualify hardware, IRQ ownership, resource API internals or kernel ABI.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def function(source, name):
    matches = list(re.finditer(r"(?m)^(?:static )?(?:void|int|unsigned int) "
                               + re.escape(name) + r"\(", source))
    assert len(matches) == 1, name
    start = matches[0].start()
    end = source.index("\n}", start) + 2
    return source[start:end]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", required=True, type=Path)
    parser.add_argument("--uapi", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--host-shims", action="store_true",
                        help="Windows-only Linux type/errno header shims; CI uses real UAPI")
    args = parser.parse_args()
    packet = Path(__file__).resolve().parent
    kernel = args.kernel.resolve()
    report = args.report.resolve()
    report.mkdir(parents=True, exist_ok=True)
    driver = kernel / "drivers/media/platform/verisilicon"
    names = ["hantro_drv.c", "hantro_h264.c", "rockchip_vpu_hw_h264_enc.c"]
    sources = {name: (driver / name).read_text() for name in names}
    parts = []
    hashes = {}
    for file, names in [
        ("hantro_h264.c", ["hantro_h264_enc_rec_luma_size", "hantro_h264_enc_rec_image_size",
                           "hantro_h264_enc_prepare_run"]),
        ("rockchip_vpu_hw_h264_enc.c", ["rk3399_vpu_h264_enc_prepare_buffers"]),
    ]:
        source = sources[file]
        parts.append(source[:source.index("#include")])  # Original license/attribution.
        for name in names:
            text = function(source, name)
            hashes[name] = digest(text.encode())
            parts.append(text)

    run = function(sources["rockchip_vpu_hw_h264_enc.c"], "rk3399_vpu_h264_enc_run")
    boundary = "\t/* Select encoder before writing registers. */"
    assert run.count(boundary) == 1
    prefix, hardware_tail = run.split(boundary)
    assert "vepu_write(" not in prefix and "hantro_watchdog_kick(" not in prefix
    assert re.findall(r"\breturn\s+([^;]+);", hardware_tail) == ["0"]
    assert not re.search(r"\bgoto\b", hardware_tail)
    assert hardware_tail.count("hantro_watchdog_kick(ctx);") == 1
    assert hardware_tail.index("hantro_watchdog_kick(ctx);") < hardware_tail.rindex(
        "vepu_write(vpu, reg, VEPU_REG_ENCODE_START);")
    hashes["rk3399_vpu_h264_enc_run_prefix"] = digest(prefix.encode())
    hashes["rk3399_vpu_h264_enc_run_hardware_tail"] = digest(hardware_tail.encode())
    # Variables used only by the removed hardware tail remain in the exact
    # prefix. Suppress only their unused warnings, locally to this extraction.
    parts.append('''#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-variable"
#pragma GCC diagnostic ignored "-Wunused-but-set-variable"
''')
    parts.append(prefix + "\tsynthetic_hardware_commit(ctx);\n\treturn 0;\n}\n")
    parts.append("#pragma GCC diagnostic pop\n")
    source = sources["hantro_drv.c"]
    parts.append(source[:source.index("#include")])
    for name in ["device_run_h264", "device_run"]:
        text = function(source, name)
        hashes[name] = digest(text.encode())
        parts.append(text)
    subject = "\n\n".join(parts) + "\n"
    (report / "prestart_subject.inc").write_text(subject, newline="\n")
    includes = [report, args.uapi.resolve(), driver]
    if args.host_shims:
        shim = report / "host-shims/linux"
        shim.mkdir(parents=True, exist_ok=True)
        (shim / "errno.h").write_text("#include <errno.h>\n")
        (shim / "const.h").write_text("/* No active const macros in this controls UAPI. */\n")
        (shim / "types.h").write_text("""#include <stdint.h>
typedef uint8_t __u8;
typedef uint16_t __u16;
typedef uint32_t __u32;
typedef uint64_t __u64;
typedef int8_t __s8;
typedef int16_t __s16;
typedef int32_t __s32;
""")
        includes.insert(0, shim.parent)
    binary = report / "prestart-test"
    command = [args.cc, "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"]
    if args.sanitize:
        command.extend(["-fsanitize=address,undefined", "-g"])
    for path in includes:
        command.extend(["-I", str(path)])
    command.extend([str(packet / "prestart_test.c"), "-o", str(binary)])
    build = subprocess.run(command, text=True, capture_output=True, timeout=120)
    result = {
        "scope": __doc__,
        "source_sha256": {name: digest((driver / name).read_bytes()) for name in sources},
        "function_sha256": hashes,
        "subject_sha256": digest(subject.encode()),
        "test_sha256": digest((packet / "prestart_test.c").read_bytes()),
        "host_shims": args.host_shims, "sanitizers": args.sanitize,
        "command": command, "compile_exit": build.returncode,
        "compile_output": build.stdout + build.stderr,
    }
    if build.returncode == 0:
        actual_binary = binary if binary.exists() else binary.with_suffix(".exe")
        test = subprocess.run([str(actual_binary)], text=True, capture_output=True, timeout=30)
        result.update(test_exit=test.returncode, test_output=test.stdout + test.stderr)
    (report / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ["compile_exit", "compile_output", "host_shims", "sanitizers"]}
                     | {k: result[k] for k in ["test_exit", "test_output"] if k in result}, indent=2))
    assert result["compile_exit"] == 0 and result["test_exit"] == 0


if __name__ == "__main__":
    main()
