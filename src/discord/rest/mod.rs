//! The REST calls the app makes against Discord's HTTP API.
//!
//! Each entry point spawns onto the shared background Tokio runtime and hands
//! its result to `on_done`, which therefore runs on that runtime's thread — not
//! gpui's foreground thread.

mod channel;
mod guild;
mod message;
mod user;

pub use channel::fetch_dms;
pub use guild::{fetch_channels, fetch_guilds};
pub use message::{
    MAX_ATTACHMENT_SIZE, MESSAGE_PAGE_SIZE, fetch_messages, send_message, toggle_reaction,
};
pub use user::{fetch_current_user, fetch_user_profile};
