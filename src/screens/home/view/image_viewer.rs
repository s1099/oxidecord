//! The full-screen image viewer: a clicked image drawn as large as the window
//! allows, over a scrim, as Discord's lightbox does. The wheel zooms it and
//! dragging pans it.

use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::ui::button::Button;
use crate::ui::depth::media_shadow;

impl HomeScreen {
    pub(super) fn render_image_viewer(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let viewer = self.image_viewer.as_ref()?;
        let fitted = viewer.fitted_size(window);
        let viewport = window.viewport_size();
        let (width, height) = (fitted.width * viewer.zoom, fitted.height * viewer.zoom);

        // Sharpest first. The best copy is always drawn, which is what loads
        // it; until it has, the best one that has loaded sits underneath —
        // only until then, or a transparent image would show both. The preview
        // is already decoded for the message, so the viewer never opens empty.
        let mut copies: Vec<SharedString> = viewer.zoomed_url.iter().cloned().collect();
        copies.extend([viewer.full_url.clone(), viewer.preview_url.clone()]);
        copies.dedup();
        let mut loaded = |url: &SharedString| {
            let resource = Resource::Uri(SharedUri::from(url.clone()));
            matches!(
                self.image_cache
                    .update(cx, |cache, cx| cache.load(&resource, window, cx)),
                Some(Ok(_))
            )
        };
        let underneath = if loaded(&copies[0]) {
            None
        } else {
            copies[1..]
                .iter()
                .find(|url| loaded(url))
                .or(copies.last())
                .filter(|url| **url != copies[0])
                .cloned()
        };
        let layer = |url: SharedString| {
            img(url.clone())
                .image_cache(&self.image_cache)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Contain)
                // Somewhere for gpui to keep an animated image's frame index;
                // keyed by copy, since two can be on screen at once.
                .id(ElementId::Name(url))
        };

        // Only an image bigger than the window has anywhere to pan to.
        let pannable = width > viewport.width || height > viewport.height;
        let dragging = viewer.drag.is_some();
        let image = div()
            .absolute()
            .left((viewport.width - width) / 2. + viewer.offset.x)
            .top((viewport.height - height) / 2. + viewer.offset.y)
            .w(width)
            .h(height)
            .rounded(px(8.))
            .shadow(media_shadow())
            .overflow_hidden()
            .when(pannable, |this| {
                this.cursor(if dragging {
                    CursorStyle::ClosedHand
                } else {
                    CursorStyle::OpenHand
                })
            })
            .children(underneath.map(layer))
            .child(layer(copies[0].clone()))
            // A press on the picture pans it, rather than reaching the scrim,
            // which would close the viewer under it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.start_image_pan(event.position, cx);
                }),
            );

        let source = viewer.source_url.clone();
        let open = h_flex()
            .absolute()
            .bottom(px(16.))
            .left_0()
            .right_0()
            .justify_center()
            .child(
                div()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("image-viewer-open")
                            .label("Open in Browser")
                            .link()
                            .small()
                            .text_color(white().opacity(0.8))
                            .on_click(move |_, _, cx| cx.open_url(&source)),
                    ),
            );

        let close = div()
            .absolute()
            .top(px(16.))
            .right(px(16.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("image-viewer-close")
                    .icon(Icon::default().path("icons/close.svg"))
                    .ghost()
                    .text_color(white())
                    .tooltip("Close (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_image_viewer(window, cx);
                    })),
            );

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .overflow_hidden()
                    // A scrim rather than a theme colour: it has to dim the
                    // screen in either theme, and the image reads against it
                    // the way it would in a photo viewer.
                    .bg(black().opacity(0.85))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_image_viewer(window, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(Self::pan_image_viewer))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_image_pan(cx)),
                    )
                    .on_scroll_wheel(cx.listener(Self::zoom_image_viewer))
                    .child(image)
                    .child(open)
                    .child(close),
            )
            // Above the popouts, which an image inside the pins can open it from.
            .with_priority(2)
            .into_any_element(),
        )
    }
}
