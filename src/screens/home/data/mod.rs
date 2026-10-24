//! State management for [`HomeScreen`](super::HomeScreen): everything that
//! talks to [`crate::discord`] and mutates the screen in response.
//!
//! [`crate::discord`]'s callbacks fire on a background Tokio thread while
//! gpui's entity handles are `!Send`, so each loader bridges the two over a
//! `Send`-safe channel that a gpui foreground task drains.

pub(super) mod attachments;
pub(super) mod autocomplete;
pub(super) mod dms;
pub(super) mod emoji;
pub(super) mod guilds;
pub(super) mod image_viewer;
pub(super) mod member_list;
pub(super) mod members;
pub(super) mod messages;
pub(super) mod pins;
pub(super) mod presence;
pub(super) mod profile;
pub(super) mod read_state;
pub(super) mod selection;
pub(super) mod stream;
pub(super) mod video;
pub(super) mod voice;
pub(super) mod voice_chat;
pub(super) mod watch;
