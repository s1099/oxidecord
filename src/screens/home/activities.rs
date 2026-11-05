//! The state behind activities: what's running in each channel, the shelf to
//! start one from, and the one the user is in.

use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, WeakEntity};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{ActivityApp, ActivityInstance};
use crate::screens::activity::ActivityWindow;

pub(super) enum ShelfState {
    Loading,
    Loaded(Arc<Vec<ActivityApp>>),
    Failed,
}

/// At most one, like Discord's desktop client. Lives from the launch request,
/// or from Discord adding this session to an instance however it was started,
/// until the user leaves the activity, the call, or closes its window.
pub(super) struct ActivitySession {
    /// Tells this session's window apart from a later one's when it reports closing.
    pub id: u64,
    pub app: ActivityApp,
    pub channel_id: Id<ChannelMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    /// Running in the call's channel, so it ends with the call. One launched
    /// in a text channel needs no call.
    pub in_call: bool,
    /// The launch interaction's, matched against its failure.
    pub nonce: Option<u64>,
    /// Set once Discord lists this session among the instance's participants.
    pub instance: Option<ActivityInstance>,
    pub proxy_ticket: Option<String>,
    pub window: Option<(AnyWindowHandle, WeakEntity<ActivityWindow>)>,
}
