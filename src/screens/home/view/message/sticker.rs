//! Stickers in a message, and the drawing the sticker picker shares.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;

use crate::discord::StickerItem;
use crate::screens::home::HomeScreen;
use crate::screens::home::data::gif::{ClipKind, gif_source};
use crate::ui::tooltip;

/// As Discord draws them in a message.
const SIZE: f32 = 160.;

/// Asked of the proxy at every size: the picker's smaller cells then share the
/// chat's still, and Discord's client asks for the same.
const REQUEST_SIZE: u32 = 160;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::screens::home::view) enum Motion {
    Still,
    Animated,
    /// Animated, keeping the still already on screen until the animation loads.
    FromStill,
}

impl HomeScreen {
    /// `key` tells one message's stickers from another's, a forward's included.
    pub(super) fn render_stickers(
        &self,
        key: u64,
        stickers: &[StickerItem],
        cx: &App,
    ) -> impl IntoElement {
        h_flex()
            .gap_2()
            .children(stickers.iter().enumerate().map(|(index, sticker)| {
                div()
                    .id(SharedString::from(format!("sticker-{key}-{index}")))
                    .size(px(SIZE))
                    .flex_shrink_0()
                    .child(self.sticker_image(
                        sticker,
                        SIZE,
                        Motion::Animated,
                        SharedString::from(format!("sticker-image-{key}-{index}")),
                        cx,
                    ))
                    .tooltip(tooltip::text(sticker.name.clone()))
            }))
    }

    /// `id` keeps an animation's place between frames; without one it stays on
    /// its first. A sticker that won't load leaves its name in its place.
    pub(in crate::screens::home::view) fn sticker_image(
        &self,
        sticker: &StickerItem,
        size: f32,
        motion: Motion,
        id: impl Into<ElementId>,
        cx: &App,
    ) -> AnyElement {
        let animated = motion != Motion::Still && sticker.animates();
        let source = self.sticker_source(sticker, size, animated);
        let name = sticker.name.clone();
        let muted = cx.theme().muted_foreground;
        let image = img(source)
            .image_cache(&self.image_cache)
            .size(px(size))
            .object_fit(ObjectFit::Contain);
        let image = if animated && motion == Motion::FromStill {
            let still = self.sticker_source(sticker, size, false);
            let cache = self.image_cache.clone();
            image.with_loading(move || {
                img(still.clone())
                    .image_cache(&cache)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            })
        } else {
            image
        };
        image
            .with_fallback(move || {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child(name.clone())
                    .into_any_element()
            })
            .id(id)
            .into_any_element()
    }

    fn sticker_source(&self, sticker: &StickerItem, size: f32, animated: bool) -> ImageSource {
        match sticker.image_url(animated, REQUEST_SIZE) {
            Some(url) => url.into(),
            None => gif_source(
                sticker.lottie_url(),
                (size, size),
                ClipKind::Lottie { animated },
                self.gifs.clone(),
            ),
        }
    }
}
