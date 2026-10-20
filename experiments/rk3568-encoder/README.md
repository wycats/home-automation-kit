# RK3568 H.264 encoder compile experiment

The maintained Rust client's device-free foundation and verification commands
are in [CLIENT.md](CLIENT.md). Hardware execution remains disabled.

The latest contained follow-on qualifies H.264 status with the VEPU2 raw IRQ bit
before acknowledging or claiming a job. Its 98 extracted-handler cases and the
separate [DMA/reset boundary audit](DMA-RESET-BOUNDARY.md) keep that software
correction distinct from hardware quiescence. No reset sequence is added.

[Reconstruction eligibility](REFERENCE-VALIDITY.md) ensures that
only final successful jobs publish references; requeue, cleanup and stream or
error epochs withdraw them. A contained MMAP capture restriction keeps allocation
changes out of streaming jobs. This extends [per-job callback ownership](JOB-LIFETIME.md): immutable
H.264 job storage, one completion worker, a drained per-job watchdog, and an
abort latch that also covers cancellation before `device_run`. Host tests exercise
the actual candidate C paths. Physical IRQ/DMA quiescence and hardware reference
surface correctness remain unresolved gates.

Browser playback needs H.264 derivatives up to 1920x1080 while keeping original
recordings. This isolates an encoder investigation from the decoder and RAM-root
bundle. It does not modify those experiments, produce a boot image, load modules,
or open a video device. Compilation is the first gate, not hardware validation.

## Why this candidate

RK3568 has distinct VEPU121/Hantro and VEPU540/RKVENC blocks. Rockchip's MPP source
selects VEPU540 for video encoding, using the vendor MPP service ABI. It is not a
drop-in client for the mainline V4L2 request decoder kernel.

Pengutronix's `v6.3/topic/rk3568-vepu-h264-stateless-bootlin` branch has a concrete
Hantro encoder adaptation for RK3568. Its 2022 test initially produced unusable
output; the author subsequently found that the prototype also broke JPEG, so
that result did not establish whether VEPU121 H.264 works. The later branch
restores kernel buffer mapping for encoders. We can compile and review this
small experiment without importing a vendor kernel. It remains an unproven
hardware path, not proof of the SoC's advertised 1080p60 rate on this block.

The September 2026 PX30 report uses a newer stateless API and GStreamer client.
PX30, VC8000E, VEPU121 and VEPU540 are not interchangeable merely because each
can be described as a video encoder. No complete current RK3568 implementation
of that newer API was located during this investigation.

## Rebase and build boundary

`manifest.json` pins Linux 7.2.11, the old prototype and its matching Bootlin
client. The single derived patch adapts moved Makefile sections, declarations,
IRQ setup, controls and queue callbacks. It preserves modern decoder buffer
limits and non-threaded IRQ behavior, uses ONESHOT for threaded handlers, and
places reconstruction state on the CAPTURE buffer that actually contains it.
Failed parameter preparation now returns an error instead of reporting success
without launching the hardware. A missing reference now fails explicitly and
uses the multi-planar capture queue. A pure helper rejects out-of-range QP table
indices, invalid slice/CABAC fields and dimensions outside the initial envelope
before register writes; host regressions execute that exact helper with ASan
and UBSan. Full control validation is still incomplete. An obsolete unaligned
include and unused register read are updated. These are source-level corrections, not hardware
regressions tested on an M1.

The workflow compiles the Hantro objects, links their relocatable driver object,
compiles both affected V4L2 control objects, installs matching UAPI headers into
a temporary directory, executes a device-free ABI probe, and compiles the legacy
client. It does **not** perform full-kernel link/modpost, create a loadable module
or claim runtime compatibility. Reports contain warnings, symbol evidence and
ABI sizes. No client binary is distributed; see NOTICE.md.

## Gates before hardware execution

This is intentionally not included in the diagnostic initramfs yet. Review:

- Control validation: QP/table indices, slice bounds, checkpoint arrays,
  reference timestamps and malformed parameter combinations need explicit
  bounds/error tests. Legacy control structs are not a stable upstream API.
- DMA and buffers: reconstructed surface size/lifetime, capture capacity,
  bytes-used bounds, format changes, allocation failures and request cancellation.
- IRQ/watchdog: the callback-ownership candidate removes H.264's current-context
  threaded completion and has host fault/race tests. Reset/DMA-idle guarantees,
  late untagged device interrupts, level deassertion, suspend/removal and repeated
  hardware sessions still need validation. A compile pass proves none of these.
- Scope: imported code also changes shared Hantro paths. Restrict enablement and
  test JPEG/decoder regressions before combining with the decoder candidate.
- Client: original program hardcodes 640x480/10 frames, discovers devices and
  creates local output files. It must gain explicit device identity, bounded
  1080p synthetic input, reliable cleanup and output limits before a live trial.
- Licensing: resolve the original client's redistribution terms or replace it.

Then use spare hardware or a separately approved outage, start at low resolution,
and validate the resulting bitstream with an independent decoder. Progress to
1080p30, then 1080p60 only after correctness. Require expected dimensions/frame
count, monotonic timestamps, no kernel/IOMMU errors, repeated-session recovery,
and measured throughput/CPU/temperature. In particular, 1080 visible lines need
1088 padded coded lines with correct SPS cropping: the old client only exercises
640x480, and an encoder claiming a padded size alone is not sufficient.
Compare decoded content and quality to
the synthetic source; lossy H.264 output is not expected to hash identically.
Measure scaling/copy costs separately before calling this full transcoding.

## Sources

- [Prototype branch](https://git.pengutronix.de/cgit/mgr/linux/log/?h=v6.3/topic/rk3568-vepu-h264-stateless-bootlin)
- [Inconclusive RK3568 test and JPEG regression](https://lkml.indiana.edu/hypermail/linux/kernel/2208.1/01945.html)
- [Newer PX30 discussion](https://lkml.iu.edu/2609.3/00096.html)
- [MPP SoC selection](https://github.com/rockchip-linux/mpp/blob/20959ec3dabb313b8c33ca0b7b09c06ffd2a9caa/osal/mpp_soc.c)
