use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, Channel};

use crate::screens::dialogs::show_error;

use super::channels::build_channel_groups;
use super::{AttachmentData, HomeScreen, PendingAttachment, View, format_size};

/// The upload filename extension for a pasted image's format.
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

/// The image format for a path's extension, when it names one we can decode for
/// a thumbnail. Anything else is staged as a plain file.
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

/// A file chosen in the picker, read and ready to stage.
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
    pub(super) fn load_guilds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            self.loading = false;
            self.error = Some("No token found. Please log in first.".into());
            return;
        };

        // `discord::fetch_guilds` runs its callback on a background Tokio
        // thread, but gpui's entity/async handles are `!Send`. Bridge the two
        // with a plain, `Send`-safe channel and let gpui's own (non-Send)
        // foreground task pick up the result.
        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_guilds(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, window, cx);
                        }
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Loads the signed-in user for the sidebar account panel. Best-effort:
    /// on failure the panel just stays empty.
    pub(super) fn load_current_user(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_current_user(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(Ok(user)) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.current_user = Some(user);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_guild = self.selected_guild == Some(guild_id);
        if self.view == View::Guild && same_guild {
            return;
        }
        self.view = View::Guild;
        // Returning from the DM view to the guild that's already loaded: switch
        // back to its channels without refetching them, reopening a channel
        // since the DM view cleared the previous selection.
        if same_guild && !self.channel_groups.is_empty() {
            if let Some(first) = self.first_text_channel() {
                self.select_channel(first, window, cx);
            }
            cx.notify();
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_groups.clear();
        self.selected_channel = None;
        self.collapsed_categories.clear();
        self.channels_error = None;
        self.channels_loading = true;
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.channels_loading = false;
            self.channels_error = Some("No token found. Please log in first.".into());
            return;
        };

        // The guild list already carries the user's guild-wide permissions, so
        // channel visibility only needs the member object on top of them.
        let (base_permissions, owner) = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| (guild.permissions, guild.owner))
            .unwrap_or((discord::Permissions::empty(), false));

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_channels(token, guild_id, base_permissions, owner, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                // The user may have clicked another guild while this request
                // was in flight; drop the stale response.
                if this.selected_guild != Some(guild_id) {
                    return;
                }
                match result {
                    Ok(channels) => {
                        this.channel_groups = build_channel_groups(channels);
                        if let Some(channel_id) = this.first_text_channel() {
                            this.select_channel(channel_id, window, cx);
                        }
                    }
                    Err(err) => this.channels_error = Some(err),
                }
                this.channels_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel == Some(channel_id) {
            return;
        }
        self.selected_channel = Some(channel_id);
        self.load_messages(window, cx);
    }

    fn load_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        self.messages.clear();
        self.messages_list.reset(0);
        // Release the previous channel's decoded images instead of letting them
        // accumulate; the new channel repopulates the cache as it renders.
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.messages_error = None;
        self.older_loading = false;
        self.reached_oldest = false;
        // A freshly opened channel starts pinned to its newest message, so
        // live messages should follow along until the user scrolls up.
        self.at_bottom = true;
        self.send_error = None;
        self.replying_to = None;
        self.pending_attachments.clear();
        self.messages_loading = true;

        let placeholder = match self.view {
            View::DirectMessages => self
                .selected_dm_info()
                .map(|dm| format!("Message @{}", dm.name))
                .unwrap_or_else(|| "Send a message".into()),
            View::Guild => self
                .selected_channel_info()
                .map(|channel| format!("Message #{}", channel.name))
                .unwrap_or_else(|| "Send a message".into()),
        };
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.messages_loading = false;
            self.messages_error = Some("No token found. Please log in first.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, None, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // The user may have clicked another channel while this request
                // was in flight; drop the stale response.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    Ok(messages) => {
                        this.reached_oldest = messages.len() < discord::MESSAGE_PAGE_SIZE;
                        this.messages = messages;
                        // Resetting also snaps the bottom-aligned list to the
                        // newest message.
                        this.messages_list.reset(this.messages.len());
                    }
                    Err(err) => this.messages_error = Some(err),
                }
                this.messages_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetches the page of messages older than the oldest loaded one and
    /// prepends it, keeping the current scroll position.
    pub(super) fn load_older_messages(&mut self, cx: &mut Context<Self>) {
        if self.messages_loading || self.older_loading || self.reached_oldest {
            return;
        }
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(oldest_id) = self.messages.first().map(|message| message.id) else {
            return;
        };
        let Some(token) = discord::load_token() else {
            return;
        };

        self.older_loading = true;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, Some(oldest_id), move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the channel changed or the messages
                // were reloaded while this request was in flight.
                if this.selected_channel != Some(channel_id)
                    || this.messages.first().map(|message| message.id) != Some(oldest_id)
                {
                    return;
                }
                this.older_loading = false;
                match result {
                    Ok(older) => {
                        if older.len() < discord::MESSAGE_PAGE_SIZE {
                            this.reached_oldest = true;
                        }
                        let count = older.len();
                        if count > 0 {
                            this.messages.splice(0..0, older);
                            this.messages_list.splice(0..0, count);
                        }
                    }
                    // Keep the messages on screen; the fetch retries the next
                    // time the user scrolls near the top.
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Handles the paste shortcut in the composer. If the clipboard holds an
    /// image, it's staged as an attachment and the event is consumed. Otherwise
    /// (plain text, or the composer isn't focused) propagation continues so the
    /// text input's own paste handling runs as usual.
    pub(super) fn on_paste_attachment(
        &mut self,
        _: &super::PasteAttachment,
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
    pub(super) fn paste_attachment_from_clipboard(
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
    pub(super) fn pick_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Takes the next id for a staged attachment, unique within the composer.
    fn reserve_attachment_id(&mut self) -> u64 {
        self.next_attachment_id += 1;
        self.next_attachment_id
    }

    /// Removes a staged attachment by its id (the thumbnail's "×" button).
    pub(super) fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.pending_attachments
            .retain(|attachment| attachment.id != id);
        cx.notify();
    }

    pub(super) fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let content = self.message_input.read(cx).value().trim().to_string();
        // Nothing to send: no text and no staged images.
        if content.is_empty() && self.pending_attachments.is_empty() {
            return;
        }

        let Some(token) = discord::load_token() else {
            self.send_error = Some("No token found. Please log in first.".into());
            cx.notify();
            return;
        };

        let reply_to = self.replying_to.as_ref().map(|target| target.message_id);
        let attachments: Vec<(String, Vec<u8>)> = self
            .pending_attachments
            .drain(..)
            .map(|attachment| (attachment.filename, attachment.data.bytes().to_vec()))
            .collect();

        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_error = None;
        self.replying_to = None;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::send_message(
            token,
            channel_id,
            content,
            reply_to,
            attachments,
            move |result| {
                let _ = tx.send(result);
            },
        );

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the user switched channels meanwhile.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    // The sent message is rendered when it arrives back over the
                    // gateway as a `MESSAGE_CREATE`, so nothing to append here;
                    // appending it too would duplicate it in the list.
                    Ok(_) => {}
                    Err(err) => this.send_error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the gateway connection and pumps live `MESSAGE_CREATE` events
    /// onto the gpui foreground, where they update the open conversation.
    ///
    /// The gateway callback fires on a background Tokio thread, but gpui's
    /// entity handles are `!Send`, so events are bridged over an unbounded,
    /// `Send`-safe channel that a foreground task drains.
    pub(super) fn start_gateway(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::mpsc::unbounded::<discord::IncomingMessage>();
        discord::connect_gateway(token, move |incoming| {
            // Returns whether the foreground receiver is still around; once it
            // isn't (the screen was dropped), the gateway loop stops.
            tx.unbounded_send(incoming).is_ok()
        });

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(incoming) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_incoming_message(incoming, cx))
                    .is_err()
                {
                    // The entity is gone; stop draining so the sender closes.
                    break;
                }
            }
        })
        .detach();
    }

    /// Appends a live message to the open conversation, if it belongs there.
    fn handle_incoming_message(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        // Only the currently open channel/DM is displayed.
        if self.selected_channel != Some(incoming.channel_id) {
            return;
        }
        // The history for this channel is still loading and will replace the
        // whole list when it lands (and include this message), so skip it now
        // to avoid a desync between `messages` and the list state.
        if self.messages_loading {
            return;
        }
        // Ignore duplicates: the echo of a message we just sent ourselves, or a
        // repeated dispatch.
        if self
            .messages
            .iter()
            .any(|message| message.id == incoming.message.id)
        {
            return;
        }

        let ix = self.messages.len();
        self.messages.push(incoming.message);
        self.messages_list.splice(ix..ix, 1);
        // Follow the conversation only when the newest message was already in
        // view; if the user has scrolled up to read history, leave them there.
        if self.at_bottom {
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    pub(super) fn selected_channel_info(&self) -> Option<&Channel> {
        let id = self.selected_channel?;
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == id)
    }

    /// The first text channel in display order used as the
    /// default selection when entering a guild
    fn first_text_channel(&self) -> Option<Id<ChannelMarker>> {
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| !channel.kind.is_voice())
            .map(|channel| channel.id)
    }
}
