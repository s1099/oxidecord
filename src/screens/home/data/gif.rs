//! GIFs that Discord serves as video: loading one into an animation gpui
//! plays by itself, and releasing it again.
//!
//! A `gifv` embed's clip is decoded whole, every frame into one multi-frame
//! [`RenderImage`] — the same shape gpui decodes a real GIF into — so `img()`
//! loops it with no player, no controls and no decoder left running, and the
//! asset cache shares one decode between every place the GIF is drawn.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use image::{Delay, Frame, ImageBuffer};

use crate::platform::video::{self, LoopingClip};
use crate::screens::home::HomeScreen;

/// The shortest a frame is held. A clip that reports frames closer together
/// than the screen refreshes would only have them skipped anyway.
const MIN_FRAME_DELAY: Duration = Duration::from_millis(10);

/// One clip at one size: the same GIF drawn on a display at another scale is
/// another decode.
#[derive(Clone, Hash, PartialEq, Eq)]
pub(in crate::screens::home) struct GifSource {
    url: String,
    /// Physical pixels the frames are scaled to fit.
    target: (u32, u32),
}

/// The GIF clips loaded since the conversation was opened, so they can be
/// released when it's left. Shared with the loaders handed to `img()`, which
/// run during layout, where the screen itself can't be reached.
pub(in crate::screens::home) type LoadedGifs = Rc<RefCell<HashSet<GifSource>>>;

enum GifLoader {}

impl Asset for GifLoader {
    type Source = GifSource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    // Not an `async fn`: that would borrow `cx` into the future, which has to
    // be `'static`.
    #[allow(clippy::manual_async_fn)]
    fn load(
        source: Self::Source,
        _cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let clip = video::decode_loop(source.url, source.target)
                .await
                .map_err(|reason| ImageCacheError::Asset(reason.into()))?;
            to_animation(clip)
                .ok_or_else(|| ImageCacheError::Asset("The clip's frames were malformed.".into()))
        }
    }
}

/// An image source that loops the clip at `url`, fitted to `size` (logical
/// pixels). Nothing is drawn until it has loaded, so the caller keeps the
/// poster underneath.
pub(in crate::screens::home) fn gif_source(
    url: String,
    size: (f32, f32),
    loaded: LoadedGifs,
) -> ImageSource {
    // gpui's custom loader is an `Arc` it only ever calls on the foreground,
    // so the `Rc` it closes over never crosses a thread.
    #[allow(clippy::arc_with_non_send_sync)]
    ImageSource::Custom(Arc::new(move |window, cx| {
        let scale = window.scale_factor();
        let source = GifSource {
            url: url.clone(),
            target: (
                (size.0 * scale).round().max(1.) as u32,
                (size.1 * scale).round().max(1.) as u32,
            ),
        };
        let image = window.use_asset::<GifLoader>(&source, cx);
        let mut loaded = loaded.borrow_mut();
        if !loaded.contains(&source) {
            loaded.insert(source);
        }
        image
    }))
}

impl HomeScreen {
    /// Lets go of every GIF clip loaded for the conversation being left: its
    /// frames in the sprite atlas, and its entry in the asset cache, which
    /// would otherwise hold the decoded frames for the rest of the session.
    pub(in crate::screens::home) fn release_gifs(&mut self, window: &mut Window, cx: &mut App) {
        for source in self.gifs.borrow_mut().drain() {
            if let Some(Ok(image)) = window.get_asset::<GifLoader>(&source, cx) {
                cx.drop_image(image, Some(window));
            }
            cx.remove_asset::<GifLoader>(&source);
        }
    }
}

/// Lays the decoded frames out as one animation, each held until the next
/// one's timestamp and the last until the clip ends.
fn to_animation(clip: LoopingClip) -> Option<Arc<RenderImage>> {
    let positions: Vec<Duration> = clip.frames.iter().map(|frame| frame.position).collect();
    // What the last frame is held for when the clip's run time doesn't reach
    // past it: the gap the frames before it kept.
    let typical = positions
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .filter(|gap| !gap.is_zero())
        .min()
        .unwrap_or(Duration::from_millis(100));

    let frames = clip
        .frames
        .into_iter()
        .enumerate()
        .map(|(index, frame)| {
            let until = positions.get(index + 1).copied().unwrap_or_else(|| {
                Some(clip.duration)
                    .filter(|end| *end > frame.position)
                    .unwrap_or(frame.position + typical)
            });
            let delay = until.saturating_sub(frame.position).max(MIN_FRAME_DELAY);
            let buffer = ImageBuffer::from_raw(frame.width, frame.height, frame.bgra)?;
            Some(Frame::from_parts(
                buffer,
                0,
                0,
                Delay::from_saturating_duration(delay),
            ))
        })
        .collect::<Option<Vec<_>>>()?;

    Some(Arc::new(RenderImage::new(frames)))
}
