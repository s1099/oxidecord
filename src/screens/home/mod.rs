//! The main app screen: the server rail, the channel/DM sidebar, and the
//! message pane.
//!
//! [`HomeScreen`] owns all state ([`state`]); [`data`] (talks to
//! [`crate::discord`], mutates state) and [`view`] (renders) extend it with
//! inherent methods.

mod activities;
mod autocomplete;
mod channels;
mod commands;
mod composer;
mod data;
mod emoji;
mod folders;
mod history_cache;
mod media;
mod member_list_cache;
mod popups;
mod quick_switcher;
mod search;
mod selection;
mod state;
mod stickers;
mod threads;
mod unread;
mod view;
mod voice;

use gpui_kit::{EventEmitter, actions};
use twilight_model::id::{Id, marker::UserMarker};

pub use state::HomeScreen;

/// Emitted when the user logs out or Discord rejects the stored token.
pub struct SessionEnded;

impl EventEmitter<SessionEnded> for HomeScreen {}

/// Asks the app to swap this screen for another saved account's.
pub struct SwitchAccount(pub Id<UserMarker>);

impl EventEmitter<SwitchAccount> for HomeScreen {}

/// Asks the app to open the Discord login window for another account.
pub struct AddAccount;

impl EventEmitter<AddAccount> for HomeScreen {}

pub const HOME_CONTEXT: &str = "HomeScreen";

pub const EDIT_CONTEXT: &str = "MessageEdit";

pub const COMPOSER_CONTEXT: &str = "MessageComposer";

pub const SEARCH_CONTEXT: &str = "MessageSearch";

use composer::{Autocomplete, EditingMessage, ReplyTarget};
use popups::{
    EmojiPicker, EmojiTarget, ImageViewer, PinsPopup, ProfilePopup, StickerPicker, ThreadsPopup,
};
use state::View;

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
        CopySelection,
        /// Ctrl+K, as in Discord; pressed again it closes.
        ToggleQuickSwitcher,
        /// Enter in the search box: takes the highlighted suggestion, or searches.
        SearchSubmit,
        SearchPrevious,
        SearchNext,
        /// Tab in the search box, taking the highlighted or first suggestion.
        SearchAccept,
        /// Ctrl+F, as in Discord.
        FocusSearch
    ]
);
