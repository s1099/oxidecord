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
mod state;
mod view;

use gpui::actions;

pub use state::HomeScreen;

use state::{ProfilePopup, ReplyTarget, View};

actions!(
    oxidecord,
    [
        /// Paste an image from the clipboard into the message composer as an
        /// attachment. Bound to the paste shortcut so it runs ahead of the text
        /// input's own paste, which only handles text.
        PasteAttachment
    ]
);
