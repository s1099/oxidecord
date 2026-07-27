//! Files staged for sending: what one holds, and how it gets there from the
//! clipboard or the file picker.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;

use crate::discord;
use crate::screens::dialogs::show_error;
use crate::screens::home::{HomeScreen, PasteAttachment};

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

fn image_extension(format: ImageFormat) -> &'static str {
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

/// The error-dialog line for a file that's over Discord's upload limit.
fn oversize_message(subject: &str, size: u64) -> String {
    format!(
        "{subject} is {}, over the {} upload limit.",
        format_size(size),
        format_size(discord::MAX_ATTACHMENT_SIZE)
    )
}

struct PickedFile {
    filename: String,
    data: AttachmentData,
}

/// Reads one picked file into an attachment, or returns the error-dialog line
/// explaining why it can't be attached. The size is checked before the contents
/// so an oversized file is never read into memory just to be turned down.
fn read_picked_file(path: PathBuf) -> Result<PickedFile, String> {
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

impl HomeScreen {
    /// Handles the paste shortcut in the composer. If the clipboard holds an
    /// image, it's staged as an attachment and the event is consumed. Otherwise
    /// (plain text, or the composer isn't focused) propagation continues so the
    /// text input's own paste handling runs as usual.
    pub(crate) fn on_paste_attachment(
        &mut self,
        _: &PasteAttachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only intercept paste aimed at the message composer; let every other
        // input (search, login, …) paste text normally.
        if !self.message_input.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
        // No image on the clipboard: fall through to the input's text paste.
        if !self.paste_attachment_from_clipboard(window, cx) {
            cx.propagate();
        }
    }

    /// Stages any image on the clipboard as an attachment, shown as a thumbnail
    /// above the composer. Returns whether the clipboard held an image, whether
    /// or not it could be staged — an image too large to upload is reported in
    /// an error dialog rather than handed on to the input's text paste.
    fn paste_attachment_from_clipboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };

        let mut found = false;
        let mut rejected = Vec::new();
        for entry in item.into_entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            if image.bytes.is_empty() {
                continue;
            }
            found = true;

            let size = image.bytes.len() as u64;
            if size > discord::MAX_ATTACHMENT_SIZE {
                rejected.push(oversize_message("The pasted image", size));
                continue;
            }

            let id = self.reserve_attachment_id();
            let filename = format!("image-{id}.{}", image_extension(image.format));
            self.pending_attachments.push(PendingAttachment {
                id,
                filename,
                data: AttachmentData::Image(Arc::new(image)),
            });
        }

        if !rejected.is_empty() {
            show_error("Can't attach image", rejected, window, cx);
        }
        if found {
            cx.notify();
        }
        found
    }

    /// Opens the platform file picker and stages every chosen file as an
    /// attachment. Files that can't be attached — over the upload limit, or
    /// unreadable — are listed in an error dialog; the rest are still staged.
    pub(crate) fn pick_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });

        cx.spawn_in(window, async move |this, cx| {
            // Dismissed without choosing anything, or the picker wouldn't open.
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };

            // Reading the files is blocking I/O, so it stays off the foreground.
            let files = cx
                .background_spawn(async move {
                    paths.into_iter().map(read_picked_file).collect::<Vec<_>>()
                })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                let mut rejected = Vec::new();
                for file in files {
                    match file {
                        Ok(file) => {
                            let id = this.reserve_attachment_id();
                            this.pending_attachments.push(PendingAttachment {
                                id,
                                filename: file.filename,
                                data: file.data,
                            });
                        }
                        Err(reason) => rejected.push(reason),
                    }
                }
                if !rejected.is_empty() {
                    show_error("Can't attach file", rejected, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reserve_attachment_id(&mut self) -> u64 {
        self.next_attachment_id += 1;
        self.next_attachment_id
    }

    pub(crate) fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.pending_attachments
            .retain(|attachment| attachment.id != id);
        cx.notify();
    }
}
