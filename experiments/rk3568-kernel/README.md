# RK3568 decoder compile laboratory

This experiment compiles a small HEVC decoder patch series for the ODROID-M1
against a pinned Linux source tree. It tests whether the driver, modules and
board device tree build together before any hardware experiment.

The workflow runs on an ephemeral Linux CI runner. It has read-only repository
permissions and no deployment credentials or device access. It does not build a
complete HAOS image, install a kernel, restart a service or contact cameras.

## Inputs and patch scope

`manifest.json` pins Linux v7.2.11 and six public HAOS configuration inputs by
commit and SHA-256. These are public upstream settings, not a running household's
configuration. The ordered patches add RK3566/RK3568 VDPU346 support, repair RCB
resizing and SPS reference-picture-set bounds, and add three local experimental
changes:

1. Keep the 64/32 reference-set bounds and enable DRAM-only decoding on M1.
2. Rebuild all SPS reference sets, validate derived sizes, propagate failures and
   complete prepared requests without starting hardware on failure.
3. Use a conservative, 64-aligned VDPU346 buffer allocation envelope. This table
   also serves H.264, although the sizing model covers HEVC linear 4:2:0 only.

The last change covers a discrepancy between two sizing sources. It is not proof
of a hardware overrun. The experiment preserves register slot ordering and leaves
the VDPU381/383 sizing policies unchanged. See [NOTICE.md](NOTICE.md) for provenance
and licensing; this directory is excluded from the MIT Rust crate package.

## Run

Use an existing Linux machine with an arm64 GCC cross compiler, host build tools
and dt-schema. The workflow documents its package list and records exact installed
versions. Source inputs are pinned; the hosted runner image, apt snapshot and
transitive Python dependencies are not immutable yet. Captured environment
versions support diagnosis, not a bit-for-bit reproducibility claim.

```sh
# A new, clean checkout at manifest.json's Linux commit is required.
bash experiments/rk3568-kernel/compile.sh /path/to/linux /new/build /new/report
```

The script verifies hashes and the clean source revision, applies the patches,
merges the public config fragments, records `olddefconfig` drift, and builds
`Image`, modules and the M1 DTB. It checks the decoder binding and the explicit
M1 DTB. Review schema diagnostics as well as exit status.

The first CI run reserves at least 20 GiB free disk and uses four build jobs.
The disk requirement is a conservative estimate, not a measured build size.
The 180-minute deadline is a guard, not a promised build duration. No preinstalled
runner tools are deleted to make space. If the preflight fails, choose a builder
with adequate resources.

Artifacts include logs, resolved public configuration, environment versions,
Image, DTB and a separate matching module/UAPI payload built by `package.sh`.
The entire build tree is deliberately excluded because it can contain a
generated module-signing private key. See the
[diagnostic payload](../rk3568-diagnostic/README.md) for userspace and RAM-root
assembly; it still requires bootloader packaging and hardware testing.

## What passing means

A green compile establishes source/configuration/link compatibility for this
specific build. Host C regressions exercise selected decoder routines separately.
Neither result establishes hardware frame correctness, DMA safety, recovery or
performance. A matching V4L2-request userspace client and isolated hardware tests
remain necessary. The intended first performance comparison is hardware HEVC
decode plus software H.264 encode; this does not add a hardware H.264 encoder.

Keep a future boot experiment separate from a working HA installation. A
container does not contain kernel or DMA faults.
