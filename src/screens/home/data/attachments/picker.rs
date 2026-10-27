//! Reading files chosen in the platform file picker.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;

use crate::discord;

use super::pending::{AttachmentData, format_size};

pub(super) struct PickedFile {
    pub filename: String,
    pub data: AttachmentData,
}

pub(super) struct UploadBudget {
    pub per_file: u64,
    pub remaining: u64,
}

impl UploadBudget {
    pub fn take(&mut self, subject: &str, size: u64) -> Result<(), String> {
        if size > self.per_file {
            return Err(format!(
                "{subject} is {}, over the {} upload limit.",
                format_size(size),
                format_size(self.per_file)
            ));
        }
        if size > self.remaining {
            return Err(format!(
                "{subject} would take this message over the {} its attachments can add up to.",
                format_size(discord::MAX_UPLOAD_TOTAL)
            ));
        }
        self.remaining -= size;
        Ok(())
    }
}

/// The size is checked before reading so an oversized file is never loaded
/// into memory just to be turned down.
pub(super) fn read_picked_file(
    path: PathBuf,
    budget: &mut UploadBudget,
) -> Result<PickedFile, String> {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "The selected file".to_string());

    let size = std::fs::metadata(&path)
        .map_err(|err| format!("{filename} couldn't be read: {err}"))?
        .len();
    budget.take(&filename, size)?;

    let bytes = std::fs::read(&path).map_err(|err| {
        budget.remaining += size;
        format!("{filename} couldn't be read: {err}")
    })?;
    let data = match image_format_from_path(&path) {
        Some(format) => AttachmentData::Image(Arc::new(Image::from_bytes(format, bytes))),
        None => AttachmentData::File(Arc::new(bytes)),
    };
    Ok(PickedFile { filename, data })
}

/// `None` stages the file plain, without a thumbnail.
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
