# Hantro encoder buffer and completion review

Date: 2026-10-19. Scope: isolated compile-only candidate; no device execution.

The candidate is **not ready for a hardware client**. Narrow allocation and payload guards are implemented, but IRQ/watchdog job ownership, request-error cleanup, and reconstruction validity remain unresolved. Successful compilation or the pure helper tests do not close those gates.

## Reviewed inputs and line references

Linux base: `a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea` (7.2.11). Imported encoder prototype: `d0897ac6844c86a8d8d6f90d4116eb068184010c`; original authors and sign-offs remain in `NOTICE.md` and the patch header. Reviewed candidate patch SHA-256:

`e07221b18ddf1b6898d5abad42f1f6e3e134e3a21396a0d61a6202daf0080aab`

Source references below identify the **patched kernel tree**, not line numbers in the patch file. All driver paths are under `drivers/media/platform/verisilicon/`.

## Narrow fixes implemented

### Preserve reconstruction allocation geometry

Before this pass, `hantro_buf_init()` allocated reconstruction storage using the OUTPUT dimensions, but `hantro_set_fmt_out()` only rejected changes if OUTPUT had buffers. Allocating CAPTURE at 640x480, then resizing an unallocated OUTPUT queue to 1920x1088 could leave a 460,800-byte reconstruction allocation while programming a chroma address 2,088,960 bytes after its base. This is a source-derived invalid-address sequence, not an observed DMA overrun.

Both encoder setters now reject format changes while **either** queue owns buffers: `hantro_v4l2.c:530–610`. This also closes the CAPTURE setter's path that resets the raw format. The conservative rule rejects even same-format S_FMT while buffers exist; callers should configure both formats before REQBUFS and release both queues before changing them. Decoder dynamic resolution behavior is unchanged.

The run path additionally checks that the current reconstruction allocation exists and is large enough before programming registers (`rockchip_vpu_hw_h264_enc.c:193–196`). Reference selection rejects the current destination as its own reference and rejects missing/undersized reconstruction storage (`:270–280`). These checks do not establish that a reference contains a successful prior frame.

### Allocate reconstruction only for H.264

`hantro_v4l2.c:859–883` now allocates reconstruction storage only for `HANTRO_MODE_H264_ENC`. Previously it allocated for every encoder CAPTURE buffer, including JPEG. At the advertised 8192x8192 JPEG limit this was an unnecessary 100,663,296 bytes per buffer and could cause allocation failures.

Cleanup at `hantro_v4l2.c:938–958` frees only an existing allocation and clears CPU address, DMA address and size. The two-queue format guard prevents converting already-allocated JPEG buffers into H.264 buffers without reallocation. This removes the unrelated JPEG allocation cost; it does not resolve the shared JPEG IRQ changes described below.

### Bound completion payload before exposing it

The previous done callback directly assigned `VEPU_REG_STR_BUF_LIMIT / 8` to `bytesused` without comparing it to the capture-plane capacity. The exact pure helper `hantro_h264_enc_payload_size()` (`hantro_h264_enc_validate.h:36–53`) now checks capacity, clears its output on rejection, and conservatively rejects non-byte-aligned bit counts instead of silently rounding them.

`rk3399_vpu_h264_enc_check_result()` (`rockchip_vpu_hw_h264_enc.c:94–107`) checks the actual plane capacity and assigns only the validated payload. The existing candidate threaded completion wrapper (`hantro_drv.c:100–136`) changes rejected completion to `VB2_BUF_STATE_ERROR` and skips the success feedback callback. The public codec callback signature and JPEG success callback remain unchanged.

The register's bit-count interpretation follows the imported prototype. Its exact hardware behavior, input capacity units, and overflow semantics still need confirmation. These checks bound reported payload; they cannot undo a DMA write that already exceeded its programmed limit. A byte-alignment rejection is an explicit first-lab restriction, not a claim that all valid hardware completions must be aligned.

## Remaining hardware gates

### P1: IRQ thread can lose the completed job's identity

`rockchip_vpu_hw.c:431–449` obtains the current context in hard IRQ, writes `ctx->result`, then wakes a thread. `rk3399_vepu_thread()` at `:382–391` obtains the current context **again** and dereferences it. `hantro_thread_done()` at `hantro_drv.c:122–136` obtains it a third time before attempting to cancel the watchdog. Meanwhile the watchdog (`:156–169`) can reset and finish the old job.

The pinned [V4L2 mem2mem implementation](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c) protects the current-context lookup only during lookup, clears the current job on completion, and schedules the next job. Thus an already-woken IRQ thread can encounter no current context or a different job, including another job on the same context. Streamoff waiting for `TRANS_RUNNING` to clear does not by itself retire that old thread. `IRQF_ONESHOT` does not serialize watchdog work with this sequence.

A null check would avoid one dereference but would not fix cross-job completion or cancellation of the next job's watchdog. The needed change is a reviewed job-ownership protocol with a stable job identity/generation, exactly one winner between completion and timeout, and quiescence before freeing DMA/context state. Do not implement a speculative broad IRQ rewrite merely to make this review appear complete.

The change also affects pre-existing JPEG paths: `rockchip_vpu_hw.c:674–689` wires vpu1, vpu2 and rk3568 encoder IRQs to this thread. The vpu1 hard IRQ at `:393–410` lacks even the vpu2 null guard. Therefore the isolated candidate must not replace a production module on the theory that only new H.264 calls would be affected.

### P1: Error completion does not balance request/PM lifecycle

`hantro_h264_enc_prepare_run()` (`hantro_h264.c:1175–1193`) applies request controls before validation can reject them. The run function can subsequently reject a bad reference. `device_run()` (`hantro_drv.c:213–239`) acquires runtime PM and enables clocks, but routes run failure straight to `hantro_job_finish_no_pm()`: it does not balance those acquisitions or complete the encoder's control request there. The PM error-label structure predates this adaptation; the added rejection paths make it particularly relevant.

Likewise timeout resets and finishes without the candidate's control-request completion wrapper. The normal buffer-done path in pinned [videobuf2-core](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/common/videobuf2/videobuf2-core.c) releases the buffer's request object; that is not evidence that the separate control-handler request object has completed. Its cancellation callback is a separate path. Fix this with explicit acquisition/cleanup ownership and tests for validation failure, missing reference, clock failure, timeout and streamoff. No broad shared-path cleanup rewrite was made in this pass.

### P1: Reference timestamp is not reconstruction validity

The P-frame path (`rockchip_vpu_hw_h264_enc.c:258–288`) still uses `vb2_find_buffer()` by timestamp. Even after the new self-reference and size checks, a matching allocated buffer is not proof that it has successfully reconstructed a prior frame, remains valid after a timeout, or belongs to the intended sequence. Newly allocated/unencoded storage and reused storage need a defined validity and generation model. The client must retain intended references, but the driver still needs to reject invalid or stale reconstruction references. Keeping hardware execution disabled avoids treating this unfinished model as safe.

### Other runtime questions

- `rockchip_vpu_hw.c:438–440` treats FRAME_READY as success without separately rejecting simultaneous BUFFER_FULL, BUS_ERROR or TIMEOUT bits. Register definitions are at `rockchip_vpu2_regs.h:299–305`; confirm hardware status precedence before interpreting such completions as success.
- Source plane DMA setup ignores `data_offset`; the first laboratory client must use zero offsets and full validated source payloads, and the driver contract should reject unsupported offsets explicitly.
- Validate DMA/IOMMU addressability and the actual stream-limit units. A 32-bit register assignment alone does not prove a mapping bug or a working DMA path.
- Reset writes stopping registers (`rockchip_vpu_hw.c:518–525`) are not proof of DMA/IRQ quiescence. Establish the required hardware stop protocol before releasing reconstruction memory after errors.

## Validation performed and next tests

Performed offline for the patch hash above:

- Forward `git apply --check --whitespace=error` against the saved pristine sparse base passed; reverse application against the candidate passed; source `git diff --check` passed.
- Exact public `validation_test.c`, exact driver helper header, and exact candidate `v4l2-controls.h` compiled with existing Clang 19.1.5, `-std=c11 -O2 -Wall -Wextra -Werror`, and executed successfully on Windows ARM64. Linux errno/const/types includes used small host shims; no helper/control logic was replaced.
- Tests cover all 52 valid QPs and invalid control/dimension bounds, plus zero/exact/over-capacity payloads, non-byte-aligned counts, maximum aligned 32-bit bit count, output clearing on failure and null output pointer. No device access occurred.

This local run was not a Linux kernel build, did not use ASan/UBSan, and did not exercise vb2 ioctls or IRQ scheduling. The existing ephemeral Linux CI must compile the new candidate and run the public exact-helper test with real installed UAPI and sanitizers before calling the patch compile-validated.

Next useful focused work is a mock/KUnit ownership test that enumerates IRQ-before-timeout, timeout-before-thread, streamoff-after-timeout and next-job-start interleavings; a reference-generation test including failed/overwritten/unencoded references; and error-injection accounting for PM, clocks and control requests. Only after those gates close should a separately approved, isolated hardware boot and bounded encode/decode correctness experiment be proposed. No encoder device backend should be enabled by this review.
