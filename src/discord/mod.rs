//! The Discord client: the app's data model, the REST calls that populate it,
//! and the gateway connection that keeps it live.
//!
//! Every request runs on the shared background Tokio runtime
//! ([`crate::runtime`]) and reports back through a callback, so nothing here
//! blocks gpui's foreground thread. Callbacks are therefore invoked from that
//! runtime's thread, not the foreground one.

mod gateway;
mod model;
mod rest;
mod token;

pub use gateway::{IncomingMessage, connect_gateway};
pub use model::{
    Channel, ChannelKind, CurrentUser, DirectMessage, Guild, ImageAttachment, Message,
    MessageReference,
};
pub use rest::{
    MAX_ATTACHMENT_SIZE, MESSAGE_PAGE_SIZE, fetch_channels, fetch_current_user, fetch_dms,
    fetch_guilds, fetch_messages, send_message,
};
pub use token::{load_token, save_token};
pub use twilight_model::guild::Permissions;
