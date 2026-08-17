//! Reading files chosen in the platform file picker.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;

use crate::discord;

use super::pending::{AttachmentData, format_size};

pub(super) struct PickedFile {
    pub filename: String,
    pub data: AttachmentData,
}

/// Reads one picked file into an attachment, or returns the error-dialog line
/// explaining why it can't be attached. The size is checked before the contents
/// so an oversized file is never read into memory just to be turned down.
pub(super) fn read_picked_file(path: PathBuf) -> Result<PickedFile, String> {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "The selected file".to_string());

    let size = std::fs::metadata(&path)
        .map_err(|err| format!("{filename} couldn't be read: {err}"))?
        .len();
    if size > discord::MAX_ATTACHMENT_SIZE {
        return Err(oversize_message(&filename, size));
    }

    let bytes =
        std::fs::read(&path).map_err(|err| format!("{filename} couldn't be read: {err}"))?;
    let data = match image_format_from_path(&path) {
        Some(format) => AttachmentData::Image(Arc::new(Image::from_bytes(format, bytes))),
        None => AttachmentData::File(Arc::new(bytes)),
    };
    Ok(PickedFile { filename, data })
}

/// The error-dialog line for a file that's over Discord's upload limit.
pub(super) fn oversize_message(subject: &str, size: u64) -> String {
    format!(
        "{subject} is {}, over the {} upload limit.",
        format_size(size),
        format_size(discord::MAX_ATTACHMENT_SIZE)
    )
}

pub(super) fn image_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

/// The image format for a path's extension. Anything else is staged as a plain
/// file rather than a thumbnail.
fn image_format_from_path(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        "svg" => Some(ImageFormat::Svg),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        _ => None,
    }
}
