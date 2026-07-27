//! State management for [`HomeScreen`](super::HomeScreen): everything that
//! talks to [`crate::discord`] and mutates the screen in response.
//!
//! [`crate::discord`]'s callbacks fire on a background Tokio thread while
//! gpui's entity handles are `!Send`, so each loader bridges the two over a
//! `Send`-safe channel that a gpui foreground task drains.

pub(super) mod attachments;
pub(super) mod dms;
pub(super) mod guilds;
pub(super) mod messages;
