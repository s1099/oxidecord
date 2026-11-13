//! The app's view of Discord's data, plus the conversions from twilight's API
//! models that build it.
//!
//! Everything the UI renders goes through these types, so the twilight models
//! stay confined to this module and [`super::rest`]/[`super::gateway`].

mod activity;
mod cdn;
mod channel;
mod command;
mod component;
mod embed;
mod emoji;
mod frecency;
mod guild;
mod link;
mod markdown;
mod member_list;
mod message;
mod read_state;
mod search;
mod settings;
mod sticker;
mod system;
mod thread;
mod time;
mod upload;
mod user;
mod voice;

pub use activity::{ActivityApp, ActivityInstance, EntryPoint};
pub use cdn::{animated_webp, scaled_url};
pub use channel::{Channel, ChannelKind, DirectMessage, DmMember};
pub use command::{
    AppCommand, CommandChoice, CommandContext, CommandIndex, CommandOption, CommandSource,
    OptionKind, SlashCommand,
};
pub use component::{
    Button, ButtonAction, ButtonStyle, Component, InputKind, Media, MediaItem, Modal, ModalField,
    ModalInput, Select, SelectKind, SelectOption, SelectValue,
};
pub use embed::{Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia, EmbedVideo};
pub use emoji::GuildEmoji;
pub use frecency::FrecencyItem;
pub use guild::{Guild, GuildAppearance, GuildMember, Role};
pub use link::MessageLink;
pub use markdown::{Block, Inline, List, Markdown, Mention};
pub use member_list::{
    ListMember, MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp, MemberListUpdate,
};
pub use message::{
    CommandUse, Delivery, FileAttachment, ForwardedMessage, ImageAttachment, MentionedUser,
    Message, MessageReference, Reaction, ReactionEmoji, Reply, SPOILER_PREFIX, SpoilerCover,
    VideoAttachment,
};
pub use read_state::{
    ChannelLatest, GuildUnreads, MuteSettings, NotifyLevel, ReadState, UnreadChannel, UnreadSetting,
};
pub use search::{
    AuthorType, SEARCH_MAX_OFFSET, SEARCH_PAGE_SIZE, SearchFilters, SearchHas, SearchHit,
    SearchResponse, SearchScope, SearchSort,
};
pub use settings::{GuildFolders, PresenceStatus, UserSettings};
#[cfg(test)]
pub use sticker::StickerFormat;
pub use sticker::{Sticker, StickerItem, StickerPack};
pub use system::{SystemEvent, WELCOMES, call_duration};
pub use thread::{Membership, Thread};
pub use upload::{DEFAULT_UPLOAD_LIMIT, MAX_UPLOAD_TOTAL, Upload, UploadProgress};
pub use user::{CurrentUser, Reactor, UserProfile};
pub use voice::{StreamServerInfo, VoiceServerInfo, VoiceUserState, stream_key};

pub(super) use activity::{
    RawActivityInstance, RawShelf, convert_activity_instance, convert_shelf, entry_point,
};
pub(super) use channel::{convert_channel, convert_dm, convert_dms};
pub(super) use command::{RawChoice, RawCommandIndex, convert_command_index};
pub(super) use component::{RawModal, convert_modal};
pub(super) use emoji::{RawEmoji, convert_guild_emoji};
pub(super) use frecency::{encode_frecency_settings, parse_frecency_settings};
pub(super) use guild::{RawMember, RawRole, convert_guild, convert_guild_appearance, convert_role};
pub(super) use member_list::{RawMemberListUpdate, convert_member_list_update};
pub(super) use message::{RawPins, author_member, convert_message, convert_reaction_emoji};
pub(super) use read_state::{
    RawChannelUnread, RawGuildChannel, RawGuildSettings, RawReadState, convert_channel_unreads,
    convert_guild_channels, convert_mute_settings, convert_read_states, versioned_list,
};
pub(super) use search::{RawSearch, convert_search};
pub(super) use settings::{RawSettingsProto, encode_status_settings, parse_user_settings};
pub(super) use sticker::{RawSticker, RawStickerPacks, convert_sticker, convert_sticker_packs};
pub(super) use thread::convert_thread;
pub(super) use upload::boost_upload_limit;
pub(super) use user::{RawProfile, convert_current_user, convert_reactor, convert_user_profile};
pub(super) use voice::{
    RawVoiceServer, RawVoiceState, convert_call_voice_states, convert_guild_voice_states,
    convert_stream_server, convert_voice_server, convert_voice_state,
};
