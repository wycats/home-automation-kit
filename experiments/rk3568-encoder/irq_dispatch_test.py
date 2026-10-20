# SPDX-License-Identifier: GPL-2.0
"""Execute extracted IRQ handlers with mocked register/framework operations.

This checks codec dispatch and status precedence, not concurrent ownership,
hardware acknowledgement semantics, DMA fences, or thread/context lifetime.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


def function(source, name):
    start = source.index("static irqreturn_t " + name + "(")
    opening = source.index("{", start)
    depth = 0
    for end in range(opening, len(source)):
        depth += (source[end] == "{") - (source[end] == "}")
        if depth == 0:
            return source[start:end + 1]
    raise ValueError("unterminated function " + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kernel", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sanitize", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    driver = args.kernel / "drivers/media/platform/verisilicon"
    source = (driver / "rockchip_vpu_hw.c").read_text()
    regs = (driver / "rockchip_vpu2_regs.h").read_text()
    h1 = (driver / "hantro_h1_regs.h").read_text()
    names = ["VEPU_REG_INTERRUPT", "VEPU_REG_AXI_CTRL", "VEPU_REG_INTERRUPT_BIT",
             "VEPU_REG_INTERRUPT_FRAME_READY", "VEPU_REG_INTERRUPT_TIMEOUT",
             "VEPU_REG_INTERRUPT_BUFFER_FULL", "VEPU_REG_INTERRUPT_BUS_ERROR",
             "H1_REG_INTERRUPT", "H1_REG_AXI_CTRL", "H1_REG_INTERRUPT_FRAME_RDY"]
    defines = []
    for name in names:
        matches = re.findall(r"^#define\s+" + name + r"\s+(.+)$", regs + "\n" + h1, re.M)
        assert len(matches) == 1, (name, matches)
        defines.append("#define " + name + " " + matches[0])
    vpu1_array = re.search(r"rockchip_vpu1_irqs\[\] = \{(.*?)\n\};", source, re.S).group(1)
    assert "rk3399_vepu_thread" not in vpu1_array
    prefix = r'''
#include <assert.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#define BIT(n) (1U << (n))
typedef uint32_t u32;
typedef int irqreturn_t;
enum { IRQ_NONE = 0, IRQ_HANDLED = 1, IRQ_WAKE_THREAD = 2, HANTRO_MODE_H264_ENC = 7 };
enum vb2_buffer_state { VB2_BUF_STATE_DONE = 3, VB2_BUF_STATE_ERROR = 4 };
struct hantro_fmt { int codec_mode; };
struct hantro_ctx { struct hantro_fmt *vpu_dst_fmt; enum vb2_buffer_state result; };
struct hantro_dev { int irqlock; bool h264_irq_mode, h264_mmio_live; };
static int held, claims;
#define spin_lock_irqsave(l, f) do { (void)(l); (f) = 0; assert(!held); held++; } while (0)
#define spin_unlock_irqrestore(l, f) do { (void)(l); (void)(f); assert(held == 1); held--; } while (0)
static u32 status_in, write_regs[2], write_vals[2];
static int reads, writes, done_calls;
static enum vb2_buffer_state done_state;

static u32 vepu_read(struct hantro_dev *unused, u32 reg) { (void)unused; (void)reg; reads++; return status_in; }
static void vepu_write(struct hantro_dev *unused, u32 value, u32 reg) {
    (void)unused; assert(writes < 2); write_regs[writes] = reg; write_vals[writes++] = value;
}
static void hantro_irq_done(struct hantro_dev *unused, enum vb2_buffer_state state) {
    (void)unused; done_calls++; done_state = state;
}
static void hantro_h264_enc_irq_locked(struct hantro_dev *unused, enum vb2_buffer_state state) {
    (void)unused; assert(held); claims++; done_state = state;
}
'''
    checks = r'''
int main(void) {
    struct hantro_dev device = {0};

    unsigned cases = 0;
    const u32 bits[] = { VEPU_REG_INTERRUPT_FRAME_READY, VEPU_REG_INTERRUPT_TIMEOUT,
                        VEPU_REG_INTERRUPT_BUFFER_FULL, VEPU_REG_INTERRUPT_BUS_ERROR,
                        VEPU_REG_INTERRUPT_BIT };
    for (unsigned mask = 0; mask < 32; mask++) {
        status_in = 0;
        for (unsigned i = 0; i < 5; i++) if (mask & (1U << i)) status_in |= bits[i];
        for (int kind = 0; kind < 3; kind++) {
            device.h264_irq_mode = kind != 1;
            device.h264_mmio_live = kind == 2;
            claims = reads = writes = done_calls = 0;
            int result = rockchip_vpu2_vepu_irq(0, &device);
            if (kind == 1 || (kind == 2 && (mask & 16))) {
                assert(writes == 2 && write_vals[0] == 0 && write_vals[1] == 0);
                assert(write_regs[0] == VEPU_REG_INTERRUPT && write_regs[1] == VEPU_REG_AXI_CTRL);
            } else assert(writes == 0);
            if (kind == 0) {
                assert(result == IRQ_HANDLED && reads == 0 && done_calls == 0 && claims == 0);
            } else if (kind == 1) {
                assert(result == IRQ_HANDLED && done_calls == 1 && claims == 0);
                assert(done_state == ((mask & 1) ? VB2_BUF_STATE_DONE : VB2_BUF_STATE_ERROR));
            } else if (!(mask & 16)) {
                assert(result == IRQ_NONE && reads == 1 && writes == 0 && done_calls == 0 && claims == 0);
            } else {
                assert(result == IRQ_HANDLED && done_calls == 0 && claims == 1);
                assert(done_state == (mask == 17 ? VB2_BUF_STATE_DONE : VB2_BUF_STATE_ERROR));
            }
            cases++;
        }
    }
    for (int ready = 0; ready < 2; ready++) {
        writes = done_calls = 0;
        status_in = ready ? H1_REG_INTERRUPT_FRAME_RDY : 0;
        assert(rockchip_vpu1_vepu_irq(0, &device) == IRQ_HANDLED);
        assert(done_calls == 1 && done_state == (ready ? VB2_BUF_STATE_DONE : VB2_BUF_STATE_ERROR));
        assert(writes == 2 && write_regs[0] == H1_REG_INTERRUPT && write_regs[1] == H1_REG_AXI_CTRL);
        assert(write_vals[0] == 0 && write_vals[1] == 0);
        cases++;
    }
    printf("%u extracted IRQ dispatch/status cases passed; no concurrency or hardware test.\n", cases);
    return 0;
}
'''
    generated = args.output / "irq-dispatch.c"
    generated.write_text(source[:source.index("#include")] + prefix + "\n".join(defines) + "\n" +
                         function(source, "rockchip_vpu1_vepu_irq") + "\n" +
                         function(source, "rockchip_vpu2_vepu_irq") + checks)
    cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("clang")
    if not cc and os.name == "nt":
        cc = "C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/ARM64/bin/clang.exe"
    assert cc, "Use an existing compiler"
    binary = args.output / ("irq-status-test.exe" if os.name == "nt" else "irq-status-test")
    command = [cc, "-std=c11", "-Wall", "-Wextra", "-Werror", "-Wno-unused-parameter"]
    if args.sanitize:
        command += ["-fsanitize=address,undefined", "-g"]
    if os.name == "nt":
        # Explicitly stay at the caller's privilege level; never use runas.
        command += ["-Xlinker", "/MANIFEST:EMBED", "-Xlinker",
                    "/MANIFESTUAC:level='asInvoker' uiAccess='false'"]
    command += [str(generated), "-o", str(binary)]
    subprocess.run(command, check=True, timeout=60)
    result = subprocess.run([str(binary.resolve())], check=True, text=True, capture_output=True, timeout=30)
    print(result.stdout, end="")
    (args.output / "result.json").write_text(json.dumps({
        "command": command, "output": result.stdout, "sanitize": args.sanitize,
        "driver_sha256": hashlib.sha256((driver / "rockchip_vpu_hw.c").read_bytes()).hexdigest(),
        "generated_sha256": hashlib.sha256(generated.read_bytes()).hexdigest(),
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
