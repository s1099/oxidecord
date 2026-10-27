//! The main app screen: the server rail, the channel/DM sidebar, and the
//! message pane.
//!
//! [`HomeScreen`] owns all state ([`state`]); [`data`] (talks to
//! [`crate::discord`], mutates state) and [`view`] (renders) extend it with
//! inherent methods.

mod autocomplete;
mod channels;
mod data;
mod emoji;
mod folders;
mod history_cache;
mod member_list_cache;
mod selection;
mod state;
mod unread;
mod view;
mod voice;

use gpui_kit::{EventEmitter, actions};

pub use state::HomeScreen;

/// Emitted when the user logs out or Discord rejects the stored token.
pub struct SessionEnded;

impl EventEmitter<SessionEnded> for HomeScreen {}

pub const HOME_CONTEXT: &str = "HomeScreen";

pub const EDIT_CONTEXT: &str = "MessageEdit";

pub const COMPOSER_CONTEXT: &str = "MessageComposer";

use state::{
    Autocomplete, EditingMessage, EmojiPicker, EmojiTarget, ImageViewer, PinsPopup, ProfilePopup,
    ReplyTarget, View,
};

actions!(
    oxidecord,
    [
        /// Paste a clipboard image as an attachment; runs ahead of the input's text-only paste.
        PasteAttachment,
        SendMessage,
        SaveEdit,
        /// Bound to the up arrow in an empty composer, like Discord.
        EditLastMessage,
        /// Falls through to cursor movement when there are no suggestions.
        AutocompletePrevious,
        AutocompleteNext,
        /// Bound to tab; enter does it too while suggestions are showing.
        AutocompleteAccept,
        /// Bound to escape, as in Discord; in the composer the input's own escape
        /// does it once nothing else needs closing.
        MarkConversationRead,
        /// Copies the message-list selection, handing on to the input's copy when empty.
        CopySelection
    ]
);
