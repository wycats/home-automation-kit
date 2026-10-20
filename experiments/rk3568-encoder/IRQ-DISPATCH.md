# Scope the encoder IRQ adaptation

Historical review of the preceding increment. [JOB-LIFETIME.md](JOB-LIFETIME.md) describes the current callback ownership follow-on and its remaining hardware gates.

2026-10-19. Patch hash `7504ef8139bcfb727e389ed86f82febeed5098a01fc7979eac0afd00d71bb7a2`.

The imported prototype routed both existing JPEG engines and new H.264 jobs through a deferred IRQ thread. That thread re-reads the current mem2mem context, so a watchdog completion can change which job it sees. This pass reduces the affected surface and corrects status handling; it does not solve H.264 job ownership.

In patched `drivers/media/platform/verisilicon/rockchip_vpu_hw.c`:

- `rockchip_vpu1_vepu_irq` and its registration return to the original hard-IRQ handler and `hantro_irq_done` path. It no longer reads or writes a context for a deferred thread.
- `rockchip_vpu2_vepu_irq` acknowledges registers as before, then routes non-H.264 jobs directly through the existing hard-IRQ completion. Only H.264 returns `IRQ_WAKE_THREAD`. The existing null-current-context guard remains.
- For H.264, any TIMEOUT, BUFFER_FULL or BUS_ERROR bit forces ERROR even when FRAME_READY is also set. This is conservative rejection; physical status/acknowledgement behavior still needs hardware validation. JPEG keeps its original status interpretation.

`irq_dispatch_test.py` extracts and compiles the actual two handlers and register definitions from the candidate, then supplies mocked register and framework operations. Fifty cases pass locally: all sixteen combinations of the four relevant status bits across missing-context, JPEG and H.264 dispatch, plus both vpu1 ready/error cases. Assertions cover callback counts, return disposition, result state and register acknowledgement writes. The test also checks that vpu1's IRQ registration does not contain a threaded callback.

The local Windows ARM64 build uses an explicit `asInvoker` manifest and an existing Clang compiler. CI executes the same test with ASan/UBSan; test executables remain in the runner's work directory and only diagnostics are uploaded.

These are synchronous function tests, not kernel scheduling, memory-order, hardware IRQ, or DMA tests. In particular, H.264's deferred thread still needs a stable job token/context, a single winner against timeout/abort, and callback/hardware quiescence before buffer or context reuse. The device backend remains disabled.
