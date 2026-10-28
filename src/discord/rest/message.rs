//! Reading a channel's history, sending to it, and reacting to what's in it.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt as _;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use twilight_http::Client as HttpClient;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::channel::Message as RawMessage;
use twilight_model::channel::message::{AllowedMentions, MentionType};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use super::request;
use crate::discord::model::{
    Message, RawPins, ReactionEmoji, Reactor, UploadProgress, convert_message, convert_reactor,
};
use crate::platform::http;

/// How many messages one [`fetch_messages`] call requests; a response with
/// fewer means the start of the channel's history was reached.
pub const MESSAGE_PAGE_SIZE: usize = 50;

/// How many reactors one [`fetch_reactors`] call requests, the most Discord
/// allows; a response with fewer means the list is complete.
pub const REACTORS_PAGE_SIZE: usize = 100;

#[derive(Clone, Copy)]
pub enum MessagePage {
    Latest,
    Before(Id<MessageMarker>),
    /// The messages just newer than this one, which needn't be real — any
    /// snowflake does, an ack's included.
    After(Id<MessageMarker>),
    Around(Id<MessageMarker>),
}

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
            MessagePage::After(after) => request.after(after).await,
            MessagePage::Around(around) => request.around(around).await,
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
/// Discord doesn't recount the mentions past a manual ack, so it's told
/// `mention_count`; a forward ack ignores it. A user-client endpoint with no
/// twilight helper.
pub async fn ack_message(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    manual: bool,
    mention_count: u32,
) -> Result<(), String> {
    request(move |client| async move {
        let body = if manual {
            serde_json::json!({ "token": null, "manual": true, "mention_count": mention_count })
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
///
/// Files go to cloud storage first, as Discord's client sends them: twilight
/// gives up on a request after ten seconds, too soon to carry a large file
/// inside the message itself, and a separate upload can report progress and be
/// cancelled through `progress`.
pub async fn send_message(
    channel_id: Id<ChannelMarker>,
    nonce: u64,
    content: String,
    reply_to: Option<Id<MessageMarker>>,
    mention_author: bool,
    attachments: Vec<(String, Vec<u8>)>,
    progress: Arc<UploadProgress>,
) -> Result<Message, String> {
    // Left out, Discord pings the replied-to author; otherwise everything else
    // still mentions, as Discord's client sends it.
    let allowed_mentions = (!mention_author).then(|| AllowedMentions {
        parse: vec![
            MentionType::Users,
            MentionType::Roles,
            MentionType::Everyone,
        ],
        ..Default::default()
    });
    request(move |client| async move {
        if attachments.is_empty() {
            let mut request = client
                .create_message(channel_id)
                .nonce(nonce)
                .content(&content);
            if let Some(message_id) = reply_to {
                request = request.reply(message_id);
            }
            if let Some(allowed_mentions) = &allowed_mentions {
                request = request.allowed_mentions(Some(allowed_mentions));
            }
            return Ok(convert_message(request.await?.model().await?));
        }

        let uploaded = upload_attachments(&client, channel_id, attachments, &progress).await?;
        let sent = async {
            anyhow::ensure!(!progress.is_cancelled(), "upload cancelled");
            // Attachment ids only need to be unique within this message, so the
            // index works.
            let attachments: Vec<_> = uploaded
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    serde_json::json!({
                        "id": index.to_string(),
                        "filename": file.filename,
                        "uploaded_filename": file.upload_filename,
                    })
                })
                .collect();
            let mut body = serde_json::json!({
                "content": content,
                "nonce": nonce.to_string(),
                "attachments": attachments,
            });
            if let Some(message_id) = reply_to {
                body["message_reference"] =
                    serde_json::json!({ "message_id": message_id, "channel_id": channel_id });
            }
            if let Some(allowed_mentions) = &allowed_mentions {
                body["allowed_mentions"] = serde_json::to_value(allowed_mentions)?;
            }
            let request =
                RequestBuilder::raw(Method::Post, format!("channels/{channel_id}/messages"))
                    .json(&body)
                    .build()?;
            Ok(client.request::<RawMessage>(request).await?.model().await?)
        }
        .await;
        if sent.is_err() {
            delete_uploads(&client, &uploaded).await;
        }
        Ok(convert_message(sent?))
    })
    .await
}

/// How much of a file is handed to the connection at a time, and so how finely
/// progress moves.
const UPLOAD_CHUNK: usize = 64 * 1024;

/// Storage slots from `POST /channels/{id}/attachments`, in request order.
#[derive(Deserialize)]
struct CloudAttachments {
    attachments: Vec<CloudAttachment>,
}

#[derive(Deserialize)]
struct CloudAttachment {
    upload_url: String,
    upload_filename: String,
}

struct UploadedFile {
    filename: String,
    upload_filename: String,
}

/// Puts every file in cloud storage at once. On failure or cancelling,
/// whatever made it up is deleted again.
async fn upload_attachments(
    client: &HttpClient,
    channel_id: Id<ChannelMarker>,
    attachments: Vec<(String, Vec<u8>)>,
    progress: &Arc<UploadProgress>,
) -> anyhow::Result<Vec<UploadedFile>> {
    let files: Vec<_> = attachments
        .iter()
        .enumerate()
        .map(|(index, (filename, file))| {
            serde_json::json!({
                "id": index.to_string(),
                "filename": filename,
                "file_size": file.len(),
            })
        })
        .collect();
    let request = RequestBuilder::raw(Method::Post, format!("channels/{channel_id}/attachments"))
        .json(&serde_json::json!({ "files": files }))
        .build()?;
    let slots = client
        .request::<CloudAttachments>(request)
        .await?
        .model()
        .await?
        .attachments;
    anyhow::ensure!(
        slots.len() == attachments.len(),
        "Discord returned {} upload slots for {} files",
        slots.len(),
        attachments.len()
    );

    let uploaded: Vec<_> = attachments
        .iter()
        .zip(&slots)
        .map(|((filename, _), slot)| UploadedFile {
            filename: filename.clone(),
            upload_filename: slot.upload_filename.clone(),
        })
        .collect();
    let puts = futures::future::try_join_all(
        attachments
            .into_iter()
            .zip(slots)
            .map(|((_, file), slot)| put_file(slot.upload_url, file, progress.clone())),
    );
    // Dropping the requests closes their connections, so a cancel takes effect
    // at once rather than when the next chunk is due.
    let result = tokio::select! {
        result = puts => result.map(drop),
        () = progress.cancelled() => Err(anyhow::anyhow!("upload cancelled")),
    };
    if let Err(err) = result {
        delete_uploads(client, &uploaded).await;
        return Err(err);
    }
    Ok(uploaded)
}

/// Streamed in chunks so `progress` follows what the connection has taken.
async fn put_file(url: String, file: Vec<u8>, progress: Arc<UploadProgress>) -> anyhow::Result<()> {
    let file = Bytes::from(file);
    let len = file.len();
    let chunks = futures::stream::iter((0..len).step_by(UPLOAD_CHUNK)).map(move |start| {
        let chunk = file.slice(start..(start + UPLOAD_CHUNK).min(len));
        progress.advance(chunk.len() as u64);
        Ok::<_, std::io::Error>(chunk)
    });
    http::client()
        .put(url)
        // A streamed body is otherwise sent chunked, with no length up front.
        .header(reqwest::header::CONTENT_LENGTH, len)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Best effort: storage that's never attached to a message expires anyway.
async fn delete_uploads(client: &HttpClient, uploaded: &[UploadedFile]) {
    for file in uploaded {
        let path = format!(
            "attachments/{}",
            utf8_percent_encode(&file.upload_filename, PATH_SEGMENT)
        );
        if let Ok(request) = RequestBuilder::raw(Method::Delete, path).build() {
            let _ = client.request::<EmptyBody>(request).await;
        }
    }
}

/// What a filename can't carry into a URL path unescaped. `/` stays, as an
/// upload filename is a path itself.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'?')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'{')
    .add(b'}');

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

/// Permitted on the user's own messages, or anywhere they hold `MANAGE_MESSAGES`.
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
