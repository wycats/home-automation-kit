# Bounded client: offline foundation

The maintained Rust `encoder-lab` binary prepares synthetic inputs and validates
capture metadata. It has **no device backend**: no enumeration, ioctl, mmap,
network access, driver loading or hardware encoding. The IRQ ownership review
must be resolved before adding device execution. These offline pieces are MIT
code in the root package; the imported kernel prototype remains GPL-2.0.

```
cargo run --locked --bin encoder-lab -- plan
cargo run --locked --bin encoder-lab -- fixture NEW_DIRECTORY 3
cargo run --locked --bin encoder-lab -- fixture NEW_DIRECTORY 2 642 482
```

The fixture command requires a new directory. It never overwrites an existing
fixture. Invalid dimensions/counts are rejected before directory creation.
Files are synthetic:

- `synthetic.nv12`: contiguous padded NV12 frames. Default visible1920x1080,
  coded1920x1088, stride1920, luma2,088,960 bytes, chroma1,044,480 bytes.
  Padding is limited-range black; the visible test pattern moves between frames.
  A future NV12M adapter must split these planes explicitly after checking the
  negotiated format; contiguous NV12 is not the same queue contract as NV12M.
- `headers.h264`: Baseline/CAVLC SPS/PPS for the initial all-IDR envelope, with
  progressive4:2:0 cropping. Four bottom crop units remove eight padded lines.
  Header timing is omitted; timestamps/fps remain an explicit client/container
  contract. The future driver controls must match the documented header fields.
- `crop-test.h264`: one software-built uncompressed I_PCM IDR, using the same
  headers. This exists to verify cropping and sample layout independently. It
  is not a practical compressed clip, throughput benchmark or hardware output.
- `manifest.json`: plan published after successful writes and syncs. An error
  can leave partial files without this marker. Do not consume them as a fixture.
  This is not a power-loss durability guarantee or an untrusted-file validator.

Limits are <=1920x1080 visible, even dimensions, 1–120 frames, <=30fps plan,
<=512MiB raw input and a30-second cooperative fixture deadline. Writes are
checked, but a blocked filesystem write cannot be interrupted by that deadline.
The eventual device runner needs pollable nonblocking operations plus an external
watchdog. The standalone capture budget rejects malformed offset/bytesused,
error buffers, duplicate/backward timestamps, clock regression, too many frames,
>64MiB accumulated output or an expired30-second deadline; any failure is terminal.
Capture checks protect userspace reads, not against kernel DMA overruns.

Rust tests exercise limits, padding, partial-write failures, deadlines and
capture-state errors. `tests/encoder_fixture.py` uses an independent FFmpeg
decoder to compare every visible NV12 byte at1080p, a size cropped on both axes,
and an aligned size. It also checks overwrite refusal and disabled hardware
commands. The generated test vector proves header/crop correctness for that
vector; it does not prove that the experimental Hantro output matches it.

## Remaining adapter work

Before any device backend: resolve the kernel IRQ/watchdog ownership gate and
complete reference-lifetime review. Then pin generated UAPI bindings and assert
sizes/offsets; require explicit device identity and media/request association;
read back all negotiated formats and plane capacities; model request/mmap/FD
ownership with teardown on every failure; use checked nonblocking dequeue and
bounded polling. Start all-IDR with synthetic input. Do not use P references
until successful reconstruction provenance and lifetime are enforced by both
client and driver. Independent output decode must validate frame counts,
dimensions, timestamps, repeat-session cleanup and pixel/quality behavior.

The header writer follows H.264 syntax, cross-checked against
[FFmpeg's parameter-set parser](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/cbs_h264_syntax_template.c).
It is intentionally narrow, not a general encoder or replacement for FFmpeg.
