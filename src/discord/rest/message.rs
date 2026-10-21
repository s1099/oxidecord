//! Reading a channel's history, sending to it, and reacting to what's in it.

use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::http::attachment::Attachment;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use super::request;
use crate::discord::model::{
    Message, RawPins, ReactionEmoji, Reactor, convert_message, convert_reactor,
};

/// How many messages one [`fetch_messages`] call requests; a response with
/// fewer means the start of the channel's history was reached.
pub const MESSAGE_PAGE_SIZE: usize = 50;

/// How many reactors one [`fetch_reactors`] call requests, the most Discord
/// allows; a response with fewer means the list is complete.
pub const REACTORS_PAGE_SIZE: usize = 100;

/// TODO: add support for higher file sizes upto 500mb with nitro checks
pub const MAX_ATTACHMENT_SIZE: u64 = 10 * 1024 * 1024;

/// Which page of a channel's history [`fetch_messages`] fetches.
#[derive(Clone, Copy)]
pub enum MessagePage {
    Latest,
    Before(Id<MessageMarker>),
    /// A page centred on this message, which needn't be among the results —
    /// a jump then reports it unavailable rather than showing the wrong spot.
    Around(Id<MessageMarker>),
    /// The messages just newer than this one, which needn't be real — any
    /// snowflake does, an ack's included.
    After(Id<MessageMarker>),
}

/// Fetches a page of messages in a channel, oldest first.
pub async fn fetch_messages(
    channel_id: Id<ChannelMarker>,
    page: MessagePage,
) -> Result<Vec<Message>, String> {
    request(move |client| async move {
        let request = client
            .channel_messages(channel_id)
            .limit(MESSAGE_PAGE_SIZE as u16);
        // Each anchor changes the request's type, so await per branch.
        let response = match page {
            MessagePage::Latest => request.await,
            MessagePage::Before(before) => request.before(before).await,
            MessagePage::Around(around) => request.around(around).await,
            MessagePage::After(after) => request.after(after).await,
        }?;
        let mut messages = response.models().await?;

        // The API returns newest first; the UI renders oldest first. Sorted
        // rather than reversed, since Discord doesn't promise the order of an
        // `after` page.
        messages.sort_by_key(|message| message.id);
        Ok(messages.into_iter().map(convert_message).collect())
    })
    .await
}

/// Fetches every message pinned in a channel, most recently pinned first.
///
/// Uses `GET /channels/{id}/messages/pins`, which replaced the old
/// `/channels/{id}/pins` (twilight's `pins`) and pages by pin time instead of
/// capping at 50. Raw, since twilight has no helper for it.
pub async fn fetch_pins(channel_id: Id<ChannelMarker>) -> Result<Vec<Message>, String> {
    request(move |client| async move {
        let mut messages = Vec::new();
        let mut before: Option<String> = None;
        loop {
            let mut path = format!("channels/{channel_id}/messages/pins?limit=50");
            if let Some(before) = &before {
                // The offset's `+` would read as a space in a query string.
                path.push_str(&format!("&before={}", before.replace('+', "%2B")));
            }
            let request = RequestBuilder::raw(Method::Get, path).build()?;
            let page = client.request::<RawPins>(request).await?.model().await?;
            before = page.items.last().map(|pin| pin.pinned_at.clone());
            messages.extend(
                page.items
                    .into_iter()
                    .map(|pin| convert_message(pin.message)),
            );
            if !page.has_more || before.is_none() {
                return Ok(messages);
            }
        }
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
        let emoji = request_emoji(&emoji);
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

/// Fetches a page of the users who reacted to a message with `emoji`, in the
/// order they reacted, starting after the user `after`.
pub async fn fetch_reactors(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    emoji: ReactionEmoji,
    after: Option<Id<UserMarker>>,
) -> Result<Vec<Reactor>, String> {
    request(move |client| async move {
        let emoji = request_emoji(&emoji);
        let request = client
            .reactions(channel_id, message_id, &emoji)
            .limit(REACTORS_PAGE_SIZE as u16);
        let request = match after {
            Some(after) => request.after(after),
            None => request,
        };
        let users = request.await?.models().await?;
        Ok(users.into_iter().map(convert_reactor).collect())
    })
    .await
}

fn request_emoji(emoji: &ReactionEmoji) -> RequestReactionType<'_> {
    match emoji {
        ReactionEmoji::Unicode(name) => RequestReactionType::Unicode { name },
        ReactionEmoji::Custom { id, name, .. } => RequestReactionType::Custom {
            id: *id,
            name: Some(name),
        },
    }
}

/// Shows the current user typing in a channel, until they send there or ten
/// seconds pass.
pub async fn trigger_typing(channel_id: Id<ChannelMarker>) -> Result<(), String> {
    request(move |client| async move {
        client.create_typing_trigger(channel_id).await?;
        Ok(())
    })
    .await
}

/// Marks a channel read up to `message_id` (`POST
/// /channels/{id}/messages/{id}/ack`), which needn't be a real message.
///
/// `manual` says the ack goes backwards, as marking a message unread does —
/// without it, other clients may ignore an ack older than the one they hold.
/// Discord doesn't recount the mentions past a manual ack, so it's given
/// none. A user-client endpoint with no twilight helper.
pub async fn ack_message(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    manual: bool,
) -> Result<(), String> {
    request(move |client| async move {
        let body = if manual {
            serde_json::json!({ "token": null, "manual": true, "mention_count": 0 })
        } else {
            serde_json::json!({ "token": null })
        };
        let request = RequestBuilder::raw(
            Method::Post,
            format!("channels/{channel_id}/messages/{message_id}/ack"),
        )
        .json(&body)
        .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// Sends a message. `nonce` comes back on the gateway's `MESSAGE_CREATE` for
/// it, which is how the optimistic copy shown meanwhile is matched up.
pub async fn send_message(
    channel_id: Id<ChannelMarker>,
    nonce: u64,
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
        let mut request = client.create_message(channel_id).nonce(nonce);
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
