//! Inline previews for a message's image attachments.

use gpui::*;

use crate::discord;

/// Largest inline preview an image is scaled down to, in pixels. Discord uses
/// similar bounds; the aspect ratio is preserved within them.
const MAX_IMAGE_WIDTH: f32 = 400.;
const MAX_IMAGE_HEIGHT: f32 = 300.;

pub(super) fn render_image(image: &discord::ImageAttachment) -> impl IntoElement {
    let mut element = img(image.url.clone())
        .rounded(px(8.))
        .max_w(px(MAX_IMAGE_WIDTH));
    match (image.width, image.height) {
        // With intrinsic dimensions we can lay out the exact scaled box, so
        // the message doesn't reflow once the image finishes loading.
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_IMAGE_WIDTH / width)
                .min(MAX_IMAGE_HEIGHT / height)
                .min(1.);
            element = element.w(px(width * scale)).h(px(height * scale));
        }
        _ => element = element.max_h(px(MAX_IMAGE_HEIGHT)),
    }
    element
}
