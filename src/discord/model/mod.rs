//! The app's view of Discord's data, plus the conversions from twilight's API
//! models that build it.
//!
//! Everything the UI renders goes through these types, so the twilight models
//! stay confined to this module and [`super::rest`]/[`super::gateway`].

mod cdn;
mod channel;
mod embed;
mod emoji;
mod guild;
mod link;
mod markdown;
mod member_list;
mod message;
mod read_state;
mod settings;
mod system;
mod time;
mod upload;
mod user;
mod voice;

pub use cdn::{animated_webp, scaled_url};
pub use channel::{Channel, ChannelKind, DirectMessage};
pub use embed::{Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia};
pub use emoji::GuildEmoji;
pub use guild::{Guild, GuildMember, Role};
pub use link::MessageLink;
pub use markdown::{Block, Inline, List, Markdown, Mention};
pub use member_list::{
    ListMember, MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp, MemberListUpdate,
};
pub use message::{
    Delivery, FileAttachment, ForwardedMessage, ImageAttachment, MentionedUser, Message,
    MessageReference, Reaction, ReactionEmoji, SPOILER_PREFIX, SpoilerCover, VideoAttachment,
};
pub use read_state::{
    ChannelLatest, GuildUnreads, MuteSettings, ReadState, UnreadChannel, UnreadSetting,
};
pub use settings::{GuildFolders, PresenceStatus, UserSettings};
pub use system::{SystemEvent, WELCOMES, call_duration};
pub use upload::{DEFAULT_UPLOAD_LIMIT, MAX_UPLOAD_TOTAL};
pub use user::{CurrentUser, Reactor, UserProfile};
pub use voice::{StreamServerInfo, VoiceServerInfo, VoiceUserState, stream_key};

pub(super) use channel::{convert_channel, convert_dms};
pub(super) use emoji::{RawEmoji, convert_guild_emoji};
pub(super) use guild::{RawMember, RawRole, convert_guild, convert_role};
pub(super) use member_list::{RawMemberListUpdate, convert_member_list_update};
pub(super) use message::{RawPins, author_member, convert_message, convert_reaction_emoji};
pub(super) use read_state::{
    RawChannelUnread, RawGuildChannel, RawGuildSettings, RawReadState, convert_channel_unreads,
    convert_guild_channels, convert_mute_settings, convert_read_states, versioned_list,
};
pub(super) use settings::{RawSettingsProto, encode_status_settings, parse_user_settings};
pub(super) use upload::boost_upload_limit;
pub(super) use user::{RawProfile, convert_current_user, convert_reactor, convert_user_profile};
pub(super) use voice::{
    RawVoiceServer, RawVoiceState, convert_call_voice_states, convert_guild_voice_states,
    convert_stream_server, convert_voice_server, convert_voice_state,
};
