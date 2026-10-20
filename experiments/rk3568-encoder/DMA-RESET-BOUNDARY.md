# VEPU IRQ qualification and DMA/reset boundary

This increment changes only H.264 IRQ status qualification. It does not add a
reset, IOMMU or power-domain operation, and does not enable hardware execution.
The remainder is a source-backed design boundary for the separate experiment.

## Contained correction

The candidate previously interpreted FRAME_READY/error flags without requiring
bit 0 of VEPU_REG_INTERRUPT. Pinned Rockchip vendor VEPU2 source names bit 0
INT_RAW and returns IRQ_NONE before acknowledgment when it is clear. The
candidate already defines the same bit as VEPU_REG_INTERRUPT_BIT.

For H.264 with MMIO available, the handler now requires that bit before clearing
interrupt/AXI registers or calling the job claimant. Without it, the handler
unlocks and returns IRQ_NONE. JPEG retains its previous handling; the clock-off
H.264 guard still performs no MMIO and returns IRQ_HANDLED. A real raw IRQ with
TIMEOUT/BUFFER_FULL/BUS_ERROR still overrides simultaneous FRAME_READY.

`irq_dispatch_test.py` executes the actual candidate handlers and register
definitions with register/claim/framework doubles: all 32 combinations of raw,
ready and three errors across H.264 MMIO-live, H.264 MMIO-off and JPEG, plus two
older VEPU1 cases (98 total). It checks the early exit performs one status read,
no writes and no claim, and that clock-off handling performs no read. Generated
test source retains the original driver's license/attribution header. Linux CI
adds ASan/UBSan; local Windows tests are unsanitized. This is not a hardware or
concurrency test, nor evidence that an old **real** IRQ belongs to the new job.

## Pinned hardware identity and vendor ordering

Vendor kernel commit `77168c8d5ab82399f65a80e9f807b50ba37cf483`:

- [RK356x DT](https://github.com/rockchip-linux/kernel/blob/77168c8d5ab82399f65a80e9f807b50ba37cf483/arch/arm64/boot/dts/rockchip/rk356x.dtsi#L1417)
  places vpu-encoder-v2 at 0xfdee0000, GIC SPI 64 level-high, ACLK_JENC/HCLK_JENC,
  reset lines SRST_A_JENC/SRST_H_JENC and PD_RGA. This is the Hantro VEPU block,
  not the separate RKVENC block.
- [VEPU2 register definitions and IRQ](https://github.com/rockchip-linux/kernel/blob/77168c8d5ab82399f65a80e9f807b50ba37cf483/drivers/video/rockchip/mpp/mpp_vepu2.c#L59)
  use interrupt offset 0x1b4, raw bit 0, clear bit 9 and disable bit 8; the IRQ
  function at lines 410–419 tests raw before acknowledging.
- [VEPU2 reset](https://github.com/rockchip-linux/kernel/blob/77168c8d5ab82399f65a80e9f807b50ba37cf483/drivers/video/rockchip/mpp/mpp_vepu2.c#L859)
  stops ENCODE_EN, delays 5 µs, requests PMU idle, asserts A/H resets, delays 5 µs,
  deasserts resets, releases idle and clears interrupt status. Its own comment
  warns that omitting PMU idle breaks IOMMU operation after reset.
- [Reset wrapper](https://github.com/rockchip-linux/kernel/blob/77168c8d5ab82399f65a80e9f807b50ba37cf483/drivers/video/rockchip/mpp/mpp_common.c#L689)
  disables encoder and IOMMU IRQs, locks IOMMU/reset groups, invokes reset,
  refreshes IOMMU then reenables interrupts. It ignores some returned errors;
  its existence is not proof of safe failure handling.
- [IOMMU refresh](https://github.com/rockchip-linux/kernel/blob/77168c8d5ab82399f65a80e9f807b50ba37cf483/drivers/video/rockchip/mpp/mpp_iommu.c#L611)
  calls vendor-exported disable/enable helpers. Those are not mainline Hantro
  interfaces that can simply be called from this candidate.

## What mainline provides—and what it does not prove

All mainline references below are Linux 7.2.11 commit
`a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea`.

### Shared power-domain ownership

[RK356x DT lines 611–639](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/arch/arm64/boot/dts/rockchip/rk356x-base.dtsi#L611)
assign RGA, VEPU and VEPU's IOMMU to PD_RGA. These are declared consumers, not
claims about active devices on any running machine. The domain's request/ack
mask is shared bit 5; it is not a VEPU-only bus-idle request.

[Rockchip PM domain code lines 394–440](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/pmdomain/rockchip/pm-domains.c#L394)
does provide bounded acknowledgment and idle polls (10 ms each), but the helper
is internal. `rockchip_pd_power` requests idle as part of domain shutdown under
the PMU mutex. Exporting or bypassing that helper would require an ownership
design covering RGA and IOMMU clients, not merely adding two reset handles.

[genpd_power_off lines 951–1017](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/pmdomain/core.c#L951)
can decline power-off when other devices are active, a governor disallows it or
other conditions hold. It returns void. `genpd_runtime_suspend` calls it then
returns 0 at lines 1269–1274. Thus a successful device runtime-suspend result is
**not proof that PD_RGA actually powered off or acknowledged idle**. The current
asynchronous autosuspend put is even less suitable as a fence.

### IOMMU stall/reset is not an encoder fence API

[Mainline rockchip-iommu.c](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/iommu/rockchip-iommu.c#L413)
has static stall/paging/reset helpers. Stall polls for an MMU status, returns
success immediately if paging is disabled, and does not itself establish that
all encoder or interconnect writes retired. MMU force-reset polls the MMU's reset
status; this is not the encoder A/H reset.

`rk_iommu_disable` (lines 913–927) explicitly continues after errors, disables
paging, clears DTE/masks IRQs and releases stall. `rk_iommu_identity_attach`
(lines 970–1005) switches to an **identity** domain and calls disable. The driver
ops expose identity and paging domains, not a blocked-domain implementation.
Consequently, detach/reattach is not a fail-closed DMA quarantine. Nor is an IOTLB
flush proof that previously issued bus writes completed. Mainline's private
helpers must not be called through ad hoc exported symbols merely to imitate the
vendor reset wrapper.

### Reset ownership and IRQ synchronization

Mainline's VEPU DT/binding has no reset properties. Hantro's optional exclusive
reset array can therefore be NULL; the reset API permits optional NULL operations
to succeed without asserting hardware. Adding handles needs a binding, DTS and
clock/reset ownership review. Rockchip's [soft-reset provider](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/drivers/clk/rockchip/softrst.c#L24)
writes reset bits and returns; it does not expose an encoder DMA-idle status.
Reset-register readback alone would not supply that missing semantic.

[IRQ synchronization](https://github.com/gregkh/linux/blob/a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea/kernel/irq/manage.c#L129)
waits for handlers and may check interrupt-controller pending state. It cannot
stop a device from asserting another level IRQ or writing memory. `disable_irq`
can sleep and synchronizes: it must run from the separate completion worker,
outside `irqlock` and outside the IRQ being drained. Hantro currently does not
retain its requested IRQ numbers in a recovery structure. The raw qualification
change neither supplies those handles nor changes interrupt disabling.

## Required protocol before repeated hardware jobs

The next reset implementation needs a process-context recovery owner with explicit
fallible stages, rather than adding more writes to the current codec reset:

1. Keep the old immutable job and all DMA allocations attached; stop scheduling
   new jobs. Win ownership once and drain the watchdog outside its own callback.
2. While clocks/PM are retained, mask the encoder source; drop `irqlock` before
   disabling/synchronizing the retained Linux IRQ. Reacquire only for short
   state/MMIO access. IOMMU IRQ coordination must belong to the IOMMU driver.
3. Establish exclusive bus/domain recovery ownership and check bounded idle/ack
   results. The shared PD_RGA graph must prevent concurrent RGA/IOMMU access.
   A driver-local mutex or a successful runtime-suspend call is insufficient.
4. Assert/deassert the correct reset handles with their specified delay; restore
   IOMMU state through a supported, reviewed interface. Check every failure.
5. With MMIO available, verify the documented post-reset status and clear/drain
   pending IRQ state. Only a proven hardware recovery may permit another job.
6. Release buffers/framework ownership/clocks only after the fence succeeds.
   On any unproven or failed stage, do not free DMA memory or run the next job.

The last failure rule is a design requirement, **not implemented quarantine**.
Holding job references alone does not cover close, STREAMOFF, unbind, system
suspend or module removal. The recovery design must pin buffer/context/device
lifetime and define a bounded operator-visible terminal failure policy; it must
not silently hang a production close or let cleanup free still-writable memory.
The current candidate still has these runtime gates, so its backend stays absent.

## Contained next test boundary

There is no production-HA change needed for this source work. A future hardware
trial needs a separately reviewed boot environment and recovery protocol. Use a
spare M1 if available, or an explicitly approved outage with production storage
left untouched, independent boot media, serial console and known power recovery.
The existing compile artifact is not itself that boot image.

Before enabling repeat jobs, the lab DT should identify every PD_RGA consumer and
avoid unrelated multimedia clients; confirm actual reset handles and IRQ/IOMMU
topology in the isolated boot. Instrument the proposed idle, reset and IRQ stages
and fault-inject their failures. Require no new job or allocation release before
the proven fence; test timeout versus IRQ, streamoff/close and repeated sessions.
Log IOMMU faults, raw IRQ status, bounded stage outcomes and guard-buffer checks.
Such checks can find corruption, but do not substitute for the hardware contract.

A one-shot diagnostic that retains all DMA storage until power-off is a possible
separate experiment if the idle contract remains unavailable. It would require
its own reviewed lifecycle implementation and explicit disposable-host approval;
the current mem2mem candidate does not implement that behavior. No unmodified
production kernel or existing recorder should be used to test these assumptions.

## Source integrity

SHA256 of exact public source bytes used for the new fence review:

| Source | SHA256 |
| --- | --- |
| Mainline rockchip-iommu.c | `093da6f9f490cb1536107bfb44f570efc676f56464f05d78c3291531164997d8` |
| Mainline drivers/pmdomain/core.c | `32e2f6b0988eba52a2f662fdc2d7132022fbe980f281b2a82babcf3d7f46e18a` |
| Mainline Rockchip softrst.c | `38f5bd31b0af9b75434c040466b3e8a9e0a3f1cecb2e5635218c29f2fc8a342e` |
| Mainline kernel/irq/manage.c | `1e1c274718c9e5982e75e108948779d22ff7c12daca03bc0ce60d10fc954dcdc` |
| Vendor mpp_iommu.c | `fba98d7a15758dac995494581a7d41c496ded08d9881e1554fdba2e169abace9` |

The register/manual guarantee for complete VEPU DMA quiescence is still missing.
No claim here relies on unobserved runtime behavior, and no hardware was run.
