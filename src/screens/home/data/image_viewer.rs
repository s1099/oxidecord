//! Opening and closing the full-screen image viewer, and zooming and panning
//! the image inside it.

use gpui_kit::*;

use crate::discord;
use crate::screens::home::{HomeScreen, ImageViewer};

const VIEWER_MARGIN_X: f32 = 80.;
/// Taller, to leave room for the link under the image.
const VIEWER_MARGIN_Y: f32 = 64.;

const MAX_ZOOM: f32 = 8.;
/// A trackpad's pixel deltas zoom at the same rate, a notch counting as
/// [`WHEEL_LINE_HEIGHT`] pixels.
const ZOOM_PER_NOTCH: f32 = 1.06;
const WHEEL_LINE_HEIGHT: f32 = 20.;
/// The original can be any size and costs four bytes a pixel decoded.
const ZOOMED_MAX_SIDE: u32 = 4096;

fn viewer_bounds(window: &Window) -> (f32, f32) {
    let viewport = window.viewport_size();
    (
        (f32::from(viewport.width) - 2. * VIEWER_MARGIN_X).max(1.),
        (f32::from(viewport.height) - 2. * VIEWER_MARGIN_Y).max(1.),
    )
}

impl ImageViewer {
    /// Scaled down into the viewer's bounds, never up past its own size. With no
    /// reported dimensions it gets the whole box.
    pub(in crate::screens::home) fn fitted_size(&self, window: &Window) -> Size<Pixels> {
        let (max_width, max_height) = viewer_bounds(window);
        match (self.width, self.height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                let (width, height) = (width as f32, height as f32);
                let scale = (max_width / width).min(max_height / height).min(1.);
                size(px(width * scale), px(height * scale))
            }
            _ => size(px(max_width), px(max_height)),
        }
    }

    /// Pans only along an axis the image overflows, and only until its edge meets
    /// the window's.
    fn clamp_offset(&mut self, window: &Window) {
        let fitted = self.fitted_size(window);
        let viewport = window.viewport_size();
        let limit = |image: Pixels, window: Pixels| ((image * self.zoom - window) / 2.).max(px(0.));
        let (x, y) = (
            limit(fitted.width, viewport.width),
            limit(fitted.height, viewport.height),
        );
        self.offset = point(self.offset.x.clamp(-x, x), self.offset.y.clamp(-y, y));
    }
}

#[derive(Clone)]
pub(in crate::screens::home) struct ViewerImage {
    pub preview_url: String,
    pub source_url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub animated_webp: bool,
}

impl From<&discord::ImageAttachment> for ViewerImage {
    fn from(image: &discord::ImageAttachment) -> Self {
        Self {
            preview_url: image.url.clone(),
            source_url: image.source.clone(),
            width: image.width,
            height: image.height,
            animated_webp: image.animated_webp,
        }
    }
}

impl From<&discord::EmbedMedia> for ViewerImage {
    fn from(image: &discord::EmbedMedia) -> Self {
        Self {
            preview_url: image.url.clone(),
            source_url: image.source.clone(),
            width: image.width,
            height: image.height,
            animated_webp: image.animated_webp,
        }
    }
}

impl HomeScreen {
    pub(in crate::screens::home) fn open_image_viewer(
        &mut self,
        image: ViewerImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Physical pixels for sharpness above 1x, but sized to the window: the original
        // can run to tens of megabytes decoded.
        let (width, height) = viewer_bounds(window);
        let scale = window.scale_factor();
        let full_url = discord::scaled_url(
            &image.source_url,
            image.width.map(u64::from),
            image.height.map(u64::from),
            (width * scale).round() as u32,
            (height * scale).round() as u32,
        );
        let full_url = if image.animated_webp {
            discord::animated_webp(full_url)
        } else {
            full_url
        };
        if let Some(previous) = self.image_viewer.take() {
            self.release_viewer_images(&previous, window, cx);
        }
        self.image_viewer = Some(ImageViewer {
            preview_url: image.preview_url.into(),
            full_url: full_url.into(),
            source_url: image.source_url,
            zoomed_url: None,
            width: image.width,
            height: image.height,
            animated_webp: image.animated_webp,
            zoom: 1.,
            offset: point(px(0.), px(0.)),
            drag: None,
        });
        cx.notify();
    }

    pub(in crate::screens::home) fn close_image_viewer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(viewer) = self.image_viewer.take() {
            self.release_viewer_images(&viewer, window, cx);
            cx.notify();
        }
    }

    /// Keeps the point under the pointer fixed, like a map.
    pub(in crate::screens::home) fn zoom_image_viewer(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = &mut self.image_viewer else {
            return;
        };
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_HEIGHT)).y;
        let notches = f32::from(delta) / WHEEL_LINE_HEIGHT;
        let zoom = (viewer.zoom * ZOOM_PER_NOTCH.powf(notches)).clamp(1., MAX_ZOOM);
        if zoom == viewer.zoom {
            return;
        }

        // The pointer's distance from the image's centre grows with the zoom;
        // moving the centre by the difference leaves that point under it.
        let viewport = window.viewport_size();
        let centre = point(viewport.width / 2., viewport.height / 2.) + viewer.offset;
        let ratio = zoom / viewer.zoom;
        viewer.offset = event.position - centre + viewer.offset - (event.position - centre) * ratio;
        viewer.zoom = zoom;
        viewer.clamp_offset(window);

        if viewer.zoomed_url.is_none() && zoom > 1. {
            viewer.zoomed_url = zoomed_url(viewer, window).map(SharedString::from);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn start_image_pan(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(viewer) = &mut self.image_viewer {
            viewer.drag = Some((position, viewer.offset));
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn pan_image_viewer(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = &mut self.image_viewer else {
            return;
        };
        let Some((origin, from)) = viewer.drag else {
            return;
        };
        // A release outside the window never reaches the viewer, so a move
        // with the button up is what ends that pan.
        if event.pressed_button != Some(MouseButton::Left) {
            viewer.drag = None;
        } else {
            viewer.offset = from + (event.position - origin);
            viewer.clamp_offset(window);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn end_image_pan(&mut self, cx: &mut Context<Self>) {
        if let Some(viewer) = &mut self.image_viewer
            && viewer.drag.take().is_some()
        {
            cx.notify();
        }
    }

    /// Drops the viewer's copies from the image cache, which would keep them until
    /// the conversation is left. The preview stays: the message still draws it.
    fn release_viewer_images(&self, viewer: &ImageViewer, window: &mut Window, cx: &mut App) {
        let urls = std::iter::once(&viewer.full_url).chain(&viewer.zoomed_url);
        for url in urls.filter(|url| **url != viewer.preview_url) {
            let resource = Resource::Uri(SharedUri::from(url.clone()));
            self.image_cache
                .update(cx, |cache, cx| cache.remove(&resource, window, cx));
        }
    }
}

/// Once zoomed in, if the window-sized copy falls short: the original, capped at
/// a side that bounds the decoded bitmap.
fn zoomed_url(viewer: &ImageViewer, window: &Window) -> Option<String> {
    let fitted = viewer.fitted_size(window);
    let scale = window.scale_factor();
    if let (Some(width), Some(height)) = (viewer.width, viewer.height)
        && (width as f32) <= f32::from(fitted.width) * scale
        && (height as f32) <= f32::from(fitted.height) * scale
    {
        return None;
    }
    let url = discord::scaled_url(
        &viewer.source_url,
        viewer.width.map(u64::from),
        viewer.height.map(u64::from),
        ZOOMED_MAX_SIDE,
        ZOOMED_MAX_SIDE,
    );
    Some(if viewer.animated_webp {
        discord::animated_webp(url)
    } else {
        url
    })
}
