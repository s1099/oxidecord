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
    Block, Channel, ChannelKind, ChannelLatest, CurrentUser, DEFAULT_UPLOAD_LIMIT, Delivery,
    DirectMessage, Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia,
    ForwardedMessage, Guild, GuildEmoji, GuildFolders, GuildMember, GuildUnreads, ImageAttachment,
    Inline, List, ListMember, MAX_UPLOAD_TOTAL, Markdown, MemberGroupKind, MemberListGroup,
    MemberListItem, MemberListOp, MemberListUpdate, Mention, MentionedUser, Message,
    MessageReference, MuteSettings, PresenceStatus, Reaction, ReactionEmoji, Reactor, ReadState,
    Role, SPOILER_PREFIX, SpoilerCover, StreamServerInfo, UnreadChannel, UserProfile,
    VideoAttachment, VoiceServerInfo, VoiceUserState, scaled_url, stream_key,
};
pub use rest::{
    MESSAGE_PAGE_SIZE, MessagePage, REACTORS_PAGE_SIZE, ack_message, delete_message, edit_message,
    fetch_channels, fetch_current_user, fetch_dms, fetch_guilds, fetch_messages, fetch_pins,
    fetch_reactors, fetch_user_profile, fetch_user_settings, log_out, ring_call, save_status,
    send_message, stop_ringing, toggle_reaction, trigger_typing,
};
pub use token::{load_token, save_token};
pub use twilight_model::guild::Permissions;
