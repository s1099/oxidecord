//! Guilds as the server rail shows them.

use serde::Deserialize;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker},
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

pub(in crate::discord) fn convert_role(role: RawRole) -> Role {
    Role {
        id: role.id,
        name: role.name,
        color: (role.color != 0).then_some(role.color),
    }
}
