# Reject H.264 submissions until DMA recovery is defined

The candidate has no proven way to stop all encoder DMA before returning buffers
after an error. The maintained Rust client has no device backend, but the kernel
prototype still accepted H.264 submissions from other clients. This increment
closes that external entry point. It does **not** implement reset or quarantine.

## Observable behavior

The mem2mem operations table now calls `hantro_device_run`. For H.264 encoding it
invalidates the selected destination's reference eligibility, completes its
control request, and returns the queued source/destination pair with ERROR using
the existing finish function. It performs no job allocation, PM/clock operation,
metadata copy, IRQ-mode change, timer/work scheduling or MMIO. An older valid
reference is not invalidated: this rejected job never reached the hardware.

There is no module parameter, capability boolean or runtime opt-in that can
bypass this boundary. Format enumeration, allocation and STREAMON are unchanged;
an attempted H.264 encode therefore produces error buffers, not a format-setup
error. This is deliberately a contained experiment, not a usable encoder API.

JPEG encoding and decoder jobs delegate to the original `device_run` unchanged.
The original preparation and completion functions remain compiled so the offline
source tests can exercise them. Their successful synthetic starts do not imply
that external H.264 dispatch can reach hardware. An isolated boot image using
this candidate cannot perform an H.264 hardware trial until the gate is replaced
by a separately reviewed implementation.

## Scheduling and cleanup audit

The source references below use Linux 7.2.11 commit
`a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea`.

- [`v4l2_m2m_try_run`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L252)
  marks the context running and drops `job_spinlock` before invoking the driver.
  The admission wrapper drops `irqlock` before request/framework completion.
- [`v4l2_m2m_buf_done_and_job_finish`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L506)
  removes the source buffer, returns capture before output, and clears running
  ownership. The next job is queued through `schedule_work` by
  [`v4l2_m2m_schedule_next_job`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L448),
  rather than recursively calling the driver's submission function. Each
  rejection consumes a pair, so a finite submitted queue drains normally.
- Capture holding does not leave these encoder pairs behind: the candidate
  encoder queue does not enable the HOLD_CAPTURE subsystem capability, and
  [vb2 clears the unsupported input flag](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/common/videobuf2/videobuf2-v4l2.c#L332).
- [`v4l2_m2m_cancel_job`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L415)
  releases its spinlock before aborting/waiting. The admission path completes
  even if abort set the context latch first: there is no armed job to drain.
- The wrapper and its existing finish function do not use the context or buffers
  after framework completion. Tests make that boundary terminal by freeing the
  synthetic context during completion.

These are source-derived framework properties. Host doubles verify the driver's
calls, not actual kernel locking or scheduling. The wrapper calls control-request
completion once; request-object internals are not modeled as a second kernel
execution in the tests.

## Why retaining the current job is insufficient

The existing completion worker can drain its watchdog, but there is still no
hardware fence. Keeping only its job/context allocation would not retain all
memory visible to the encoder:

- The hardware programs OUTPUT plane DMA addresses directly (MMAP or imported
  DMA-BUF), CAPTURE stream storage, destination/reference reconstruction surfaces,
  and optionally a context CABAC table. Their owners and release paths differ.
- `hantro_release` invokes
  [`v4l2_m2m_ctx_release`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L1280),
  which cancels/waits then releases both vb2 queues. The candidate's buffer cleanup
  frees reconstruction storage; streamoff calls the codec exit that frees CABAC
  tables. Completing the framework job permits these releases. Withholding
  completion indefinitely instead blocks close/streamoff: it is not a bounded
  or usable failure policy.
- The platform remove callback is void and tears down registrations/clocks/PM;
  later managed-device teardown is not prevented by keeping a file/job pointer.
  A device reference alone does not promise that the bound driver's resources,
  IRQ registration, IOMMU mappings or shared-domain ownership survive unbind.
- Current system sleep callbacks use runtime-PM force suspend/resume. They do
  not implement a retained-H.264 quarantine owner. Holding a PM usage reference
  is not a documented system-sleep or shared PD_RGA idle fence.

Adding ad hoc references or returning early from remove would therefore create
new lifetime hazards. This increment makes none of those changes and makes no
claim about unrelated JPEG/decoder suspend/removal behavior.

The [DMA/reset audit](DMA-RESET-BOUNDARY.md) remains the next hardware gate:
bounded shared-domain ownership, a real encoder/bus idle guarantee, coordinated
IOMMU/reset recovery, late raw-IRQ draining, and a failure policy covering close,
unbind and sleep. If recovery cannot be proven, a separate one-shot disposable
diagnostic with memory retained until power-off is a different architecture; the
current mem2mem driver does not implement it.

## Focused checks

`admission_test.py` extracts the actual admission wrapper and existing finish
function, retains source attribution and includes the real reference helper.
It checks the actual operations-table wiring and executes 64 bounded submissions
with/without requests and draining flags, a terminal context-free case, and JPEG
plus H.264-decoder delegation. Resource/MMIO APIs are deliberately absent from
the test's interface; adding those calls requires explicit test/review changes.
Only Linux's `__u64` typedef is supplied through a host header shim.

Local Windows execution uses the existing compiler, without sanitizers. CI runs
the same focused test with ASan/UBSan and separately cross-checks ARM64 object
compilation. No broad test, device access, kernel load, reboot or production
configuration change is part of this verification.

Pinned framework source SHA256 values used for the audit:

| File | SHA256 |
| --- | --- |
| drivers/media/v4l2-core/v4l2-mem2mem.c | `8cba59ee8aec8d14f7f2b0c81829fe188d25c99399de613f3d23a5cf5eb95854` |
| drivers/media/common/videobuf2/videobuf2-v4l2.c | `60b388cd10837ae13187f9047f7e5c9b03af46a1116485789fef57a7ba682844` |
