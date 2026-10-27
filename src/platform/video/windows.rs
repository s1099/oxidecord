//! Media Foundation decode.
//!
//! The source reader is asked for `RGB32`, which on little-endian is byte-for-byte
//! the BGRA gpui wants, converted by the reader's video processor.
//!
//! Scaling is asked for too but rarely granted (the reader rejects sizes the
//! file doesn't natively have), so frames are box-averaged down to the drawn
//! size as they're copied out of the decoder's buffer.
//!
//! Audio comes from the same reader, resampled by Media Foundation to the
//! output device's rate. The streams are read separately rather than through
//! `ANY_STREAM`, each only when its own buffer has room, which keeps the
//! decoder paced to the speakers.

use std::collections::VecDeque;
use std::os::windows::ffi::OsStrExt as _;
use std::time::Duration;

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, Interface as _, PCWSTR};

use super::clock::{AudioRing, CHANNELS, Clock};
use super::{Session, VideoEvent, VideoFrame};

/// Enough to absorb decode-time jitter without running ahead of the clock;
/// each waiting frame is memory held for a picture nobody has seen.
const MAX_PENDING_FRAMES: usize = 3;

/// Half a frame at 60Hz. Releasing on the exact timestamp makes every frame
/// fractionally late by draw time, which reads as stutter.
const RELEASE_SLACK: Duration = Duration::from_millis(8);

const IDLE: Duration = Duration::from_millis(4);

/// Consecutive empty reads before `decode_all` gives up.
const MAX_EMPTY_READS: u32 = 256;

/// Media Foundation counts in 100ns units.
const HNS_PER_SEC: u64 = 10_000_000;

/// `RGB32` leaves its fourth byte undefined and Media Foundation zeroes it,
/// which gpui reads as fully transparent; copied as-is a video draws as nothing.
const OPAQUE: u8 = 255;

pub(super) fn decode(session: Session) -> Result<(), String> {
    let _platform = MediaFoundation::startup()?;
    run(session)
}

/// Video only. Frames are thinned to an even stride when all of them would
/// exceed `budget` bytes; with an unknown frame rate the clip is cut at the budget.
pub(super) fn decode_all(
    path: &std::path::Path,
    target: (u32, u32),
    budget: usize,
) -> Result<(Vec<VideoFrame>, Duration), String> {
    let _platform = MediaFoundation::startup()?;

    let reader = open_reader(path)?;
    let mut format = configure_video(&reader, target)?;
    // A selected stream nobody drains stalls the reader once its queue fills.
    unsafe {
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, false);
    }
    let duration = duration_of(&reader);

    let frame_bytes = format.width as usize * format.height as usize * 4;
    let max_frames = (budget / frame_bytes.max(1)).max(1);
    let expected = format
        .frame_rate
        .map_or(0, |fps| (duration.as_secs_f64() * fps).ceil() as usize);
    let stride = expected.div_ceil(max_frames).max(1);

    let mut frames = Vec::new();
    let mut index = 0usize;
    // No clock to idle against, so a reader returning empty forever would spin.
    let mut retries = 0;
    while frames.len() < max_frames && retries < MAX_EMPTY_READS {
        let read = if index.is_multiple_of(stride) {
            read_video(&reader, &mut format)?.map(|frame| frames.push(frame))
        } else {
            // Decoding is sequential, so a skipped frame is still read, just not copied.
            read_sample(
                &reader,
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                Some(&mut format),
            )?
            .map(drop)
        };
        match read {
            Read::Ready(()) => {
                index += 1;
                retries = 0;
            }
            Read::Retry => retries += 1,
            Read::End => break,
        }
    }

    if frames.is_empty() {
        return Err(String::from("The clip has no frames."));
    }
    Ok((frames, duration))
}

fn run(session: Session) -> Result<(), String> {
    let Session {
        path,
        target,
        control,
        audio,
        emit,
    } = session;

    let reader = open_reader(path)?;
    let mut video = configure_video(&reader, target)?;
    // Unconfigurable audio plays silently.
    let audio = audio.filter(|(_, rate)| configure_audio(&reader, *rate).is_ok());

    let mut clock = match &audio {
        Some((ring, _)) => Clock::Audio(ring.clone()),
        None => Clock::wall(),
    };
    let ring = audio.as_ref().map(|(ring, _)| ring.clone());

    if !emit(VideoEvent::Opened {
        duration: duration_of(&reader),
    }) {
        return Ok(());
    }

    let mut pending: VecDeque<VideoFrame> = VecDeque::new();
    // Audio the ring had no room for.
    let mut leftover: Vec<f32> = Vec::new();
    let mut was_paused = false;
    let mut video_done = false;
    let mut audio_done = audio.is_none();

    loop {
        if control.is_stopped() {
            return Ok(());
        }

        if let Some(ring) = &ring {
            ring.set_muted(control.is_muted());
        }

        let paused = control.is_paused();
        if paused != was_paused {
            clock.set_paused(paused);
            was_paused = paused;
        }

        if let Some(position) = control.take_seek() {
            seek(&reader, position)?;
            pending.clear();
            leftover.clear();
            clock.reset_to(position);
            video_done = false;
            audio_done = audio.is_none();
        }

        // When several frames are due, drop the earlier ones: the UI fell
        // behind, and a burst of stale frames looks worse than skipping.
        let now = clock.position();
        let mut due = None;
        while pending
            .front()
            .is_some_and(|frame| frame.position <= now + RELEASE_SLACK)
        {
            due = pending.pop_front();
        }
        if let Some(frame) = due {
            // No slot: the foreground hasn't drawn what it has, so drop this one too.
            if control.reserve_frame() && !emit(VideoEvent::Frame(frame)) {
                return Ok(());
            }
        }

        if paused {
            std::thread::sleep(IDLE);
            continue;
        }

        if video_done && audio_done && pending.is_empty() && drained(ring.as_deref()) {
            return Ok(());
        }

        // Keeps samples in order.
        if let (Some(ring), false) = (&ring, leftover.is_empty()) {
            let taken = ring.try_push(&leftover);
            leftover.drain(..taken);
        }
        if !leftover.is_empty() {
            std::thread::sleep(IDLE);
            continue;
        }

        // Audio first: it's the clock, and an underrun is heard where a late frame is only seen.
        let want_audio = !audio_done && ring.as_ref().is_some_and(|ring| ring.has_room());
        let want_video = !video_done && pending.len() < MAX_PENDING_FRAMES;
        // Except with no picture queued: at clip start the ring has room for
        // everything, so audio-first would decode its whole buffer before the
        // first frame, by which time those frames are stale and dropped.
        let starving = want_video && pending.is_empty();

        if want_audio && !starving {
            match read_audio(&reader)? {
                Read::Ready(samples) => {
                    let taken = ring.as_ref().map_or(0, |ring| ring.try_push(&samples));
                    leftover.extend_from_slice(&samples[taken..]);
                }
                Read::Retry => {}
                Read::End => audio_done = true,
            }
        } else if want_video {
            match read_video(&reader, &mut video)? {
                Read::Ready(frame) => pending.push_back(frame),
                Read::Retry => {}
                Read::End => video_done = true,
            }
        } else {
            std::thread::sleep(IDLE);
        }
    }
}

fn drained(ring: Option<&AudioRing>) -> bool {
    ring.is_none_or(AudioRing::is_drained)
}

struct MediaFoundation {
    /// False if the thread was already in an apartment; uninitializing one we
    /// didn't open would tear it out from under its owner.
    owns_com: bool,
}

impl MediaFoundation {
    fn startup() -> Result<Self, String> {
        // MTA, since the reader's worker threads call back into the source and an STA would marshal
        // each.
        let owns_com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();

        match unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
            Ok(()) => Ok(Self { owns_com }),
            Err(err) => {
                if owns_com {
                    unsafe { CoUninitialize() };
                }
                Err(format!("Couldn't start Media Foundation: {err}"))
            }
        }
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            if self.owns_com {
                CoUninitialize();
            }
        }
    }
}

fn open_reader(path: &std::path::Path) -> Result<IMFSourceReader, String> {
    // Lets the output type ask for a format the file doesn't natively have by
    // inserting a converter instead of rejecting the request.
    let attributes = unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)
            .map_err(|err| format!("Couldn't configure the decoder: {err}"))?;
        let attributes =
            attributes.ok_or_else(|| String::from("Couldn't configure the decoder."))?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(|err| format!("Couldn't configure the decoder: {err}"))?;
        attributes
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attributes) }
        .map_err(|err| format!("Nothing on this machine can play this video: {err}"))
}

struct VideoFormat {
    /// The visible area, smaller than the decoded buffer when the decoder pads it.
    source_width: u32,
    source_height: u32,
    crop_x: u32,
    crop_y: u32,
    /// Padding included.
    frame_height: u32,
    /// Negative means the image is stored bottom-up.
    stride: i32,
    /// Never larger than the source.
    width: u32,
    height: u32,
    frame_rate: Option<f64>,
    /// Kept to re-read the format when the decoder changes it mid-stream.
    target: (u32, u32),
}

impl VideoFormat {
    fn needs_scaling(&self) -> bool {
        self.width != self.source_width || self.height != self.source_height
    }
}

fn configure_video(reader: &IMFSourceReader, target: (u32, u32)) -> Result<VideoFormat, String> {
    let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    unsafe {
        // An unselected stream isn't decoded, so subtitles or extra audio tracks cost nothing.
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        let _ = reader.SetStreamSelection(video, true);
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true);
    }

    // Worth asking since it would scale on the GPU, but the reader normally
    // refuses a non-native size (`MF_E_INVALIDMEDIATYPE`) even with video
    // processing, so `read_video` scales instead.
    if set_video_type(reader, video, Some(target)).is_err() {
        set_video_type(reader, video, None)
            .map_err(|err| format!("Nothing on this machine can decode this video: {err}"))?;
    }

    // After a fallback the agreed format isn't what was asked for.
    current_format(reader, target)
}

fn current_format(reader: &IMFSourceReader, target: (u32, u32)) -> Result<VideoFormat, String> {
    let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let actual = unsafe { reader.GetCurrentMediaType(video) }
        .map_err(|err| format!("Couldn't read the video format: {err}"))?;
    let size = unsafe { actual.GetUINT64(&MF_MT_FRAME_SIZE) }
        .map_err(|err| format!("Couldn't read the video size: {err}"))?;
    let (frame_width, frame_height) = ((size >> 32) as u32, size as u32);

    if frame_width == 0 || frame_height == 0 {
        return Err(String::from("The video reports no picture size."));
    }

    // Absent for some pipelines; tightly packed top-down is the safe guess.
    // Only a fallback: the buffer's own pitch wins, since this is what the
    // format wants, not what the decoder did.
    let stride = unsafe { actual.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
        .map_or(frame_width as i32 * 4, |stride| stride as i32);

    // H.264 decodes in 16-pixel blocks, so the frame is padded and the aperture
    // says which part is picture. Drawn whole, the padding is a green strip on
    // the right and bottom.
    let (crop_x, crop_y, source_width, source_height) =
        aperture(&actual, frame_width, frame_height).unwrap_or((0, 0, frame_width, frame_height));

    // Anamorphic sources (common in phone footage) store a narrower picture
    // and expect it stretched, so shape the output by display size and let the
    // sampling read the stored one.
    let (par_num, par_den) = unsafe { actual.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO) }
        .map_or((1u64, 1u64), |par| {
            ((par >> 32).max(1), (par as u32 as u64).max(1))
        });
    let display_width = source_width as f64 * par_num as f64 / par_den as f64;

    let (width, height) = fit(display_width, f64::from(source_height), target);

    let frame_rate = unsafe { actual.GetUINT64(&MF_MT_FRAME_RATE) }
        .ok()
        .and_then(|rate| {
            let (numerator, denominator) = ((rate >> 32) as f64, (rate as u32) as f64);
            (numerator > 0. && denominator > 0.).then(|| numerator / denominator)
        });

    Ok(VideoFormat {
        source_width,
        source_height,
        crop_x,
        crop_y,
        frame_height,
        stride,
        width,
        height,
        frame_rate,
        target,
    })
}

/// `(x, y, width, height)` of the visible part, when the format names one inside the frame.
fn aperture(media_type: &IMFMediaType, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    [MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_GEOMETRIC_APERTURE]
        .iter()
        .find_map(|key| {
            // An `MFVideoArea`: fixed-point offsets (only the whole part matters), then the size.
            let mut blob = [0u8; 16];
            unsafe { media_type.GetBlob(key, &mut blob, None) }.ok()?;
            let x = i16::from_le_bytes([blob[2], blob[3]]);
            let y = i16::from_le_bytes([blob[6], blob[7]]);
            let area_width = i32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]);
            let area_height = i32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]);
            let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
            let (area_width, area_height) = (
                u32::try_from(area_width).ok()?,
                u32::try_from(area_height).ok()?,
            );
            (area_width > 0
                && area_height > 0
                && x + area_width <= width
                && y + area_height <= height)
                .then_some((x, y, area_width, area_height))
        })
}

/// The largest box with the display aspect ratio that fits inside `target`.
/// Settled here, not by the caller: Discord's reported size can disagree with
/// the file over anamorphic pixels, or be absent.
fn fit(display_width: f64, display_height: f64, target: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(target.0) / display_width)
        .min(f64::from(target.1) / display_height)
        // Upscaling would cost memory to invent detail; a small video is drawn larger instead.
        .min(1.);
    (
        ((display_width * scale).round() as u32).max(1),
        ((display_height * scale).round() as u32).max(1),
    )
}

fn set_video_type(
    reader: &IMFSourceReader,
    stream: u32,
    size: Option<(u32, u32)>,
) -> windows::core::Result<()> {
    unsafe {
        let media_type = MFCreateMediaType()?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        // BGRA in memory order on little-endian: the layout `RenderImage` takes.
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        if let Some((width, height)) = size {
            media_type.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(width.max(1)) << 32) | u64::from(height.max(1)),
            )?;
        }
        reader.SetCurrentMediaType(stream, None, &media_type)
    }
}

/// Stereo `f32` at the device's rate; Media Foundation resamples and downmixes.
fn configure_audio(reader: &IMFSourceReader, sample_rate: u32) -> windows::core::Result<()> {
    const BITS: u32 = 32;
    let block_align = BITS / 8 * CHANNELS as u32;

    unsafe {
        let media_type = MFCreateMediaType()?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        media_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
        media_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
        media_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, BITS)?;
        media_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align)?;
        media_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, sample_rate * block_align)?;
        reader.SetCurrentMediaType(
            MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
            None,
            &media_type,
        )
    }
}

/// Zero when the source won't say, which leaves the progress bar empty.
fn duration_of(reader: &IMFSourceReader) -> Duration {
    unsafe {
        let Ok(mut value) =
            reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        else {
            return Duration::ZERO;
        };
        let hns = value.Anonymous.Anonymous.Anonymous.uhVal;
        let _ = PropVariantClear(&mut value);
        Duration::from_nanos(hns.saturating_mul(100))
    }
}

fn seek(reader: &IMFSourceReader, position: Duration) -> Result<(), String> {
    let hns = (position.as_secs_f64() * HNS_PER_SEC as f64) as i64;

    unsafe {
        let mut value = PROPVARIANT::default();
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = hns;

        // A null time format means the default 100ns units.
        let result = reader.SetCurrentPosition(&GUID::from_u128(0), &value);
        let _ = PropVariantClear(&mut value);
        result.map_err(|err| format!("Couldn't seek: {err}"))
    }
}

fn read_video(
    reader: &IMFSourceReader,
    format: &mut VideoFormat,
) -> Result<Read<VideoFrame>, String> {
    let (sample, timestamp) = match read_sample(
        reader,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
        Some(format),
    )? {
        Read::Ready(ready) => ready,
        Read::Retry => return Ok(Read::Retry),
        Read::End => return Ok(Read::End),
    };

    let mut bgra = vec![0u8; format.width as usize * format.height as usize * 4];

    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|err| format!("Couldn't read a video frame: {err}"))?;

        // Prefer the 2D interface: it reports the pitch the buffer is actually
        // laid out with. `MF_MT_DEFAULT_STRIDE` is only what the format would
        // like, and a decoder padding rows wider shears the picture.
        let two_d = buffer.cast::<IMF2DBuffer>().ok();

        let source = match &two_d {
            Some(two_d) => {
                let mut scanline0 = std::ptr::null_mut();
                let mut pitch = 0i32;
                two_d
                    .Lock2D(&mut scanline0, &mut pitch)
                    .map_err(|err| format!("Couldn't read a video frame: {err}"))?;
                // Lock2D returns the picture's first row, so a bottom-up buffer
                // is a negative pitch walking back from it.
                Source {
                    scanline0,
                    pitch: pitch as isize,
                    format,
                }
                .cropped()
            }
            None => {
                let mut data = std::ptr::null_mut();
                let mut length = 0;
                buffer
                    .Lock(&mut data, None, Some(&mut length))
                    .map_err(|err| format!("Couldn't read a video frame: {err}"))?;

                let Some(source) =
                    Source::from_flat(data, length as usize, format).map(Source::cropped)
                else {
                    let _ = buffer.Unlock();
                    // Too short for its picture; skip the frame rather than read past the buffer.
                    return Ok(Read::Retry);
                };
                source
            }
        };

        if format.needs_scaling() {
            downscale(&source, &mut bgra);
        } else {
            copy_rows(&source, &mut bgra);
        }

        match &two_d {
            Some(two_d) => {
                let _ = two_d.Unlock2D();
            }
            None => {
                let _ = buffer.Unlock();
            }
        }
    }

    Ok(Read::Ready(VideoFrame {
        width: format.width,
        height: format.height,
        bgra,
        position: hns_to_duration(timestamp),
    }))
}

/// `scanline0` is always the picture's top row and `pitch` is signed, so
/// nothing below needs to know whether the source is stored bottom-up.
struct Source<'a> {
    scanline0: *const u8,
    pitch: isize,
    format: &'a VideoFormat,
}

impl<'a> Source<'a> {
    /// For a buffer locked through plain `IMFMediaBuffer`, where only the
    /// advertised pitch is known. `None` when it's too short for its picture.
    fn from_flat(data: *const u8, available: usize, format: &'a VideoFormat) -> Option<Self> {
        let pitch = format.stride as isize;
        // The whole decoded frame, padding included; `cropped` trims it.
        let row_bytes = (format.crop_x + format.source_width) as usize * 4;
        let rows = format.frame_height as usize;
        let needed = pitch.unsigned_abs() * (rows - 1) + row_bytes;
        if needed > available {
            return None;
        }

        // Negative stride: bottom-up, so the top row is last in the buffer.
        let scanline0 = if pitch < 0 {
            // SAFETY: `needed` above proved the buffer spans every row.
            unsafe { data.add(pitch.unsigned_abs() * (rows - 1)) }
        } else {
            data
        };

        Some(Self {
            scanline0,
            pitch,
            format,
        })
    }

    fn cropped(self) -> Self {
        let offset = self.format.crop_y as isize * self.pitch + self.format.crop_x as isize * 4;
        Self {
            // SAFETY: the aperture was checked to lie inside the frame.
            scanline0: unsafe { self.scanline0.offset(offset) },
            ..self
        }
    }

    /// # Safety
    ///
    /// Only while the buffer is locked, and for `y` inside the picture's height.
    unsafe fn row(&self, y: usize) -> &[u8] {
        let row_bytes = self.format.source_width as usize * 4;
        unsafe {
            let ptr = self.scanline0.offset(y as isize * self.pitch);
            std::slice::from_raw_parts(ptr, row_bytes)
        }
    }
}

/// For when the decoder already produced the drawn size.
fn copy_rows(source: &Source, out: &mut [u8]) {
    let row_bytes = source.format.width as usize * 4;
    for y in 0..source.format.height as usize {
        let row = unsafe { source.row(y) };
        out[y * row_bytes..][..row_bytes].copy_from_slice(&row[..row_bytes]);
    }
    for pixel in out.chunks_exact_mut(4) {
        pixel[3] = OPAQUE;
    }
}

/// Box-averages down to the drawn size. This is where the memory saving
/// happens: a 1440p frame arrives at ~4.5MB and leaves at a few hundred KB.
/// Averaging the box rather than picking a pixel keeps text and thin lines in
/// screen recordings readable.
fn downscale(source: &Source, out: &mut [u8]) {
    let format = source.format;
    let (out_width, out_height) = (format.width as usize, format.height as usize);
    let (src_width, src_height) = (format.source_width as usize, format.source_height as usize);

    // Source columns each output pixel covers; the same spans serve every row.
    let spans: Vec<(usize, usize)> = (0..out_width)
        .map(|x| {
            let start = x * src_width / out_width;
            let end = (((x + 1) * src_width).div_ceil(out_width)).max(start + 1);
            (start, end.min(src_width))
        })
        .collect();

    let mut sums = vec![0u32; out_width * 3];
    for y in 0..out_height {
        let first = y * src_height / out_height;
        let last = (((y + 1) * src_height).div_ceil(out_height))
            .max(first + 1)
            .min(src_height);

        sums.fill(0);
        let rows = (last - first) as u32;
        for sy in first..last {
            let row = unsafe { source.row(sy) };
            for (x, &(start, end)) in spans.iter().enumerate() {
                let slot = &mut sums[x * 3..][..3];
                for pixel in row[start * 4..end * 4].chunks_exact(4) {
                    // BGR only; alpha is forced, see `OPAQUE`.
                    slot[0] += u32::from(pixel[0]);
                    slot[1] += u32::from(pixel[1]);
                    slot[2] += u32::from(pixel[2]);
                }
            }
        }
        for (x, &(start, end)) in spans.iter().enumerate() {
            let count = rows * (end - start) as u32;
            let slot = &sums[x * 3..][..3];
            let out_pixel = &mut out[(y * out_width + x) * 4..][..4];
            for channel in 0..3 {
                out_pixel[channel] = (slot[channel] / count) as u8;
            }
            out_pixel[3] = OPAQUE;
        }
    }
}

fn read_audio(reader: &IMFSourceReader) -> Result<Read<Vec<f32>>, String> {
    let sample = match read_sample(reader, MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None)? {
        Read::Ready((sample, _)) => sample,
        Read::Retry => return Ok(Read::Retry),
        Read::End => return Ok(Read::End),
    };

    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|err| format!("Couldn't read the audio: {err}"))?;

        let mut data = std::ptr::null_mut();
        let mut length = 0;
        buffer
            .Lock(&mut data, None, Some(&mut length))
            .map_err(|err| format!("Couldn't read the audio: {err}"))?;

        let bytes = std::slice::from_raw_parts(data, length as usize);
        // Not reinterpreted: nothing promises `f32` alignment.
        let samples = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        let _ = buffer.Unlock();
        Ok(Read::Ready(samples))
    }
}

/// `Retry` is a read with no sample but no end of stream (a format change, or
/// nothing ready); treating it as the end would truncate the clip.
enum Read<T> {
    Ready(T),
    Retry,
    End,
}

impl<T> Read<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Read<U> {
        match self {
            Self::Ready(value) => Read::Ready(f(value)),
            Self::Retry => Read::Retry,
            Self::End => Read::End,
        }
    }
}

/// A video stream passes its `format`, re-read when the decoder changes it:
/// H.264 decoders often start at the container's size and switch to the padded
/// size a few frames in, and copying at the old size shears the picture.
fn read_sample(
    reader: &IMFSourceReader,
    stream: u32,
    format: Option<&mut VideoFormat>,
) -> Result<Read<(IMFSample, i64)>, String> {
    let mut flags = 0u32;
    let mut timestamp = 0i64;
    let mut sample = None;

    unsafe {
        reader.ReadSample(
            stream,
            0,
            None,
            Some(&mut flags),
            Some(&mut timestamp),
            Some(&mut sample),
        )
    }
    .map_err(|err| format!("Couldn't decode the video: {err}"))?;

    if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0
        && let Some(format) = format
    {
        *format = current_format(reader, format.target)?;
    }

    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
        return Ok(Read::End);
    }

    Ok(match sample {
        Some(sample) => Read::Ready((sample, timestamp)),
        None => Read::Retry,
    })
}

fn hns_to_duration(hns: i64) -> Duration {
    Duration::from_nanos(hns.max(0) as u64 * 100)
}
