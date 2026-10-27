//! Member lists recently shown, drawn at once on opening a channel until the
//! subscription's sync replaces them. Channels visible to the same people share
//! a list, and only Discord says which, so each channel's list id is learned
//! from its syncs.

use std::collections::{HashMap, VecDeque};

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, MemberListItem, MemberListOp};

/// Lists are held whole so Discord's edits still line up; a large guild's runs
/// to thousands of rows.
const CACHED_LISTS: usize = 12;

pub(super) struct CachedMemberList {
    pub id: String,
    pub groups: Vec<discord::MemberListGroup>,
    pub rows: Vec<Option<MemberListItem>>,
}

#[derive(Default)]
pub(super) struct MemberListCache {
    channel_lists: HashMap<Id<ChannelMarker>, String>,
    /// Most recently left first. List ids are unique only within a guild
    /// (`everyone` is every guild's).
    lists: VecDeque<(Id<GuildMarker>, CachedMemberList)>,
}

impl MemberListCache {
    pub(super) fn learn(&mut self, channel_id: Id<ChannelMarker>, list_id: &str) {
        if self.channel_lists.get(&channel_id).map(String::as_str) != Some(list_id) {
            self.channel_lists.insert(channel_id, list_id.to_owned());
        }
    }

    pub(super) fn list_id(&self, channel_id: Id<ChannelMarker>) -> Option<&str> {
        self.channel_lists.get(&channel_id).map(String::as_str)
    }

    pub(super) fn store(&mut self, guild_id: Id<GuildMarker>, list: CachedMemberList) {
        if list.rows.is_empty() {
            return;
        }
        self.lists
            .retain(|(guild, cached)| *guild != guild_id || cached.id != list.id);
        self.lists.push_front((guild_id, list));
        self.lists.truncate(CACHED_LISTS);
    }

    pub(super) fn has(&self, guild_id: Id<GuildMarker>, channel_id: Id<ChannelMarker>) -> bool {
        self.list_id(channel_id).is_some_and(|list_id| {
            self.lists
                .iter()
                .any(|(guild, list)| *guild == guild_id && list.id == list_id)
        })
    }

    /// Removes the entry; it's stored again when no longer shown.
    pub(super) fn take(
        &mut self,
        guild_id: Id<GuildMarker>,
        channel_id: Id<ChannelMarker>,
    ) -> Option<CachedMemberList> {
        let list_id = self.channel_lists.get(&channel_id)?;
        let ix = self
            .lists
            .iter()
            .position(|(guild, list)| *guild == guild_id && list.id == *list_id)?;
        self.lists.remove(ix).map(|(_, list)| list)
    }

    /// Edits for a list that isn't shown: the guild's subscription can outlive
    /// a switch to another guild.
    pub(super) fn apply(&mut self, update: &discord::MemberListUpdate) {
        let Some((_, list)) = self
            .lists
            .iter_mut()
            .find(|(guild, list)| *guild == update.guild_id && list.id == update.list_id)
        else {
            return;
        };
        // Discord invalidates ranges it stops sending; the old rows are still
        // the best guess until the list is synced again.
        let ops = update
            .ops
            .iter()
            .filter(|op| !matches!(op, MemberListOp::Invalidate { .. }))
            .cloned();
        apply_ops(&mut list.rows, ops);
        list.groups = update.groups.clone();
        list.rows.resize(list_len(&list.groups), None);
    }
}

pub(super) fn apply_ops(
    rows: &mut Vec<Option<MemberListItem>>,
    ops: impl IntoIterator<Item = MemberListOp>,
) {
    for op in ops {
        match op {
            MemberListOp::Sync { start, items } => {
                let end = start + items.len();
                if rows.len() < end {
                    rows.resize(end, None);
                }
                for (row, item) in rows[start..end].iter_mut().zip(items) {
                    *row = Some(item);
                }
            }
            MemberListOp::Insert { index, item } => {
                if rows.len() < index {
                    rows.resize(index, None);
                }
                rows.insert(index, Some(item));
            }
            MemberListOp::Update { index, item } => {
                if rows.len() <= index {
                    rows.resize(index + 1, None);
                }
                rows[index] = Some(item);
            }
            MemberListOp::Delete { index } => {
                if index < rows.len() {
                    rows.remove(index);
                }
            }
            MemberListOp::Invalidate { start, end } => {
                let end = (end + 1).min(rows.len());
                if start < end {
                    rows[start..end].fill(None);
                }
            }
        }
    }
}

/// A header per non-empty group plus its members; unsynced rows are placeholders.
pub(super) fn list_len(groups: &[discord::MemberListGroup]) -> usize {
    groups
        .iter()
        .filter(|group| group.count > 0)
        .map(|group| group.count + 1)
        .sum()
}
