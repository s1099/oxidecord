//! The member list beside a guild conversation: which channel's list the
//! gateway is subscribed to, and Discord's edits to it as they arrive.

use std::ops::Range;

use gpui_kit::*;

use crate::discord::{self, ChannelKind, MemberListItem, MemberListOp};
use crate::platform::prefs;
use crate::screens::home::member_list_cache::{self, CachedMemberList};
use crate::screens::home::state::MemberList;
use crate::screens::home::{HomeScreen, View};

/// How many rows each subscribed range covers, as in Discord's client.
const RANGE_SIZE: usize = 100;

/// The most ranges asked for at once: the first, always, so the top of the
/// list stays live, and the two the view can straddle.
const MAX_RANGES: usize = 3;

impl HomeScreen {
    pub(in crate::screens::home) fn has_member_list(&self) -> bool {
        self.view == View::Guild
            && self.selected_channel_info().is_some_and(|channel| {
                matches!(channel.kind, ChannelKind::Text | ChannelKind::Announcement)
            })
    }

    /// Called every render so every path that changes the selection is caught; does
    /// nothing unless the channel changed.
    pub(in crate::screens::home) fn sync_member_list(&mut self) {
        let wanted = self
            .has_member_list()
            .then(|| self.selected_guild.zip(self.selected_channel))
            .flatten();
        let Some((guild_id, channel_id)) = wanted else {
            self.stash_member_list();
            return;
        };
        let known = self.member_list_cache.list_id(channel_id);
        match &mut self.member_list {
            Some(list) if list.guild_id == guild_id && list.channel_id == channel_id => return,
            // Another channel in the same guild with the list that's shown,
            // or one whose list hasn't been kept: what's shown stays up until
            // Discord says which list this channel has.
            Some(list)
                if list.guild_id == guild_id
                    && (known.is_none_or(|known| list.id.as_deref() == Some(known))
                        || !self.member_list_cache.has(guild_id, channel_id)) =>
            {
                list.channel_id = channel_id;
                list.ranges = vec![[0, RANGE_SIZE - 1]];
                list.scroll = UniformListScrollHandle::new();
            }
            _ => {
                self.stash_member_list();
                // The list this channel had last time, drawn until the sync
                // the subscription brings replaces it.
                let cached = self.member_list_cache.take(guild_id, channel_id);
                self.member_list = Some(match cached {
                    Some(cached) => MemberList {
                        guild_id,
                        channel_id,
                        id: Some(cached.id),
                        groups: cached.groups,
                        rows: cached.rows,
                        ranges: vec![[0, RANGE_SIZE - 1]],
                        scroll: UniformListScrollHandle::new(),
                    },
                    None => MemberList {
                        guild_id,
                        channel_id,
                        id: None,
                        groups: Vec::new(),
                        rows: Vec::new(),
                        ranges: vec![[0, RANGE_SIZE - 1]],
                        scroll: UniformListScrollHandle::new(),
                    },
                });
            }
        }
        self.resubscribe_member_list();
    }

    fn stash_member_list(&mut self) {
        if let Some(list) = self.member_list.take()
            && let Some(id) = list.id
        {
            self.member_list_cache.store(
                list.guild_id,
                CachedMemberList {
                    id,
                    groups: list.groups,
                    rows: list.rows,
                },
            );
        }
    }

    /// Sends the subscription again after a new session, which starts with none.
    pub(in crate::screens::home) fn resubscribe_member_list(&self) {
        if let (Some(list), Some(gateway)) = (&self.member_list, &self.gateway) {
            gateway.subscribe_member_list(list.guild_id, list.channel_id, &list.ranges);
        }
    }

    /// Follows scroll so rows coming into view get synced.
    pub(in crate::screens::home) fn track_member_list_range(&mut self, visible: Range<usize>) {
        let Some(list) = &mut self.member_list else {
            return;
        };
        let first = visible.start / RANGE_SIZE;
        let last = visible.end.saturating_sub(1) / RANGE_SIZE;
        let ranges: Vec<[usize; 2]> = std::iter::once(0)
            .chain((first..=last).filter(|&chunk| chunk > 0))
            .take(MAX_RANGES)
            .map(|chunk| [chunk * RANGE_SIZE, (chunk + 1) * RANGE_SIZE - 1])
            .collect();
        if ranges != list.ranges {
            list.ranges = ranges;
            self.resubscribe_member_list();
        }
    }

    pub(in crate::screens::home) fn apply_member_list_update(
        &mut self,
        update: discord::MemberListUpdate,
        cx: &mut Context<Self>,
    ) {
        // Whatever list they're from, the members' roles and statuses are
        // current, and colour their names and avatars elsewhere too.
        let roles = self.member_roles.entry(update.guild_id).or_default();
        for op in &update.ops {
            let items = match op {
                MemberListOp::Sync { items, .. } => items.as_slice(),
                MemberListOp::Insert { item, .. } | MemberListOp::Update { item, .. } => {
                    std::slice::from_ref(item)
                }
                MemberListOp::Delete { .. } | MemberListOp::Invalidate { .. } => &[],
            };
            for item in items {
                if let MemberListItem::Member(member) = item {
                    roles.insert(member.user_id, member.roles.clone());
                    if Some(member.user_id) != self.self_user_id {
                        self.user_presences.insert(member.user_id, member.status);
                    }
                }
            }
        }

        let Some(list) = self
            .member_list
            .as_mut()
            .filter(|list| list.guild_id == update.guild_id)
        else {
            self.member_list_cache.apply(&update);
            return;
        };
        let synced = update
            .ops
            .iter()
            .any(|op| matches!(op, MemberListOp::Sync { .. }));
        if list.id.as_deref() != Some(update.list_id.as_str()) {
            // Edits to a list that isn't the one shown are a channel left
            // behind. A sync of another means the open channel's list has
            // arrived in its place.
            if !synced {
                self.member_list_cache.apply(&update);
                return;
            }
            if let Some(id) = list.id.replace(update.list_id.clone()) {
                self.member_list_cache.store(
                    list.guild_id,
                    CachedMemberList {
                        id,
                        groups: std::mem::take(&mut list.groups),
                        rows: std::mem::take(&mut list.rows),
                    },
                );
            }
        }
        if synced {
            self.member_list_cache
                .learn(list.channel_id, &update.list_id);
        }

        member_list_cache::apply_ops(&mut list.rows, update.ops);
        list.groups = update.groups;
        list.rows
            .resize(member_list_cache::list_len(&list.groups), None);
        cx.notify();
    }

    pub(in crate::screens::home) fn toggle_member_list(&mut self, cx: &mut Context<Self>) {
        self.member_list_hidden = !self.member_list_hidden;
        let hidden = self.member_list_hidden;
        prefs::update(|prefs| prefs.member_list_hidden = hidden);
        cx.notify();
    }
}
