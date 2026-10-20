# Runtime PM before reset

The experimental tree now includes the teardown change from media.git commit
[`8e26d4c20ed218db6c3123480a4a4e2545680566`](https://git.linuxtv.org/media.git/commit/?id=8e26d4c20ed218db6c3123480a4a4e2545680566),
“media: hantro: disable runtime PM before hardware teardown,” authored by
Tharit Tangkijwanichakul. It was reviewed by Benjamin Gaignard and signed off
upstream by Nicolas Dufresne and Hans Verkuil. The exact commit was fetched
from the upstream Git repository and inspected. Its provenance is also recorded
in `manifest.json`; the code is folded into the existing derived patch.

The reported upstream failure is an RK3588 SError: asserting reset before
`pm_runtime_dont_use_autosuspend()` allows the latter's immediate power-domain
transition to touch hardware after reset. The same ordering is present in our
pinned 7.2.11 source. This is a shared Hantro cleanup correction, not evidence
that the reported crash has occurred on our RK3568.

The change moves reset assertion after runtime-PM shutdown on remove and all
probe failures after successful reset deassertion. Failed deassertion retains
the separate PM-only unwind. The original probe error is preserved. No clock,
reset binding, IRQ, IOMMU, codec or runtime-PM callback implementation is changed.

## What the tests establish

`teardown_test.py` extracts the real probe error-label tail and complete remove
function. It derives each failure entry from the corresponding operation's
actual `goto`. Its C doubles exercise seven probe failures and removal with
normal, failing and absent optional resets: 24 cases. They check that PM
callbacks complete before reset, that acquired framework resources are released
once, and that probe errors survive cleanup.

The old candidate fails the same test at reset-before-PM. The new candidate
passes locally on Windows ARM64 without device access. CI also runs this test
with ASan/UBSan and compiles the complete affected ARM64 Hantro/control objects.
CI results must be inspected separately before claiming that validation passed.

This is not a hardware emulator. In particular, the test does not establish
genpd behavior, DMA idle, reset success, IRQ deassertion or safe open-file removal.
The upstream code still ignores reset-assertion failure and permits absent
optional reset handles; the tests deliberately expose those as separate cases,
not successful hardware recovery.

## Remaining implementation boundary

The [H.264 admission gate](ADMISSION-BOUNDARY.md) is unchanged. This patch must
not be used as justification to enable DMA. Safe recovery still requires:

1. A shared-domain owner that excludes competing clients and coordinates with
   the IOMMU driver. A private encoder mutex cannot provide that ownership.
2. A bounded, checked bus-idle/ack protocol with an authoritative guarantee for
   outstanding transactions, plus required reset handles. Runtime suspend's
   return value and an optional reset array do not provide that guarantee.
3. Process-context IRQ draining with retained IRQ identities, outside the spinlock;
   checked reset and IOMMU restoration; and a post-reset status contract.
4. Buffer and device lifetimes that survive a failed fence across close,
   STREAMOFF, unbind and sleep. Returning a vb2 buffer or dropping its allocation
   while DMA remains possible is not an acceptable terminal-error strategy.

The next implementation should live at the owner of the shared recovery
resources, with fallible stages and fault injection. The encoder can consume
that operation only after it provides a real fence. Adding an encoder-local
“quiesced” flag or treating timeout as success would hide the missing contract.
See [DMA-RESET-BOUNDARY.md](DMA-RESET-BOUNDARY.md) for pinned source evidence.
