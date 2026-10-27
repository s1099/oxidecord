//! Scales captured BGRA frames into the encoder's NV12 on the GPU's video processor.

use std::mem::ManuallyDrop;

use windows::Graphics::Capture::Direct3D11CaptureFrame;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::core::Interface as _;

use super::{Context, texture};

/// A hardware encoder may still be reading a texture when the next frame is
/// converted; four covers the deepest pipeline common encoders keep.
const RING: usize = 4;

/// Studio-range black, for bars around a source that doesn't fill the frame.
const BLACK: D3D11_VIDEO_COLOR_YCbCrA = D3D11_VIDEO_COLOR_YCbCrA {
    Y: 16. / 255.,
    Cb: 0.5,
    Cr: 0.5,
    A: 1.,
};

pub(super) struct Converter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    out_size: (u32, u32),
    fps: u32,
    ring: Vec<ID3D11Texture2D>,
    next: usize,
    /// Rebuilt on source resize; the processor is created for one input size.
    pipeline: Option<Pipeline>,
}

struct Pipeline {
    size: (u32, u32),
    /// Our own copy: the capture's textures return to its pool at once and lack
    /// the bindings the video processor wants as input.
    input: ID3D11Texture2D,
    input_view: ID3D11VideoProcessorInputView,
    outputs: Vec<ID3D11VideoProcessorOutputView>,
    processor: ID3D11VideoProcessor,
}

impl Converter {
    pub(super) fn new(
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

    pub(super) fn load(
        &mut self,
        frame: &Direct3D11CaptureFrame,
        content: SizeInt32,
    ) -> Result<(), String> {
        let surface = frame.Surface().context("Couldn't read a captured frame")?;
        let access: IDirect3DDxgiInterfaceAccess =
            surface.cast().context("Couldn't read a captured frame")?;
        let captured: ID3D11Texture2D =
            unsafe { access.GetInterface() }.context("Couldn't read a captured frame")?;

        // The buffer can exceed the content while a resize settles.
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
        // Studio-range BT.709 out is what decoders assume of an unlabelled stream.
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
            // Denoising and edge enhancement smear text.
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

    pub(super) fn convert(&mut self) -> Result<ID3D11Texture2D, String> {
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

/// Even pixels so the bars don't split a chroma sample.
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
