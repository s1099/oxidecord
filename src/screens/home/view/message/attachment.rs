//! Inline previews for a message's image attachments.

use gpui::*;

use crate::discord;

/// Largest inline preview an image is scaled down to, in pixels. Discord uses
/// similar bounds; the aspect ratio is preserved within them.
const MAX_IMAGE_WIDTH: f32 = 400.;
const MAX_IMAGE_HEIGHT: f32 = 300.;

pub(super) fn render_image(
    image: &discord::ImageAttachment,
    cache: &Entity<RetainAllImageCache>,
) -> impl IntoElement {
    // The cache has to be named on the element itself. An ancestor
    // `image_cache(..)` only pushes onto the cache stack during layout and
    // paint, and `list` renders its items during *prepaint* — so images inside
    // the message list would otherwise miss the stack entirely and fall back to
    // gpui's global asset cache, which never evicts.
    let mut element = img(image.url.clone())
        .image_cache(cache)
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
