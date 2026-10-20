# H.264 reconstruction eligibility

This compile-only follow-on prevents a copied timestamp from making a failed or
reused capture allocation eligible as a P-frame reference. It does not establish
that hardware has stopped DMA, produced the expected surface layout, or encoded
a correct bitstream. Hardware execution remains disabled; IRQ/reset code is
unchanged from the per-job ownership candidate.

## Publication and withdrawal

Each capture buffer carries an eligibility bit, successful source timestamp and
epoch. The context carries a stream/recovery epoch outside the codec union.
`hantro_h264_enc_ref.h` is the exact shared implementation used by the driver and
host tests, not a parallel arithmetic model.

- Job entry withdraws its destination before allocating the job or acquiring PM
  or clocks. Acquisition, request and control-validation failures therefore
  cannot leave the destination eligible. They preserve other references.
- The immutable job records its source timestamp and epoch before metadata copy.
  Publication occurs inside the completion worker's final `irqlock` section,
  after payload validation, codec `done()`, request completion and the second
  abort check. A late abort cannot publish a reference. Epoch mismatch rejects
  publication too.
- A failed **started** job advances the recovery epoch, conservatively rejecting
  all older references. It does not claim those surfaces were all physically
  corrupted. This policy requires an intra frame after a started-job error.
- Buffer initialization, prepare, queue and cleanup withdraw eligibility.
  QBUF withdraws again even when PREPARE_BUF already ran. Reuse of a capture
  buffer therefore ends its availability to future P requests; userspace must
  retain references it still needs. A buffer reencoded successfully gets a new
  successful timestamp.
- STREAMON and STREAMOFF advance the epoch. Either queue's restart invalidates
  old references. Counter overflow poisons the context permanently until close;
  it cannot wrap into an old epoch. STREAMON returns EOVERFLOW and job entry
  refuses an exhausted/zero domain.
- Lookup scans allocated capture slots under `irqlock`, accepting exactly one
  successful match in the current epoch. Missing, self, duplicate, old-epoch,
  missing-storage and undersized references fail before register programming.
  Generic `copied_timestamp` is no longer an eligibility criterion.

## Allocation and locking boundary

Eligibility metadata is protected by `irqlock`. This alone cannot retain a DMA
allocation. The isolated H.264 lab path additionally allows **MMAP capture only**
and rejects nonzero capture CREATE_BUFS/REMOVE_BUFS while **either** queue streams.
Zero-count queries delegate unchanged. Raw OUTPUT, JPEG and decoder paths bypass
these new guards. REQBUFS retains the core's streaming check, including count0.
No new capability is enabled in the client.

Hantro's video-device ioctl lock and both vb2 queue locks are `vpu_mutex`
(`hantro_drv.c` queue setup/video-device setup). This serializes allocation ioctls
with STREAMON/OFF; it does not serialize asynchronous completion, hence the
separate metadata spinlock. Once both queues can run a job, capture slot storage
and reconstruction allocations cannot change until streamoff drains the job.
Both source and destination formats already reject changes while either encoder
queue owns buffers. They cannot switch an allocated DMABUF capture queue from
JPEG into H.264, or resize reconstruction underneath allocated capture buffers.

QBUF or PREPARE_BUF of a dequeued reference only clears eligibility and updates
capture payload metadata. It does not free, reallocate or write `rec_buf`.
The one active mem2mem job can retain the selected allocation after lookup's lock
is released; a later queued job cannot overwrite it until the current job retires.
Reconstruction is a separate internal coherent allocation, not the userspace
MMAP encoded-output plane. Userspace writing mapped bitstream bytes therefore
does not directly mutate reconstruction. Exporting the encoded output does not
export that internal reconstruction allocation.

### Pinned framework audit

These claims are tied to Linux7.2.11 commit
`a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea`, not inferred from host mocks:

- [`vb2_core_reqbufs`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/common/videobuf2/videobuf2-core.c#L867)
  checks `q->streaming` before handling count0 (lines877–880). Nonstreaming
  count0 cancels queued buffers and frees storage (lines889–914).
- [`vb2_core_remove_bufs`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/common/videobuf2/videobuf2-core.c#L1720)
  checks only existence/range and DEQUEUED state before freeing (lines1720–1755),
  not streaming. A dequeued successful reference can still be used by an active
  job, which is why the new REMOVE guard is necessary.
- [`v4l2_m2m_streamoff`](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/v4l2-core/v4l2-mem2mem.c#L869)
  calls `v4l2_m2m_cancel_job` before `vb2_streamoff` (lines877–881). Cancellation
  invokes job_abort and waits for TRANS_RUNNING to clear (lines416–431).
  Thus capture STREAMOFF drains the active job before capture becomes
  nonstreaming, even if OUTPUT remains streaming. Subsequent capture REQBUFS(0)
  can safely follow the software drain. Context release also cancels before
  releasing either queue (lines1283–1288).
- [`vb2` imported-memory preparation](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/media/common/videobuf2/videobuf2-core.c#L1284)
  can invoke buf_cleanup on memory reacquisition; MMAP capture avoids that route.
  Driver init invalidates before allocation, and a failed allocation remains
  ineligible. Cleanup invalidates before freeing and zeroes the allocation.

SHA256 of the exact audited public source bytes:

| File | SHA256 |
| --- | --- |
| videobuf2-core.c | `86222aaa61047a3ac9ecefec2f1ed4a9c3392026120009c59e862767c4618228` |
| v4l2-mem2mem.c | `8cba59ee8aec8d14f7f2b0c81829fe188d25c99399de613f3d23a5cf5eb95854` |
| videobuf2-v4l2.c | `60b388cd10837ae13187f9047f7e5c9b03af46a1116485789fef57a7ba682844` |

These are software-lifetime arguments. The existing reset writes have **not**
been shown to fence outstanding hardware DMA. Late untagged IRQs, DMA quiescence,
unbind/suspend and actual reconstruction surface correctness remain separate
gates. Invalidation cannot make a still-writing engine safe to free.

## Focused validation

`job_lifetime_test.py` extracts actual job/claim/finish paths and actual reference
preflight. Its 18 deterministic scenarios include successful completion followed
by a P request against the completed surface, allocation/PM/clock/setup failures,
payload rejection, late abort after request completion, exhaustion, immediate
context release and next-job reuse. It also exercises an active-timeout drain and
1000 three-producer native host-thread races. Resource/workqueue/MMIO APIs are
host doubles; this is not a kernel concurrency test.

`reference_test.py` extracts actual lookup, buffer init/prepare/queue/cleanup and
allocation ioctl functions. It checks duplicate/self/missing/old references,
PREPARE then QBUF, retained allocation bytes after requeue, failed allocation,
cleanup/reallocation, timestamp reuse, epoch overflow, and H.264 capture guards
with JPEG/decoder/OUTPUT bypass. REQBUFS delegation uses a core-contract double;
the preceding source audit is the evidence for the real framework semantics.
STREAMON/OFF epoch hooks and format guards are source invariants, not execution
of full vb2 streaming callbacks. Tests do not emulate physical DMA.

The existing pre-start fault suite still runs unchanged scenarios against the
new lookup and metadata path. Local Windows ARM64 tests pass without sanitizers.
`compile.sh` adds ASan/UBSan host tests on Linux plus the existing ARM64 candidate
driver-object/UAPI compile. Until that CI runs, this increment does not claim a
successful Linux kernel compile. Diagnostics retain extracted source and result
hashes; host executables stay out of uploaded reports.
