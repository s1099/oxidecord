//! Voice states, voice servers and stream servers.
//!
//! Parsed from raw dispatches rather than twilight's models, because a DM
//! call's `VOICE_SERVER_UPDATE` has no guild id and twilight requires one.

use std::collections::HashMap;

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use super::guild::RawMember;

#[derive(Clone)]
pub struct VoiceUserState {
    pub user_id: Id<UserMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    /// `None` once they've left voice.
    pub channel_id: Option<Id<ChannelMarker>>,
    pub session_id: String,
    pub self_mute: bool,
    pub self_deaf: bool,
    /// Set by a moderator; the user can't undo it.
    pub mute: bool,
    pub deaf: bool,
    /// Go Live.
    pub self_stream: bool,
    /// Only present when the dispatch includes the member.
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// From `VOICE_SERVER_UPDATE`.
#[derive(Clone)]
pub struct VoiceServerInfo {
    /// `None` while Discord reallocates the call; the next update has the new one.
    pub endpoint: Option<String>,
    pub token: String,
    /// Absent for a DM call.
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
    self_stream: bool,
    #[serde(default)]
    member: Option<RawMember>,
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

pub(in crate::discord) fn convert_voice_state(raw: RawVoiceState) -> VoiceUserState {
    convert(raw, None, None)
}

/// States bundled into a guild object (`READY`, `GUILD_CREATE`) carry neither
/// guild id nor member, so both come from the object.
pub(in crate::discord) fn convert_guild_voice_states<'a>(
    guild_id: Id<GuildMarker>,
    states: Vec<RawVoiceState>,
    members: impl IntoIterator<Item = &'a RawMember>,
) -> Vec<VoiceUserState> {
    if states.is_empty() {
        return Vec::new();
    }
    let members: HashMap<_, _> = members
        .into_iter()
        .filter_map(|member| Some((member.user_id()?, member)))
        .collect();
    states
        .into_iter()
        .map(|state| {
            let member = members.get(&state.user_id).copied();
            convert(state, Some(guild_id), member)
        })
        .collect()
}

/// States in a `CALL_CREATE` may omit the channel; they belong to the call.
pub(in crate::discord) fn convert_call_voice_states(
    channel_id: Id<ChannelMarker>,
    states: Vec<RawVoiceState>,
) -> Vec<VoiceUserState> {
    states
        .into_iter()
        .map(|mut state| {
            state.channel_id.get_or_insert(channel_id);
            convert(state, None, None)
        })
        .collect()
}

fn convert(
    raw: RawVoiceState,
    guild_id: Option<Id<GuildMarker>>,
    member: Option<&RawMember>,
) -> VoiceUserState {
    let member = raw.member.as_ref().or(member);
    let name = member.and_then(RawMember::display_name);
    let avatar_url = member.and_then(RawMember::avatar_url);

    VoiceUserState {
        user_id: raw.user_id,
        guild_id: raw.guild_id.or(guild_id),
        channel_id: raw.channel_id,
        session_id: raw.session_id,
        self_mute: raw.self_mute,
        self_deaf: raw.self_deaf,
        mute: raw.mute,
        deaf: raw.deaf,
        self_stream: raw.self_stream,
        name,
        avatar_url,
    }
}

pub(in crate::discord) fn convert_voice_server(raw: RawVoiceServer) -> VoiceServerInfo {
    VoiceServerInfo {
        endpoint: raw.endpoint.map(secure_endpoint),
        token: raw.token,
        guild_id: raw.guild_id,
        channel_id: raw.channel_id,
    }
}

/// From `STREAM_SERVER_UPDATE`.
#[derive(Clone, Deserialize)]
pub struct StreamServerInfo {
    pub stream_key: String,
    /// Absent while Discord reallocates the stream.
    #[serde(default)]
    pub endpoint: Option<String>,
    pub token: String,
}

pub(in crate::discord) fn convert_stream_server(raw: StreamServerInfo) -> StreamServerInfo {
    StreamServerInfo {
        endpoint: raw.endpoint.map(secure_endpoint),
        ..raw
    }
}

/// Discord has handed out endpoints ending in `:80`, but the websocket is TLS;
/// dropping it leaves the default 443.
fn secure_endpoint(endpoint: String) -> String {
    endpoint
        .strip_suffix(":80")
        .map_or(endpoint.clone(), str::to_owned)
}

/// Discord's key for a user's stream; a DM call uses the `call:` prefix.
pub fn stream_key(
    guild_id: Option<Id<GuildMarker>>,
    channel_id: Id<ChannelMarker>,
    user_id: Id<UserMarker>,
) -> String {
    match guild_id {
        Some(guild_id) => format!("guild:{guild_id}:{channel_id}:{user_id}"),
        None => format!("call:{channel_id}:{user_id}"),
    }
}
