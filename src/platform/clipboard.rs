//! Clipboard images in the form Discord embeds.

use gpui_kit::{Image, ImageFormat};

/// The clipboard's registered `PNG` format, when an app put one there.
///
/// gpui keeps only the first image format the source listed, and ShareX and
/// Flameshot list `CF_DIB` ahead of their PNG, which would upload as a `.bmp`.
#[cfg(windows)]
pub fn read_png() -> Option<Image> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    unsafe {
        let format = RegisterClipboardFormatW(windows::core::w!("PNG"));
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        OpenClipboard(None).ok()?;
        let bytes = GetClipboardData(format).ok().and_then(|handle| {
            let global = HGLOBAL(handle.0);
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, GlobalSize(global)).to_vec();
            let _ = GlobalUnlock(global);
            Some(bytes)
        });
        let _ = CloseClipboard();
        bytes
            .filter(|bytes| bytes.starts_with(b"\x89PNG"))
            .map(|bytes| Image::from_bytes(ImageFormat::Png, bytes))
    }
}

#[cfg(not(windows))]
pub fn read_png() -> Option<Image> {
    None
}

/// Whether Discord shows this format inline; others upload as a file card.
pub fn embeds(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::Webp
    )
}

/// Re-encodes a pasted bitmap as PNG, as Discord's own client does. Slow for a
/// large screenshot, so keep it off the foreground thread.
pub fn to_png(image: &Image) -> Option<Image> {
    let format = match image.format {
        ImageFormat::Bmp => image::ImageFormat::Bmp,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        _ => return None,
    };
    let mut decoded = image::load_from_memory_with_format(&image.bytes, format).ok()?;
    // A 32-bit DIB often leaves its fourth byte zero, which would read as fully transparent.
    if let Some(rgba) = decoded.as_rgba8()
        && rgba.pixels().all(|pixel| pixel[3] == 0)
    {
        decoded = image::DynamicImage::ImageRgb8(decoded.to_rgb8());
    }
    let mut bytes = Vec::new();
    decoded
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(Image::from_bytes(ImageFormat::Png, bytes))
}
