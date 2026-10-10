//! The Discord client: the app's data model, the REST calls that populate it,
//! and the gateway connection that keeps it live.
//!
//! Every request runs on the shared background Tokio runtime
//! ([`crate::platform::runtime`]) and is awaited from gpui's executor, so
//! nothing here blocks the foreground thread.

mod gateway;
mod model;
mod rest;
mod token;

pub use gateway::{GatewayEvent, GatewaySender, IncomingMessage, ReactionChange, connect_gateway};
pub use model::{
    Block, Channel, ChannelKind, CurrentUser, Delivery, DirectMessage, Embed, EmbedAuthor,
    EmbedField, EmbedFooter, EmbedLayout, EmbedMedia, ForwardedMessage, Guild, GuildEmoji,
    GuildFolders, ImageAttachment, Inline, List, Markdown, Mention, MentionedUser, Message,
    MessageReference, PresenceStatus, Reaction, ReactionEmoji, Role, SPOILER_PREFIX, SpoilerCover,
    StreamServerInfo, UserProfile, VideoAttachment, VoiceServerInfo, VoiceUserState, stream_key,
};
pub use rest::{
    MAX_ATTACHMENT_SIZE, MESSAGE_PAGE_SIZE, delete_message, edit_message, fetch_channels,
    fetch_current_user, fetch_dms, fetch_guilds, fetch_messages, fetch_user_profile,
    fetch_user_settings, log_out, ring_call, save_status, send_message, toggle_reaction,
    trigger_typing,
};
pub use token::{load_token, save_token};
pub use twilight_model::guild::Permissions;
