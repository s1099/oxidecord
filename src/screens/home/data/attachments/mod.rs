//! Files staged for sending: what one holds, and how it gets there from the
//! clipboard or the file picker.

mod pending;
mod picker;

use std::sync::Arc;

use gpui_kit::*;

use crate::discord;
use crate::screens::home::{HomeScreen, PasteAttachment, View};
use crate::ui::dialogs::show_error;

pub(crate) use pending::{AttachmentData, PendingAttachment, format_size};

use picker::{UploadBudget, read_picked_file};

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
        if !self.message_input.focus_handle(cx).is_focused(window) {
            cx.propagate();
            return;
        }
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
        let mut budget = self.upload_budget();
        for entry in item.into_entries() {
            let ClipboardEntry::Image(image) = entry else {
                continue;
            };
            if image.bytes.is_empty() {
                continue;
            }
            found = true;

            if let Err(reason) = budget.take("The pasted image", image.bytes.len() as u64) {
                rejected.push(reason);
                continue;
            }

            let id = self.reserve_attachment_id();
            let filename = format!("image-{id}.{}", image.format.extension());
            self.pending_attachments.push(PendingAttachment {
                id,
                filename,
                data: AttachmentData::Image(Arc::new(image)),
                spoiler: false,
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

        // Taken now rather than when the files are back, so they're checked
        // before they're read. The picker is modal, so nothing else gets
        // staged meanwhile.
        let mut budget = self.upload_budget();

        cx.spawn_in(window, async move |this, cx| {
            // Dismissed without choosing anything, or the picker wouldn't open.
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };

            // Reading the files is blocking I/O, so it stays off the foreground.
            let files = cx
                .background_spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| read_picked_file(path, &mut budget))
                        .collect::<Vec<_>>()
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
                                spoiler: false,
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

    /// What may still be staged for the open conversation. A file can be as
    /// large as the more generous of the user's Nitro and the guild's boosts.
    fn upload_budget(&self) -> UploadBudget {
        let user_limit = self
            .current_user
            .as_ref()
            .map_or(discord::DEFAULT_UPLOAD_LIMIT, |user| user.upload_limit);
        let guild_limit = self
            .selected_guild
            .filter(|_| self.view == View::Guild)
            .and_then(|guild_id| self.guild_upload_limits.get(&guild_id))
            .copied()
            .unwrap_or_default();
        let staged: u64 = self
            .pending_attachments
            .iter()
            .map(|attachment| attachment.data.bytes().len() as u64)
            .sum();
        UploadBudget {
            // Nitro allows a file larger than one message request can carry.
            per_file: user_limit.max(guild_limit).min(discord::MAX_UPLOAD_TOTAL),
            remaining: discord::MAX_UPLOAD_TOTAL.saturating_sub(staged),
        }
    }

    fn reserve_attachment_id(&mut self) -> u64 {
        self.next_attachment_id += 1;
        self.next_attachment_id
    }

    pub(in crate::screens::home) fn toggle_attachment_spoiler(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(attachment) = self
            .pending_attachments
            .iter_mut()
            .find(|attachment| attachment.id == id)
        {
            attachment.spoiler = !attachment.spoiler;
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        for attachment in self
            .pending_attachments
            .iter()
            .filter(|attachment| attachment.id == id)
        {
            attachment.release_preview(cx);
        }
        self.pending_attachments
            .retain(|attachment| attachment.id != id);
        cx.notify();
    }
}
