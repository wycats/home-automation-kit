#!/usr/bin/env bash
# Ephemeral arm64 CI only. Produces files; never writes a boot device.
set -euo pipefail
packet="$(cd -- "$(dirname -- "$0")" && pwd)"
kernel="$(realpath -- "${1:?Kernel payload archive}")"
userspace="$(realpath -- "${2:?Userspace report directory}")"
work="$(realpath -m -- "${3:?New assembly directory}")"
[[ "$(uname -m)" == aarch64 && ! -e "$work" ]]
mkdir -p "$work/kernel" "$work/rootfs" "$work/output"
python3 - "$kernel" "$userspace/userspace-rootfs.tar.xz" "$work" <<'PY'
import pathlib,sys,tarfile
for source,dest in [(sys.argv[1], 'kernel'), (sys.argv[2], 'rootfs')]:
    with tarfile.open(source) as archive:
        archive.extractall(pathlib.Path(sys.argv[3])/dest, filter='data')
PY
python3 "$packet/validate_bundle.py" verify "$work/kernel"
# Userspace must have compiled against the same exported UAPI bytes.
diff -qr "$work/kernel/uapi/include" "$userspace/uapi/include"
cp -a "$work/kernel/rootfs/lib" "$work/rootfs/"
release=$(cat "$work/kernel/kernelrelease")
depmod -b "$work/rootfs" "$release"
for module in rockchip-vdec rockchipdrm; do
  modprobe --dirname "$work/rootfs" --set-version "$release" --show-depends "$module" \
    > "$work/output/$module-dependencies.txt"
done
sudo chroot "$work/rootfs" /bin/busybox sh -n /init
sudo chroot "$work/rootfs" /usr/bin/ffmpeg -v error -i /fixtures/test-720p.mkv -pix_fmt yuv420p -f framemd5 /tmp/replay.md5
cmp "$work/rootfs/fixtures/test-720p.software.framemd5" "$work/rootfs/tmp/replay.md5"
sudo rm -- "$work/rootfs/tmp/replay.md5"
python3 "$packet/rootfs_manifest.py" "$work/rootfs" > "$work/output/rootfs-manifest.json"
# Only this conventional console device node is added; no device is opened.
sudo mknod -m 600 "$work/rootfs/dev/console" c 5 1
(cd "$work/rootfs"; find . -print0 | LC_ALL=C sort -z | sudo cpio --null -o --format=newc --owner=0:0 --reproducible) \
  | gzip -n > "$work/output/initramfs.cpio.gz"
cp "$work/kernel/boot/Image" "$work/kernel/boot/odroid-m1.dtb" "$work/kernel/kernelrelease" "$work/output/"
cp "$work/kernel/manifest.json" "$work/output/kernel-manifest.json"
cp "$packet/README.md" "$work/output/README.md"
cp "$packet/sources.json" "$work/output/userspace-sources.json"
cp "$userspace/packages.txt" "$work/output/userspace-packages.txt"
cp "$userspace/software-smoke.txt" "$work/output/software-smoke.txt"
sha256sum "$work/output/"* > "$work/output-hashes.txt"
echo 'Payload assembled. No bootloader/disk image, hardware decode or deployment validated.'
