//! Files staged for sending: what one holds, and how it gets there from the
//! clipboard or the file picker.

mod pending;
mod picker;

use std::sync::Arc;

use gpui::*;

use crate::discord;
use crate::screens::home::{HomeScreen, PasteAttachment};
use crate::ui::dialogs::show_error;

pub(crate) use pending::{AttachmentData, PendingAttachment, format_size};

use picker::{image_extension, oversize_message, read_picked_file};

impl HomeScreen {
    /// Handles the paste shortcut in the composer. If the clipboard holds an
    /// image, it's staged as an attachment and the event is consumed. Otherwise
    /// (plain text, or the composer isn't focused) propagation continues so the
    /// text input's own paste handling runs as usual.
    pub(in crate::screens::home) fn on_paste_attachment(
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
    pub(in crate::screens::home) fn pick_attachments(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::screens::home) fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.pending_attachments
            .retain(|attachment| attachment.id != id);
        cx.notify();
    }
}
