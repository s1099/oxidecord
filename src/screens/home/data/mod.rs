//! State for [`HomeScreen`](super::HomeScreen): everything that talks to
//! [`crate::discord`] and mutates the screen in response.
//!
//! [`crate::discord`]'s callbacks fire on a Tokio thread while gpui entity
//! handles are `!Send`, so each loader bridges the two over a channel that a
//! foreground task drains.

pub(super) mod attachments;
pub(super) mod autocomplete;
pub(super) mod dms;
pub(super) mod emoji;
pub(super) mod gif;
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
pub(super) mod type_to_compose;
pub(super) mod video;
pub(super) mod voice;
pub(super) mod voice_chat;
pub(super) mod watch;
