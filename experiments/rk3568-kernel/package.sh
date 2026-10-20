#!/usr/bin/env bash
# Stage a diagnostic payload from this build only; never install on the host.
set -euo pipefail
packet="$(cd -- "$(dirname -- "$0")" && pwd)"
src="$(realpath -- "${1:?Linux source}")"
out="$(realpath -- "${2:?Kernel build}")"
report="$(realpath -- "${3:?Report}")"
stage="$out/diagnostic-payload"
[[ ! -e "$stage" ]]
mkdir -p "$stage/boot" "$stage/rootfs"
release=$(make -s -C "$src" O="$out" ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- kernelrelease)
[[ "$release" =~ ^[a-zA-Z0-9._+-]+$ ]]
printf '%s\n' "$release" > "$stage/kernelrelease"
make -C "$src" O="$out" ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- \
  INSTALL_MOD_PATH="$stage/rootfs" DEPMOD=true modules_install
# Kernel build/source links point outside the payload and are never shipped.
for link in build source; do
  path="$stage/rootfs/lib/modules/$release/$link"
  if [[ -L "$path" ]]; then unlink "$path"; fi
done
depmod -b "$stage/rootfs" -F "$out/System.map" "$release"
cp "$out/arch/arm64/boot/Image" "$stage/boot/Image"
cp "$out/arch/arm64/boot/dts/rockchip/rk3568-odroid-m1.dtb" "$stage/boot/odroid-m1.dtb"
make -C "$src" O="$out" ARCH=arm64 INSTALL_HDR_PATH="$stage/uapi" headers_install
python3 "$packet/../rk3568-diagnostic/validate_bundle.py" kernel "$stage"
for module in rockchip-vdec rockchipdrm; do
  modprobe --dirname "$stage/rootfs" --set-version "$release" --show-depends "$module" \
    > "$report/$module-dependencies.txt"
done
# Archive only the whitelisted staging tree, never the build/signing directory.
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  -C "$stage" -cJf "$report/kernel-payload.tar.xz" .
sha256sum "$report/kernel-payload.tar.xz" > "$report/kernel-payload.sha256"
