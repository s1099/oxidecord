//! Guilds as the server rail shows them.

use serde::Deserialize;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::channel::DmMember;

#[derive(Clone)]
pub struct Guild {
    pub id: Id<GuildMarker>,
    pub name: String,
    pub icon_url: Option<String>,
    /// Guild-wide (`@everyone` plus roles), before channel overwrites. Sent with
    /// the guild list, so visibility resolves without refetching roles.
    pub permissions: Permissions,
    /// Owners bypass permissions.
    pub owner: bool,
}

pub(in crate::discord) fn convert_guild(guild: twilight_model::user::CurrentUserGuild) -> Guild {
    Guild {
        id: guild.id,
        name: guild.name,
        icon_url: guild
            .icon
            .map(|hash| cdn::guild_icon_url(guild.id, &hash.to_string())),
        permissions: guild.permissions,
        owner: guild.owner,
    }
}

/// What the guild header draws beyond the name. Only the gateway carries the banner.
#[derive(Clone, Default)]
pub struct GuildAppearance {
    pub banner_url: Option<String>,
    pub verified: bool,
}

pub(in crate::discord) fn convert_guild_appearance(
    guild_id: Id<GuildMarker>,
    banner: Option<&str>,
    features: &[String],
) -> GuildAppearance {
    GuildAppearance {
        banner_url: banner.map(|hash| cdn::guild_banner_url(guild_id, hash)),
        verified: features.iter().any(|feature| feature == "VERIFIED"),
    }
}

#[derive(Clone)]
pub struct Role {
    pub id: Id<RoleMarker>,
    pub name: String,
    /// Packed `0xRRGGBB`; `None` for a role without a colour.
    pub color: Option<u32>,
    /// A member's name takes the colour of their highest role that has one.
    pub position: i64,
    /// Anyone may ping it; otherwise only members with `MENTION_EVERYONE`.
    pub mentionable: bool,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawRole {
    pub(super) id: Id<RoleMarker>,
    name: String,
    #[serde(default)]
    color: u32,
    #[serde(default)]
    position: i64,
    #[serde(default)]
    mentionable: bool,
    /// Used to decide which channels count toward a guild's unread mark.
    #[serde(default)]
    pub(super) permissions: Option<Permissions>,
}

/// The user id is nested in `user` in most payloads, but `READY`'s
/// `merged_members` carry a bare `user_id` and no user object.
#[derive(Deserialize)]
pub(in crate::discord) struct RawMember {
    #[serde(default)]
    user: Option<RawMemberUser>,
    #[serde(default)]
    user_id: Option<Id<UserMarker>>,
    #[serde(default)]
    nick: Option<String>,
    #[serde(default)]
    pub(in crate::discord) roles: Vec<Id<RoleMarker>>,
}

#[derive(Deserialize)]
struct RawMemberUser {
    id: Id<UserMarker>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    discriminator: Option<String>,
    #[serde(default)]
    bot: bool,
}

impl RawMember {
    pub(in crate::discord) fn user_id(&self) -> Option<Id<UserMarker>> {
        self.user.as_ref().map(|user| user.id).or(self.user_id)
    }

    /// Nickname, else global name, else username. Cleared names arrive as
    /// empty strings as well as nulls.
    pub(in crate::discord) fn display_name(&self) -> Option<String> {
        let user = self.user.as_ref();
        [
            self.nick.as_ref(),
            user.and_then(|user| user.global_name.as_ref()),
            user.and_then(|user| user.username.as_ref()),
        ]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .cloned()
    }

    pub(in crate::discord) fn avatar_url(&self) -> Option<String> {
        let user = self.user.as_ref()?;
        Some(cdn::small_user_avatar(
            user.id.get(),
            user.avatar.as_deref(),
            super::user::parse_discriminator(user.discriminator.as_deref()),
        ))
    }

    /// Group DM recipient events carry the same `user` and `nick` as a member.
    pub(in crate::discord) fn dm_member(&self) -> Option<DmMember> {
        Some(DmMember {
            user_id: self.user_id()?,
            name: self.display_name()?,
            avatar_url: self.avatar_url(),
            bot: self.is_bot(),
        })
    }

    pub(in crate::discord) fn is_bot(&self) -> bool {
        self.user.as_ref().is_some_and(|user| user.bot)
    }

    pub(in crate::discord) fn guild_member(&self) -> Option<GuildMember> {
        let user = self.user.as_ref()?;
        Some(GuildMember {
            user_id: user.id,
            name: self.display_name()?,
            username: user.username.clone().filter(|name| !name.is_empty()),
            avatar_url: self.avatar_url(),
        })
    }
}

/// A member as a member chunk names them, for mention suggestions.
#[derive(Clone)]
pub struct GuildMember {
    pub user_id: Id<UserMarker>,
    pub name: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
}

pub(in crate::discord) fn convert_role(role: RawRole) -> Role {
    Role {
        id: role.id,
        name: role.name,
        color: (role.color != 0).then_some(role.color),
        position: role.position,
        mentionable: role.mentionable,
    }
}
