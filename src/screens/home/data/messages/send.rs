//! Sending what's in the composer to the open conversation: shown at once as
//! a pending message, then swapped for the server's copy once it's confirmed.

use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::autocomplete::resolve_mentions;
use crate::screens::home::{HomeScreen, View};

/// How often an uploading message's progress bar is redrawn.
const PROGRESS_REDRAW: Duration = Duration::from_millis(200);

/// Discord's epoch, 2015-01-01, in Unix milliseconds.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// A snowflake for now, used as the nonce and pending id as Discord's client
/// does; the low bits count up so same-millisecond sends differ.
fn next_nonce() -> (u64, i64) {
    static INCREMENT: AtomicU64 = AtomicU64::new(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let millis = (now.as_millis() as u64).saturating_sub(DISCORD_EPOCH_MS);
    let increment = INCREMENT.fetch_add(1, Ordering::Relaxed) & 0xfff;
    // Never zero: ids are non-zero, and `millis` is only zero on a broken clock.
    ((millis << 22) | increment | 1 << 12, now.as_secs() as i64)
}

impl HomeScreen {
    pub(in crate::screens::home) fn send_current_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // Guard the send path too: the composer is replaced by a notice on
        // channels the user can't post in, but Enter could still reach here.
        if self.view != View::DirectMessages
            && !self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_send)
        {
            return;
        }

        let content = resolve_mentions(
            self.message_input.read(cx).value().trim(),
            &self.composer_mentions,
        );
        let content = self.resolve_emoji_names(&content);
        if content.is_empty() && self.pending_attachments.is_empty() {
            return;
        }

        let reply_to = self.replying_to.as_ref().map(|target| target.message_id);
        let mention_author = self
            .replying_to
            .as_ref()
            .is_none_or(|target| target.mention);
        let attachments: Vec<(String, Vec<u8>)> = self
            .pending_attachments
            .drain(..)
            .map(|attachment| {
                attachment.release_preview(cx);
                (
                    attachment.upload_filename(),
                    attachment.data.bytes().to_vec(),
                )
            })
            .collect();

        let progress = discord::UploadProgress::new(
            attachments.iter().map(|(_, file)| file.len() as u64).sum(),
        );
        let upload = (!attachments.is_empty()).then(|| discord::Upload {
            filenames: attachments
                .iter()
                .map(|(filename, _)| filename.clone())
                .collect(),
            progress: progress.clone(),
        });

        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.composer_mentions.clear();
        self.send_error = None;
        self.replying_to = None;
        // Speaking up reads the conversation, as in Discord: the divider goes
        // and the channel can be acked again.
        self.set_unread_marker(None);
        self.marked_unread = false;
        // The message ends the indicator, so the next keystroke starts it anew.
        self.typing_sent = None;

        // Sending from further back in history jumps to the present first,
        // as Discord does; the message comes in with it, so there's no
        // pending copy to show meanwhile.
        if !self.reached_newest {
            self.jump_to_present(window, cx);
        }

        let (nonce, timestamp) = next_nonce();
        let pending_id = Id::new(nonce);
        // While history is loading it will replace the list wholesale, and
        // bring the message with it once sent; without a signed-in user there's
        // no one to show as the author.
        if !self.messages_loading
            && let Some(author) = &self.current_user
        {
            let replied = reply_to.and_then(|id| self.messages.iter().find(|m| m.id == id));
            let pending = discord::Message::pending(
                pending_id,
                author,
                content.clone(),
                timestamp,
                replied,
                upload.clone(),
            );
            let ix = self.messages.len();
            self.messages.push(pending);
            self.splice_messages(ix..ix, 1);
            // The user's own message is always followed, wherever they'd
            // scrolled to.
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let send = discord::send_message(
                channel_id,
                nonce,
                content,
                reply_to,
                mention_author,
                attachments,
                progress.clone(),
            )
            .fuse();
            let mut send = pin!(send);
            let result = loop {
                // Progress lives in atomics the upload moves, so the bar is
                // redrawn on a timer rather than on each chunk.
                let mut tick = cx.background_executor().timer(PROGRESS_REDRAW).fuse();
                futures::select_biased! {
                    result = send => break result,
                    _ = tick => {
                        if upload.is_some() && this.update(cx, |_, cx| cx.notify()).is_err() {
                            return;
                        }
                    }
                }
            };
            let _ = this.update(cx, |this, cx| {
                // Drop the response if the user switched channels meanwhile, or
                // cancelled: the pending copy is gone already.
                if this.selected_channel != Some(channel_id) || progress.is_cancelled() {
                    return;
                }
                match result {
                    Ok(sent) => {
                        this.confirm_pending_message(pending_id, sent);
                    }
                    Err(err) => {
                        if let Some(message) = this.pending_message_mut(pending_id) {
                            message.delivery = discord::Delivery::Failed;
                        }
                        this.send_error = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops a pending message's upload and drops the message, as Discord does.
    pub(in crate::screens::home) fn cancel_upload(
        &mut self,
        pending_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.messages.iter().position(|message| {
            message.id == pending_id
                && matches!(
                    message.delivery,
                    discord::Delivery::Sending { upload: Some(_) }
                )
        }) else {
            return;
        };
        if let discord::Delivery::Sending {
            upload: Some(upload),
        } = &self.messages.remove(ix).delivery
        {
            upload.progress.cancel();
        }
        self.splice_messages(ix..ix + 1, 0);
        cx.notify();
    }

    /// Whichever of the send's response and the gateway's echo lands first does it;
    /// the second finds no pending copy.
    pub(in crate::screens::home) fn confirm_pending_message(
        &mut self,
        pending_id: Id<MessageMarker>,
        sent: discord::Message,
    ) -> bool {
        let Some(ix) = self.messages.iter().position(|message| {
            message.id == pending_id && message.delivery != discord::Delivery::Sent
        }) else {
            return false;
        };
        self.messages[ix] = sent;
        // The confirmed copy can be taller: its images and embeds.
        self.splice_messages(ix..ix + 1, 1);
        true
    }

    pub(in crate::screens::home) fn has_pending_message(
        &self,
        pending_id: Id<MessageMarker>,
    ) -> bool {
        self.messages
            .iter()
            .any(|message| message.id == pending_id && message.delivery != discord::Delivery::Sent)
    }

    fn pending_message_mut(
        &mut self,
        pending_id: Id<MessageMarker>,
    ) -> Option<&mut discord::Message> {
        self.messages
            .iter_mut()
            .find(|message| message.id == pending_id && message.delivery != discord::Delivery::Sent)
    }
}
