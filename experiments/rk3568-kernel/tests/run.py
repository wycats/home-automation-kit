#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Public host-C regressions. Inputs are read-only; output must be a new directory.

This does not build/load a kernel or communicate with any device. Function bodies
are extracted verbatim; the C fixtures mock allocation, request and MMIO services.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
REL = Path("drivers/media/platform/rockchip/rkvdec")
EXE = ".exe" if os.name == "nt" else ""


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def structure(text: str, name: str) -> str:
    found = re.search(r"struct " + name + r"\s*\{.*?\};", text, re.S)
    if not found:
        raise ValueError(f"Cannot extract struct {name}")
    return found.group(0)


def table(text: str, name: str) -> str:
    found = re.search(r"static const struct rcb_size_info " + name + r"\[\] = \{.*?\n};", text, re.S)
    if not found:
        raise ValueError(f"Cannot extract table {name}")
    return found.group(0)


def attribution(text: str) -> str:
    return text[:text.index("#include")]


class Runner:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.output = args.output.resolve()
        self.sources: dict[str, dict[str, str]] = {}
        self.commands: list[dict] = []
        self.extractions: list[dict] = []

    def command(self, name: str, command: list[str], cwd: Path | None = None,
                expected: int = 0) -> str:
        result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, timeout=300)
        text = result.stdout + result.stderr
        (self.output / f"{name}.log").write_text(text, encoding="utf-8")
        # Keep portable evidence: never record absolute checkout/output/compiler paths.
        self.commands.append({"name": name, "executable": Path(command[0]).name,
                              "returncode": result.returncode, "expected": expected})
        if result.returncode != expected:
            raise RuntimeError(f"{name}: expected {expected}, got {result.returncode}\n{text}")
        return text

    def extract(self, variant: str, path: str, start: str, end: str) -> str:
        text = self.sources[variant][path]
        begin = text.index(start)
        finish = text.index(end, begin)
        part = text[begin:finish]
        self.extractions.append({"variant": variant, "path": path,
                                 "source_sha256_lf": sha(text.encode()),
                                 "start_line": text[:begin].count("\n") + 1,
                                 "end_line_inclusive": text[:finish].count("\n"),
                                 "extracted_sha256_lf": sha(part.encode())})
        return part

    def compile_c(self, name: str, fixture: str, directory: Path,
                  defines: list[str] | None = None) -> Path:
        binary = directory / ("test" + EXE)
        command = [self.args.cc, "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                   "-Wno-sign-compare", *(defines or []), "-I", str(directory),
                   str(HERE / fixture), "-o", str(binary)]
        self.command(name + "-compile", command)
        return binary

    def load(self) -> None:
        lock = json.loads((HERE / "source-lock.json").read_text())
        for f in lock["files"]:
            content = (self.args.clean_tree / f["path"]).read_bytes()
            if sha(content.replace(b"\r\n", b"\n")) != f["sha256_lf"]:
                raise ValueError(f"Clean source mismatch: {f['path']}")
        self.output.mkdir(parents=True, exist_ok=False)
        baseline = self.output / "six-patch-baseline"
        baseline.mkdir()
        for f in lock["files"]:
            path = Path(f["path"])
            target = baseline / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((self.args.patched_tree / path).read_bytes())
        self.command("baseline-init", ["git", "-C", str(baseline), "init", "-q"])
        self.command("baseline-line-endings", ["git", "-C", str(baseline), "config", "core.autocrlf", "false"])
        manifest = json.loads((HERE.parent / "manifest.json").read_text())
        wanted = ["lab-08-optional-vdpu346-conservative-rcb.patch", "lab-07-hevc-rps-fail-closed.patch"]
        for name in wanted:
            entry = next(f for f in manifest["patches"] if Path(f["path"]).name == name)
            patch = HERE.parent / entry["path"]
            if sha(patch.read_bytes()) != entry["sha256"]:
                raise ValueError(f"Patch mismatch: {name}")
            self.command("reverse-" + name[:6], ["git", "-C", str(baseline), "apply", "-R", str(patch)])
        for variant, root in [("before", baseline), ("after", self.args.patched_tree)]:
            self.sources[variant] = {f["path"]: (root / f["path"]).read_text(encoding="utf-8")
                                     for f in lock["files"]}

    def rps(self) -> None:
        for variant in ["before", "after"]:
            source = self.sources[variant]
            common_path = (REL / "rkvdec-hevc-common.c").as_posix()
            common = source[common_path]
            bit = source[(REL / "rkvdec-bitwriter.h").as_posix()]
            bit = re.sub(r"^#include .*\n", "", bit, flags=re.M)
            defs = self.extract(variant, (REL / "rkvdec-hevc-common.h").as_posix(),
                                "#define RPS_LT_", "struct rkvdec_hevc_run")
            uapi = source["include/uapi/linux/v4l2-controls.h"]
            mock = """struct rkvdec_hevc_run {
 const struct v4l2_ctrl_hevc_sps *sps;
 const struct v4l2_ctrl_hevc_ext_sps_st_rps *ext_sps_st_rps;
 const struct v4l2_ctrl_hevc_ext_sps_lt_rps *ext_sps_lt_rps;
};"""
            functions = self.extract(variant, common_path,
                                     "static void rkvdec_hevc_assemble_hw_lt_rps", "\nstruct vb2_buffer *")
            unit = "\n".join([attribution(common), bit, defs,
                              uapi[:uapi.index("#ifndef")],
                              structure(uapi, "v4l2_ctrl_hevc_ext_sps_st_rps"),
                              structure(uapi, "v4l2_ctrl_hevc_ext_sps_lt_rps"), mock,
                              structure(common, "calculated_rps_st_set"), functions])
            directory = self.output / ("rps-" + variant)
            directory.mkdir()
            (directory / "subject.inc").write_text(unit, encoding="utf-8")
            binary = self.compile_c("rps-" + variant, "rps.c", directory,
                                    [f"-DFIXED={int(variant == 'after')}"])
            text = self.command("rps-" + variant + "-run", [str(binary)], expected=int(variant == "before"))
            expected = "RESULT tests=10 failures=8" if variant == "before" else "RESULT tests=14 failures=0"
            if expected not in text:
                raise RuntimeError(f"RPS result mismatch: {variant}\n{text}")

    def backends(self) -> None:
        for variant in ["before", "after"]:
            for backend in [381, 383]:
                path = (REL / f"rkvdec-vdpu{backend}-hevc.c").as_posix()
                source = self.sources[variant][path]
                function = self.extract(variant, path, "static int rkvdec_hevc_run(", "\nstatic int rkvdec_hevc_try_ctrl")
                name = f"backend-{backend}-{variant}"
                directory = self.output / name
                directory.mkdir()
                (directory / "backend.inc").write_text(attribution(source) + function, encoding="utf-8")
                binary = self.compile_c(name, "backend.c", directory,
                                        [f"-DBACKEND={backend}", f"-DFIXED={int(variant == 'after')}"])
                text = self.command(name + "-run", [str(binary)], expected=int(variant == "before"))
                expected = f"RESULT tests=4 failures={3 if variant == 'before' else 0}"
                if expected not in text:
                    raise RuntimeError(f"Backend result mismatch: {name}\n{text}")

    def rcb(self) -> None:
        before, after = self.sources["before"], self.sources["after"]
        core = (REL / "rkvdec.c").as_posix()
        rcb = (REL / "rkvdec-rcb.c").as_posix()
        header = self.extract("after", (REL / "rkvdec-rcb.h").as_posix(), "enum rcb_axis", "int rkvdec_allocate_rcb")
        function = self.extract("after", rcb, "static size_t rkvdec_rcb_size", "\ndma_addr_t rkvdec_rcb_buf_dma_addr")
        old_function = self.extract("before", rcb, "static size_t rkvdec_rcb_size", "\ndma_addr_t rkvdec_rcb_buf_dma_addr")
        for variant in ["vdpu381", "vdpu383"]:
            if table(before[core], variant + "_rcb_sizes") != table(after[core], variant + "_rcb_sizes"):
                raise ValueError(f"Existing {variant} table changed")
        tables = "\n".join(table(after[core], n + "_rcb_sizes") for n in ["vdpu346", "vdpu381", "vdpu383"])
        old_tables = table(before[core], "vdpu381_rcb_sizes").replace("vdpu381_rcb_sizes", "original_rcb_sizes")
        old_tables += "\n" + table(before[core], "vdpu383_rcb_sizes").replace("vdpu383_rcb_sizes", "original_383_rcb_sizes")
        unit = attribution(after[core]) + attribution(after[rcb]) + """
#include <stdint.h>
#include <stddef.h>
typedef uint8_t u8;
#define ALIGN(x,a) (((x)+(a)-1)&~((a)-1))
""" + header + function + old_function.replace("rkvdec_rcb_size", "original_size") + tables + old_tables
        unit += (HERE / "rcb-wrappers.c").read_text()
        directory = self.output / "rcb"
        directory.mkdir()
        c_file = directory / "actual-size.c"
        c_file.write_text(unit, encoding="utf-8")
        obj = directory / "actual-size.o"
        self.command("rcb-compile-c", [self.args.cc, "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                     "-Wno-missing-field-initializers", "-c", str(c_file), "-o", str(obj)])
        binary = directory / ("checks" + EXE)
        self.command("rcb-compile-rust", [self.args.rustc, "--edition=2021", "--test", "-O",
                     str(HERE / "rcb.rs"), "-C", "link-arg=" + str(obj), "-o", str(binary)])
        text = self.command("rcb-tests", [str(binary)])
        if "12 passed; 0 failed" not in text:
            raise RuntimeError("Expected all 12 RCB tests, including exhaustive enumeration")

    def save(self, status: str) -> None:
        if self.output.exists():
            result = {"status": status, "scope": "Extracted host C with mocks and Rust model; not kernel or hardware validation",
                      "commands": self.commands, "extractions": self.extractions,
                      "source_files": {v: {p: sha(t.encode()) for p, t in files.items()}
                                       for v, files in self.sources.items()}}
            (self.output / "results.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clean-tree", type=Path, required=True)
    parser.add_argument("--patched-tree", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"), help="Host C compiler executable, without extra arguments")
    parser.add_argument("--rustc", default="rustc")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("--output must not exist")
    for compiler in [args.cc, args.rustc, "git"]:
        if not shutil.which(compiler):
            parser.error(f"Executable not found: {compiler}")
    runner = Runner(args)
    try:
        runner.load()
        runner.rps()
        runner.backends()
        runner.rcb()
    except Exception as error:
        runner.save("failed")
        print(error, file=sys.stderr)
        return 1
    runner.save("passed")
    print("PASS: 14 common-RPS cases, 8 backend cases, 12 RCB/model tests (3,952,794 combinations).")
    print("Baseline failures reproduced; supplied source trees unchanged. Host tests only.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
