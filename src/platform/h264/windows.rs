//! Media Foundation's H.264 decoder, driven by hand.
//!
//! No source reader: the stream is network access units, so the transform is
//! fed directly. Low latency makes it return each picture at once instead of
//! holding some for reordering, which a live stream never needs. Output is
//! NV12, converted to BGRA by [`super::Nv12`].

use std::mem::ManuallyDrop;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::Interface as _;

use super::{ColorSpace, Control, DecodedFrame, DecoderEvent, Nv12, fit};

/// How often an idle thread checks it's still wanted.
const POLL: Duration = Duration::from_millis(200);

/// Nominal duration in 100ns units; the decoder only needs timestamps that advance.
const FRAME_HNS: i64 = 333_333;

pub(super) fn run(
    control: &Control,
    input: &Receiver<Vec<u8>>,
    emit: &mut dyn FnMut(DecoderEvent) -> bool,
) -> Result<(), String> {
    let _mf = MediaFoundation::startup()?;
    let mut decoder = Decoder::new()?;
    let mut time = 0;
    let mut queued = None;

    loop {
        let frame = match queued.take() {
            Some(frame) => frame,
            None => match input.recv_timeout(POLL) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) if !control.is_stopped() => continue,
                Err(_) => return Ok(()),
            },
        };
        if control.is_stopped() {
            return Ok(());
        }
        // With more waiting, this picture would be stale when drawn. Still
        // decode it (later units refer back), but convert only the newest so a
        // lagging viewer catches up.
        let behind = match input.try_recv() {
            Ok(next) => {
                queued = Some(next);
                true
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => false,
        };

        decoder.feed(&frame, time)?;
        time += FRAME_HNS;
        let mut gone = false;
        decoder.drain(&mut |picture| {
            if behind || gone || !control.reserve_frame() {
                return;
            }
            let size = fit(
                picture.width as u32,
                picture.height as u32,
                control.target(),
            );
            let frame = DecodedFrame {
                width: size.0,
                height: size.1,
                bgra: picture.to_bgra(size),
            };
            gone = !emit(DecoderEvent::Frame(frame));
        })?;
        if gone {
            return Ok(());
        }
    }
}

/// Learned when the decoder settles on a format after the first keyframe.
struct Format {
    /// Padded out to whole macroblocks.
    coded: (u32, u32),
    visible: (u32, u32),
    color: ColorSpace,
    /// Bytes per output sample, for a decoder that doesn't allocate its own.
    buffer_size: u32,
    provides_samples: bool,
}

struct Decoder {
    transform: IMFTransform,
    format: Option<Format>,
}

impl Decoder {
    fn new() -> Result<Self, String> {
        let transform: IMFTransform =
            unsafe { CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER) }
                .context("Couldn't start the H.264 decoder")?;

        (|| unsafe {
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform.SetInputType(0, &input, 0)?;
            choose_nv12(&transform)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
        })()
        .context("The H.264 decoder refused the stream")?;

        // The decoder writes into caller-allocated samples even for the call
        // that reports the real format, so start with whatever format it opens with.
        let format = read_format(&transform).ok();
        Ok(Self { transform, format })
    }

    fn feed(&mut self, frame: &[u8], time: i64) -> Result<(), String> {
        let sample = (|| unsafe {
            let buffer = MFCreateMemoryBuffer(frame.len() as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(frame.as_ptr(), data, frame.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(frame.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(FRAME_HNS)?;
            windows::core::Result::Ok(sample)
        })()
        .context("Couldn't hand a frame to the decoder")?;

        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Ok(()) => Ok(()),
            // Full: take its output first. Nothing is shown; this only happens when the UI is
            // behind.
            Err(err) if err.code() == MF_E_NOTACCEPTING => {
                self.drain(&mut |_| {})?;
                unsafe { self.transform.ProcessInput(0, &sample, 0) }
                    .context("The decoder refused a frame")
            }
            Err(err) => Err(format!("The decoder refused a frame: {}", err.message())),
        }
    }

    fn drain(&mut self, show: &mut dyn FnMut(&Nv12)) -> Result<(), String> {
        loop {
            let Some(format) = &self.format else {
                // The first output call is what reports a format.
                if !self.take_output(None, show)? {
                    return Ok(());
                }
                continue;
            };
            let provided = if format.provides_samples {
                None
            } else {
                Some(
                    (|| unsafe {
                        let sample = MFCreateSample()?;
                        sample.AddBuffer(&MFCreateMemoryBuffer(format.buffer_size)?)?;
                        windows::core::Result::Ok(sample)
                    })()
                    .context("Decoding failed")?,
                )
            };
            if !self.take_output(provided, show)? {
                return Ok(());
            }
        }
    }

    /// Returns whether there may be more.
    fn take_output(
        &mut self,
        provided: Option<IMFSample>,
        show: &mut dyn FnMut(&Nv12),
    ) -> Result<bool, String> {
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(provided),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        let mut status = 0;
        let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
        let sample = unsafe { ManuallyDrop::take(&mut buffers[0].pSample) };
        unsafe { ManuallyDrop::drop(&mut buffers[0].pEvents) };

        match result {
            Ok(()) => {}
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
            // Settled on a size after reading the stream's parameters.
            Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                choose_nv12(&self.transform).context("Decoding failed")?;
                self.format = Some(read_format(&self.transform).context("Decoding failed")?);
                return Ok(true);
            }
            // Before the first keyframe there's no format to allocate for.
            Err(_) if self.format.is_none() => return Ok(false),
            Err(err) => return Err(format!("Decoding failed: {}", err.message())),
        }

        let (Some(sample), Some(format)) = (sample, &self.format) else {
            return Ok(true);
        };
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }.context("Decoding failed")?;
        read_picture(&buffer, format, show).context("Decoding failed")?;
        Ok(true)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
    }
}

fn choose_nv12(transform: &IMFTransform) -> windows::core::Result<()> {
    let mut index = 0;
    loop {
        let offered = unsafe { transform.GetOutputAvailableType(0, index) }?;
        if unsafe { offered.GetGUID(&MF_MT_SUBTYPE) }? == MFVideoFormat_NV12 {
            return unsafe { transform.SetOutputType(0, &offered, 0) };
        }
        index += 1;
    }
}

fn read_format(transform: &IMFTransform) -> windows::core::Result<Format> {
    let current = unsafe { transform.GetOutputCurrentType(0) }?;
    let size = unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }?;
    let coded = ((size >> 32) as u32, size as u32);

    // The aperture crops the padding back off (1080 coded as 1088). Laid out
    // as MFVideoArea: offset, then size.
    let mut area = [0u8; 16];
    let visible = match unsafe { current.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, None) }
    {
        Ok(()) => {
            let width = i32::from_le_bytes(area[8..12].try_into().unwrap_or_default());
            let height = i32::from_le_bytes(area[12..16].try_into().unwrap_or_default());
            (
                (width.max(1) as u32).min(coded.0),
                (height.max(1) as u32).min(coded.1),
            )
        }
        Err(_) => coded,
    };

    let matrix = unsafe { current.GetUINT32(&MF_MT_YUV_MATRIX) }.unwrap_or(0);
    let range = unsafe { current.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }.unwrap_or(0);
    let info = unsafe { transform.GetOutputStreamInfo(0) }?;
    let provides =
        (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;

    Ok(Format {
        coded,
        visible,
        color: ColorSpace {
            bt709: matrix == MFVideoTransferMatrix_BT709.0 as u32,
            full_range: range == MFNominalRange_0_255.0 as u32,
        },
        buffer_size: if info.cbSize > 0 {
            info.cbSize
        } else {
            (coded.0 * coded.1 * 3 / 2).max(1)
        },
        provides_samples: info.dwFlags & provides != 0,
    })
}

fn read_picture(
    buffer: &IMFMediaBuffer,
    format: &Format,
    show: &mut dyn FnMut(&Nv12),
) -> windows::core::Result<()> {
    let (coded_width, coded_height) = (format.coded.0 as usize, format.coded.1 as usize);

    // A 2D buffer knows its pitch, which can exceed the picture width.
    if let Ok(planar) = buffer.cast::<IMF2DBuffer>() {
        let mut scanline = std::ptr::null_mut();
        let mut pitch = 0;
        unsafe { planar.Lock2D(&mut scanline, &mut pitch) }?;
        let pitch = pitch.unsigned_abs() as usize;
        if pitch >= coded_width {
            let length = pitch * coded_height * 3 / 2;
            let data = unsafe { std::slice::from_raw_parts(scanline, length) };
            show_planes(data, pitch, format, show);
        }
        return unsafe { planar.Unlock2D() };
    }

    let mut data = std::ptr::null_mut();
    let mut length = 0;
    unsafe { buffer.Lock(&mut data, None, Some(&mut length)) }?;
    let data = unsafe { std::slice::from_raw_parts(data, length as usize) };
    if data.len() >= coded_width * coded_height * 3 / 2 {
        show_planes(data, coded_width, format, show);
    }
    unsafe { buffer.Unlock() }
}

fn show_planes(data: &[u8], pitch: usize, format: &Format, show: &mut dyn FnMut(&Nv12)) {
    let (luma, chroma) = data.split_at(pitch * format.coded.1 as usize);
    show(&Nv12 {
        luma,
        chroma,
        pitch,
        width: format.visible.0 as usize,
        height: format.visible.1 as usize,
        color: format.color,
    });
}

struct MediaFoundation {
    /// False if the thread already had an apartment of another model.
    owns_com: bool,
}

impl MediaFoundation {
    fn startup() -> Result<Self, String> {
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

trait Context<T> {
    fn context(self, what: &str) -> Result<T, String>;
}

impl<T> Context<T> for windows::core::Result<T> {
    fn context(self, what: &str) -> Result<T, String> {
        self.map_err(|err| format!("{what}: {}", err.message()))
    }
}
