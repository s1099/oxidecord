//! The Discord client: the app's data model, the REST calls that populate it, and the
//! gateway connection that keeps it live.
//!
//! Every request runs on the shared background Tokio runtime ([`crate::platform::runtime`])
//! and is awaited from gpui's executor, so nothing blocks the foreground thread.

mod gateway;
mod model;
mod rest;
mod token;

pub use gateway::{
    GatewayEvent, GatewaySender, IncomingMessage, ReactionChange, connect_gateway, system_locale,
};
pub use model::{
    ActivityApp, ActivityInstance, AppCommand, AuthorType, Block, Channel, ChannelKind,
    ChannelLatest, CommandChoice, CommandContext, CommandIndex, CommandOption, CommandSource,
    CommandUse, CurrentUser, DEFAULT_UPLOAD_LIMIT, Delivery, DirectMessage, DmMember, Embed,
    EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia, EmbedVideo, FileAttachment,
    ForwardedMessage, FrecencyItem, Guild, GuildAppearance, GuildEmoji, GuildFolders, GuildMember,
    GuildUnreads, ImageAttachment, Inline, List, ListMember, MAX_UPLOAD_TOTAL, Markdown,
    MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp, MemberListUpdate, Membership,
    Mention, MentionedUser, Message, MessageLink, MessageReference, MuteSettings, NotifyLevel,
    OptionKind, PresenceStatus, Reaction, ReactionEmoji, Reactor, ReadState, Reply, Role,
    SEARCH_MAX_OFFSET, SEARCH_PAGE_SIZE, SPOILER_PREFIX, SearchFilters, SearchHas, SearchHit,
    SearchResponse, SearchScope, SearchSort, SlashCommand, SpoilerCover, StreamServerInfo,
    SystemEvent, Thread, UnreadChannel, UnreadSetting, Upload, UploadProgress, UserProfile,
    VideoAttachment, VoiceServerInfo, VoiceUserState, WELCOMES, animated_webp, call_duration,
    scaled_url, stream_key,
};
pub use rest::{
    ActivityLaunch, CommandArgument, CommandIndexScope, CommandInvocation, MESSAGE_PAGE_SIZE,
    MessagePage, OAuthRequest, REACTORS_PAGE_SIZE, ack_message, authorize, delete_message,
    edit_message, fetch_activity_shelf, fetch_channels, fetch_command_index, fetch_current_user,
    fetch_dms, fetch_entry_point, fetch_frecency, fetch_guilds, fetch_install_type, fetch_messages,
    fetch_oauth_identity, fetch_pins, fetch_proxy_ticket, fetch_reactors, fetch_user_object,
    fetch_user_profile, fetch_user_settings, join_activity, leave_activity, log_out,
    preview_authorization, remove_account, request_autocomplete, ring_call, run_command,
    save_frecency, save_status, search_messages, send_message, start_activity, stop_ringing,
    toggle_reaction, trigger_typing,
};
pub use rest::{
    ThreadSearch, fetch_thread, search_threads, set_thread_archived, set_thread_joined,
    start_thread, start_thread_from_message,
};
pub use token::{
    Account, accounts, active_account, load_token, remember_profile, save_token, switch_account,
};
pub use twilight_model::guild::Permissions;
