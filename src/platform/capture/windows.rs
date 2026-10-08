//! Windows.Graphics.Capture into Media Foundation's H.264 encoder.
//!
//! Frames stay on the GPU from capture to encoder where the machine allows
//! it: the captured BGRA texture is copied into one of ours, the Direct3D
//! video processor scales and converts it to NV12 in a single pass, and a
//! hardware encoder takes the NV12 texture as it is. Only the software
//! encoder, the fallback for a machine without one, reads the frame back to
//! system memory.
//!
//! Capture is polled rather than driven by the frame pool's event: the stream
//! wants frames at a steady rate, and a static screen produces none, so each
//! tick takes the newest frame if one arrived and re-sends the last one if
//! not. Encoders make a repeated frame almost free.

use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCapturePicker,
    GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows::core::Interface as _;

use super::{CaptureEvent, CaptureSettings, Control, EncodedFrame, nal_units};

/// NV12 textures the converter cycles through. A hardware encoder may still
/// be reading one when the next frame is converted, so each frame gets its
/// own; four covers the deepest pipeline the common encoders keep.
const RING: usize = 4;

/// Media Foundation counts in 100ns units.
const HNS_PER_SEC: u64 = 10_000_000;

/// Studio-range black, for the bars around a source that doesn't fill the
/// frame.
const BLACK: D3D11_VIDEO_COLOR_YCbCrA = D3D11_VIDEO_COLOR_YCbCrA {
    Y: 16. / 255.,
    Cb: 0.5,
    Cr: 0.5,
    A: 1.,
};

/// A window or display the user picked.
pub struct CaptureSource {
    item: GraphicsCaptureItem,
}

impl CaptureSource {
    /// What the system calls the source — the window title or display name.
    pub fn name(&self) -> String {
        self.item
            .DisplayName()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
    }
}

pub(super) fn is_supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

pub(super) fn pick(
    hwnd: isize,
) -> Result<impl Future<Output = Option<CaptureSource>> + use<>, String> {
    let picker = GraphicsCapturePicker::new().context("Couldn't open the capture picker")?;
    // A desktop app has no CoreWindow for the picker to attach to, so it's
    // told which window to be modal over instead.
    let init: IInitializeWithWindow = picker.cast().context("Couldn't open the capture picker")?;
    unsafe { init.Initialize(HWND(hwnd as *mut _)) }.context("Couldn't open the capture picker")?;
    let pick = picker
        .PickSingleItemAsync()
        .context("Couldn't open the capture picker")?;

    Ok(async move {
        // Kept alive until the pick resolves.
        let _picker = picker;
        // A cancelled pick resolves to a null item, which surfaces as an
        // error; either way there's nothing to share.
        pick.await.ok().map(|item| CaptureSource { item })
    })
}

pub(super) fn capture(
    source: CaptureSource,
    settings: CaptureSettings,
    control: &Control,
    events: &UnboundedSender<CaptureEvent>,
) -> Result<(), String> {
    let _platform = MediaFoundation::startup()?;

    let (device, context) = create_device()?;
    let rt_device = winrt_device(&device)?;

    let item = source.item;
    let mut pool_size = item.Size().context("Couldn't read the source's size")?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &rt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        pool_size,
    )
    .context("Couldn't start capturing")?;
    let session = pool
        .CreateCaptureSession(&item)
        .context("Couldn't start capturing")?;
    // Both are newer than capture itself; on a build without them, the yellow
    // border stays and the cursor is captured anyway.
    let _ = session.SetIsBorderRequired(false);
    let _ = session.SetIsCursorCaptureEnabled(true);

    let closed = Arc::new(AtomicBool::new(false));
    let closed_flag = closed.clone();
    item.Closed(&TypedEventHandler::new(move |_, _| {
        closed_flag.store(true, Ordering::Release);
        Ok(())
    }))
    .context("Couldn't start capturing")?;
    session.StartCapture().context("Couldn't start capturing")?;

    let out_size = output_size(pool_size, &settings);
    let mut converter = Converter::new(&device, &context, out_size, settings.fps)?;
    let mut encoder = Encoder::new(&device, &context, out_size, &settings)?;

    if events
        .unbounded_send(CaptureEvent::Started {
            width: out_size.0,
            height: out_size.1,
        })
        .is_err()
    {
        return Ok(());
    }

    let interval = Duration::from_secs(1) / settings.fps;
    let started = Instant::now();
    let mut next_tick = started;
    let mut have_frame = false;

    loop {
        if control.is_stopped() || closed.load(Ordering::Acquire) {
            break;
        }

        // Several frames may have queued since the last tick; only the newest
        // is worth sending.
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        while let Ok(frame) = pool.TryGetNextFrame() {
            newest = Some(frame);
        }
        if let Some(frame) = newest {
            let size = frame
                .ContentSize()
                .context("Couldn't read a captured frame")?;
            converter.load(&frame, size)?;
            // A resized window keeps arriving in the old buffers, cropped or
            // padded, until the pool is rebuilt at the new size.
            if size.Width != pool_size.Width || size.Height != pool_size.Height {
                pool.Recreate(
                    &rt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    2,
                    size,
                )
                .context("Couldn't follow the source's new size")?;
                pool_size = size;
            }
            have_frame = true;
        }

        next_tick += interval;

        if have_frame {
            let texture = converter.convert()?;
            let keyframe = control.take_keyframe();
            // Stamped with when it was taken rather than counted, so a tick
            // the loop fell behind on doesn't put the stream's clock out.
            let taken = started.elapsed();
            for frame in encoder.encode(&texture, taken, keyframe, next_tick)? {
                if events.unbounded_send(CaptureEvent::Frame(frame)).is_err() {
                    return Ok(());
                }
            }
        }

        let now = Instant::now();
        match next_tick.checked_duration_since(now) {
            Some(wait) => std::thread::sleep(wait),
            // Running behind — a slow encode, or the machine was asleep.
            // Start counting again from now rather than racing to catch up.
            None => next_tick = now,
        }
    }

    let _ = session.Close();
    let _ = pool.Close();
    Ok(())
}

/// The stream's frame size: the source fitted inside the configured maximum,
/// rounded down to even dimensions, which NV12's half-size chroma needs.
fn output_size(source: SizeInt32, settings: &CaptureSettings) -> (u32, u32) {
    let width = source.Width.max(2) as f64;
    let height = source.Height.max(2) as f64;
    let scale = (f64::from(settings.max_width) / width)
        .min(f64::from(settings.max_height) / height)
        .min(1.);
    let even = |value: f64| ((value * scale) as u32 & !1).max(2);
    (even(width), even(height))
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .context("Couldn't create a Direct3D device")?;
    let (Some(device), Some(context)) = (device, context) else {
        return Err("Couldn't create a Direct3D device".into());
    };

    // The encoder's own threads use the device alongside this one.
    if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
        unsafe {
            let _ = multithread.SetMultithreadProtected(true);
        }
    }
    Ok((device, context))
}

/// The same device, as the WinRT capture API takes it.
fn winrt_device(device: &ID3D11Device) -> Result<IDirect3DDevice, String> {
    let dxgi: IDXGIDevice = device
        .cast()
        .context("Couldn't share the Direct3D device")?;
    unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .and_then(|inspectable| inspectable.cast())
        .context("Couldn't share the Direct3D device")
}

/// Scales and converts captured frames into the encoder's NV12.
struct Converter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    out_size: (u32, u32),
    fps: u32,
    ring: Vec<ID3D11Texture2D>,
    next: usize,
    /// Rebuilt whenever the source changes size, since the processor is
    /// created for one input size.
    pipeline: Option<Pipeline>,
}

struct Pipeline {
    size: (u32, u32),
    /// Our own copy of the latest frame. The capture's textures go back to
    /// its pool as soon as they're copied, and theirs aren't created with the
    /// bindings the video processor wants as input anyway.
    input: ID3D11Texture2D,
    input_view: ID3D11VideoProcessorInputView,
    outputs: Vec<ID3D11VideoProcessorOutputView>,
    processor: ID3D11VideoProcessor,
}

impl Converter {
    fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        out_size: (u32, u32),
        fps: u32,
    ) -> Result<Self, String> {
        let video_device: ID3D11VideoDevice = device
            .cast()
            .context("This GPU can't convert video frames")?;
        let video_context: ID3D11VideoContext = context
            .cast()
            .context("This GPU can't convert video frames")?;

        let ring = (0..RING)
            .map(|_| {
                texture(
                    device,
                    out_size,
                    DXGI_FORMAT_NV12,
                    D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0,
                    D3D11_USAGE_DEFAULT,
                    0,
                )
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            device: device.clone(),
            context: context.clone(),
            video_device,
            video_context,
            out_size,
            fps,
            ring,
            next: 0,
            pipeline: None,
        })
    }

    /// Takes a copy of `frame`, which can then go back to the pool.
    fn load(&mut self, frame: &Direct3D11CaptureFrame, content: SizeInt32) -> Result<(), String> {
        let surface = frame.Surface().context("Couldn't read a captured frame")?;
        let access: IDirect3DDxgiInterfaceAccess =
            surface.cast().context("Couldn't read a captured frame")?;
        let captured: ID3D11Texture2D =
            unsafe { access.GetInterface() }.context("Couldn't read a captured frame")?;

        // The buffer can be bigger than what's in it while a resize is
        // settling; only the content is copied.
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { captured.GetDesc(&mut desc) };
        let size = (
            (content.Width.max(1) as u32).min(desc.Width),
            (content.Height.max(1) as u32).min(desc.Height),
        );

        if self.pipeline.as_ref().is_none_or(|p| p.size != size) {
            self.pipeline = Some(self.build(size)?);
        }
        let Some(pipeline) = &self.pipeline else {
            return Ok(());
        };

        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: size.0,
            bottom: size.1,
            back: 1,
        };
        unsafe {
            self.context.CopySubresourceRegion(
                &pipeline.input,
                0,
                0,
                0,
                0,
                &captured,
                0,
                Some(&region),
            );
        }
        Ok(())
    }

    fn build(&self, size: (u32, u32)) -> Result<Pipeline, String> {
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: self.fps,
                Denominator: 1,
            },
            InputWidth: size.0,
            InputHeight: size.1,
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: self.fps,
                Denominator: 1,
            },
            OutputWidth: self.out_size.0,
            OutputHeight: self.out_size.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        let enumerator = unsafe { self.video_device.CreateVideoProcessorEnumerator(&content) }
            .context("This GPU can't convert video frames")?;
        let processor = unsafe { self.video_device.CreateVideoProcessor(&enumerator, 0) }
            .context("This GPU can't convert video frames")?;

        let input = texture(
            &self.device,
            size,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0,
            D3D11_USAGE_DEFAULT,
            0,
        )?;

        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                &input,
                &enumerator,
                &input_desc,
                Some(&mut input_view),
            )
        }
        .context("This GPU can't convert video frames")?;
        let input_view = input_view.ok_or("This GPU can't convert video frames")?;

        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let outputs = self
            .ring
            .iter()
            .map(|texture| {
                let mut view = None;
                unsafe {
                    self.video_device.CreateVideoProcessorOutputView(
                        texture,
                        &enumerator,
                        &output_desc,
                        Some(&mut view),
                    )
                }
                .context("This GPU can't convert video frames")?;
                view.ok_or_else(|| "This GPU can't convert video frames".to_string())
            })
            .collect::<Result<_, _>>()?;

        let source = rect((0, 0), size);
        let target = rect((0, 0), self.out_size);
        let fitted = fit(size, self.out_size);
        // Full-range RGB in; studio-range BT.709 out, which is what every
        // decoder assumes of a stream that doesn't say otherwise.
        let rgb_full = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 };
        let bt709_studio = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: (1 << 2) | (1 << 4),
        };
        let background = D3D11_VIDEO_COLOR {
            Anonymous: D3D11_VIDEO_COLOR_0 { YCbCr: BLACK },
        };
        unsafe {
            let vc = &self.video_context;
            vc.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            vc.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&source));
            vc.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&fitted));
            vc.VideoProcessorSetOutputTargetRect(&processor, true, Some(&target));
            vc.VideoProcessorSetStreamColorSpace(&processor, 0, &rgb_full);
            vc.VideoProcessorSetOutputColorSpace(&processor, &bt709_studio);
            vc.VideoProcessorSetOutputBackgroundColor(&processor, true, &background);
            // Denoising and edge enhancement are for camera footage; on text
            // they only smear.
            vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        }

        Ok(Pipeline {
            size,
            input,
            input_view,
            outputs,
            processor,
        })
    }

    /// Converts the latest frame into the next NV12 texture of the ring.
    fn convert(&mut self) -> Result<ID3D11Texture2D, String> {
        let Some(pipeline) = &self.pipeline else {
            return Err("No frame has been captured yet".into());
        };
        let index = self.next;
        self.next = (self.next + 1) % self.ring.len();

        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(pipeline.input_view.clone())),
            ..Default::default()
        };
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &pipeline.processor,
                &pipeline.outputs[index],
                0,
                std::slice::from_ref(&stream),
            )
        };
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        result.context("Couldn't convert a captured frame")?;

        Ok(self.ring[index].clone())
    }
}

/// `inner` scaled to fit inside `outer` and centred in it, on even pixels so
/// the bars don't split a chroma sample.
fn fit(inner: (u32, u32), outer: (u32, u32)) -> RECT {
    let scale =
        (f64::from(outer.0) / f64::from(inner.0)).min(f64::from(outer.1) / f64::from(inner.1));
    let width = ((f64::from(inner.0) * scale) as u32 & !1).clamp(2, outer.0);
    let height = ((f64::from(inner.1) * scale) as u32 & !1).clamp(2, outer.1);
    let left = ((outer.0 - width) / 2) & !1;
    let top = ((outer.1 - height) / 2) & !1;
    rect((left, top), (width, height))
}

fn rect(origin: (u32, u32), size: (u32, u32)) -> RECT {
    RECT {
        left: origin.0 as i32,
        top: origin.1 as i32,
        right: (origin.0 + size.0) as i32,
        bottom: (origin.1 + size.1) as i32,
    }
}

fn texture(
    device: &ID3D11Device,
    size: (u32, u32),
    format: DXGI_FORMAT,
    bind: i32,
    usage: D3D11_USAGE,
    cpu_access: i32,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind as u32,
        CPUAccessFlags: cpu_access as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .context("Couldn't allocate a video frame")?;
    texture.ok_or_else(|| "Couldn't allocate a video frame".into())
}

/// A Media Foundation H.264 encoder, driven one frame at a time.
///
/// Hardware encoders are asynchronous: they announce when they want input and
/// when they have output as events, and refuse input they didn't ask for.
/// The software encoder is synchronous. Both are driven through the same
/// [`Encoder::encode`], which feeds one frame and hands back whatever output
/// is ready.
struct Encoder {
    transform: IMFTransform,
    /// The event queue of an asynchronous encoder; `None` for a synchronous
    /// one.
    events: Option<IMFMediaEventGenerator>,
    codec: Option<ICodecAPI>,
    /// How many inputs the encoder has asked for and not yet been given.
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
    /// The encoder allocates its own output samples.
    provides_samples: bool,
    buffer_size: u32,
}

impl Encoder {
    fn new(
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

        // A machine can list several hardware encoders — one per GPU — and
        // only the one on the GPU the device was made on will accept it.
        for activate in encoders(MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER) {
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

        for activate in encoders(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER) {
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
            // An asynchronous encoder stays locked until the caller says it
            // knows how to drive one.
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

    fn software(
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

    /// Feeds one frame and returns whatever the encoder has finished by
    /// `deadline`, when the next frame is due.
    fn encode(
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

        // Input it didn't ask for is refused, so wait until it does — which
        // is never long, since it asks again as soon as it has room.
        while self.wanted == 0 {
            let event = unsafe { events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) }
                .context("Encoding failed")?;
            self.handle(&event, &mut out)?;
        }
        unsafe { self.transform.ProcessInput(0, &sample, 0) }.context("Encoding failed")?;
        self.wanted -= 1;

        // Give the encoder until the next frame is due to finish this one, so
        // it goes out now rather than a tick later with the next. An encoder
        // that pipelines may not finish it at all until it has more input;
        // waiting stops at the deadline either way, so pacing doesn't slip.
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

    /// Wraps a converted frame in a sample the encoder will take.
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

    /// Copies an NV12 texture into system memory, tightly packed.
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
            // The chroma plane starts right after the luma plane's rows,
            // which are `pitch` apart rather than `width`.
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

    /// Pulls one finished frame out of the encoder. Returns whether there may
    /// be more.
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
            // The encoder settled on its output format; take what it offers.
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

    /// Makes sure a keyframe carries its SPS and PPS. Some encoders put them
    /// only on the first one, and a viewer who joins later needs them on
    /// whichever keyframe they start from.
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

/// Sets up an encoder for the stream: low latency, constant bitrate, no
/// B-frames, then the output and input formats, in that order — encoders
/// insist on the output first.
fn configure(
    transform: &IMFTransform,
    codec: Option<&ICodecAPI>,
    size: (u32, u32),
    settings: &CaptureSettings,
) -> windows::core::Result<()> {
    if let Some(codec) = codec {
        // Each is a preference: an encoder that doesn't support one still
        // produces a stream viewers can watch.
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
            // B-frames are decoded out of order, which a viewer pays for in
            // latency; a live stream has no use for them.
            let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0u32));
            // Keyframes are mostly sent on request, when a viewer joins or
            // loses one. This is the backstop for a request that's lost.
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
    // Constrained baseline is what every WebRTC decoder is guaranteed to
    // handle; not every encoder offers it, so step up until one is accepted.
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
        // Some encoders don't say; an uncompressed frame is a bound no
        // compressed one exceeds.
        buffer_size: if info.cbSize > 0 {
            info.cbSize
        } else {
            size.0 * size.1 * 3 / 2
        },
    })
}

/// The encoder's SPS and PPS, as Annex B.
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

fn encoders(flags: MFT_ENUM_FLAG) -> Vec<IMFActivate> {
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
    let found = unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            Some(&input),
            Some(&output),
            &mut list,
            &mut count,
        )
    };
    if found.is_err() || list.is_null() {
        return Vec::new();
    }
    // The array is ours to free, and each entry ours to release; taking them
    // out leaves the array empty for the free.
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

/// COM and Media Foundation, for the life of the capture thread.
struct MediaFoundation {
    /// Whether this thread's COM apartment is ours to close.
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

/// Attaches a human-readable cause to a Windows error.
trait Context<T> {
    fn context(self, what: &str) -> Result<T, String>;
}

impl<T> Context<T> for windows::core::Result<T> {
    fn context(self, what: &str) -> Result<T, String> {
        self.map_err(|err| format!("{what}: {}", err.message()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use futures::StreamExt as _;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

    use super::*;

    /// Runs the whole pipeline against the primary display, without the
    /// picker: capture, conversion, and whichever encoder this machine picks.
    #[test]
    #[ignore = "captures the primary display"]
    fn encodes_the_primary_display() {
        let _com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .expect("capture interop");
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(monitor) }.expect("capture item for the display");

        let control = Arc::new(Control {
            stopped: AtomicBool::new(false),
            keyframe: AtomicBool::new(false),
        });
        let settings = CaptureSettings {
            max_width: 1280,
            max_height: 720,
            fps: 30,
            bitrate: 2_500_000,
        };
        let (events, mut received) = futures::channel::mpsc::unbounded();

        let timer = control.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            timer.keyframe.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(1500));
            timer.stopped.store(true, Ordering::Release);
        });
        capture(CaptureSource { item }, settings, &control, &events).expect("capture runs");
        drop(events);

        let mut size = None;
        let mut frames = Vec::new();
        while let Some(event) = futures::executor::block_on(received.next()) {
            match event {
                CaptureEvent::Started { width, height } => size = Some((width, height)),
                CaptureEvent::Frame(frame) => frames.push(frame),
                CaptureEvent::SourceClosed => panic!("the display closed"),
                CaptureEvent::Failed(err) => panic!("capture failed: {err}"),
            }
        }

        let (width, height) = size.expect("capture reported its size");
        let keyframes = frames.iter().filter(|frame| frame.keyframe).count();
        let bytes: usize = frames.iter().map(|frame| frame.data.len()).sum();
        let span = frames.last().unwrap().timestamp - frames[0].timestamp;
        eprintln!(
            "{width}x{height}: {} frames over {span:?}, {keyframes} keyframes, {bytes} bytes",
            frames.len()
        );
        assert!(width <= 1280 && height <= 720 && width % 2 == 0 && height % 2 == 0);
        // Three seconds at 30fps, less what the encoder is still holding.
        assert!(frames.len() > 60, "only {} frames", frames.len());

        let first = &frames[0];
        assert!(first.keyframe, "the stream must open on a keyframe");
        let kinds: Vec<u8> = nal_units(&first.data)
            .iter()
            .map(|nal| nal[0] & 0x1f)
            .collect();
        for (kind, name) in [(7, "SPS"), (8, "PPS"), (5, "IDR slice")] {
            assert!(
                kinds.contains(&kind),
                "first frame has no {name}: {kinds:?}"
            );
        }
        // The one asked for halfway through, on top of the first.
        assert!(keyframes >= 2, "a requested keyframe never came");
        for frame in frames.iter().filter(|frame| frame.keyframe) {
            let kinds: Vec<u8> = nal_units(&frame.data)
                .iter()
                .map(|nal| nal[0] & 0x1f)
                .collect();
            assert!(kinds.contains(&7), "a keyframe without its SPS: {kinds:?}");
        }

        if let Ok(path) = std::env::var("CAPTURE_DUMP") {
            let stream: Vec<u8> = frames.iter().flat_map(|frame| frame.data.clone()).collect();
            std::fs::write(path, stream).expect("dump written");
        }
    }

    /// The fallback for a machine without a hardware encoder, which reads
    /// frames back from the GPU rather than handing over textures.
    #[test]
    #[ignore = "needs a GPU"]
    fn software_encoder_encodes() {
        let _platform = MediaFoundation::startup().expect("media foundation");
        let (device, context) = create_device().expect("device");
        let settings = CaptureSettings {
            max_width: 640,
            max_height: 360,
            fps: 30,
            bitrate: 1_000_000,
        };
        let size = (640, 360);
        let activate = encoders(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER)
            .into_iter()
            .next()
            .expect("a software H.264 encoder");
        let mut encoder =
            Encoder::software(&activate, &device, &context, size, &settings).expect("encoder");
        let frame = texture(
            &device,
            size,
            DXGI_FORMAT_NV12,
            D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0,
            D3D11_USAGE_DEFAULT,
            0,
        )
        .expect("frame");

        let mut frames = Vec::new();
        for n in 0..30u32 {
            let taken = Duration::from_millis(u64::from(n) * 33);
            let deadline = Instant::now() + Duration::from_millis(33);
            frames.extend(
                encoder
                    .encode(&frame, taken, n == 15, deadline)
                    .expect("encodes"),
            );
        }

        assert!(frames.len() >= 25, "only {} frames", frames.len());
        assert!(frames[0].keyframe, "the stream must open on a keyframe");
        let kinds: Vec<u8> = nal_units(&frames[0].data)
            .iter()
            .map(|nal| nal[0] & 0x1f)
            .collect();
        assert!(kinds.contains(&7) && kinds.contains(&5), "{kinds:?}");
        assert!(
            frames.iter().skip(1).any(|frame| frame.keyframe),
            "requested keyframe missing"
        );
    }
}
