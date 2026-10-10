//! The main app screen: the server rail, the channel/DM sidebar, and the
//! message pane.
//!
//! [`HomeScreen`] owns all of the screen's state ([`state`]); the rest is split
//! across two groups of modules that both extend it with inherent methods:
//!
//! - [`data`] — everything that talks to [`crate::discord`] and mutates state.
//! - [`view`] — everything that renders that state.

mod channels;
mod data;
mod emoji;
mod folders;
mod state;
mod view;
mod voice;

use gpui::{EventEmitter, actions};

pub use state::HomeScreen;

/// Emitted when Discord rejects the stored token, so the app can send the user
/// back to login instead of leaving them on a screen that can't load.
pub struct SessionExpired;

impl EventEmitter<SessionExpired> for HomeScreen {}

/// Key context of the inline edit box, which rebinds enter to save.
pub const EDIT_CONTEXT: &str = "MessageEdit";

/// Key context of the message composer, which rebinds enter to send and binds
/// the up arrow to editing the last message.
pub const COMPOSER_CONTEXT: &str = "MessageComposer";

use state::{EditingMessage, EmojiPicker, EmojiTarget, ProfilePopup, ReplyTarget, View};

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
        EditLastMessage
    ]
);
