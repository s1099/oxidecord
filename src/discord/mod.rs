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

pub use gateway::{GatewayEvent, GatewaySender, IncomingMessage, connect_gateway};
pub use model::{
    Block, Channel, ChannelKind, CurrentUser, DirectMessage, Embed, EmbedAuthor, EmbedField,
    EmbedFooter, EmbedLayout, EmbedMedia, Guild, GuildFolders, ImageAttachment, Inline, List,
    Markdown, Mention, MentionedUser, Message, MessageReference, Reaction, ReactionEmoji, Role,
    UserProfile, VideoAttachment, VoiceServerInfo, VoiceUserState,
};
pub use rest::{
    MAX_ATTACHMENT_SIZE, MESSAGE_PAGE_SIZE, delete_message, edit_message, fetch_channels,
    fetch_current_user, fetch_dms, fetch_guild_folders, fetch_guilds, fetch_messages,
    fetch_user_profile, send_message, toggle_reaction,
};
pub use token::{load_token, save_token};
pub use twilight_model::guild::Permissions;
