//! Windows.Graphics.Capture into Media Foundation's H.264 encoder.
//!
//! Frames stay on the GPU from capture to a hardware encoder; only the software
//! fallback reads back to system memory.
//!
//! Capture is polled rather than driven by the frame pool's event: the stream
//! wants a steady rate and a static screen produces no frames, so each tick
//! takes the newest frame or re-sends the last. Encoders make a repeat nearly free.

mod convert;
mod encode;
#[cfg(test)]
mod tests;

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
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows::core::Interface as _;

use super::{CaptureEvent, CaptureSettings, Control};
use convert::Converter;
use encode::Encoder;

/// A window or display the user picked.
pub struct CaptureSource {
    item: GraphicsCaptureItem,
}

impl CaptureSource {
    /// The window title or display name.
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
    // A desktop app has no CoreWindow for the picker to attach to.
    let init: IInitializeWithWindow = picker.cast().context("Couldn't open the capture picker")?;
    unsafe { init.Initialize(HWND(hwnd as *mut _)) }.context("Couldn't open the capture picker")?;
    let pick = picker
        .PickSingleItemAsync()
        .context("Couldn't open the capture picker")?;

    Ok(async move {
        // Kept alive until the pick resolves.
        let _picker = picker;
        // A cancelled pick resolves to a null item, which surfaces as an error.
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
    // Newer than capture itself; older builds keep the yellow border and cursor.
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

        // Several frames may have queued; only the newest is worth sending.
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
            // padded, until the pool is recreated.
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
            // Wall-clock stamp, not a tick count, so falling behind doesn't skew the stream's
            // clock.
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
            // Behind (slow encode, sleep): restart from now rather than race to catch up.
            None => next_tick = now,
        }
    }

    let _ = session.Close();
    let _ = pool.Close();
    Ok(())
}

/// Source fitted inside the maximum, rounded down to even for NV12's half-size chroma.
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

fn winrt_device(device: &ID3D11Device) -> Result<IDirect3DDevice, String> {
    let dxgi: IDXGIDevice = device
        .cast()
        .context("Couldn't share the Direct3D device")?;
    unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .and_then(|inspectable| inspectable.cast())
        .context("Couldn't share the Direct3D device")
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
