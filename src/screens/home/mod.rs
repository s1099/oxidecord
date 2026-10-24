//! The main app screen: the server rail, the channel/DM sidebar, and the
//! message pane.
//!
//! [`HomeScreen`] owns all of the screen's state ([`state`]); the rest is split
//! across two groups of modules that both extend it with inherent methods:
//!
//! - [`data`] — everything that talks to [`crate::discord`] and mutates state.
//! - [`view`] — everything that renders that state.

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

/// Emitted when the user logs out or Discord rejects the stored token, so the
/// app can send the user back to login instead of leaving them on a screen
/// that can't load.
pub struct SessionEnded;

impl EventEmitter<SessionEnded> for HomeScreen {}

/// Key context of the whole screen, where escape reads the conversation.
pub const HOME_CONTEXT: &str = "HomeScreen";

/// Key context of the inline edit box, which rebinds enter to save.
pub const EDIT_CONTEXT: &str = "MessageEdit";

/// Key context of the message composer, which rebinds enter to send and binds
/// the up arrow to editing the last message.
pub const COMPOSER_CONTEXT: &str = "MessageComposer";

use state::{
    Autocomplete, EditingMessage, EmojiPicker, EmojiTarget, ImageViewer, PinsPopup, ProfilePopup,
    ReplyTarget, View,
};

actions!(
    oxidecord,
    [
        /// Paste an image from the clipboard into the message composer as an
        /// attachment. Bound to the paste shortcut so it runs ahead of the text
        /// input's own paste, which only handles text.
        PasteAttachment,
        /// Send what's in the composer. Bound to enter there, where the input
        /// would otherwise insert a newline.
        SendMessage,
        /// Save the message being edited. Bound to enter inside the edit box,
        /// where the input would otherwise insert a newline.
        SaveEdit,
        /// Open the user's most recent message in the conversation for
        /// editing. Bound to the up arrow in an empty composer, like Discord.
        EditLastMessage,
        /// Highlight the previous autocomplete suggestion. Bound to the up
        /// arrow in the composer, where it moves the cursor when there are
        /// none.
        AutocompletePrevious,
        /// Highlight the next autocomplete suggestion. Bound to the down
        /// arrow in the composer.
        AutocompleteNext,
        /// Put the highlighted autocomplete suggestion in the composer. Bound
        /// to tab there; enter does it too while suggestions are showing.
        AutocompleteAccept,
        /// Read the open conversation through to its newest message and jump
        /// there. Bound to escape, as in Discord; in the composer the input's
        /// own escape does it, once there's nothing else for escape to close.
        MarkConversationRead,
        /// Copy the text selected in the message list. Bound to the copy
        /// shortcut ahead of the text inputs' own copy, and handed on to it
        /// when nothing there is selected.
        CopySelection
    ]
);
