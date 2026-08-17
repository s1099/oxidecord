//! The app's view of Discord's data, plus the conversions from twilight's API
//! models that build it.
//!
//! Everything the UI renders goes through these types, so the twilight models
//! stay confined to this module and [`super::rest`]/[`super::gateway`].

mod cdn;
mod channel;
mod guild;
mod message;
mod time;
mod user;

pub use channel::{Channel, ChannelKind, DirectMessage};
pub use guild::Guild;
pub use message::{ImageAttachment, Message, MessageReference, Reaction, ReactionEmoji};
pub use user::{CurrentUser, UserProfile};

pub(super) use channel::{convert_channel, convert_dms};
pub(super) use guild::convert_guild;
pub(super) use message::convert_message;
pub(super) use user::{RawProfile, convert_current_user, convert_user_profile};
