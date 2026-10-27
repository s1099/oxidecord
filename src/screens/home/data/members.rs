//! Guild members' roles and names: the signed-in user's roles (what pings them,
//! which emoji they can use), and everyone else's (name colours and nicknames).

use std::cmp::Reverse;
use std::iter;

use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(in crate::screens::home) fn self_roles(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> Option<&[Id<RoleMarker>]> {
        let self_id = self.self_user_id?;
        Some(self.member_roles.get(&guild_id)?.get(&self_id)?)
    }

    pub(in crate::screens::home) fn mentions_self_role(
        &self,
        message: &discord::Message,
        guild_id: Id<GuildMarker>,
    ) -> bool {
        self.self_roles(guild_id).is_some_and(|roles| {
            message
                .mention_roles
                .iter()
                .any(|role| roles.contains(role))
        })
    }

    /// Packed `0xRRGGBB` of the highest role with a colour, ties to the older role
    /// as in Discord. `None` in a DM or while roles are unknown.
    pub(in crate::screens::home) fn role_color(&self, user_id: Id<UserMarker>) -> Option<u32> {
        let guild_id = self.selected_guild.filter(|_| self.view == View::Guild)?;
        let roles = self.guild_roles.get(&guild_id)?;
        self.member_roles
            .get(&guild_id)?
            .get(&user_id)?
            .iter()
            .filter_map(|id| roles.get(id))
            .filter(|role| role.color.is_some())
            .max_by_key(|role| (role.position, Reverse(role.id)))?
            .color
    }

    /// `None` in a DM. Messages are drawn by this rather than `author_name`, which is
    /// the nickname only for messages that arrived live: fetched history carries no
    /// members, so an author would flip between names as cached and fetched messages
    /// merge.
    pub(in crate::screens::home) fn member_name(&self, user_id: Id<UserMarker>) -> Option<&str> {
        let guild_id = self.selected_guild.filter(|_| self.view == View::Guild)?;
        let member = self.guild_members.get(&guild_id)?.get(&user_id)?;
        Some(&member.name)
    }

    pub(in crate::screens::home) fn author_name<'a>(
        &'a self,
        user_id: Id<UserMarker>,
        fallback: &'a str,
    ) -> &'a str {
        self.member_name(user_id).unwrap_or(fallback)
    }

    /// Requests members of authors, and of authors replies quote, not yet named.
    /// Fetched history carries none and a user session is sent only a few up front,
    /// so without this most names would lack colours and show global names instead of
    /// nicknames. Called as each page of history lands.
    pub(in crate::screens::home) fn request_author_members(&mut self) {
        let Some(guild_id) = self.selected_guild.filter(|_| self.view == View::Guild) else {
            return;
        };
        let Some(gateway) = &self.gateway else {
            return;
        };
        // Roles alone come from places that don't name the member, like the
        // member list, so it's the name that says whether to ask.
        let known = self.guild_members.get(&guild_id);
        let mut wanted = Vec::new();
        let authors = self.messages.iter().flat_map(|message| {
            iter::once(message.author_id).chain(message.reply.as_ref().map(|reply| reply.author_id))
        });
        for user_id in authors {
            if known.is_some_and(|known| known.contains_key(&user_id)) {
                continue;
            }
            if self.requested_members.insert((guild_id, user_id)) {
                wanted.push(user_id);
            }
        }
        if !wanted.is_empty() {
            gateway.request_members(guild_id, &wanted);
        }
    }
}
