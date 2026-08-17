//! What a file staged for sending holds.

use std::sync::Arc;

use gpui::*;

/// A file staged for sending, picked from disk or pasted from the clipboard
pub(crate) struct PendingAttachment {
    /// Unique within the composer; keys the card element and targets removal.
    pub id: u64,
    /// The upload filename: the file's own name, or one derived from the image
    /// format for a pasted image (e.g. `image-1.png`).
    pub filename: String,
    /// The file's contents, rendered as a preview and uploaded on send.
    pub data: AttachmentData,
}

/// The contents of a staged attachment. An image keeps the decoded [`Image`], so
/// the same bytes back both its thumbnail and the upload; any other file is held
/// as raw bytes and previewed as a file card instead.
pub(crate) enum AttachmentData {
    Image(Arc<Image>),
    File(Arc<Vec<u8>>),
}

impl AttachmentData {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Image(image) => &image.bytes,
            Self::File(bytes) => bytes,
        }
    }

    pub fn image(&self) -> Option<Arc<Image>> {
        match self {
            Self::Image(image) => Some(image.clone()),
            Self::File(_) => None,
        }
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    const MB: f64 = 1024. * KB;

    let bytes = bytes as f64;
    if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}
