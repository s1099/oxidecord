//! The state behind the composer: replying, editing, autocomplete, and who
//! else is typing.

use std::ops::Range;
use std::time::Instant;

use gpui_kit::Entity;
use gpui_kit::component::input::TextareaState;
use twilight_model::id::{
    Id,
    marker::{MessageMarker, UserMarker},
};

use super::autocomplete::{Suggestion, Trigger};

#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub message_id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    /// Whether the reply pings its author, as Discord's "@ ON" toggle.
    pub mention: bool,
}

/// The input is made per edit so it starts with no undo history from the last.
pub(super) struct EditingMessage {
    pub message_id: Id<MessageMarker>,
    pub input: Entity<TextareaState>,
}

/// Shown until `until` passes or their message arrives.
pub(super) struct Typist {
    pub user_id: Id<UserMarker>,
    /// From the dispatch; a DM's typists are named from its messages instead.
    pub name: Option<String>,
    pub until: Instant,
}

pub(super) struct Autocomplete {
    pub trigger: Trigger,
    /// Byte range of the word, trigger included; what accepting replaces.
    pub range: Range<usize>,
    pub query: String,
    pub items: Vec<Suggestion>,
    pub selected: usize,
}
