//! The state behind message components: interactions waiting on an app, the
//! picks shown in select menus, the select open for picking, and an open modal.

use std::collections::HashMap;

use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord::{ChannelKind, ReactionEmoji};

use super::view::components::{ModalDialog, SelectPicker};

#[derive(Default)]
pub(super) struct Components {
    /// By nonce.
    pub pending: HashMap<u64, PendingInteraction>,
    /// Shown under a message's components until it changes or another
    /// interaction on it starts.
    pub failed: HashMap<Id<MessageMarker>, String>,
    /// What a select shows as picked, until its message changes: Discord's
    /// client keeps a pick on screen whether or not the app re-renders.
    pub picks: HashMap<(Id<MessageMarker>, String), Vec<Choice>>,
    pub open_select: Option<OpenSelect>,
    pub modal: Option<OpenModal>,
}

pub(super) struct PendingInteraction {
    pub message_id: Id<MessageMarker>,
    pub custom_id: String,
    /// Discord's client gives up on an app that never answers.
    pub _timeout: Task<()>,
}

pub(super) struct OpenSelect {
    pub message_id: Id<MessageMarker>,
    pub custom_id: String,
    pub picker: Entity<SelectPicker>,
    pub _picked: Subscription,
}

pub(super) struct OpenModal {
    pub dialog: WeakEntity<ModalDialog>,
    /// The submission's, once sent.
    pub nonce: Option<u64>,
}

/// One option in a select menu, its own or one of the guild's users, roles or
/// channels.
#[derive(Clone, PartialEq)]
pub(super) struct Choice {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    pub icon: ChoiceIcon,
}

#[derive(Clone, PartialEq)]
pub(super) enum ChoiceIcon {
    None,
    Emoji(ReactionEmoji),
    Avatar(Option<String>),
    Role(Option<u32>),
    Channel(ChannelKind),
}
