//! A guild's custom emoji, as the emoji picker offers them.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{EmojiMarker, RoleMarker},
};

use super::cdn;

#[derive(Clone)]
pub struct GuildEmoji {
    pub id: Id<EmojiMarker>,
    pub name: String,
    pub animated: bool,
    /// False once the guild loses the boost level that paid for the slot; the
    /// emoji stays listed but nobody can send it.
    pub available: bool,
    /// The roles allowed to use it. Empty means everyone.
    pub roles: Vec<Id<RoleMarker>>,
}

impl GuildEmoji {
    pub fn url(&self) -> String {
        cdn::emoji_url(self.id, self.animated)
    }

    /// The form Discord renders as this emoji when it's sent in a message.
    pub fn markdown(&self) -> String {
        let prefix = if self.animated { "a" } else { "" };
        format!("<{prefix}:{}:{}>", self.name, self.id)
    }
}

/// An emoji as `READY`, `GUILD_CREATE` and `GUILD_EMOJIS_UPDATE` send it.
#[derive(Deserialize)]
pub(in crate::discord) struct RawEmoji {
    id: Id<EmojiMarker>,
    name: String,
    #[serde(default)]
    animated: bool,
    #[serde(default = "available_default")]
    available: bool,
    #[serde(default)]
    roles: Vec<Id<RoleMarker>>,
}

fn available_default() -> bool {
    true
}

pub(in crate::discord) fn convert_guild_emoji(emoji: RawEmoji) -> GuildEmoji {
    GuildEmoji {
        id: emoji.id,
        name: emoji.name,
        animated: emoji.animated,
        available: emoji.available,
        roles: emoji.roles,
    }
}
