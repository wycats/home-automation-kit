# H.264 synchronous rejection cleanup

2026-10-19. Compile-only candidate; no device/backend enablement.

This pass fixes synchronous resource/request cleanup without changing H.264's asynchronous IRQ ownership. The latter and reference reconstruction validity remain hardware gates described in `BUFFER-IRQ-REVIEW.md`. That review's original error-path findings describe the earlier patch snapshot; the changes below supersede its pre-start portion only.

Combined candidate patch SHA-256 (including the separately reviewed JPEG IRQ dispatch/status changes):

`7504ef8139bcfb727e389ed86f82febeed5098a01fc7979eac0afd00d71bb7a2`

## Actual driver changes

References are to patched kernel sources under `drivers/media/platform/verisilicon/`.

- `hantro_drv.c:217–248`: dedicated `device_run_h264()` records successful runtime-PM and clock acquisitions. On every synchronous rejection it completes the source's queued control request, disables only clocks it acquired, releases only PM it acquired, and then finishes the buffers with ERROR. Request completion also happens when PM/clock acquisition fails before request setup. A successful run transfers the resources to the existing asynchronous completion path; it does not complete the request or release resources early.
- `hantro_drv.c:250–260`: only `is_encoder && HANTRO_MODE_H264_ENC` dispatches to that path. The existing decoder/JPEG `device_run` resource/error path is preserved, including its pre-existing limitations.
- `hantro_h264.c:1175–1201`: H.264 preparation checks and propagates `v4l2_ctrl_request_setup()` failure. It no longer calls the generic void preparation helper that discards the result. The skipped postprocessing branches were decoder-only.
- `rockchip_vpu_hw_h264_enc.c:158–188`: reconstruction, reference and CABAC allocation checks are centralized before register programming. The P-frame lookup still rejects missing/self references and insufficient reconstruction storage; it does **not** prove a prior frame reconstructed successfully. CABAC table memory must exist when entropy coding is requested. The index has already passed the exact control validator before this static helper is called.
- The actual run prefix performs both control and buffer validation before the first register write. Its hardware-programming tail has no error returns or error jumps; its only return is success after watchdog arm and ENCODE_ENABLE. Thus these new synchronous rejection paths do not leave an armed watchdog or newly enabled encoder behind. This is a source property, not a hardware test of register semantics.

The framework must provide queued source/destination buffers when calling `device_run`, as with the existing common path. The new entry does not attempt to complete a nonexistent mem2mem job. Clock-failure cleanup relies on the bulk-clock API's own rollback of partially enabled clocks; this host test mocks that API boundary rather than its internal implementation.

## Fault injection executes the candidate's C

`prestart_test.py` extracts these exact functions from the patched kernel:

1. Reconstruction luma/image size functions.
2. `hantro_h264_enc_prepare_run()`.
3. `rk3399_vpu_h264_enc_prepare_buffers()`.
4. The complete `rk3399_vpu_h264_enc_run()` prefix before register programming.
5. `device_run_h264()` and the actual `device_run()` dispatcher.

Original source license/copyright headers accompany the generated include. The test includes the exact candidate control-validation header. Source/function/extraction hashes and the full compile command are written to `prestart/results.json` in CI.

Resource APIs, context/buffer structures and control lookup are host doubles. The run's register-programming tail is replaced by a synthetic watchdog/start counter. The runner additionally rejects a source whose real hardware tail contains a fallible return/goto or whose prefix contains register writes/watchdog arm. This exercises the actual new control flow and cleanup, while clearly excluding hardware programming, Linux resource API internals, ABI/layout correctness, concurrent callbacks and DMA.

Local result: existing Windows ARM64 Clang 19.1.5, `-std=c11 -O2 -Wall -Wextra -Werror`, with only Linux header shims and no sanitizer:

- 15 rejection scenarios passed: PM, clocks, request setup, two missing-control cases, invalid QP, invalid dimensions, missing/undersized current reconstruction, missing CABAC memory, invalid CABAC index, and four missing/self/undersized/unallocated reference cases.
- Every rejection asserted one control completion, one ERROR buffer finish, balanced acquired resources, and zero synthetic watchdog/start calls. Setup failure also asserted no subsequent control reads.
- Three successful handoffs passed: intra, inter, CABAC; positive runtime-PM success was included. Each retained one PM/clock acquisition and deferred request/buffer completion.
- Two checks executed the actual dispatcher with JPEG/decoder identities and confirmed both bypass the new H.264 path.

The Linux compile job invokes:

```sh
python3 "$packet/prestart_test.py" --kernel "$work/linux" \
  --uapi "$work/uapi/include" --report "$work/prestart" --cc gcc --sanitize
```

That uses real installed UAPI with ASan/UBSan. Only the JSON result and extracted C include are copied into the diagnostics artifact; the host test executable stays in the ephemeral work directory. Its result and the full candidate object build remain pending until the next CI run; local host success is not Linux kernel compile validation.

## Boundaries that remain

No timeout or delayed IRQ cleanup was changed here. The IRQ thread can still lose the job's identity; callback/token lifetime and hardware quiescence require the separate claim/generation design and integration review. Invalid/stale reconstruction references still require a validity/generation model. Error paths after hardware has started still need single-owner request/PM/clock completion. The runtime device backend remains absent; no hardware execution, module installation or boot is justified by this pass.
