//! Reading a channel's history, sending to it, and reacting to what's in it.

use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_model::http::attachment::Attachment;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use super::request;
use crate::discord::model::{Message, ReactionEmoji, convert_message};

/// How many messages one [`fetch_messages`] call requests; a response with
/// fewer means the start of the channel's history was reached.
pub const MESSAGE_PAGE_SIZE: usize = 50;

/// TODO: add support for higher file sizes upto 500mb with nitro checks
pub const MAX_ATTACHMENT_SIZE: u64 = 10 * 1024 * 1024;

/// Fetches a page of messages in a channel, oldest first. With `before`,
/// fetches the page older than that message; otherwise the most recent page.
pub async fn fetch_messages(
    channel_id: Id<ChannelMarker>,
    before: Option<Id<MessageMarker>>,
) -> Result<Vec<Message>, String> {
    request(move |client| async move {
        let request = client
            .channel_messages(channel_id)
            .limit(MESSAGE_PAGE_SIZE as u16);
        // `.before()` changes the request's type, so await per branch.
        let response = match before {
            Some(before) => request.before(before).await,
            None => request.await,
        }?;
        let mut messages = response.models().await?;

        // The API returns newest first; the UI renders oldest first.
        messages.reverse();
        Ok(messages.into_iter().map(convert_message).collect())
    })
    .await
}

/// Adds (`add`) or removes the current user's reaction with `emoji`.
pub async fn toggle_reaction(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    emoji: ReactionEmoji,
    add: bool,
) -> Result<(), String> {
    request(move |client| async move {
        let emoji = match &emoji {
            ReactionEmoji::Unicode(name) => RequestReactionType::Unicode { name },
            ReactionEmoji::Custom { id, name, .. } => RequestReactionType::Custom {
                id: *id,
                name: Some(name),
            },
        };
        if add {
            client
                .create_reaction(channel_id, message_id, &emoji)
                .await?;
        } else {
            client
                .delete_current_user_reaction(channel_id, message_id, &emoji)
                .await?;
        }
        Ok(())
    })
    .await
}

pub async fn send_message(
    channel_id: Id<ChannelMarker>,
    content: String,
    reply_to: Option<Id<MessageMarker>>,
    attachments: Vec<(String, Vec<u8>)>,
) -> Result<Message, String> {
    request(move |client| async move {
        // Attachment ids only need to be unique within this message, so the
        // slice index works.
        let attachments: Vec<Attachment> = attachments
            .into_iter()
            .enumerate()
            .map(|(index, (filename, file))| Attachment::from_bytes(filename, file, index as u64))
            .collect();

        // Discord requires at least one of content/attachments; both the
        // content and attachments borrows must outlive the awaited request.
        let mut request = client.create_message(channel_id);
        if !content.is_empty() {
            request = request.content(&content);
        }
        if !attachments.is_empty() {
            request = request.attachments(&attachments);
        }
        if let Some(message_id) = reply_to {
            request = request.reply(message_id);
        }
        let message = request.await?.model().await?;

        Ok(convert_message(message))
    })
    .await
}

/// Replaces a message's text. Only ever permitted on the current user's own
/// messages; attachments and embeds are left as they are.
pub async fn edit_message(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    content: String,
) -> Result<Message, String> {
    request(move |client| async move {
        let message = client
            .update_message(channel_id, message_id)
            .content(Some(&content))
            .await?
            .model()
            .await?;
        Ok(convert_message(message))
    })
    .await
}

/// Deletes a message. Permitted for the author's own messages, or anywhere the
/// user holds `MANAGE_MESSAGES`.
pub async fn delete_message(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
) -> Result<(), String> {
    request(move |client| async move {
        client.delete_message(channel_id, message_id).await?;
        Ok(())
    })
    .await
}
