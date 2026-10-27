//! Commands sent up the gateway.

use twilight_gateway::MessageSender;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::discord::model::PresenceStatus;

/// Cloneable; commands queue on a channel the shard drains, so they survive a reconnect.
#[derive(Clone)]
pub struct GatewaySender {
    pub(super) inner: MessageSender,
}

impl GatewaySender {
    /// Sends `VOICE_STATE_UPDATE` (opcode 4); a `None` channel leaves. `guild_id` is `None`
    /// for a DM call, a null field twilight's command type can't express, so the payload is
    /// built here.
    pub fn update_voice_state(
        &self,
        guild_id: Option<Id<GuildMarker>>,
        channel_id: Option<Id<ChannelMarker>>,
        self_mute: bool,
        self_deaf: bool,
    ) {
        let payload = serde_json::json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": self_mute,
                "self_deaf": self_deaf,
                "self_video": false,
            }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `PRESENCE_UPDATE` (opcode 3) for the rest of the session. Activities are empty.
    pub fn update_presence(&self, status: PresenceStatus) {
        let payload = serde_json::json!({
            "op": 3,
            "d": {
                "status": status.as_str(),
                "since": 0,
                "activities": [],
                "afk": false,
            }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `REQUEST_CALL_CONNECT` (opcode 13). Without `AUTO_CALL_CONNECT`, a call that
    /// started before the session is only reported on request, as `CALL_CREATE`; nothing
    /// comes back when there's none.
    pub fn request_call_connect(&self, channel_id: Id<ChannelMarker>) {
        let payload = serde_json::json!({
            "op": 13,
            "d": { "channel_id": channel_id }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `STREAM_CREATE` (opcode 18); the stream's server comes back as dispatches.
    pub fn create_stream(&self, guild_id: Option<Id<GuildMarker>>, channel_id: Id<ChannelMarker>) {
        let payload = serde_json::json!({
            "op": 18,
            "d": {
                "type": if guild_id.is_some() { "guild" } else { "call" },
                "guild_id": guild_id,
                "channel_id": channel_id,
                "preferred_region": null,
            }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `STREAM_SET_PAUSED` (opcode 22). A new stream starts paused.
    pub fn set_stream_paused(&self, stream_key: &str, paused: bool) {
        let payload = serde_json::json!({
            "op": 22,
            "d": { "stream_key": stream_key, "paused": paused }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `STREAM_WATCH` (opcode 20). Answered with the stream's server as dispatches,
    /// or a `STREAM_DELETE` saying why not.
    pub fn watch_stream(&self, stream_key: &str) {
        let payload = serde_json::json!({
            "op": 20,
            "d": { "stream_key": stream_key }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `REQUEST_GUILD_MEMBERS` (opcode 8) for specific users; replies are
    /// `GUILD_MEMBERS_CHUNK`s. A user session gets only a sliver of each guild's members up
    /// front and fetched history carries none, so this is how an author gets a role colour.
    /// Discord takes at most 100 ids a request.
    pub fn request_members(&self, guild_id: Id<GuildMarker>, user_ids: &[Id<UserMarker>]) {
        for user_ids in user_ids.chunks(100) {
            let payload = serde_json::json!({
                "op": 8,
                "d": {
                    "guild_id": [guild_id],
                    "user_ids": user_ids,
                    "presences": false,
                }
            });
            let _ = self.inner.send(payload.to_string());
        }
    }

    /// Sends `REQUEST_GUILD_MEMBERS` (opcode 8) for up to `limit` members whose username or
    /// nickname starts with `query`; replies like [`Self::request_members`]'s.
    pub fn search_members(&self, guild_id: Id<GuildMarker>, query: &str, limit: u8) {
        let payload = serde_json::json!({
            "op": 8,
            "d": {
                "guild_id": [guild_id],
                "query": query,
                "limit": limit,
                "presences": false,
            }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `GUILD_SUBSCRIPTIONS_BULK` (opcode 37): subscribes to `channel_id`'s member list
    /// in inclusive index ranges (a hundred apiece, as Discord's client asks), replacing the
    /// guild's earlier subscription. Discord syncs each range with `GUILD_MEMBER_LIST_UPDATE`,
    /// then keeps them current.
    ///
    /// Undocumented; the other flags are what Discord's own client sends.
    pub fn subscribe_member_list(
        &self,
        guild_id: Id<GuildMarker>,
        channel_id: Id<ChannelMarker>,
        ranges: &[[usize; 2]],
    ) {
        let payload = serde_json::json!({
            "op": 37,
            "d": {
                "subscriptions": {
                    guild_id.to_string(): {
                        "typing": true,
                        "activities": true,
                        "threads": true,
                        "channels": { channel_id.to_string(): ranges },
                    }
                }
            }
        });
        let _ = self.inner.send(payload.to_string());
    }

    /// Sends `STREAM_DELETE` (opcode 19): stops streaming, or watching someone else's stream.
    pub fn delete_stream(&self, stream_key: &str) {
        let payload = serde_json::json!({
            "op": 19,
            "d": { "stream_key": stream_key }
        });
        let _ = self.inner.send(payload.to_string());
    }
}
