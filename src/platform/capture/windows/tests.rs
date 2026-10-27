use std::sync::atomic::AtomicBool;

use futures::StreamExt as _;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use super::encode::encoders;
use super::*;
use crate::platform::capture::nal_units;

/// Whole pipeline against the primary display, bypassing the picker.
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
    // 3s at 30fps, less what the encoder still holds.
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
    // The one requested halfway, plus the first.
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

/// The software encoder path, which reads frames back from the GPU.
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
    let activate = encoders(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER, None)
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

    // Round trip through the viewer's decoder.
    let (events, mut decoded) = futures::channel::mpsc::unbounded();
    let (decoder, input) = crate::platform::h264::start((320, 320), events);
    for frame in &frames {
        assert!(input.decode(frame.data.clone()), "the decoder stopped");
    }
    let mut pictures = 0;
    while pictures < frames.len() - 5 {
        match futures::executor::block_on(decoded.next()) {
            Some(crate::platform::h264::DecoderEvent::Frame(picture)) => {
                assert_eq!((picture.width, picture.height), (320, 180));
                assert_eq!(picture.bgra.len(), 320 * 180 * 4);
                decoder.frame_consumed();
                pictures += 1;
            }
            Some(crate::platform::h264::DecoderEvent::Failed(err)) => {
                panic!("decoding failed: {err}")
            }
            None => break,
        }
    }
    assert!(pictures >= frames.len() - 5, "only {pictures} decoded");
}
