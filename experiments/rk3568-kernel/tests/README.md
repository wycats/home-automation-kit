# Host regression tests

These tests compile selected **actual kernel C functions** with host mocks. They
do not compile or boot a kernel, map device registers, access a camera, or prove
that hardware decoding works. No household configuration is required.

## Run

Requirements: Python 3.10+, Git, a native C compiler (GCC or Clang), and native
Rust 1.82+ with its linker. Use a host compiler, not the ARM64 cross compiler.

```sh
python3 experiments/rk3568-kernel/tests/run.py \
  --clean-tree /tmp/kernel-clean-snapshot \
  --patched-tree /tmp/linux \
  --output /tmp/host-regressions \
  --cc gcc
```

The clean input is either the pinned upstream checkout or a snapshot of the nine
paths listed in `source-lock.json`, saved **before** applying any experiment
patches. The patched input has all eight manifest patches applied. The output
directory must not exist. Both input trees remain unchanged. The runner creates
its own nine-file copy and reverses patches 8 and 7 there to reproduce the
six-patch baseline. Patch files are verified against the enclosing manifest.

`source-lock.json` has `repository`, `commit`, and `files`, with each file carrying
a kernel-relative `path` and `sha256_lf`. Hashes normalize CRLF to LF. They bind
the clean snapshot to Linux v7.2.11 at
`a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea`.

## What is checked

- Common RPS: baseline 10 cases with eight expected failures; repaired 14 cases
  with zero failures. Covers stale later SPS sets, initial zero sets, invalid
  counts/prediction, allocation errors, atomic table updates, and clearing.
- Actual VDPU381/383 backend run functions: each baseline has three expected
  failures out of four cases; repaired versions preserve request cleanup and
  avoid device start on assembler errors. Request, watchdog, and MMIO boundaries
  are mocks, not actual kernel services.
- RCB: exact C descriptor tables and sizing helper link into Rust tests through
  thin FFI wrappers. Twelve tests include 3,952,794 input combinations, comparison
  with the independent MPP formula model, unaligned dimensions, slot identity,
  and preservation of the VDPU381/383 calculations. This models HEVC 4:2:0 linear
  output at 8/10-bit, CTU16/32/64. It does not cover every decoder format or prove
  a physical overrun in the earlier allocation.

The source functions are extracted verbatim. Host type/service definitions are
substituted by the fixtures. The old size helper and two old descriptor tables
are renamed only to coexist with the patched version. Kernel includes are
removed from the extracted bitwriter. Source copyright/license headers are
retained, including the separately licensed V4L2 UAPI declarations.

The output contains extracted C, logs, binaries, and `results.json` with source
and extraction hashes, line ranges, and command return codes. Any unexpected
baseline result, missing extraction anchor, compiler failure, or changed table
fails the run. Ordinary compiler diagnostics can contain local paths; publish CI
artifacts only from clean public checkouts.

## Attribution and verification boundary

`run.py` is original MIT orchestration. Test fixtures and the independent Rust
formula model use GPL-2.0 terms as marked in the files; extracted Linux code
retains its original attribution. The reference formulas are from Rockchip MPP
commit `20959ec3dabb313b8c33ca0b7b09c06ffd2a9caa`:
`hal_h265d_vdpu34x.c:665-765`, `vdpu34x_com.c:31-77`, and `vdpu_com.h:17-18`.
See the enclosing `NOTICE.md` and `LICENSES/` for kernel patch provenance.

The portable runner passed locally with native Clang/Rust on Windows ARM64.
Ubuntu/GCC execution and the full kernel Image/modules/DT build are separate CI
checks. These host tests are not ABI/layout, DMA, sanitizer, boot, or hardware
validation. RPS mocks deliberately do not claim kernel structure layout fidelity.
