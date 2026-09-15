//! Voice states: who is in which voice channel, and what they've silenced.
//!
//! These arrive as raw gateway dispatches rather than through twilight's
//! models, because a DM call's `VOICE_SERVER_UPDATE` carries no guild id and
//! twilight's model requires one.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use super::cdn;

/// One user's presence in a voice channel, as the gateway last reported it.
#[derive(Clone)]
pub struct VoiceUserState {
    pub user_id: Id<UserMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    /// The channel they're in, or `None` when they've left voice entirely.
    pub channel_id: Option<Id<ChannelMarker>>,
    /// Identifies this voice session to the voice server; needed to connect.
    pub session_id: String,
    pub self_mute: bool,
    pub self_deaf: bool,
    /// Muted or deafened by a moderator, which the user can't undo themselves.
    pub mute: bool,
    pub deaf: bool,
    /// Display name and avatar, carried by dispatches that include the member.
    /// Absent ones fall back to whatever the app already knows about the user.
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Where a call's voice websocket lives, from `VOICE_SERVER_UPDATE`.
#[derive(Clone)]
pub struct VoiceServerInfo {
    /// `None` while Discord is reallocating the call to another server; the
    /// next update carries the new one.
    pub endpoint: Option<String>,
    pub token: String,
    /// The guild for a channel call, absent for a DM call.
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Option<Id<ChannelMarker>>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawVoiceState {
    user_id: Id<UserMarker>,
    #[serde(default)]
    guild_id: Option<Id<GuildMarker>>,
    #[serde(default)]
    channel_id: Option<Id<ChannelMarker>>,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    self_mute: bool,
    #[serde(default)]
    self_deaf: bool,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    deaf: bool,
    #[serde(default)]
    member: Option<RawMember>,
}

#[derive(Deserialize)]
struct RawMember {
    #[serde(default)]
    user: Option<RawVoiceUser>,
}

#[derive(Deserialize)]
struct RawVoiceUser {
    id: Id<UserMarker>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawVoiceServer {
    #[serde(default)]
    endpoint: Option<String>,
    token: String,
    #[serde(default)]
    guild_id: Option<Id<GuildMarker>>,
    #[serde(default)]
    channel_id: Option<Id<ChannelMarker>>,
}

/// `guild_id` stands in for dispatches that carry the guild outside the voice
/// state itself, as `GUILD_CREATE`'s bundled states do.
pub(in crate::discord) fn convert_voice_state(
    raw: RawVoiceState,
    guild_id: Option<Id<GuildMarker>>,
) -> VoiceUserState {
    let user = raw.member.and_then(|member| member.user);
    let name = user.as_ref().and_then(|user| {
        user.global_name
            .clone()
            .or_else(|| user.username.clone())
            .filter(|name| !name.is_empty())
    });
    let avatar_url = user.as_ref().and_then(|user| {
        user.avatar
            .as_ref()
            .map(|hash| cdn::small_avatar_url(user.id.get(), hash))
    });

    VoiceUserState {
        user_id: raw.user_id,
        guild_id: raw.guild_id.or(guild_id),
        channel_id: raw.channel_id,
        session_id: raw.session_id,
        self_mute: raw.self_mute,
        self_deaf: raw.self_deaf,
        mute: raw.mute,
        deaf: raw.deaf,
        name,
        avatar_url,
    }
}

pub(in crate::discord) fn convert_voice_server(raw: RawVoiceServer) -> VoiceServerInfo {
    VoiceServerInfo {
        // The voice websocket is spoken over TLS, but Discord has handed out
        // endpoints with a plaintext `:80` on them; dropping it leaves the
        // default 443 the connection actually wants.
        endpoint: raw.endpoint.map(|endpoint| {
            endpoint
                .strip_suffix(":80")
                .map_or(endpoint.clone(), str::to_owned)
        }),
        token: raw.token,
        guild_id: raw.guild_id,
        channel_id: raw.channel_id,
    }
}
