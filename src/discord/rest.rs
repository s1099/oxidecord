//! The REST calls the app makes against Discord's HTTP API.
//!
//! Each entry point spawns onto the shared background Tokio runtime and hands
//! its result to `on_done`, which therefore runs on that runtime's thread — not
//! gpui's foreground thread.

use twilight_http::Client as HttpClient;
use twilight_http::request::Request;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::response::marker::ListBody;
use twilight_http::routing::Route;
use twilight_model::http::attachment::Attachment;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker},
};
use twilight_util::permission_calculator::PermissionCalculator;

use crate::runtime;

use super::Permissions;
use super::model::{
    Channel, CurrentUser, DirectMessage, Guild, Message, ReactionEmoji, convert_channel,
    convert_current_user, convert_dms, convert_guild, convert_message,
};

/// How many messages one [`fetch_messages`] call requests; a response with
/// fewer means the start of the channel's history was reached.
pub const MESSAGE_PAGE_SIZE: usize = 50;

/// TODO: add support for higher file sizes upto 500mb with nitro checks
pub const MAX_ATTACHMENT_SIZE: u64 = 10 * 1024 * 1024;

/// Fetches the signed-in user (`GET /users/@me`).
pub fn fetch_current_user(
    token: String,
    on_done: impl FnOnce(Result<CurrentUser, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client.current_user().await.map_err(|err| err.to_string())?;
            let user = response.model().await.map_err(|err| err.to_string())?;
            Ok(convert_current_user(user))
        }
        .await;

        on_done(result);
    });
}

pub fn fetch_guilds(
    token: String,
    on_done: impl FnOnce(Result<Vec<Guild>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .current_user_guilds()
                .await
                .map_err(|err| err.to_string())?;
            let guilds = response.models().await.map_err(|err| err.to_string())?;

            Ok(guilds.into_iter().map(convert_guild).collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// Fetches a guild's channels.
///
/// Only channels the current user can view (holds the `VIEW_CHANNEL`
/// permission on) are returned; the rest are the channels Discord itself
/// hides from the sidebar. `base_permissions` and `owner` come from the guild
/// list (see [`Guild`]) and are the user's guild-wide permissions, so the only
/// per-guild extra request here is the member object — which of their roles
/// apply — used to resolve each channel's overwrites locally.
pub fn fetch_channels(
    token: String,
    guild_id: Id<GuildMarker>,
    base_permissions: Permissions,
    owner: bool,
    on_done: impl FnOnce(Result<Vec<Channel>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);

            // The member object tells us which roles the user has, so
            // role-specific channel overwrites can be matched.
            let member = client
                .current_user_guild_member(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .model()
                .await
                .map_err(|err| err.to_string())?;
            let channels = client
                .guild_channels(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .models()
                .await
                .map_err(|err| err.to_string())?;

            let user_id = member.user.id;
            // We already know the user's aggregate guild-wide permissions, so
            // seed the calculator with those as the `@everyone` baseline and
            // list the member's roles with empty permissions — the roles are
            // only needed by id, to match channel overwrites, not to recompute
            // the baseline.
            let member_roles: Vec<(Id<RoleMarker>, Permissions)> = member
                .roles
                .iter()
                .filter(|id| id.get() != guild_id.get())
                .map(|id| (*id, Permissions::empty()))
                .collect();

            Ok(channels
                .into_iter()
                .filter_map(|channel| {
                    // Apply the channel's own overwrites to the baseline and
                    // drop the channel if the user can't even view it.
                    let visible = {
                        let overwrites = channel.permission_overwrites.as_deref().unwrap_or(&[]);
                        let mut calculator = PermissionCalculator::new(
                            guild_id,
                            user_id,
                            base_permissions,
                            &member_roles,
                        );
                        if owner {
                            calculator = calculator.owner_id(user_id);
                        }
                        calculator
                            .in_channel(channel.kind, overwrites)
                            .contains(Permissions::VIEW_CHANNEL)
                    };

                    visible.then(|| convert_channel(channel)).flatten()
                })
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// Fetches the current user's open DM and group-DM conversations, ordered
/// most-recently-active first.
///
/// twilight has no typed helper for `GET /users/@me/channels`, so this issues
/// the route through the client's low-level request path.
pub fn fetch_dms(
    token: String,
    on_done: impl FnOnce(Result<Vec<DirectMessage>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let request = Request::from_route(&Route::GetUserPrivateChannels);
            let response = client
                .request::<ListBody<twilight_model::channel::Channel>>(request)
                .await
                .map_err(|err| err.to_string())?;
            let channels = response.models().await.map_err(|err| err.to_string())?;

            Ok(convert_dms(channels))
        }
        .await;

        on_done(result);
    });
}

/// Fetches a page of messages in a channel, oldest first. With `before`,
/// fetches the page older than that message; otherwise the most recent page.
pub fn fetch_messages(
    token: String,
    channel_id: Id<ChannelMarker>,
    before: Option<Id<MessageMarker>>,
    on_done: impl FnOnce(Result<Vec<Message>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let request = client
                .channel_messages(channel_id)
                .limit(MESSAGE_PAGE_SIZE as u16);
            // `.before()` changes the request's type, so await per branch.
            let response = match before {
                Some(before) => request.before(before).await,
                None => request.await,
            }
            .map_err(|err| err.to_string())?;
            let mut messages = response.models().await.map_err(|err| err.to_string())?;

            // The API returns newest first; the UI renders oldest first.
            messages.reverse();
            Ok(messages
                .into_iter()
                .map(convert_message)
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// Adds (`add`) or removes the current user's reaction with `emoji`.
pub fn toggle_reaction(
    token: String,
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    emoji: ReactionEmoji,
    add: bool,
    on_done: impl FnOnce(Result<(), String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
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
                    .await
                    .map_err(|err| err.to_string())?;
            } else {
                client
                    .delete_current_user_reaction(channel_id, message_id, &emoji)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        .await;

        on_done(result);
    });
}

pub fn send_message(
    token: String,
    channel_id: Id<ChannelMarker>,
    content: String,
    reply_to: Option<Id<MessageMarker>>,
    attachments: Vec<(String, Vec<u8>)>,
    on_done: impl FnOnce(Result<Message, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            // Attachment ids only need to be unique within this message, so the
            // slice index works.
            let attachments: Vec<Attachment> = attachments
                .into_iter()
                .enumerate()
                .map(|(index, (filename, file))| {
                    Attachment::from_bytes(filename, file, index as u64)
                })
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
            let response = request.await.map_err(|err| err.to_string())?;
            let message = response.model().await.map_err(|err| err.to_string())?;

            Ok(convert_message(message))
        }
        .await;

        on_done(result);
    });
}
