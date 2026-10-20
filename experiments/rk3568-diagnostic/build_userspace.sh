#!/usr/bin/env bash
set -euo pipefail
packet="$(cd -- "$(dirname -- "$0")" && pwd)"
work="$(realpath -m -- "${1:?New work directory}")"
report="$(realpath -m -- "${2:?New report directory}")"
[[ "$(uname -m)" == aarch64 && ! -e "$work" && ! -e "$report" ]]
mkdir -p "$work" "$report"
dpkg-query -W > "$report/packages.txt"
gcc --version > "$report/gcc.txt"
python3 "$packet/prepare_ffmpeg.py" "$work/ffmpeg"
# The decoder patches do not alter UAPI; use the exact kernel base for headers.
pin=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["kernel"]["commit"])' "$packet/../rk3568-kernel/manifest.json")
git init "$work/linux"
git -C "$work/linux" fetch --depth=1 https://github.com/gregkh/linux.git "$pin"
git -C "$work/linux" checkout --detach FETCH_HEAD
make -C "$work/linux" ARCH=arm64 INSTALL_HDR_PATH="$work/uapi" headers_install > "$report/headers.log" 2>&1
mkdir "$work/build"
cd "$work/build"
"$work/ffmpeg/configure" --prefix=/usr --disable-autodetect --disable-network \
  --disable-doc --disable-debug --disable-shared --enable-static --enable-gpl \
  --enable-libdrm --enable-libudev --enable-libx264 --enable-libx265 \
  --enable-v4l2-request --disable-vulkan --disable-vaapi --disable-vdpau \
  --extra-cflags="-I$work/uapi/include" > "$report/configure.log" 2>&1
cp config.h config_components.h ffbuild/config.log "$report/"
grep -qx '#define CONFIG_HEVC_V4L2REQUEST_HWACCEL 1' config_components.h
make -j"${JOBS:-4}" > "$report/build.log" 2>&1
root="$work/rootfs"
mkdir -p "$root/usr/bin" "$root/bin" "$root/sbin" "$root/etc" "$root/proc" "$root/sys" "$root/dev" "$root/tmp" "$root/run" "$root/fixtures"
cp ffmpeg ffprobe "$root/usr/bin/"
python3 "$packet/stage_runtime.py" "$root"
cp "$packet/init" "$root/init"
cp "$packet/README.md" "$root/README.txt"
chmod 755 "$root/init"
./ffmpeg -hide_banner -hwaccels > "$report/hwaccels.txt" 2>&1
grep -qx v4l2request "$report/hwaccels.txt"
# Synthetic fixture only; no camera input, device, network or household media.
./ffmpeg -hide_banner -f lavfi -i testsrc2=size=1280x720:rate=10 -frames:v 20 \
  -c:v libx265 -preset ultrafast -x265-params pools=1:frame-threads=1 -pix_fmt yuv420p \
  "$root/fixtures/test-720p.mkv" > "$report/fixture.log" 2>&1
./ffmpeg -v error -i "$root/fixtures/test-720p.mkv" -pix_fmt yuv420p \
  -f framemd5 "$root/fixtures/test-720p.software.framemd5"
./ffprobe -v error -count_frames -show_streams -of json "$root/fixtures/test-720p.mkv" > "$report/fixture.json"
python3 - "$report/fixture.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]))['streams'][0]
assert s['nb_read_frames']=='20' and s['width']==1280 and s['height']==720 and s['pix_fmt']=='yuv420p'
PY
# Chroot is confined to an ephemeral CI runner; these tests mount nothing.
sudo chroot "$root" /usr/bin/ffmpeg -v error -i /fixtures/test-720p.mkv -pix_fmt yuv420p -f framemd5 /tmp/replayed.md5
cmp "$root/fixtures/test-720p.software.framemd5" "$root/tmp/replayed.md5"
sudo chroot "$root" /usr/bin/ffmpeg -v error -i /fixtures/test-720p.mkv -c:v libx264 -threads 2 -pix_fmt yuv420p /tmp/encoded.mp4
sudo chroot "$root" /usr/bin/ffprobe -v error -count_frames -show_entries stream=nb_read_frames -of csv=p=0 /tmp/encoded.mp4 | grep -qx 20
sudo rm -- "$root/tmp/replayed.md5" "$root/tmp/encoded.mp4"
printf '20 HEVC frames replayed in staged root; 20 H.264 output frames. Hardware untested.\n' > "$report/software-smoke.txt"
cp -r "$work/uapi" "$report/uapi"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -C "$root" -cJf "$report/userspace-rootfs.tar.xz" .
# Preserve complete corresponding FFmpeg source and original patch attribution.
tar --exclude=.git -C "$work/ffmpeg" -cJf "$report/ffmpeg-source.tar.xz" .
sha256sum "$report/"*.tar.xz > "$report/archive-hashes.txt"
