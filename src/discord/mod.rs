//! The Discord client: the app's data model, the REST calls that populate it, and the
//! gateway connection that keeps it live.
//!
//! Every request runs on the shared background Tokio runtime ([`crate::platform::runtime`])
//! and is awaited from gpui's executor, so nothing blocks the foreground thread.

mod gateway;
mod model;
mod rest;
mod token;

pub use gateway::{GatewayEvent, GatewaySender, IncomingMessage, ReactionChange, connect_gateway};
pub use model::{
    AuthorType, Block, Channel, ChannelKind, ChannelLatest, CurrentUser, DEFAULT_UPLOAD_LIMIT,
    Delivery, DirectMessage, Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia,
    EmbedVideo, FileAttachment, ForwardedMessage, FrecencyItem, Guild, GuildAppearance, GuildEmoji,
    GuildFolders, GuildMember, GuildUnreads, ImageAttachment, Inline, List, ListMember,
    MAX_UPLOAD_TOTAL, Markdown, MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp,
    MemberListUpdate, Mention, MentionedUser, Message, MessageLink, MessageReference, MuteSettings,
    PresenceStatus, Reaction, ReactionEmoji, Reactor, ReadState, Reply, Role, SEARCH_MAX_OFFSET,
    SEARCH_PAGE_SIZE, SPOILER_PREFIX, SearchFilters, SearchHas, SearchHit, SearchResponse,
    SearchScope, SearchSort, SpoilerCover, StreamServerInfo, SystemEvent, UnreadChannel,
    UnreadSetting, Upload, UploadProgress, UserProfile, VideoAttachment, VoiceServerInfo,
    VoiceUserState, WELCOMES, animated_webp, call_duration, scaled_url, stream_key,
};
pub use rest::{
    MESSAGE_PAGE_SIZE, MessagePage, REACTORS_PAGE_SIZE, ack_message, delete_message, edit_message,
    fetch_channels, fetch_current_user, fetch_dms, fetch_frecency, fetch_guilds, fetch_messages,
    fetch_pins, fetch_reactors, fetch_user_profile, fetch_user_settings, log_out, remove_account,
    ring_call, save_frecency, save_status, search_messages, send_message, stop_ringing,
    toggle_reaction, trigger_typing,
};
pub use token::{
    Account, accounts, active_account, load_token, remember_profile, save_token, switch_account,
};
pub use twilight_model::guild::Permissions;
