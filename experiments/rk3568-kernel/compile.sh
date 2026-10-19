#!/usr/bin/env bash
# Standalone compile/check only. No installation, image assembly, boot or device access.
set -euo pipefail
packet="$(cd -- "$(dirname -- "$0")" && pwd)"
src="$(realpath -- "${1:?Existing full Linux checkout required}")"
out="$(realpath -m -- "${2:?New build directory required}")"
report="$(realpath -m -- "${3:?New report directory required}")"
[[ "$(uname -s)" == Linux && ! -e "$out" && ! -e "$report" ]]
export ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-
for tool in git make gcc "${CROSS_COMPILE}gcc" bc bison flex perl python3 openssl dt-validate rustc; do command -v "$tool" >/dev/null; done
[[ -f "$src/Makefile" && -f "$src/scripts/kconfig/merge_config.sh" ]]
[[ "$(git -C "$src" rev-parse HEAD)" == a2de78ee6ed8bf8cd99d177c74865a6e8c3cb6ea ]]
[[ -z "$(git -C "$src" status --porcelain)" ]]
mkdir -p -- "$out" "$report"
python3 - "$packet" "$src" "$out/test-source-before" <<'PY'
import hashlib,json,pathlib,shutil,subprocess,sys
p=pathlib.Path(sys.argv[1]);s=sys.argv[2];before=pathlib.Path(sys.argv[3]);m=json.loads((p/'manifest.json').read_text())
for f in [*m['configs'],*m['patches']]:
 assert hashlib.sha256((p/f['path']).read_bytes()).hexdigest()==f['sha256'],f['path']
for f in json.loads((p/'tests/source-lock.json').read_text())['files']:
 path=before/f['path'];path.parent.mkdir(parents=True,exist_ok=True)
 shutil.copyfile(pathlib.Path(s)/f['path'],path)
for f in m['patches']:
 subprocess.run(['git','-C',s,'apply','--check',str(p/f['path'])],check=True)
 subprocess.run(['git','-C',s,'apply','--whitespace=error',str(p/f['path'])],check=True)
PY
python3 "$packet/tests/run.py" --clean-tree "$out/test-source-before" --patched-tree "$src" --output "$report/host-tests" --cc gcc 2>&1 | tee "$report/host-tests.log"
mapfile -t configs < <(python3 -c 'import json,pathlib,sys;p=pathlib.Path(sys.argv[1]);print("\n".join(str(p/f["path"]) for f in json.loads((p/"manifest.json").read_text())["configs"]))' "$packet")
KCONFIG_CONFIG="$out/.config" "$src/scripts/kconfig/merge_config.sh" -m -O "$out" "${configs[@]}" 2>&1 | tee "$report/merge.log"
cp -- "$out/.config" "$report/config-before"
make -C "$src" O="$out" olddefconfig 2>&1 | tee "$report/configure.log"
cp -- "$out/.config" "$report/config-after"
"$src/scripts/diffconfig" "$report/config-before" "$report/config-after" > "$report/config-drift.txt"
for setting in CONFIG_VIDEO_ROCKCHIP_VDEC=m CONFIG_ROCKCHIP_IOMMU=y CONFIG_SRAM=y CONFIG_GENERIC_ALLOCATOR=y; do
 grep -qx "$setting" "$out/.config" || { echo "Missing $setting" >&2; exit 3; }
done
make -C "$src" O="$out" -j"${JOBS:-4}" Image modules rockchip/rk3568-odroid-m1.dtb 2>&1 | tee "$report/build.log"
make -C "$src" O="$out" dt_binding_check DT_SCHEMA_FILES=media/rockchip,vdec.yaml 2>&1 | tee "$report/schema.log"
# Explicit board target avoids validating every Rockchip board enabled by the config.
make -C "$src" O="$out" CHECK_DTBS=y rockchip/rk3568-odroid-m1.dtb DT_SCHEMA_FILES=media/rockchip,vdec.yaml 2>&1 | tee "$report/dtb-check.log"
cp -- "$out/arch/arm64/boot/Image" "$report/Image.compile-only"
cp -- "$out/arch/arm64/boot/dts/rockchip/rk3568-odroid-m1.dtb" "$report/odroid-m1.compile-only.dtb"
find "$out" -type f -name '*.ko' -print0 | sort -z | xargs -0 -r sha256sum > "$report/module-hashes.txt"
sha256sum "$report/Image.compile-only" "$report/odroid-m1.compile-only.dtb" > "$report/output-hashes.txt"
echo 'Compilation finished; inspect build/schema diagnostics. No boot or hardware result.'
