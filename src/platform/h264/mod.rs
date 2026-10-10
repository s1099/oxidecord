//! Live H.264 decode, for watching someone else's Go Live stream.
//!
//! Like [`super::video`], nothing here ships a codec: Windows decodes through
//! Media Foundation's H.264 decoder. Unlike a clip, a stream has no file and
//! no clock of its own to pace against — frames are shown the moment they're
//! decoded, since the network already delivered them at the sender's rate.
//!
//! One [`LiveDecoder`] runs on a thread of its own. The stream connection
//! feeds it Annex B access units through the [`DecoderInput`] half, and
//! decoded pictures come back as [`DecoderEvent`]s, already converted to BGRA
//! and scaled to the size they'll be drawn at. Every access unit is decoded
//! — later frames are predicted from it — but only as many pictures are
//! converted as the UI is keeping up with, the same frames-in-flight budget
//! inline video uses.
//!
//! Dropping either half stops the thread.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;

use futures::channel::mpsc::UnboundedSender;

/// Decoded pictures that may be waiting on the UI before more are skipped.
/// See [`super::video`]'s own limit for why this is small.
const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// One decoded picture.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGRA, top row first, alpha opaque.
    pub bgra: Vec<u8>,
}

pub enum DecoderEvent {
    Frame(DecodedFrame),
    /// Nothing on the machine would decode the stream.
    Failed(String),
}

/// State shared with the decoder thread. Atomics, because the thread reads
/// them for every picture and must never wait on the UI.
struct Control {
    stopped: AtomicBool,
    frames_in_flight: AtomicUsize,
    /// The size pictures are scaled to fit, width in the high half.
    target: AtomicU64,
}

impl Control {
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn target(&self) -> (u32, u32) {
        let packed = self.target.load(Ordering::Acquire);
        ((packed >> 32) as u32, packed as u32)
    }

    /// Whether another picture may be sent, reserving a slot when it may.
    fn reserve_frame(&self) -> bool {
        self.frames_in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |in_flight| {
                (in_flight < MAX_FRAMES_IN_FLIGHT).then_some(in_flight + 1)
            })
            .is_ok()
    }
}

/// The UI's half: where pictures are drawn, and when they've been.
pub struct LiveDecoder {
    control: Arc<Control>,
}

/// The connection's half: where access units go in.
pub struct DecoderInput {
    frames: mpsc::Sender<Vec<u8>>,
}

impl DecoderInput {
    /// Queues one access unit. Returns false once the decoder has stopped.
    pub fn decode(&self, frame: Vec<u8>) -> bool {
        self.frames.send(frame).is_ok()
    }
}

/// Starts a decoder thread, delivering pictures on `events` scaled to fit
/// `target` (physical pixels).
pub fn start(
    target: (u32, u32),
    events: UnboundedSender<DecoderEvent>,
) -> (LiveDecoder, DecoderInput) {
    let control = Arc::new(Control {
        stopped: AtomicBool::new(false),
        frames_in_flight: AtomicUsize::new(0),
        target: AtomicU64::new(pack(target)),
    });
    let (frames, input) = mpsc::channel();

    let thread_control = control.clone();
    let spawned = std::thread::Builder::new()
        .name("stream-decode".into())
        .spawn(move || {
            let mut emit = |event| events.unbounded_send(event).is_ok();
            if let Err(err) = backend::run(&thread_control, &input, &mut emit)
                && !thread_control.is_stopped()
            {
                emit(DecoderEvent::Failed(err));
            }
        });
    if let Err(err) = spawned {
        eprintln!("couldn't start the stream decoder: {err}");
    }

    (LiveDecoder { control }, DecoderInput { frames })
}

impl LiveDecoder {
    /// Changes the size pictures are scaled to fit, from the next one on.
    pub fn set_target(&self, target: (u32, u32)) {
        self.control.target.store(pack(target), Ordering::Release);
    }

    /// Reports that a picture has been drawn and released, freeing a slot.
    pub fn frame_consumed(&self) {
        let _ = self.control.frames_in_flight.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |in_flight| in_flight.checked_sub(1),
        );
    }
}

impl Drop for LiveDecoder {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
    }
}

fn pack((width, height): (u32, u32)) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

/// The size a `width` × `height` picture is drawn at inside `target`: as large
/// as fits, keeping its shape, and never larger than it is.
fn fit(width: u32, height: u32, target: (u32, u32)) -> (u32, u32) {
    let (width, height) = (f64::from(width), f64::from(height));
    let scale = (f64::from(target.0.max(1)) / width)
        .min(f64::from(target.1.max(1)) / height)
        .min(1.);
    (
        ((width * scale).round() as u32).max(1),
        ((height * scale).round() as u32).max(1),
    )
}

/// How a decoder's YUV maps to RGB.
#[derive(Clone, Copy)]
struct ColorSpace {
    /// BT.709 rather than BT.601.
    bt709: bool,
    /// 0–255 rather than studio swing (16–235).
    full_range: bool,
}

/// Fixed-point (16.16) coefficients for one colour space: luma scale and
/// offset, then V→R, U→G, V→G, U→B.
struct Coefficients {
    y_scale: i32,
    y_offset: i32,
    vr: i32,
    ug: i32,
    vg: i32,
    ub: i32,
}

impl ColorSpace {
    fn coefficients(self) -> Coefficients {
        let fixed = |value: f64| (value * 65536.).round() as i32;
        let (vr, ug, vg, ub) = match (self.bt709, self.full_range) {
            (false, false) => (1.596, 0.392, 0.813, 2.017),
            (true, false) => (1.793, 0.213, 0.533, 2.112),
            (false, true) => (1.402, 0.344, 0.714, 1.772),
            (true, true) => (1.575, 0.187, 0.468, 1.856),
        };
        let (y_scale, y_offset) = if self.full_range {
            (1., 0)
        } else {
            (255. / 219., 16)
        };
        Coefficients {
            y_scale: fixed(y_scale),
            y_offset,
            vr: fixed(vr),
            ug: fixed(ug),
            vg: fixed(vg),
            ub: fixed(ub),
        }
    }
}

/// An NV12 picture as a decoder left it: a luma plane, then interleaved
/// half-resolution chroma, both `pitch` bytes a row.
struct Nv12<'a> {
    luma: &'a [u8],
    chroma: &'a [u8],
    pitch: usize,
    /// The visible part, which a decoder pads out to whole macroblocks.
    width: usize,
    height: usize,
    color: ColorSpace,
}

impl Nv12<'_> {
    /// Converts to BGRA, scaled to `size`. Luma is sampled bilinearly, which
    /// is what keeps a shared screen's text legible when it's drawn smaller
    /// than it was sent; chroma is half resolution already and only picked.
    fn to_bgra(&self, size: (u32, u32)) -> Vec<u8> {
        let (out_width, out_height) = (size.0 as usize, size.1 as usize);
        let mut out = vec![0u8; out_width * out_height * 4];
        let c = self.color.coefficients();
        // Source position of each output pixel's centre, in 16.16.
        let step_x = ((self.width << 16) / out_width) as i64;
        let step_y = ((self.height << 16) / out_height) as i64;
        let max_x = (self.width - 1) as i64;
        let max_y = (self.height - 1) as i64;

        for (row, line) in out.chunks_exact_mut(out_width * 4).enumerate() {
            let sy = ((row as i64 * step_y + step_y / 2) - 32768).clamp(0, max_y << 16);
            let y0 = (sy >> 16) as usize;
            let y1 = (y0 + 1).min(max_y as usize);
            let fy = sy & 0xFFFF;
            let luma0 = &self.luma[y0 * self.pitch..];
            let luma1 = &self.luma[y1 * self.pitch..];
            let chroma = &self.chroma[(y0 / 2) * self.pitch..];

            for (col, pixel) in line.chunks_exact_mut(4).enumerate() {
                let sx = ((col as i64 * step_x + step_x / 2) - 32768).clamp(0, max_x << 16);
                let x0 = (sx >> 16) as usize;
                let x1 = (x0 + 1).min(max_x as usize);
                let fx = sx & 0xFFFF;

                let top = i64::from(luma0[x0]) * (65536 - fx) + i64::from(luma0[x1]) * fx;
                let bottom = i64::from(luma1[x0]) * (65536 - fx) + i64::from(luma1[x1]) * fx;
                let y = ((top >> 16) * (65536 - fy) + (bottom >> 16) * fy) >> 16;

                let pair = (x0 / 2) * 2;
                let u = i64::from(chroma[pair]) - 128;
                let v = i64::from(chroma[pair + 1]) - 128;
                // Half a unit up, so the shifts below round rather than truncate.
                let y = (y - i64::from(c.y_offset)) * i64::from(c.y_scale) + 32768;

                let r = (y + i64::from(c.vr) * v) >> 16;
                let g = (y - i64::from(c.ug) * u - i64::from(c.vg) * v) >> 16;
                let b = (y + i64::from(c.ub) * u) >> 16;
                pixel[0] = b.clamp(0, 255) as u8;
                pixel[1] = g.clamp(0, 255) as u8;
                pixel[2] = r.clamp(0, 255) as u8;
                pixel[3] = 255;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_shape_and_never_upscales() {
        assert_eq!(fit(1920, 1080, (960, 960)), (960, 540));
        assert_eq!(fit(640, 360, (1920, 1080)), (640, 360));
        assert_eq!(fit(100, 100, (0, 0)), (1, 1));
    }

    #[test]
    fn grey_converts_to_grey_and_white_to_white() {
        let (width, height) = (4, 2);
        let luma = vec![235u8; width * height];
        let chroma = vec![128u8; width * height / 2];
        let picture = Nv12 {
            luma: &luma,
            chroma: &chroma,
            pitch: width,
            width,
            height,
            color: ColorSpace {
                bt709: false,
                full_range: false,
            },
        };
        let out = picture.to_bgra((2, 1));
        assert_eq!(out, [255, 255, 255, 255, 255, 255, 255, 255]);
    }
}
