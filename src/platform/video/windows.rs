//! Media Foundation decode.
//!
//! The source reader is asked for `RGB32`, which on a little-endian machine is
//! byte-for-byte the BGRA gpui wants, so the pixel format costs nothing: the
//! video processor the reader inserts handles it, on the GPU where the driver
//! supports it.
//!
//! Scaling is asked for the same way but rarely granted — the source reader
//! rejects an output size the file doesn't natively have — so frames are
//! box-averaged down to the size they'll be drawn at as they're copied out of
//! the decoder's buffer. That pass is the one place this backend touches every
//! pixel, and it's what keeps a frame in the UI at a few hundred kilobytes
//! instead of the several megabytes a full-size one would be.
//!
//! Audio is pulled from the same reader, resampled by Media Foundation to
//! whatever rate the output device opened at, so no resampler is needed on
//! this side either.
//!
//! The two streams are read separately rather than through `ANY_STREAM`: each
//! is read only when its own buffer has room, which is what keeps the decoder
//! paced to the speakers instead of racing ahead of them.

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

/// Decoded frames held back waiting for their moment.
///
/// Three is enough to absorb the jitter in decode times without letting the
/// decoder run meaningfully ahead of the clock — which is the point, since
/// every frame waiting here is memory held for a picture nobody has seen yet.
const MAX_PENDING_FRAMES: usize = 3;

/// How early a frame may be released. Half a frame at 60Hz: releasing on the
/// exact timestamp means every frame is fractionally late by the time it is
/// drawn, which reads as a stutter.
const RELEASE_SLACK: Duration = Duration::from_millis(8);

/// How long the loop idles when every buffer is full. Short enough that a
/// frame coming due is not noticeably late.
const IDLE: Duration = Duration::from_millis(4);

/// Media Foundation counts in 100ns units.
const HNS_PER_SEC: u64 = 10_000_000;

/// The alpha every decoded pixel is given on the way out.
///
/// `RGB32` leaves its fourth byte undefined and Media Foundation fills it with
/// zero, which gpui reads as fully transparent — copied through as-is, a video
/// draws as nothing at all.
const OPAQUE: u8 = 255;

pub(super) fn decode(session: Session) -> Result<(), String> {
    let _platform = MediaFoundation::startup()?;
    run(session)
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
    let video = configure_video(&reader, target)?;
    // A file whose audio can't be configured still plays, silently; so does
    // one on a machine with no output device.
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
    // Audio the ring had no room for yet, carried to the next pass.
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

        // Release whichever frame is current. When several have come due at
        // once the earlier ones are dropped rather than drawn: the UI fell
        // behind, and catching up by showing stale frames in a burst looks
        // worse than skipping to the right one.
        let now = clock.position();
        let mut due = None;
        while pending
            .front()
            .is_some_and(|frame| frame.position <= now + RELEASE_SLACK)
        {
            due = pending.pop_front();
        }
        if let Some(frame) = due {
            // No slot means the foreground has not drawn the frames it already
            // has, so this one is dropped too.
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

        // Audio that didn't fit last time goes in before anything new is read,
        // so samples stay in order.
        if let (Some(ring), false) = (&ring, leftover.is_empty()) {
            let taken = ring.try_push(&leftover);
            leftover.drain(..taken);
        }
        if !leftover.is_empty() {
            std::thread::sleep(IDLE);
            continue;
        }

        // Audio first: it is the clock, and an underrun is heard where a late
        // frame is only seen.
        let want_audio = !audio_done && ring.as_ref().is_some_and(|ring| ring.has_room());
        let want_video = !video_done && pending.len() < MAX_PENDING_FRAMES;
        // Except when there is no picture queued at all. At the start of a clip
        // the ring has room for everything, so audio-first would decode the
        // whole buffer before the first frame — by which time the clock has run
        // on and those frames are already stale enough to be dropped.
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
            match read_video(&reader, &video)? {
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

/// Holds COM and the Media Foundation platform open for as long as decoding
/// runs, and closes both however the decoder exits.
struct MediaFoundation {
    /// Whether COM was initialized *by this thread*. It won't have been if the
    /// thread is somehow already in an apartment, and uninitializing one this
    /// guard didn't open would tear it out from under whoever did.
    owns_com: bool,
}

impl MediaFoundation {
    fn startup() -> Result<Self, String> {
        // MTA, because the source reader's own worker threads call back into
        // the media source and an STA would marshal every one of those.
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
    // `MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING` is what lets the output type
    // below ask for a format and a size the file doesn't natively have: it
    // tells the reader to insert a converter rather than reject the request.
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

/// How the decoded video comes out, and what it is turned into on the way to
/// the UI.
struct VideoFormat {
    /// What Media Foundation actually decodes at.
    source_width: u32,
    source_height: u32,
    /// Bytes between the start of one row and the next, as Media Foundation
    /// reports it. Negative means the image is stored bottom-up.
    stride: i32,
    /// What frames are handed over at, never larger than the source.
    width: u32,
    height: u32,
}

impl VideoFormat {
    fn needs_scaling(&self) -> bool {
        self.width != self.source_width || self.height != self.source_height
    }
}

fn configure_video(reader: &IMFSourceReader, target: (u32, u32)) -> Result<VideoFormat, String> {
    let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    unsafe {
        // Everything off, then only what is going to be read — an unselected
        // stream isn't decoded, so this is what stops a subtitle or a second
        // audio track costing anything.
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        let _ = reader.SetStreamSelection(video, true);
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true);
    }

    // Worth asking, because a pipeline that scales for us does it on the GPU
    // for free. In practice the source reader refuses an output size the file
    // doesn't natively have (`MF_E_INVALIDMEDIATYPE`) even with video
    // processing enabled, so the native size is the path normally taken and
    // `read_video` scales on the way out.
    if set_video_type(reader, video, Some(target)).is_err() {
        set_video_type(reader, video, None)
            .map_err(|err| format!("Nothing on this machine can decode this video: {err}"))?;
    }

    // Read back what was actually agreed to, which after a fallback is not
    // what was asked for.
    let actual = unsafe { reader.GetCurrentMediaType(video) }
        .map_err(|err| format!("Couldn't read the video format: {err}"))?;
    let size = unsafe { actual.GetUINT64(&MF_MT_FRAME_SIZE) }
        .map_err(|err| format!("Couldn't read the video size: {err}"))?;
    let (source_width, source_height) = ((size >> 32) as u32, size as u32);

    if source_width == 0 || source_height == 0 {
        return Err(String::from("The video reports no picture size."));
    }

    // Absent for some pipelines, where a tightly packed top-down image is the
    // safe assumption. Only a fallback now: the buffer's own pitch is
    // preferred where it has one, because this is what the format wants rather
    // than what the decoder did.
    let stride = unsafe { actual.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
        .map_or(source_width as i32 * 4, |stride| stride as i32);

    // Stored pixels aren't always square. Anamorphic sources — plenty of phone
    // and camera footage — store a narrower picture and expect it stretched on
    // display, so shape the output by the display size and let the sampling
    // below read the stored one.
    let (par_num, par_den) = unsafe { actual.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO) }
        .map_or((1u64, 1u64), |par| {
            ((par >> 32).max(1), (par as u32 as u64).max(1))
        });
    let display_width = source_width as f64 * par_num as f64 / par_den as f64;

    let (width, height) = fit(display_width, f64::from(source_height), target);

    Ok(VideoFormat {
        source_width,
        source_height,
        stride,
        width,
        height,
    })
}

/// The largest box with the picture's display aspect ratio that fits inside
/// `target`.
///
/// The frame's true shape is settled here rather than by the caller, because
/// what the caller knows is what Discord reported — which can disagree with
/// the file over anamorphic pixels, and is absent altogether for some
/// attachments.
fn fit(display_width: f64, display_height: f64, target: (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(target.0) / display_width)
        .min(f64::from(target.1) / display_height)
        // Upscaling here would cost memory to invent detail the source doesn't
        // have; a small video is handed over at its own size and drawn larger.
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
        // RGB32 is BGRA in memory order on a little-endian machine, so the
        // frames land in exactly the layout `RenderImage` takes.
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        if let Some((width, height)) = size {
            // Width in the high half, height in the low half.
            media_type.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(width.max(1)) << 32) | u64::from(height.max(1)),
            )?;
        }
        reader.SetCurrentMediaType(stream, None, &media_type)
    }
}

/// Asks for the audio track as stereo `f32` at the output device's own rate,
/// leaving the resampling and downmixing to Media Foundation.
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

/// The clip's run time. Zero when the source won't say, which leaves the
/// progress bar empty rather than stopping playback.
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

        // A null time format means the default, which is the 100ns units the
        // rest of the reader speaks.
        let result = reader.SetCurrentPosition(&GUID::from_u128(0), &value);
        let _ = PropVariantClear(&mut value);
        result.map_err(|err| format!("Couldn't seek: {err}"))
    }
}

/// One decoded video frame, if the reader had one ready.
fn read_video(reader: &IMFSourceReader, format: &VideoFormat) -> Result<Read<VideoFrame>, String> {
    let (sample, timestamp) =
        match read_sample(reader, MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)? {
            Read::Ready(ready) => ready,
            Read::Retry => return Ok(Read::Retry),
            Read::End => return Ok(Read::End),
        };

    let mut bgra = vec![0u8; format.width as usize * format.height as usize * 4];

    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|err| format!("Couldn't read a video frame: {err}"))?;

        // The 2D interface where the buffer has one, because it reports the
        // pitch the buffer is *actually* laid out with and a pointer to the
        // top row. `MF_MT_DEFAULT_STRIDE` only says what the format would
        // like, and a decoder that pads its rows to a wider alignment than
        // that leaves every row offset a little further than the last — which
        // is seen as the picture shearing across the frame.
        let two_d = buffer.cast::<IMF2DBuffer>().ok();

        let source = match &two_d {
            Some(two_d) => {
                let mut scanline0 = std::ptr::null_mut();
                let mut pitch = 0i32;
                two_d
                    .Lock2D(&mut scanline0, &mut pitch)
                    .map_err(|err| format!("Couldn't read a video frame: {err}"))?;
                // Lock2D hands back the first row of the *picture*, so a
                // bottom-up buffer is already accounted for by a negative
                // pitch walking backwards from it.
                Source {
                    scanline0,
                    pitch: pitch as isize,
                    format,
                }
            }
            None => {
                let mut data = std::ptr::null_mut();
                let mut length = 0;
                buffer
                    .Lock(&mut data, None, Some(&mut length))
                    .map_err(|err| format!("Couldn't read a video frame: {err}"))?;

                let Some(source) = Source::from_flat(data, length as usize, format) else {
                    let _ = buffer.Unlock();
                    // Too short to hold the picture it claims to; skipping one
                    // frame is better than reading past the buffer.
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

/// A locked decoder buffer, addressed one picture row at a time.
///
/// `scanline0` always points at the *top* row of the picture and `pitch` is
/// signed, so a bottom-up buffer is just a negative step backwards from it and
/// nothing below has to know which way round the source was stored.
struct Source<'a> {
    scanline0: *const u8,
    pitch: isize,
    format: &'a VideoFormat,
}

impl<'a> Source<'a> {
    /// A buffer locked through the plain `IMFMediaBuffer` interface, where the
    /// only pitch on offer is the one the media type advertised.
    ///
    /// `None` when the buffer is too short to hold the picture it claims to,
    /// which is the one case where reading rows would run off the end of it.
    fn from_flat(data: *const u8, available: usize, format: &'a VideoFormat) -> Option<Self> {
        let pitch = format.stride as isize;
        let row_bytes = format.source_width as usize * 4;
        let rows = format.source_height as usize;
        let needed = pitch.unsigned_abs() * (rows - 1) + row_bytes;
        if needed > available {
            return None;
        }

        // A negative stride means the picture is stored bottom-up: the top row
        // of the picture is the last row in the buffer.
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

    /// One row of the picture.
    ///
    /// # Safety
    ///
    /// Only valid while the buffer is locked, and only for `y` inside the
    /// picture's height.
    unsafe fn row(&self, y: usize) -> &[u8] {
        let row_bytes = self.format.source_width as usize * 4;
        unsafe {
            let ptr = self.scanline0.offset(y as isize * self.pitch);
            std::slice::from_raw_parts(ptr, row_bytes)
        }
    }
}

/// Copies the picture out unchanged, for the case where the decoder already
/// produced the size being drawn.
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

/// Box-averages the picture down to the size it will be drawn at.
///
/// This is where the memory saving actually happens: the source reader refuses
/// to scale for us, so a 1440p frame arrives at four and a half megabytes and
/// leaves at a few hundred kilobytes. Averaging the whole box rather than
/// picking one pixel per output costs a single pass over the source and is the
/// difference between a readable thumbnail and an aliased one — text and thin
/// lines in a screen recording fall apart under nearest-neighbour.
fn downscale(source: &Source, out: &mut [u8]) {
    let format = source.format;
    let (out_width, out_height) = (format.width as usize, format.height as usize);
    let (src_width, src_height) = (format.source_width as usize, format.source_height as usize);

    // Which source rows and columns each output pixel covers. Precomputed for
    // the columns because the same spans are walked for every row.
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
                    // Blue, green, red. The fourth byte isn't summed because
                    // it isn't kept — see `OPAQUE`.
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

/// One packet of decoded audio as interleaved stereo `f32`.
fn read_audio(reader: &IMFSourceReader) -> Result<Read<Vec<f32>>, String> {
    let sample = match read_sample(reader, MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32)? {
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
        // Read four bytes at a time rather than reinterpreting the buffer:
        // nothing promises it is aligned for `f32`.
        let samples = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        let _ = buffer.Unlock();
        Ok(Read::Ready(samples))
    }
}

/// What one read off a stream produced.
///
/// [`Read::Retry`] is the one worth spelling out: a read can legitimately come
/// back with no sample without the stream having ended — a format change, or
/// the reader simply having nothing ready — and treating that as the end would
/// truncate the clip.
enum Read<T> {
    Ready(T),
    Retry,
    End,
}

fn read_sample(reader: &IMFSourceReader, stream: u32) -> Result<Read<(IMFSample, i64)>, String> {
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
