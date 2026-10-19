//! Device-free preparation and capture validation for the isolated Hantro lab.
//!
//! No ioctl backend is enabled. These checks do not establish kernel DMA safety.
mod headers;
use serde::Serialize;
use std::{io, io::Write, ops::Range, time::Duration};

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    visible_width: u32,
    visible_height: u32,
    coded_width: u32,
    coded_height: u32,
    stride: usize,
    luma_bytes: usize,
    chroma_bytes: usize,
    frame_bytes: usize,
    frames: u32,
    fps: u32,
    fixture_bytes: u64,
    capture_limit_bytes: u64,
    deadline_ms: u64,
    crop_right_420_units: u32,
    crop_bottom_420_units: u32,
    hardware_backend_enabled: bool,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl Plan {
    /// Initial lab envelope: progressive 8-bit 4:2:0, <=1080p30, <=120 frames.
    /// Crop units apply only to progressive 4:2:0 SPS, not other chroma formats.
    pub fn new(width: u32, height: u32, frames: u32, fps: u32) -> io::Result<Self> {
        if !(16..=1920).contains(&width)
            || !(16..=1080).contains(&height)
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
            || !(1..=120).contains(&frames)
            || !(1..=30).contains(&fps)
        {
            return Err(invalid("outside bounded progressive 4:2:0 lab envelope"));
        }
        let coded_width = width.div_ceil(16) * 16;
        let coded_height = height.div_ceil(16) * 16;
        let stride = coded_width as usize;
        let luma_bytes = stride
            .checked_mul(coded_height as usize)
            .ok_or_else(|| invalid("luma allocation overflow"))?;
        let chroma_bytes = luma_bytes / 2;
        let frame_bytes = luma_bytes
            .checked_add(chroma_bytes)
            .ok_or_else(|| invalid("frame allocation overflow"))?;
        let fixture_bytes = (frame_bytes as u64)
            .checked_mul(frames as u64)
            .ok_or_else(|| invalid("fixture allocation overflow"))?;
        if fixture_bytes > 512 * 1024 * 1024 {
            return Err(invalid("fixture exceeds 512 MiB"));
        }
        Ok(Self {
            visible_width: width,
            visible_height: height,
            coded_width,
            coded_height,
            stride,
            luma_bytes,
            chroma_bytes,
            frame_bytes,
            frames,
            fps,
            fixture_bytes,
            capture_limit_bytes: 64 * 1024 * 1024,
            deadline_ms: 30_000,
            crop_right_420_units: (coded_width - width) / 2,
            crop_bottom_420_units: (coded_height - height) / 2,
            hardware_backend_enabled: false,
        })
    }

    pub fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }
    pub fn frames(&self) -> u32 {
        self.frames
    }
    pub fn fixture_bytes(&self) -> u64 {
        self.fixture_bytes
    }

    /// One contiguous NV12 frame; padding is deterministic limited-range black.
    /// The future driver adapter must split planes if NV12M is negotiated.
    pub fn frame(&self, index: u32) -> io::Result<Vec<u8>> {
        if index >= self.frames {
            return Err(invalid("frame index exceeds plan"));
        }
        let mut bytes = vec![16; self.frame_bytes];
        bytes[self.luma_bytes..].fill(128);
        let width = self.visible_width as usize;
        let height = self.visible_height as usize;
        let moving_x = index as usize * 7 % width;
        for y in 0..height {
            for x in 0..width {
                let bar = x * 8 / width;
                bytes[y * self.stride + x] = if x.abs_diff(moving_x) < 8 && y < 32 {
                    235
                } else {
                    24 + bar as u8 * 26
                };
            }
        }
        for y in 0..height / 2 {
            for x in (0..width).step_by(2) {
                let offset = self.luma_bytes + y * self.stride + x;
                bytes[offset] = 80 + (x * 8 / width) as u8 * 12;
                bytes[offset + 1] = 176 - (x * 8 / width) as u8 * 12;
            }
        }
        Ok(bytes)
    }

    /// Checked writes, one frame of memory. Clock checked between frames.
    /// A blocking filesystem write cannot be interrupted by this cooperative clock.
    pub fn write_fixture<W: Write>(
        &self,
        sink: &mut W,
        mut elapsed: impl FnMut() -> Duration,
    ) -> io::Result<()> {
        for index in 0..self.frames {
            if elapsed() >= Duration::from_millis(self.deadline_ms) {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "fixture deadline"));
            }
            sink.write_all(&self.frame(index)?)?;
        }
        sink.flush()?;
        if elapsed() >= Duration::from_millis(self.deadline_ms) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "fixture deadline"));
        }
        Ok(())
    }

    pub fn capture_budget(&self) -> CaptureBudget {
        CaptureBudget {
            frame_limit: self.frames,
            byte_limit: self.capture_limit_bytes,
            deadline: Duration::from_millis(self.deadline_ms),
            frames: 0,
            bytes: 0,
            last_timestamp_ns: None,
            last_elapsed: Duration::ZERO,
            failed: false,
        }
    }
}

/// Validate dequeued metadata before reading a mapped buffer. This is separate
/// from DMA protection: a bad kernel could already have overwritten that mapping.
pub struct CaptureBudget {
    frame_limit: u32,
    byte_limit: u64,
    deadline: Duration,
    frames: u32,
    bytes: u64,
    last_timestamp_ns: Option<u64>,
    last_elapsed: Duration,
    failed: bool,
}

impl CaptureBudget {
    /// This initial client admits one complete access unit per capture buffer.
    /// Timestamp is the driver's copied monotonic input timestamp, not DTS guesses.
    pub fn accept(
        &mut self,
        elapsed: Duration,
        timestamp_ns: u64,
        capacity: usize,
        data_offset: usize,
        bytes_used: usize,
        buffer_error: bool,
    ) -> io::Result<Range<usize>> {
        let result = self.validate(
            elapsed,
            timestamp_ns,
            capacity,
            data_offset,
            bytes_used,
            buffer_error,
        );
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn validate(
        &mut self,
        elapsed: Duration,
        timestamp_ns: u64,
        capacity: usize,
        data_offset: usize,
        bytes_used: usize,
        buffer_error: bool,
    ) -> io::Result<Range<usize>> {
        if self.failed {
            return Err(invalid("capture session already failed"));
        }
        if elapsed >= self.deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "capture deadline"));
        }
        if elapsed < self.last_elapsed
            || buffer_error
            || data_offset >= bytes_used
            || bytes_used > capacity
        {
            return Err(invalid("invalid capture buffer or clock"));
        }
        if self.frames >= self.frame_limit
            || self.last_timestamp_ns.is_some_and(|t| timestamp_ns <= t)
        {
            return Err(invalid("capture count or timestamp violation"));
        }
        let bytes = self
            .bytes
            .checked_add((bytes_used - data_offset) as u64)
            .ok_or_else(|| invalid("capture byte count overflow"))?;
        if bytes > self.byte_limit {
            return Err(invalid("capture output limit"));
        }
        self.frames += 1;
        self.bytes = bytes;
        self.last_timestamp_ns = Some(timestamp_ns);
        self.last_elapsed = elapsed;
        Ok(data_offset..bytes_used)
    }

    pub fn finish(&self) -> io::Result<()> {
        if self.failed || self.frames != self.frame_limit {
            return Err(invalid("capture incomplete or failed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_hd_layout_and_padding() {
        let p = Plan::new(1920, 1080, 3, 30).unwrap();
        assert_eq!((p.coded_width, p.coded_height), (1920, 1088));
        assert_eq!(
            (p.luma_bytes, p.chroma_bytes, p.frame_bytes),
            (2_088_960, 1_044_480, 3_133_440)
        );
        assert_eq!(p.crop_bottom_420_units, 4);
        let frame = p.frame(0).unwrap();
        assert!(frame[1920 * 1080..p.luma_bytes].iter().all(|v| *v == 16));
        assert!(frame[p.luma_bytes + 1920 * 540..].iter().all(|v| *v == 128));
        assert_ne!(frame, p.frame(1).unwrap());
        assert!(p.frame(3).is_err());
    }

    #[test]
    fn rejects_outside_envelope_and_handles_both_crop_axes() {
        for (w, h, n, fps) in [
            (0, 1080, 3, 30),
            (u32::MAX, 1080, 3, 30),
            (1920, 1081, 3, 30),
            (1919, 1080, 3, 30),
            (1920, 1080, 0, 30),
            (1920, 1080, 121, 30),
            (1920, 1080, 3, 0),
            (1920, 1080, 3, 31),
        ] {
            assert!(Plan::new(w, h, n, fps).is_err());
        }
        let p = Plan::new(642, 482, 1, 1).unwrap();
        assert_eq!((p.crop_right_420_units, p.crop_bottom_420_units), (7, 7));
        let f = p.frame(0).unwrap();
        assert!(f[642..656].iter().all(|v| *v == 16));
    }

    #[test]
    fn checked_fixture_writes_propagate_failures_and_deadlines() {
        struct ShortWriter {
            left: usize,
        }
        impl Write for ShortWriter {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                if self.left == 0 {
                    return Err(io::Error::other("injected disk failure"));
                }
                let n = b.len().min(self.left).min(13);
                self.left -= n;
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let p = Plan::new(16, 16, 2, 1).unwrap();
        let mut good = Vec::new();
        p.write_fixture(&mut good, || Duration::ZERO).unwrap();
        assert_eq!(good.len() as u64, p.fixture_bytes());
        assert!(
            p.write_fixture(
                &mut ShortWriter {
                    left: p.frame_bytes() + 1
                },
                || Duration::ZERO
            )
            .is_err()
        );
        assert_eq!(
            p.write_fixture(&mut Vec::new(), || Duration::from_secs(30))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn capture_rejects_invalid_metadata_and_failure_is_terminal() {
        let p = Plan::new(16, 16, 2, 1).unwrap();
        for (capacity, offset, used, error) in [
            (10, 0, 11, false),
            (10, 10, 10, false),
            (10, 0, 0, false),
            (10, 0, 10, true),
        ] {
            let mut b = p.capture_budget();
            assert!(
                b.accept(Duration::ZERO, 0, capacity, offset, used, error)
                    .is_err()
            );
            assert!(b.accept(Duration::ZERO, 1, 10, 0, 10, false).is_err());
            assert!(b.finish().is_err());
        }
        let mut b = p.capture_budget();
        assert_eq!(
            b.accept(Duration::ZERO, 0, 10, 2, 10, false).unwrap(),
            2..10
        );
        assert!(b.finish().is_err());
        assert!(
            b.accept(Duration::from_secs(1), 1, 10, 0, 10, false)
                .is_ok()
        );
        assert!(b.finish().is_ok());
        assert!(
            b.accept(Duration::from_secs(2), 2, 10, 0, 10, false)
                .is_err()
        );
    }

    #[test]
    fn capture_caps_time_bytes_and_timestamp_order() {
        let p = Plan::new(16, 16, 2, 1).unwrap();
        let mut b = p.capture_budget();
        assert!(
            b.accept(Duration::from_secs(30), 0, 10, 0, 10, false)
                .is_err()
        );
        let mut b = p.capture_budget();
        assert!(
            b.accept(
                Duration::ZERO,
                0,
                64 * 1024 * 1024 + 1,
                0,
                64 * 1024 * 1024 + 1,
                false
            )
            .is_err()
        );
        for timestamp in [9, 10] {
            let mut b = p.capture_budget();
            b.accept(Duration::from_secs(1), 10, 10, 0, 10, false)
                .unwrap();
            assert!(
                b.accept(Duration::from_secs(2), timestamp, 10, 0, 10, false)
                    .is_err()
            );
        }
        let mut b = p.capture_budget();
        b.accept(Duration::from_secs(1), 0, 10, 0, 10, false)
            .unwrap();
        assert!(b.accept(Duration::ZERO, 1, 10, 0, 10, false).is_err());
    }
}
