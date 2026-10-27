//! Media Foundation's H.264 encoder: finding, configuring and driving one.

use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::VARIANT;
use windows::core::Interface as _;

use super::{Context, texture};
use crate::platform::capture::{CaptureSettings, EncodedFrame, nal_units};

/// Media Foundation counts in 100ns units.
const HNS_PER_SEC: u64 = 10_000_000;

/// Hardware encoders are asynchronous: they announce wanted input and ready
/// output as events and refuse input they didn't ask for. The software encoder
/// is synchronous. [`Encoder::encode`] drives both.
pub(super) struct Encoder {
    transform: IMFTransform,
    /// `None` for a synchronous encoder.
    events: Option<IMFMediaEventGenerator>,
    codec: Option<ICodecAPI>,
    wanted: u32,
    /// Readback for the software encoder, which can't take GPU textures.
    staging: Option<ID3D11Texture2D>,
    context: ID3D11DeviceContext,
    size: (u32, u32),
    frame_hns: u64,
    output: OutputInfo,
    /// SPS and PPS, for encoders that don't repeat them on every keyframe.
    parameter_sets: Option<Vec<u8>>,
    /// Kept alive for the encoder, which holds only a weak claim on it.
    _manager: Option<IMFDXGIDeviceManager>,
}

#[derive(Clone, Copy)]
struct OutputInfo {
    provides_samples: bool,
    buffer_size: u32,
}

impl Encoder {
    pub(super) fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        size: (u32, u32),
        settings: &CaptureSettings,
    ) -> Result<Self, String> {
        let mut reset_token = 0;
        let mut manager = None;
        unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager) }
            .context("Couldn't share the GPU with the encoder")?;
        let manager = manager.ok_or("Couldn't share the GPU with the encoder")?;
        unsafe { manager.ResetDevice(device, reset_token) }
            .context("Couldn't share the GPU with the encoder")?;

        // Only the encoder on the device's own GPU will accept it; asking for
        // that one alone spares the others a failed start, which some drivers
        // (NVIDIA's) report loudly.
        let adapter = adapter_luid(device);
        for activate in encoders(
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            adapter,
        ) {
            match Self::hardware(&activate, &manager, context, size, settings) {
                Ok(encoder) => return Ok(encoder),
                Err(err) => {
                    eprintln!("skipping hardware encoder {}: {err}", name_of(&activate));
                    unsafe {
                        let _ = activate.ShutdownObject();
                    }
                }
            }
        }

        for activate in encoders(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER, None) {
            match Self::software(&activate, device, context, size, settings) {
                Ok(encoder) => return Ok(encoder),
                Err(err) => eprintln!("skipping encoder {}: {err}", name_of(&activate)),
            }
        }

        Err("No H.264 encoder on this machine would take the stream".into())
    }

    fn hardware(
        activate: &IMFActivate,
        manager: &IMFDXGIDeviceManager,
        context: &ID3D11DeviceContext,
        size: (u32, u32),
        settings: &CaptureSettings,
    ) -> windows::core::Result<Self> {
        let transform: IMFTransform = unsafe { activate.ActivateObject() }?;
        let attributes = unsafe { transform.GetAttributes() }?;
        unsafe {
            // An asynchronous encoder stays locked until told the caller can drive one.
            attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
        }

        let codec = transform.cast::<ICodecAPI>().ok();
        configure(&transform, codec.as_ref(), size, settings)?;
        let events = transform.cast::<IMFMediaEventGenerator>()?;

        let encoder = Self {
            output: output_info(&transform, size)?,
            transform,
            events: Some(events),
            codec,
            wanted: 0,
            staging: None,
            context: context.clone(),
            size,
            frame_hns: HNS_PER_SEC / u64::from(settings.fps),
            parameter_sets: None,
            _manager: Some(manager.clone()),
        };
        encoder.begin()?;
        eprintln!("streaming with hardware encoder {}", name_of(activate));
        Ok(encoder)
    }

    pub(super) fn software(
        activate: &IMFActivate,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        size: (u32, u32),
        settings: &CaptureSettings,
    ) -> Result<Self, String> {
        let transform: IMFTransform =
            unsafe { activate.ActivateObject() }.context("Couldn't start the encoder")?;
        if let Ok(attributes) = unsafe { transform.GetAttributes() } {
            let _ = unsafe { attributes.SetUINT32(&MF_LOW_LATENCY, 1) };
        }
        let codec = transform.cast::<ICodecAPI>().ok();
        configure(&transform, codec.as_ref(), size, settings)
            .context("The encoder refused the stream's format")?;

        let staging = texture(
            device,
            size,
            DXGI_FORMAT_NV12,
            0,
            D3D11_USAGE_STAGING,
            D3D11_CPU_ACCESS_READ.0,
        )?;

        let encoder = Self {
            output: output_info(&transform, size).context("Couldn't start the encoder")?,
            transform,
            events: None,
            codec,
            wanted: 0,
            staging: Some(staging),
            context: context.clone(),
            size,
            frame_hns: HNS_PER_SEC / u64::from(settings.fps),
            parameter_sets: None,
            _manager: None,
        };
        encoder.begin().context("Couldn't start the encoder")?;
        eprintln!("streaming with software encoder {}", name_of(activate));
        Ok(encoder)
    }

    fn begin(&self) -> windows::core::Result<()> {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
        }
    }

    /// Returns whatever the encoder has finished by `deadline`, when the next frame is due.
    pub(super) fn encode(
        &mut self,
        frame: &ID3D11Texture2D,
        taken: Duration,
        keyframe: bool,
        deadline: Instant,
    ) -> Result<Vec<EncodedFrame>, String> {
        if keyframe && let Some(codec) = &self.codec {
            let _ =
                unsafe { codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32)) };
        }

        let sample = self.sample(frame, taken)?;
        let mut out = Vec::new();

        let Some(events) = self.events.clone() else {
            unsafe { self.transform.ProcessInput(0, &sample, 0) }.context("Encoding failed")?;
            while self.take_output(&mut out)? {}
            return Ok(out);
        };

        // Unrequested input is refused, so wait for the request.
        while self.wanted == 0 {
            let event = unsafe { events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) }
                .context("Encoding failed")?;
            self.handle(&event, &mut out)?;
        }
        unsafe { self.transform.ProcessInput(0, &sample, 0) }.context("Encoding failed")?;
        self.wanted -= 1;

        // Wait until the deadline so this frame goes out now, not a tick late. A
        // pipelining encoder may not finish it before more input; the deadline
        // keeps pacing from slipping.
        let before = out.len();
        loop {
            match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(event) => self.handle(&event, &mut out)?,
                Err(err) if err.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    if out.len() > before || Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(err) => return Err(format!("Encoding failed: {err}")),
            }
        }
        Ok(out)
    }

    fn handle(&mut self, event: &IMFMediaEvent, out: &mut Vec<EncodedFrame>) -> Result<(), String> {
        let kind = unsafe { event.GetType() }.context("Encoding failed")?;
        if kind == METransformNeedInput.0 as u32 {
            self.wanted += 1;
        } else if kind == METransformHaveOutput.0 as u32 {
            self.take_output(out)?;
        }
        Ok(())
    }

    fn sample(&self, frame: &ID3D11Texture2D, taken: Duration) -> Result<IMFSample, String> {
        let buffer = match &self.staging {
            None => {
                let buffer =
                    unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, frame, 0, false) }
                        .context("Couldn't hand a frame to the encoder")?;
                if let Ok(planar) = buffer.cast::<IMF2DBuffer>() {
                    let length = unsafe { planar.GetContiguousLength() }
                        .context("Couldn't hand a frame to the encoder")?;
                    unsafe { buffer.SetCurrentLength(length) }
                        .context("Couldn't hand a frame to the encoder")?;
                }
                buffer
            }
            Some(staging) => self.read_back(frame, staging)?,
        };

        let frame_hns = self.frame_hns;
        (|| unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime((taken.as_nanos() / 100) as i64)?;
            sample.SetSampleDuration(frame_hns as i64)?;
            Ok(sample)
        })()
        .context("Couldn't hand a frame to the encoder")
    }

    fn read_back(
        &self,
        frame: &ID3D11Texture2D,
        staging: &ID3D11Texture2D,
    ) -> Result<IMFMediaBuffer, String> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let length = width * height * 3 / 2;

        unsafe { self.context.CopyResource(staging, frame) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .context("Couldn't read a frame back for the encoder")?;

        let result = (|| {
            let buffer = unsafe { MFCreateMemoryBuffer(length as u32) }?;
            let mut data = std::ptr::null_mut();
            unsafe { buffer.Lock(&mut data, None, None) }?;
            let pitch = mapped.RowPitch as usize;
            let source = mapped.pData as *const u8;
            // Chroma follows the luma rows, which are `pitch` apart, not `width`.
            for row in 0..height + height / 2 {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        source.add(row * pitch),
                        data.add(row * width),
                        width,
                    );
                }
            }
            unsafe {
                buffer.Unlock()?;
                buffer.SetCurrentLength(length as u32)?;
            }
            windows::core::Result::Ok(buffer)
        })();

        unsafe { self.context.Unmap(staging, 0) };
        result.context("Couldn't read a frame back for the encoder")
    }

    /// Returns whether there may be more output.
    fn take_output(&mut self, out: &mut Vec<EncodedFrame>) -> Result<bool, String> {
        let provided = if self.output.provides_samples {
            None
        } else {
            let sample = unsafe { MFCreateSample() }.context("Encoding failed")?;
            let buffer = unsafe { MFCreateMemoryBuffer(self.output.buffer_size) }
                .context("Encoding failed")?;
            unsafe { sample.AddBuffer(&buffer) }.context("Encoding failed")?;
            Some(sample)
        };

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
            // The encoder settled on its output format; accept it.
            Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                (|| unsafe {
                    let offered = self.transform.GetOutputAvailableType(0, 0)?;
                    self.transform.SetOutputType(0, &offered, 0)
                })()
                .context("Encoding failed")?;
                self.output = output_info(&self.transform, self.size).context("Encoding failed")?;
                self.parameter_sets = None;
                return Ok(true);
            }
            Err(err) => return Err(format!("Encoding failed: {err}")),
        }

        let Some(sample) = sample else {
            return Ok(true);
        };
        let keyframe = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0;
        let timestamp = unsafe { sample.GetSampleTime() }.unwrap_or(0).max(0) as u64;

        let data = (|| unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut pointer = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut pointer, None, Some(&mut length))?;
            let data = std::slice::from_raw_parts(pointer, length as usize).to_vec();
            buffer.Unlock()?;
            Ok(data)
        })()
        .context("Encoding failed")?;

        let data = if keyframe {
            self.with_parameter_sets(data)
        } else {
            data
        };
        out.push(EncodedFrame {
            data,
            keyframe,
            timestamp: Duration::from_nanos(timestamp * 100),
        });
        Ok(true)
    }

    /// Some encoders put SPS/PPS only on the first keyframe, but a late joiner
    /// needs them on whichever one they start from.
    fn with_parameter_sets(&mut self, frame: Vec<u8>) -> Vec<u8> {
        if nal_units(&frame)
            .iter()
            .any(|nal| nal.first().is_some_and(|b| b & 0x1f == 7))
        {
            return frame;
        }
        if self.parameter_sets.is_none() {
            self.parameter_sets = sequence_header(&self.transform);
        }
        match &self.parameter_sets {
            Some(header) => [header.as_slice(), &frame].concat(),
            None => frame,
        }
    }
}

impl Drop for Encoder {
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

/// Low latency, CBR, no B-frames, then output and input types; encoders insist
/// on the output type first.
fn configure(
    transform: &IMFTransform,
    codec: Option<&ICodecAPI>,
    size: (u32, u32),
    settings: &CaptureSettings,
) -> windows::core::Result<()> {
    if let Some(codec) = codec {
        // Each is a preference; an encoder lacking one still streams fine.
        unsafe {
            let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            let _ = codec.SetValue(
                &CODECAPI_AVEncCommonRateControlMode,
                &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
            );
            let _ = codec.SetValue(
                &CODECAPI_AVEncCommonMeanBitRate,
                &VARIANT::from(settings.bitrate),
            );
            // B-frames cost viewers latency.
            let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0u32));
            // Keyframes mostly come on request; this backstops a lost request.
            let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(settings.fps * 4));
        }
    }

    let output = unsafe { MFCreateMediaType() }?;
    unsafe {
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        output.SetUINT32(&MF_MT_AVG_BITRATE, settings.bitrate)?;
        output.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1))?;
        output.SetUINT64(&MF_MT_FRAME_RATE, pack(settings.fps, 1))?;
        output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    }
    // Constrained baseline is what every WebRTC decoder handles; not every
    // encoder offers it, so step up until one is accepted.
    let mut accepted = Err(windows::core::Error::from(MF_E_INVALIDMEDIATYPE));
    for profile in [
        eAVEncH264VProfile_ConstrainedBase,
        eAVEncH264VProfile_Base,
        eAVEncH264VProfile_Main,
    ] {
        unsafe { output.SetUINT32(&MF_MT_MPEG2_PROFILE, profile.0 as u32)? };
        accepted = unsafe { transform.SetOutputType(0, &output, 0) };
        if accepted.is_ok() {
            break;
        }
    }
    accepted?;

    let input = unsafe { MFCreateMediaType() }?;
    unsafe {
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1))?;
        input.SetUINT64(&MF_MT_FRAME_RATE, pack(settings.fps, 1))?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        transform.SetInputType(0, &input, 0)
    }
}

fn output_info(transform: &IMFTransform, size: (u32, u32)) -> windows::core::Result<OutputInfo> {
    let info = unsafe { transform.GetOutputStreamInfo(0) }?;
    let provides =
        (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
    Ok(OutputInfo {
        provides_samples: info.dwFlags & provides != 0,
        // Some encoders don't say; an uncompressed frame bounds a compressed one.
        buffer_size: if info.cbSize > 0 {
            info.cbSize
        } else {
            size.0 * size.1 * 3 / 2
        },
    })
}

fn sequence_header(transform: &IMFTransform) -> Option<Vec<u8>> {
    unsafe {
        let current = transform.GetOutputCurrentType(0).ok()?;
        let size = current.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()?;
        let mut header = vec![0; size as usize];
        current
            .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut header, None)
            .ok()?;
        Some(header)
    }
}

fn adapter_luid(device: &ID3D11Device) -> Option<LUID> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter() }.ok()?;
    unsafe { adapter.GetDesc() }
        .ok()
        .map(|desc| desc.AdapterLuid)
}

/// H.264 encoders taking NV12, limited to one GPU's when `adapter` is given.
pub(super) fn encoders(flags: MFT_ENUM_FLAG, adapter: Option<LUID>) -> Vec<IMFActivate> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut list = std::ptr::null_mut();
    let mut count = 0;
    let attributes = adapter.and_then(|luid| {
        let mut attributes = None;
        unsafe {
            MFCreateAttributes(&mut attributes, 1).ok()?;
            let attributes = attributes?;
            let bytes =
                std::slice::from_raw_parts((&luid as *const LUID).cast::<u8>(), size_of::<LUID>());
            attributes.SetBlob(&MFT_ENUM_ADAPTER_LUID, bytes).ok()?;
            Some(attributes)
        }
    });
    let found = unsafe {
        MFTEnum2(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            Some(&input),
            Some(&output),
            attributes.as_ref(),
            &mut list,
            &mut count,
        )
    };
    if found.is_err() || list.is_null() {
        return Vec::new();
    }
    // The array and each entry are ours to free; taking the entries leaves the array empty for the
    // free.
    let activates = unsafe { std::slice::from_raw_parts_mut(list, count as usize) }
        .iter_mut()
        .filter_map(Option::take)
        .collect();
    unsafe { CoTaskMemFree(Some(list as *const _)) };
    activates
}

fn name_of(activate: &IMFActivate) -> String {
    let mut name = windows::core::PWSTR::null();
    let mut length = 0;
    unsafe {
        if activate
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name, &mut length)
            .is_err()
        {
            return "(unnamed)".into();
        }
        let text = name.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name.0 as *const _));
        text
    }
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}
