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
mod markdown;
mod message;
mod settings;
mod time;
mod user;
mod voice;

pub use channel::{Channel, ChannelKind, DirectMessage};
pub use embed::{Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedLayout, EmbedMedia};
pub use emoji::GuildEmoji;
pub use guild::{Guild, Role};
pub use markdown::{Block, Inline, List, Markdown, Mention};
pub use message::{
    ImageAttachment, MentionedUser, Message, MessageReference, Reaction, ReactionEmoji,
    SPOILER_PREFIX, SpoilerCover, VideoAttachment,
};
pub use settings::GuildFolders;
pub use user::{CurrentUser, UserProfile};
pub use voice::{VoiceServerInfo, VoiceUserState};

pub(super) use channel::{convert_channel, convert_dms};
pub(super) use emoji::{RawEmoji, convert_guild_emoji};
pub(super) use guild::{RawMember, RawRole, convert_guild, convert_role};
pub(super) use message::{convert_message, convert_reaction_emoji};
pub(super) use settings::{RawSettingsProto, parse_guild_folders};
pub(super) use user::{RawProfile, convert_current_user, convert_user_profile};
pub(super) use voice::{
    RawVoiceServer, RawVoiceState, convert_guild_voice_states, convert_voice_server,
    convert_voice_state,
};
