//! Guilds as the server rail shows them.

use serde::Deserialize;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use super::cdn;

#[derive(Clone)]
pub struct Guild {
    pub id: Id<GuildMarker>,
    pub name: String,
    pub icon_url: Option<String>,
    /// The user's guild-wide permissions (from `@everyone` plus their roles,
    /// before any channel overwrites). Discord hands these to us with the
    /// guild list, so channel visibility can be resolved without refetching
    /// the guild's roles.
    pub permissions: Permissions,
    /// Whether the current user owns this guild (owners bypass permissions).
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

/// A guild role, as far as a role mention needs it.
#[derive(Clone)]
pub struct Role {
    pub id: Id<RoleMarker>,
    pub name: String,
    /// Packed `0xRRGGBB`; `None` for a role without a colour.
    pub color: Option<u32>,
}

/// A role as the gateway sends it, read here rather than through twilight's
/// model since only three fields matter.
#[derive(Deserialize)]
pub(in crate::discord) struct RawRole {
    id: Id<RoleMarker>,
    name: String,
    #[serde(default)]
    color: u32,
}

/// A guild member as the gateway sends it: inside a voice state, in a guild's
/// `members`, or on its own in `GUILD_MEMBER_UPDATE`. Where the user's id is
/// depends on the payload — a nested user in most, a bare `user_id` in
/// `READY`'s `merged_members`, which carry no user object at all.
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
}

impl RawMember {
    pub(in crate::discord) fn user_id(&self) -> Option<Id<UserMarker>> {
        self.user.as_ref().map(|user| user.id).or(self.user_id)
    }

    /// The name the guild shows them by: their nickname there, else their
    /// global display name, else their username. Discord sends cleared names
    /// as empty strings as well as nulls.
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
        let hash = user.avatar.as_ref()?;
        Some(cdn::small_avatar_url(user.id.get(), hash))
    }
}

pub(in crate::discord) fn convert_role(role: RawRole) -> Role {
    Role {
        id: role.id,
        name: role.name,
        color: (role.color != 0).then_some(role.color),
    }
}
