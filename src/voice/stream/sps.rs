//! Rewrites the H.264 SPS into the shape WebRTC's receiver would rewrite it
//! to anyway.
//!
//! A WebRTC receiver (the official clients' included) runs every incoming
//! SPS through its VUI rewriter, which adds bitstream restrictions saying no
//! frames are reordered, and drops a video signal type that only repeats the
//! defaults. Without end-to-end encryption that's harmless. With DAVE the SPS
//! travels in the clear but authenticated, so a rewritten SPS fails the
//! frame's tag and the viewer decrypts nothing: a black screen. Sending the
//! SPS already rewritten makes the receiver's pass a no-op.
//!
//! The rewrite always writes bitstream restrictions and never a video signal
//! type, which is a fixed point of WebRTC's rewriter whatever the encoder
//! put there. Viewers lose the colour description, which Discord's own
//! senders don't set either.

use std::borrow::Cow;

use crate::platform::capture::nal_units;

const NAL_SPS: u8 = 7;

/// Profiles whose SPS carries the chroma and bit-depth fields.
const HIGH_PROFILES: [u32; 11] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 144];

/// The frame with its SPS rewritten, or the frame as it was if it has none
/// (or one this can't parse, which is sent as is rather than not at all).
pub(super) fn rewrite_frame(frame: &[u8]) -> Cow<'_, [u8]> {
    let units = nal_units(frame);
    if !units.iter().any(|unit| unit[0] & 0x1F == NAL_SPS) {
        return Cow::Borrowed(frame);
    }

    let mut out = Vec::with_capacity(frame.len() + 16);
    for unit in units {
        out.extend_from_slice(&[0, 0, 0, 1]);
        match (unit[0] & 0x1F == NAL_SPS)
            .then(|| rewrite_sps(unit))
            .flatten()
        {
            Some(sps) => out.extend_from_slice(&sps),
            None => out.extend_from_slice(unit),
        }
    }
    Cow::Owned(out)
}

/// One SPS NAL unit, header byte included, rewritten.
fn rewrite_sps(unit: &[u8]) -> Option<Vec<u8>> {
    let (&header, body) = unit.split_first()?;
    let rbsp = unescape(body);
    let mut r = Reader::new(&rbsp);
    let mut w = Writer::default();

    let profile_idc = r.copy(&mut w, 8)?;
    r.copy(&mut w, 8)?; // constraint flags
    r.copy(&mut w, 8)?; // level_idc
    r.copy_ue(&mut w)?; // seq_parameter_set_id

    if HIGH_PROFILES.contains(&profile_idc) {
        let chroma_format_idc = r.copy_ue(&mut w)?;
        if chroma_format_idc == 3 {
            r.copy(&mut w, 1)?; // separate_colour_plane_flag
        }
        r.copy_ue(&mut w)?; // bit_depth_luma_minus8
        r.copy_ue(&mut w)?; // bit_depth_chroma_minus8
        r.copy(&mut w, 1)?; // qpprime_y_zero_transform_bypass_flag
        if r.copy(&mut w, 1)? == 1 {
            // seq_scaling_matrix_present_flag
            let lists = if chroma_format_idc == 3 { 12 } else { 8 };
            for i in 0..lists {
                if r.copy(&mut w, 1)? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    copy_scaling_list(&mut r, &mut w, size)?;
                }
            }
        }
    }

    r.copy_ue(&mut w)?; // log2_max_frame_num_minus4
    match r.copy_ue(&mut w)? {
        // pic_order_cnt_type
        0 => {
            r.copy_ue(&mut w)?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            r.copy(&mut w, 1)?; // delta_pic_order_always_zero_flag
            r.copy_ue(&mut w)?; // offset_for_non_ref_pic (se, same bits)
            r.copy_ue(&mut w)?; // offset_for_top_to_bottom_field
            for _ in 0..r.copy_ue(&mut w)? {
                r.copy_ue(&mut w)?; // offset_for_ref_frame
            }
        }
        _ => {}
    }

    let max_num_ref_frames = r.copy_ue(&mut w)?;
    r.copy(&mut w, 1)?; // gaps_in_frame_num_value_allowed_flag
    r.copy_ue(&mut w)?; // pic_width_in_mbs_minus1
    r.copy_ue(&mut w)?; // pic_height_in_map_units_minus1
    if r.copy(&mut w, 1)? == 0 {
        // frame_mbs_only_flag
        r.copy(&mut w, 1)?; // mb_adaptive_frame_field_flag
    }
    r.copy(&mut w, 1)?; // direct_8x8_inference_flag
    if r.copy(&mut w, 1)? == 1 {
        // frame_cropping_flag
        for _ in 0..4 {
            r.copy_ue(&mut w)?;
        }
    }

    let vui_present = r.read(1)?;
    w.write(1, 1);
    if vui_present == 0 {
        // aspect ratio, overscan, video signal type, chroma location,
        // timing, NAL HRD, VCL HRD, pic struct: all absent.
        w.write(0, 8);
        w.write(1, 1); // bitstream_restriction_flag
        write_restrictions(&mut w, max_num_ref_frames);
    } else {
        if r.copy(&mut w, 1)? == 1 {
            // aspect_ratio_info_present_flag
            if r.copy(&mut w, 8)? == 255 {
                r.copy(&mut w, 16)?; // sar_width
                r.copy(&mut w, 16)?; // sar_height
            }
        }
        if r.copy(&mut w, 1)? == 1 {
            // overscan_info_present_flag
            r.copy(&mut w, 1)?;
        }
        // The video signal type is read past and left out.
        w.write(0, 1);
        if r.read(1)? == 1 {
            r.read(4)?; // video_format, video_full_range_flag
            if r.read(1)? == 1 {
                r.read(24)?; // colour primaries, transfer, matrix
            }
        }
        if r.copy(&mut w, 1)? == 1 {
            // chroma_loc_info_present_flag
            r.copy_ue(&mut w)?;
            r.copy_ue(&mut w)?;
        }
        if r.copy(&mut w, 1)? == 1 {
            // timing_info_present_flag
            r.copy(&mut w, 32)?; // num_units_in_tick
            r.copy(&mut w, 32)?; // time_scale
            r.copy(&mut w, 1)?; // fixed_frame_rate_flag
        }
        let nal_hrd = r.copy(&mut w, 1)?;
        if nal_hrd == 1 {
            copy_hrd(&mut r, &mut w)?;
        }
        let vcl_hrd = r.copy(&mut w, 1)?;
        if vcl_hrd == 1 {
            copy_hrd(&mut r, &mut w)?;
        }
        if nal_hrd == 1 || vcl_hrd == 1 {
            r.copy(&mut w, 1)?; // low_delay_hrd_flag
        }
        r.copy(&mut w, 1)?; // pic_struct_present_flag

        let restricted = r.read(1)?;
        w.write(1, 1);
        if restricted == 0 {
            write_restrictions(&mut w, max_num_ref_frames);
        } else {
            r.copy(&mut w, 1)?; // motion_vectors_over_pic_boundaries_flag
            r.copy_ue(&mut w)?; // max_bytes_per_pic_denom
            r.copy_ue(&mut w)?; // max_bits_per_mb_denom
            r.copy_ue(&mut w)?; // log2_max_mv_length_horizontal
            r.copy_ue(&mut w)?; // log2_max_mv_length_vertical
            r.read_ue()?; // max_num_reorder_frames
            r.read_ue()?; // max_dec_frame_buffering
            w.write_ue(0);
            w.write_ue(max_num_ref_frames);
        }
    }

    let mut out = vec![header];
    out.extend(escape(&w.finish()));
    Some(out)
}

/// The restrictions WebRTC adds: the spec's defaults, except that no frame
/// is reordered and the decoder holds no more than the reference frames.
fn write_restrictions(w: &mut Writer, max_num_ref_frames: u32) {
    w.write(1, 1); // motion_vectors_over_pic_boundaries_flag
    w.write_ue(2); // max_bytes_per_pic_denom
    w.write_ue(1); // max_bits_per_mb_denom
    w.write_ue(16); // log2_max_mv_length_horizontal
    w.write_ue(16); // log2_max_mv_length_vertical
    w.write_ue(0); // max_num_reorder_frames
    w.write_ue(max_num_ref_frames); // max_dec_frame_buffering
}

fn copy_scaling_list(r: &mut Reader, w: &mut Writer, size: usize) -> Option<()> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            let delta = r.copy_se(w)?;
            next = (last + delta + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

fn copy_hrd(r: &mut Reader, w: &mut Writer) -> Option<()> {
    let cpb_cnt_minus1 = r.copy_ue(w)?;
    r.copy(w, 8)?; // bit_rate_scale, cpb_size_scale
    for _ in 0..=cpb_cnt_minus1 {
        r.copy_ue(w)?; // bit_rate_value_minus1
        r.copy_ue(w)?; // cpb_size_value_minus1
        r.copy(w, 1)?; // cbr_flag
    }
    r.copy(w, 20)?; // four 5-bit delay and offset lengths
    Some(())
}

/// Strips emulation prevention bytes.
fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &byte in data {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// Adds emulation prevention bytes.
fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    let mut zeros = 0;
    for &byte in data {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

struct Reader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = self.data.get(self.bit / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(value)
    }

    fn read_ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.read(zeros)?)
    }

    fn copy(&mut self, w: &mut Writer, count: u32) -> Option<u32> {
        let value = self.read(count)?;
        w.write(value, count);
        Some(value)
    }

    fn copy_ue(&mut self, w: &mut Writer) -> Option<u32> {
        let value = self.read_ue()?;
        w.write_ue(value);
        Some(value)
    }

    fn copy_se(&mut self, w: &mut Writer) -> Option<i64> {
        let value = self.copy_ue(w)?;
        let magnitude = i64::from(value.div_ceil(2));
        Some(if value % 2 == 1 {
            magnitude
        } else {
            -magnitude
        })
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: usize,
}

impl Writer {
    fn write(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            if self.bit.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 == 1 {
                *self.bytes.last_mut().unwrap() |= 0x80 >> (self.bit % 8);
            }
            self.bit += 1;
        }
    }

    fn write_ue(&mut self, value: u32) {
        let coded = u64::from(value) + 1;
        let bits = 64 - coded.leading_zeros();
        self.write(0, bits - 1);
        for i in (0..bits).rev() {
            self.write(((coded >> i) & 1) as u32, 1);
        }
    }

    /// The RBSP with its stop bit and zero padding.
    fn finish(mut self) -> Vec<u8> {
        self.write(1, 1);
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A baseline 1280x720 SPS with no VUI, as many encoders write one.
    fn bare_sps() -> Vec<u8> {
        let mut w = Writer::default();
        w.write(66, 8); // profile_idc
        w.write(0xC0, 8); // constraint flags
        w.write(31, 8); // level_idc
        w.write_ue(0); // seq_parameter_set_id
        w.write_ue(0); // log2_max_frame_num_minus4
        w.write_ue(2); // pic_order_cnt_type
        w.write_ue(1); // max_num_ref_frames
        w.write(0, 1); // gaps
        w.write_ue(79); // width in mbs - 1
        w.write_ue(44); // height in map units - 1
        w.write(1, 1); // frame_mbs_only_flag
        w.write(1, 1); // direct_8x8_inference_flag
        w.write(0, 1); // frame_cropping_flag
        w.write(0, 1); // vui_parameters_present_flag
        let mut sps = vec![0x67];
        sps.extend(escape(&w.finish()));
        sps
    }

    /// One with a VUI carrying timing, a default video signal type, and
    /// restrictions that allow reordering.
    fn vui_sps() -> Vec<u8> {
        let mut w = Writer::default();
        w.write(77, 8);
        w.write(0x40, 8);
        w.write(31, 8);
        w.write_ue(0);
        w.write_ue(0);
        w.write_ue(0); // pic_order_cnt_type 0
        w.write_ue(2); // log2_max_pic_order_cnt_lsb_minus4
        w.write_ue(2); // max_num_ref_frames
        w.write(0, 1);
        w.write_ue(79);
        w.write_ue(44);
        w.write(1, 1);
        w.write(1, 1);
        w.write(0, 1);
        w.write(1, 1); // vui present
        w.write(0, 1); // aspect ratio
        w.write(0, 1); // overscan
        w.write(1, 1); // video signal type present
        w.write(5, 3);
        w.write(0, 1);
        w.write(1, 1); // colour description
        w.write(0x010101, 24);
        w.write(0, 1); // chroma loc
        w.write(1, 1); // timing
        w.write(1, 32);
        w.write(60, 32);
        w.write(0, 1);
        w.write(0, 1); // nal hrd
        w.write(0, 1); // vcl hrd
        w.write(0, 1); // pic struct
        w.write(1, 1); // bitstream restriction
        w.write(1, 1);
        w.write_ue(2);
        w.write_ue(1);
        w.write_ue(16);
        w.write_ue(16);
        w.write_ue(2); // reorder frames
        w.write_ue(4); // dec frame buffering
        let mut sps = vec![0x67];
        sps.extend(escape(&w.finish()));
        sps
    }

    #[test]
    fn the_rewrite_is_a_fixed_point() {
        for sps in [bare_sps(), vui_sps()] {
            let once = rewrite_sps(&sps).expect("parses");
            assert_ne!(once, sps);
            assert_eq!(rewrite_sps(&once).expect("parses again"), once);
        }
    }

    #[test]
    fn a_bare_sps_gains_restrictions_without_reordering() {
        let rewritten = rewrite_sps(&bare_sps()).unwrap();
        let rbsp = unescape(&rewritten[1..]);
        let mut r = Reader::new(&rbsp);
        r.read(24).unwrap();
        for _ in 0..3 {
            r.read_ue().unwrap();
        }
        assert_eq!(r.read_ue(), Some(1)); // max_num_ref_frames
        r.read(1).unwrap();
        r.read_ue().unwrap();
        r.read_ue().unwrap();
        r.read(3).unwrap();
        assert_eq!(r.read(1), Some(1)); // vui present
        assert_eq!(r.read(8), Some(0));
        assert_eq!(r.read(1), Some(1)); // bitstream_restriction_flag
        r.read(1).unwrap();
        for _ in 0..4 {
            r.read_ue().unwrap();
        }
        assert_eq!(r.read_ue(), Some(0)); // max_num_reorder_frames
        assert_eq!(r.read_ue(), Some(1)); // max_dec_frame_buffering
    }

    #[test]
    fn frames_without_an_sps_pass_through_untouched() {
        let frame = [0, 0, 0, 1, 0x41, 9, 9, 9];
        assert!(matches!(rewrite_frame(&frame), Cow::Borrowed(_)));
    }

    #[test]
    fn other_units_survive_the_rewrite() {
        let frame = [
            &[0, 0, 0, 1][..],
            &bare_sps(),
            &[0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80],
            &[0, 0, 1, 0x65, 1, 2, 3],
        ]
        .concat();
        let rewritten = rewrite_frame(&frame);
        let units = nal_units(&rewritten);
        assert_eq!(units.len(), 3);
        assert_eq!(units[1], [0x68, 0xCE, 0x3C, 0x80]);
        assert_eq!(units[2], [0x65, 1, 2, 3]);
    }

    #[test]
    fn emulation_prevention_round_trips() {
        let data = [0, 0, 0, 0, 0, 1, 0, 0, 3, 7];
        assert_eq!(unescape(&escape(&data)), data);
    }
}
