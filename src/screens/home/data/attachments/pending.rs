//! What a file staged for sending holds.

use std::sync::Arc;

use gpui_kit::*;

use crate::discord;

pub(crate) struct PendingAttachment {
    /// Unique within the composer; keys the card element and targets removal.
    pub id: u64,
    /// The upload filename; derived (`image-1.png`) for a pasted image.
    pub filename: String,
    pub data: AttachmentData,
    pub spoiler: bool,
}

/// An image keeps the decoded [`Image`] so one set of bytes backs both the
/// thumbnail and the upload; anything else is raw bytes shown as a file card.
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

impl PendingAttachment {
    /// Discord has no spoiler flag on an attachment, only a filename prefix.
    pub fn upload_filename(&self) -> String {
        if self.spoiler && !self.filename.starts_with(discord::SPOILER_PREFIX) {
            format!("{}{}", discord::SPOILER_PREFIX, self.filename)
        } else {
            self.filename.clone()
        }
    }

    /// `img` decodes into the global asset cache, which never evicts, so a staged
    /// image would otherwise stay resident after it is sent or removed.
    pub fn release_preview(&self, cx: &mut App) {
        if let Some(image) = self.data.image() {
            ImageSource::Image(image).remove_asset(cx);
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
