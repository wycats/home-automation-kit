#!/usr/bin/env bash
# Compile public sources only. Never run the encoder or publish its binary.
set -euo pipefail
packet="$(cd -- "$(dirname -- "$0")" && pwd)"
work="$(realpath -m -- "${1:?new work directory}")"
report="$(realpath -m -- "${2:?new report directory}")"
[[ "$(uname -m)" == aarch64 && ! -e "$work" && ! -e "$report" ]]
mkdir -p "$work" "$report"
python3 - "$packet" <<'PY'
import hashlib, json, pathlib, sys
p = pathlib.Path(sys.argv[1]); m = json.loads((p/'manifest.json').read_text())
assert hashlib.sha256((p/m['patch']['path']).read_bytes()).hexdigest() == m['patch']['sha256']
PY
value() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]][sys.argv[3]])' "$packet/manifest.json" "$1" "$2"; }
git init "$work/linux"
git -C "$work/linux" fetch --depth=1 "$(value kernel repository)" "$(value kernel commit)"
git -C "$work/linux" checkout --detach FETCH_HEAD
test "$(git -C "$work/linux" rev-parse HEAD)" = "$(value kernel commit)"
git -C "$work/linux" apply --check --whitespace=error "$packet/$(value patch path)"
git -C "$work/linux" apply --whitespace=error "$packet/$(value patch path)"
cd "$work/linux"
make ARCH=arm64 defconfig > "$report/config.log" 2>&1
scripts/config --enable MEDIA_SUPPORT --enable MEDIA_PLATFORM_SUPPORT --enable MEDIA_PLATFORM_DRIVERS --enable VIDEO_DEV --enable V4L_MEM2MEM_DRIVERS --module VIDEO_HANTRO --enable VIDEO_HANTRO_ROCKCHIP
make ARCH=arm64 olddefconfig >> "$report/config.log" 2>&1
cp .config "$report/resolved.config"
grep -qx CONFIG_VIDEO_HANTRO=m .config
grep -qx CONFIG_VIDEO_HANTRO_ROCKCHIP=y .config
# Directory build compiles/links driver objects without a full kernel or modpost.
make ARCH=arm64 -j2 drivers/media/platform/verisilicon/ drivers/media/v4l2-core/v4l2-ctrls-core.o drivers/media/v4l2-core/v4l2-ctrls-defs.o > "$report/build.log" 2>&1
test -s drivers/media/platform/verisilicon/rockchip_vpu_hw_h264_enc.o
test -s drivers/media/platform/verisilicon/hantro-vpu.o
file drivers/media/platform/verisilicon/hantro-vpu.o > "$report/objects.txt"
nm drivers/media/platform/verisilicon/hantro-vpu.o > "$report/symbols.txt"
grep -q ' T rk3399_vpu_h264_enc_run$' "$report/symbols.txt"
grep -q ' T hantro_h264_enc_prepare_run$' "$report/symbols.txt"
make ARCH=arm64 INSTALL_HDR_PATH="$work/uapi" headers_install > "$report/headers.log" 2>&1
gcc -Wall -Wextra -Werror -I"$work/uapi/include" "$packet/abi.c" -o "$work/abi"
"$work/abi" > "$report/abi.json"
gcc -Wall -Wextra -Werror -fsanitize=address,undefined -g \
  -I"$work/uapi/include" -I"$work/linux/drivers/media/platform/verisilicon" \
  "$packet/validation_test.c" -o "$work/validation-test"
"$work/validation-test" > "$report/validation-tests.txt" 2>&1
git init "$work/client"
git -C "$work/client" fetch --depth=1 "$(value client repository)" "$(value client commit)"
git -C "$work/client" checkout --detach FETCH_HEAD
test "$(git -C "$work/client" rev-parse HEAD)" = "$(value client commit)"
# Old client headers relied on transitive stdint declarations from Linux headers.
make -C "$work/client" -j2 CFLAGS="-include stdint.h -I. -I$work/uapi/include $(pkg-config --cflags cairo libudev) -O2 -Wall -Wextra" > "$report/client-build.log" 2>&1
file "$work/client/v4l2-hantro-h264-encoder" > "$report/client-elf.txt"
readelf -d "$work/client/v4l2-hantro-h264-encoder" > "$report/client-dynamic.txt"
sha256sum "$work/client/v4l2-hantro-h264-encoder" > "$report/client-hash.txt"
printf 'Kernel objects and matching legacy client compiled. No module load, device access, hardware encoding or boot test. Client binary deliberately omitted.\n' > "$report/RESULT.txt"
cp "$packet/manifest.json" "$report/manifest.json"
