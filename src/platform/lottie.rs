//! Lottie animations rasterized on the CPU into a [`LoopingClip`], so they loop
//! like a GIF. Most of Discord's own sticker packs are Lottie, which no OS
//! decoder draws.

use std::time::Duration;

use rasterlottie::{Animation, RenderConfig, Renderer, Rgba8};

use super::video::{LoopingClip, VideoFrame};
use super::{http, runtime};

/// Discord's stickers run at 60fps; at sticker size half that looks the same
/// and holds half the frames.
const MAX_FPS: f32 = 30.;

/// Matches the budget a looping video clip gets.
const BUDGET: usize = 24 * 1024 * 1024;

/// `size` is the longest side in physical pixels. A still is the first frame
/// alone. Rasterizes on whichever thread awaits this, so call it from a
/// background executor.
pub async fn render(url: String, size: u32, animated: bool) -> Result<LoopingClip, String> {
    let json = runtime::run(async move {
        http::client()
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| format!("Couldn't download the sticker: {err}"))?;
    rasterize(&json, size, animated)
}

fn rasterize(json: &str, size: u32, animated: bool) -> Result<LoopingClip, String> {
    let animation = Animation::from_json_str(json).map_err(|err| err.to_string())?;
    let prepared = Renderer::default()
        .prepare(&animation)
        .map_err(|err| err.to_string())?;
    let longest = animation.width.max(animation.height).max(1) as f32;
    let config = RenderConfig::new(Rgba8::TRANSPARENT, size as f32 / longest);
    let fps = animation.frame_rate.max(1.);
    let total = animation.duration_frames();

    let frame_bytes = (size as usize).pow(2) * 4;
    let budget_frames = (BUDGET / frame_bytes.max(1)).max(1) as f32;
    let step = (fps / MAX_FPS).max(total / budget_frames).ceil().max(1.);

    let mut frames = Vec::new();
    let mut at = 0.;
    while at < total || frames.is_empty() {
        let raster = prepared
            .render_frame(animation.in_point + at, config)
            .map_err(|err| err.to_string())?;
        frames.push(VideoFrame {
            width: raster.width,
            height: raster.height,
            bgra: to_bgra(raster.pixels),
            position: Duration::from_secs_f32(at / fps),
        });
        if !animated {
            break;
        }
        at += step;
    }
    Ok(LoopingClip {
        frames,
        duration: Duration::from_secs_f32(total / fps),
    })
}

/// tiny-skia draws premultiplied RGBA; gpui takes straight BGRA.
fn to_bgra(mut pixels: Vec<u8>) -> Vec<u8> {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r#"{"v":"5.7.6","fr":60,"ip":0,"op":120,"w":64,"h":64,"layers":[]}"#;

    #[test]
    fn thins_to_the_frame_rate_cap() {
        let clip = rasterize(SQUARE, 32, true).unwrap();
        assert_eq!(clip.frames.len(), 60);
        assert_eq!(clip.duration, Duration::from_secs(2));
        assert_eq!((clip.frames[0].width, clip.frames[0].height), (32, 32));
        assert_eq!(clip.frames[1].position, Duration::from_secs_f32(2. / 60.));
    }

    #[test]
    fn a_still_is_one_frame() {
        assert_eq!(rasterize(SQUARE, 32, false).unwrap().frames.len(), 1);
    }

    #[test]
    fn unpremultiplies_and_swaps() {
        assert_eq!(to_bgra(vec![10, 20, 64, 128]), [128, 40, 20, 128]);
        assert_eq!(to_bgra(vec![1, 2, 3, 0]), [3, 2, 1, 0]);
    }
}
