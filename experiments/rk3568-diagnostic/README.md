# ODROID-M1 diagnostic payload

This builds a matching kernel/module bundle, patched arm64 FFmpeg and a RAM-root
initramfs for a future isolated HEVC decoder test. It does not install software
on an HA host, write media, configure a bootloader or produce an HAOS update.

The first artifact is **not a bootable disk image**. Board-specific bootloader
packaging and load-address/size review remain separate work. No hardware decode,
DMA safety, performance or boot success follows from compilation or chroot tests.
The same M1 cannot run this kernel and the household's current HA kernel at once.

## Build and verification

`kernel-compile.yml` builds the existing decoder experiment and stages installed
modules from that exact build. Every module's release is checked with modinfo;
depmod generates dependency indexes, and modprobe resolves the decoder and DRM
dependencies without loading them. Only allowlisted staging files are archived,
excluding build/source links and generated module-signing private keys.

In parallel, a native arm64 runner builds pinned FFmpeg with 14 hash-verified
V4L2-request patches, Linux headers from the same kernel base, libdrm/libudev and
software x264/x265. The selected kernel patches do not modify UAPI. Assembly
compares both exported header trees and fails on any difference. FFmpeg networking
is disabled. A synthetic 20-frame HEVC fixture is decoded in the staged chroot and
encoded to H.264; frame hashes/counts must match. No hardware device is used.

Runtime executables and their ELF dependency closure are copied explicitly,
with no systemd, DHCP client, credentials, production UUIDs or automount rules.
The initramfs mounts only proc, sysfs, devtmpfs and RAM tmp/run. It opens a serial
shell and loads no modules automatically. Device discovery may occur when the
kernel boots; a RAM root does not contain driver or DMA faults.

Source revisions and patch bytes are pinned. Ubuntu runner images, apt packages
and build timestamps are not yet immutable; recorded versions and manifests do
not establish bit-for-bit reproducibility. Uploaded source and copyright records
support review; this laboratory payload is not a supported binary distribution.

## First hardware test, after boot/recovery review

Use a spare M1, or an explicitly scheduled outage with physical recovery and the
original HA storage protected. No live camera inputs are needed. Prefer the
serial console; HDMI/DRM modules are staged but not automatically loaded.

1. Verify kernel release, DTB and manifest hashes and confirm RAM-only mounts.
2. Manually load `rockchip-vdec`; inspect kernel logs and enumerate media/video
   devices with `media-ctl` and `v4l2-ctl`. Stop on warnings or IOMMU faults.
3. Check the built FFmpeg options and explicitly select `v4l2request` hardware
   frames. Decode the synthetic 8-bit 4:2:0 fixture, download NV12 and normalize
   to yuv420p before comparing visible-plane frame hashes with software output.
   A silent software fallback must fail acceptance.
4. Expand to tiled 4K and changing reference sets only after correctness. Test
   reopen/recovery before measuring throughput, CPU, memory and temperatures.

This work accelerates HEVC decoding only. H.264 encoding remains software. Packed
10-bit NV15/NV20 download needs separate work; begin with 8-bit linear NV12.

## Source and licensing

Original packaging scripts use the repository MIT license. Kernel inputs retain
the licenses and authorship documented in `../rk3568-kernel/NOTICE.md`.
`sources.json` pins FFmpeg n8.1.3 and the selected patches from nyok92/HAOS-rock5b;
the first 13 preserve Jonas Karlman's attribution, and the SPS RPS follow-up
preserves gregordinary's attribution. Full patched FFmpeg source is retained in
CI, including its COPYING files. Enabling GPL x264/x265 makes this FFmpeg build
GPL; it is not covered by the repository's MIT license. Ubuntu runtime package
versions and copyright notices accompany the userspace payload. Audit complete
corresponding-source obligations before distributing a release to others.
