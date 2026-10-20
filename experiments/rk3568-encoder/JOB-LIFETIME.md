# H.264 callback ownership candidate

This follow-on replaces the prototype's H.264 threaded-IRQ/current-context lookup with a per-job allocation and one completion worker. It is an isolated source/compile experiment. The Rust client still has no device backend, and nothing here qualifies hardware execution.

## Software lifetime boundary

Each H.264 job holds immutable context/source/destination pointers, its own delayed watchdog, and a separate completion work item. The framework keeps the running context and buffers alive until `hantro_job_finish_no_pm()` releases the mem2mem job. No H.264 asynchronous callback looks up the framework's newest current context.

Under `irqlock`, the job progresses from PREPARING to RUNNING, then exactly one of IRQ, timeout or abort claims it. The winner queues completion under the same lock. IRQ and abort callers retain no job pointer after unlocking. A timer callback has its own job through `container_of`; the completion worker drains it with `cancel_delayed_work_sync()` before freeing the object. It runs neither in that timer callback nor in an IRQ thread, avoiding a synchronous self-wait. The H.264 IRQ thread and its registration are removed; JPEG retains hard-IRQ completion.

The job is allocated before PM acquisition, clocks, request setup or register writes. Allocation/PM/clock/setup/validation failures complete the queued request and release only acquired resources. PREPARING cannot be asynchronously claimed. A streamoff/release abort arriving before `device_run()` sets a context latch; only STREAMON clears it. An abort during preparation cannot launch a cleanup worker while preparation still owns the context. The final watchdog arm and hardware enable are serialized under `irqlock`; cancellation at that boundary rejects enable. An external abort may therefore reject after register preparation, but no fallible control/buffer validation moved past enable.

The completion worker checks the abort latch before result processing and again before detach. A late abort after codec `done()` can cause ERROR after reference/feedback updates; this does **not** repair reconstruction-reference validity. Detach is the software completion linearization boundary; an abort after detach cannot retroactively change a result already selected for framework completion.

After draining the watchdog and completing the request, the worker invokes the existing VEPU2 reset under `irqlock`, clears the MMIO-live guard, and detaches the job from both device and context. It then releases clocks/PM and finishes the framework job. The only subsequent operation is freeing the separate job allocation; there is no later context/device/buffer access. This permits immediate context release or immediate scheduling of the next job, including another job in the same context, before the old work function returns. The start path likewise accesses no context/job data after successful start unlock, since completion may already have run.

## IRQ and hardware boundary

H.264 mode persists while idle and across decoder work until another encoder job selects its mode. The VEPU2 top half uses that device mode instead of reading a potentially released current context. Its MMIO and job access are under `irqlock`. H.264 interrupts with the MMIO-live guard clear return without accessing registers or a context, including the detach-to-framework-finish interval and clocks-off idle. JPEG retains its prior status precedence and completion behavior; H.264 error bits still override FRAME_READY.

There is no retained IRQ number and no `synchronize_irq()` call. For **software pointer lifetime**, the shared lock and no-pointer-after-unlock rule are the fence: detach cannot overlap an IRQ job access, and a later IRQ sees no job. The separate completion worker drains the immutable timer. This does not make the lock a hardware IRQ/DMA fence or qualify driver unbind/removal with an in-flight request. IRQ registration/device lifetime remains the existing driver's responsibility.

The existing reset writes interrupt-disable, ENCODE_START=0 and AXI=0 before detach. Those writes have no demonstrated DMA-idle acknowledgement in this candidate. Returning IRQ_HANDLED without MMIO while clocks are off does not prove that a stale level interrupt has deasserted. A late **untagged physical status** arriving after another job is armed may still be attributed to that new job: software allocation identity cannot identify a hardware event that contains no job identity. Reset ordering, pending interrupt clearing, level deassertion, bus/DMA quiescence, readback requirements, suspend/removal, and reconstruction-reference validity remain runtime/source-review gates. No test result below removes them.

## Tests and qualification

`job_lifetime_test.py` extracts the actual candidate enum, job structure, claim, completion, timeout, IRQ-claim, abort, arm and `device_run` functions. Original GPL headers are retained. Source hashes and generated-subject hashes are recorded. Resource APIs, workqueue contracts, register start and framework completion are host doubles; the kernel scheduler, spinlocks and device are not executed.

The local Windows ARM64 Clang run passed:

- 15 deterministic scenarios: allocation/PM/clock/run rejection, abort before entry and during preparation, each winning claimant with subsequent losers, malformed payload, abort before result processing, abort after request completion, immediate same/different-context next jobs, and immediate completion that actually frees the context before `device_run()` returns.
- One active-timeout drain: an actual host callback thread pauses after its claim/unlock; the separate completion thread blocks in the mocked synchronous drain until that callback returns. No request completion, framework finish or free happens while it is paused. This tests the candidate's use of a drain contract, not Linux workqueue internals.
- 1,000 actual host-thread races among IRQ, watchdog and abort producers using a native host mutex, with one queued completion, one request completion and one framework finish each. Completion runs after those producers join in these race cases; the separate active-timeout case covers overlap during drain.

The runner also checks the exact start/unlock/return source tail and checks STREAMON's lock-protected abort-latch reset precedes framework streaming update, with no reset in `device_run`. These are source invariants, not an executed STREAMON test. Existing extracted pre-start tests now pass 17 rejection cases, three handoffs and two JPEG/decoder bypass checks. The extracted IRQ status tests pass 50 cases including clocks-off H.264 suppression.

Local tests used no sanitizers. The isolated ARM64 compile workflow is prepared to run these tests with ASan/UBSan and compile the candidate kernel objects; that new run has not yet been executed. Host binaries stay in the disposable work directory; uploaded diagnostics contain only logs, JSON and extracted source. This packet makes no full-kernel, module-load, hardware-encode, bitstream-correctness or performance claim.

```sh
python3 experiments/rk3568-encoder/job_lifetime_test.py /path/to/patched/linux /tmp/job-test --sanitize
```

Pinned lifecycle source remains Linux `a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea`, particularly `drivers/media/v4l2-core/v4l2-mem2mem.c`: cancellation marks TRANS_ABORT, calls `job_abort` outside `job_spinlock`, and waits for TRANS_RUNNING to clear. Framework finish clears that state and permits buffer/context release; the candidate's drain/detach must precede that boundary.
