//! Minimal progressive Baseline/CAVLC parameter sets for the initial all-IDR lab.
//! Not a general H.264 writer. No CABAC, B pictures, HRD or alternate chroma modes.
use super::Plan;

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: u8,
}
impl Bits {
    fn bit(&mut self, value: bool) {
        if self.used == 0 {
            self.bytes.push(0);
        }
        if value {
            *self.bytes.last_mut().unwrap() |= 1 << (7 - self.used);
        }
        self.used = (self.used + 1) % 8;
    }
    fn fixed(&mut self, value: u32, width: u32) {
        for shift in (0..width).rev() {
            self.bit(value & (1 << shift) != 0);
        }
    }
    fn ue(&mut self, value: u32) {
        // All callers are bounded plan fields/constants, well below u32::MAX.
        let code = value + 1;
        let width = 32 - code.leading_zeros();
        for _ in 1..width {
            self.bit(false);
        }
        self.fixed(code, width);
    }
    fn align(&mut self) {
        while self.used != 0 {
            self.bit(false);
        }
    }
    fn nal(mut self, header: u8) -> Vec<u8> {
        self.bit(true);
        self.align();
        let mut output = vec![0, 0, 0, 1, header];
        let mut zeros = 0;
        for value in self.bytes {
            if zeros == 2 && value <= 3 {
                output.push(3);
                zeros = 0;
            }
            output.push(value);
            zeros = if value == 0 { zeros + 1 } else { 0 };
        }
        output
    }
}

impl Plan {
    /// Header contract: Baseline, frame_num=4 bits, POC type 2, one reference,
    /// CAVLC, initial QP26, deblocking control present. A future ioctl adapter
    /// must match these fields; passing synthetic decode does not validate it.
    pub fn annex_b_headers(&self) -> Vec<u8> {
        let mut s = Bits::default();
        s.fixed(66, 8);
        s.fixed(0xc0, 8);
        s.fixed(40, 8);
        s.ue(0); // SPS id
        s.ue(0); // log2_max_frame_num_minus4
        s.ue(2); // pic_order_cnt_type
        s.ue(1);
        s.bit(false); // max refs, no frame_num gaps
        s.ue(self.coded_width / 16 - 1);
        s.ue(self.coded_height / 16 - 1);
        s.bit(true);
        s.bit(true); // frame_mbs_only, direct8x8
        let cropped = self.crop_right_420_units != 0 || self.crop_bottom_420_units != 0;
        s.bit(cropped);
        if cropped {
            s.ue(0);
            s.ue(self.crop_right_420_units);
            s.ue(0);
            s.ue(self.crop_bottom_420_units);
        }
        s.bit(false); // VUI omitted: timing remains an explicit container/client contract
        let mut headers = s.nal(0x67);
        let mut p = Bits::default();
        p.ue(0);
        p.ue(0);
        p.bit(false);
        p.bit(false);
        p.ue(0);
        p.ue(0);
        p.ue(0); // slice groups, ref defaults
        p.bit(false);
        p.fixed(0, 2); // no weighted prediction
        p.ue(0);
        p.ue(0);
        p.ue(0); // signed zero QP/QS/chroma offsets
        p.bit(true);
        p.bit(false);
        p.bit(false);
        headers.extend(p.nal(0x68));
        headers
    }

    /// One software-built I_PCM IDR for independent header/crop verification.
    /// This is an uncompressed test vector, not an accelerated or practical clip.
    /// All macroblocks are PCM; deblocking is disabled for exact pixel comparison.
    pub fn pcm_test_vector(&self, index: u32) -> std::io::Result<Vec<u8>> {
        let nv12 = self.frame(index)?;
        let mut b = Bits::default();
        b.ue(0);
        b.ue(2);
        b.ue(0); // first MB, I slice, PPS id
        b.fixed(0, 4);
        b.ue(index); // IDR resets frame_num; varying idr_pic_id
        b.bit(false);
        b.bit(false); // no_output_of_prior_pics, long_term_reference
        b.ue(0);
        b.ue(1); // signed slice_qp_delta zero, disable deblocking
        for my in 0..self.coded_height as usize / 16 {
            for mx in 0..self.coded_width as usize / 16 {
                b.ue(25);
                b.align(); // I_PCM, pcm_alignment_zero_bit
                for y in 0..16 {
                    let offset = (my * 16 + y) * self.stride + mx * 16;
                    b.bytes.extend_from_slice(&nv12[offset..offset + 16]);
                }
                for component in 0..2 {
                    for y in 0..8 {
                        for x in 0..8 {
                            b.bytes.push(
                                nv12[self.luma_bytes
                                    + (my * 8 + y) * self.stride
                                    + mx * 16
                                    + x * 2
                                    + component],
                            );
                        }
                    }
                }
            }
        }
        let mut output = self.annex_b_headers();
        output.extend(b.nal(0x65));
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_prevents_accidental_start_codes() {
        let b = Bits {
            bytes: vec![0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 3],
            used: 0,
        };
        let n = b.nal(0x67);
        assert!(
            !n[5..]
                .windows(3)
                .any(|w| w == [0, 0, 1] || w == [0, 0, 0] || w == [0, 0, 2])
        );
    }
    #[test]
    fn headers_are_bounded_and_cropped_size_changes_them() {
        let p = Plan::new(1920, 1080, 1, 30).unwrap();
        assert!(p.annex_b_headers().len() < 128);
        assert_ne!(
            p.annex_b_headers(),
            Plan::new(1920, 1072, 1, 30).unwrap().annex_b_headers()
        );
        assert!(p.pcm_test_vector(1).is_err());
    }
}
