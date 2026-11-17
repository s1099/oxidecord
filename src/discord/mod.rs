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
#[cfg(test)]
pub use model::StickerFormat;
pub use model::{
    Activity, ActivityEmoji, ActivityKind, GameInfo, Presence, ProfileConnection, connection_label,
    widget_tag_label,
};
pub use model::{
    ActivityApp, ActivityInstance, AppCommand, AuthorType, Block, CHANNEL_FAVORITED, Channel,
    ChannelKind, ChannelLatest, CommandChoice, CommandContext, CommandIndex, CommandOption,
    CommandSource, CommandUse, CurrentUser, DEFAULT_UPLOAD_LIMIT, Delivery, DirectMessage,
    DmMember, Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia, EmbedVideo,
    FileAttachment, ForwardedMessage, FrecencyItem, Guild, GuildAppearance, GuildEmoji,
    GuildFolders, GuildMember, GuildUnreads, ImageAttachment, Inline, List, ListMember,
    MAX_UPLOAD_TOTAL, Markdown, MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp,
    MemberListUpdate, Membership, Mention, MentionedUser, Message, MessageLink, MessageReference,
    MuteSettings, NotifyLevel, OptionKind, PresenceStatus, ProfileBadge, Reaction, ReactionEmoji,
    Reactor, ReadState, Reply, Role, SEARCH_MAX_OFFSET, SEARCH_PAGE_SIZE, SPOILER_PREFIX,
    SearchFilters, SearchHas, SearchHit, SearchResponse, SearchScope, SearchSort, SlashCommand,
    SpoilerCover, Sticker, StickerItem, StickerPack, StreamServerInfo, SystemEvent, Thread,
    UnreadChannel, UnreadSetting, Upload, UploadProgress, UserProfile, VideoAttachment,
    VoiceServerInfo, VoiceUserState, WELCOMES, animated_webp, call_duration, scaled_url,
    stream_key,
};
pub use model::{
    Button, ButtonAction, ButtonStyle, Component, InputKind, Media, MediaItem, Modal, ModalField,
    Select, SelectKind, SelectOption, SelectValue,
};
pub use model::{Poll, PollDraft};
pub use rest::{
    ActivityLaunch, CommandArgument, CommandIndexScope, CommandInvocation, MESSAGE_PAGE_SIZE,
    MessageDraft, MessagePage, OAuthRequest, REACTORS_PAGE_SIZE, ack_bulk, ack_message, authorize,
    delete_message, edit_message, fetch_activity_shelf, fetch_channels, fetch_command_index,
    fetch_current_user, fetch_dms, fetch_entry_point, fetch_frecency, fetch_games, fetch_guilds,
    fetch_install_type, fetch_messages, fetch_oauth_identity, fetch_pins, fetch_proxy_ticket,
    fetch_reactors, fetch_sticker_packs, fetch_user_note, fetch_user_object, fetch_user_profile,
    fetch_user_settings, join_activity, leave_activity, leave_guild, log_out, open_dm_with,
    preview_authorization, remove_account, request_autocomplete, ring_call, run_command,
    save_dm_override_flags, save_frecency, save_status, save_user_note, search_messages,
    send_message, start_activity, stop_ringing, toggle_reaction, trigger_typing,
};
pub use rest::{ChannelHome, fetch_channel_home};
pub use rest::{
    ComponentInteraction, ModalAnswer, ModalEntry, ModalSubmission, interact_component,
    submit_modal,
};
pub use rest::{
    ThreadSearch, fetch_thread, search_threads, set_thread_archived, set_thread_joined,
    start_thread, start_thread_from_message,
};
pub use rest::{end_poll, vote_poll};
pub use token::{
    Account, accounts, active_account, load_token, remember_profile, save_token, switch_account,
};
pub use twilight_model::guild::Permissions;
