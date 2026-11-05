//! Clipboard images: pasted ones in the form Discord embeds, and copied ones in
//! the forms other apps read.

use gpui_kit::{App, Image, ImageFormat};

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

/// A downloaded image, decoded and re-encoded for the clipboard.
pub struct CopiedImage {
    png: Vec<u8>,
    /// gpui writes only `PNG`, which Paint and Office ignore in favour of a bitmap.
    #[cfg(windows)]
    dib: Vec<u8>,
}

/// Slow for a large image, so keep it off the foreground thread.
pub fn prepare_copy(bytes: &[u8]) -> Option<CopiedImage> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(CopiedImage {
        png,
        #[cfg(windows)]
        dib: to_dib(&decoded.to_rgba8()),
    })
}

/// A bottom-up 32-bit `BITMAPINFOHEADER` bitmap, as `CF_DIB` expects.
#[cfg(windows)]
fn to_dib(rgba: &image::RgbaImage) -> Vec<u8> {
    let (width, height) = rgba.dimensions();
    let pixels = width as usize * height as usize * 4;
    let mut dib = Vec::with_capacity(40 + pixels);
    dib.extend(40u32.to_le_bytes());
    dib.extend((width as i32).to_le_bytes());
    dib.extend((height as i32).to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    // BI_RGB, then the image size and the four fields left at zero.
    dib.extend(0u32.to_le_bytes());
    dib.extend((pixels as u32).to_le_bytes());
    dib.extend([0; 16]);
    for row in rgba.rows().rev() {
        for pixel in row {
            dib.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    dib
}

#[cfg(windows)]
pub fn write_image(image: &CopiedImage, _: &mut App) {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_DIB;

    unsafe fn set(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
            let ptr = GlobalLock(global) as *mut u8;
            if ptr.is_null() {
                let err = windows::core::Error::from_thread();
                let _ = GlobalFree(Some(global));
                return Err(err);
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(global);
            // The clipboard owns the memory only once this succeeds.
            if let Err(err) = SetClipboardData(format, Some(HANDLE(global.0))) {
                let _ = GlobalFree(Some(global));
                return Err(err);
            }
            Ok(())
        }
    }

    unsafe {
        if let Err(err) = OpenClipboard(None) {
            eprintln!("failed to open the clipboard: {err}");
            return;
        }
        let png = RegisterClipboardFormatW(windows::core::w!("PNG"));
        let result = EmptyClipboard()
            .and_then(|()| set(png, &image.png))
            .and_then(|()| set(CF_DIB.0 as u32, &image.dib));
        let _ = CloseClipboard();
        if let Err(err) = result {
            eprintln!("failed to copy the image: {err}");
        }
    }
}

#[cfg(not(windows))]
pub fn write_image(image: &CopiedImage, cx: &mut App) {
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&Image::from_bytes(
        ImageFormat::Png,
        image.png.clone(),
    )));
}
