//! Member lists recently shown, kept so that opening a channel draws its list
//! at once.
//!
//! Discord's own client does the same: the list a channel had last time is
//! drawn straight away, and the sync the subscription brings replaces it when
//! it lands. Channels the same people can see share one list, and only
//! Discord says which list a channel has, so each channel's is remembered
//! from the syncs that come for it.

use std::collections::{HashMap, VecDeque};

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, MemberListItem, MemberListOp};

/// How many lists are kept. A list is held whole so Discord's edits still
/// line up with it, and a large guild's runs to thousands of rows.
const CACHED_LISTS: usize = 12;

pub(super) struct CachedMemberList {
    pub id: String,
    pub groups: Vec<discord::MemberListGroup>,
    pub rows: Vec<Option<MemberListItem>>,
}

#[derive(Default)]
pub(super) struct MemberListCache {
    /// The list each channel was last synced with.
    channel_lists: HashMap<Id<ChannelMarker>, String>,
    /// Most recently left first. List ids are only unique within a guild:
    /// `everyone` is every guild's.
    lists: VecDeque<(Id<GuildMarker>, CachedMemberList)>,
}

impl MemberListCache {
    pub(super) fn learn(&mut self, channel_id: Id<ChannelMarker>, list_id: &str) {
        if self.channel_lists.get(&channel_id).map(String::as_str) != Some(list_id) {
            self.channel_lists.insert(channel_id, list_id.to_owned());
        }
    }

    /// The list `channel_id` was last synced with, if it's been seen.
    pub(super) fn list_id(&self, channel_id: Id<ChannelMarker>) -> Option<&str> {
        self.channel_lists.get(&channel_id).map(String::as_str)
    }

    /// Keeps a list that's no longer shown.
    pub(super) fn store(&mut self, guild_id: Id<GuildMarker>, list: CachedMemberList) {
        if list.rows.is_empty() {
            return;
        }
        self.lists
            .retain(|(guild, cached)| *guild != guild_id || cached.id != list.id);
        self.lists.push_front((guild_id, list));
        self.lists.truncate(CACHED_LISTS);
    }

    /// Whether the list `channel_id` had last time is kept.
    pub(super) fn has(&self, guild_id: Id<GuildMarker>, channel_id: Id<ChannelMarker>) -> bool {
        self.list_id(channel_id).is_some_and(|list_id| {
            self.lists
                .iter()
                .any(|(guild, list)| *guild == guild_id && list.id == list_id)
        })
    }

    /// Takes the list `channel_id` had last time out of the cache. It goes
    /// back in when it's no longer shown.
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

    /// Discord's edits to a list that isn't shown: the guild's subscription
    /// can outlive a switch to another guild.
    pub(super) fn apply(&mut self, update: &discord::MemberListUpdate) {
        let Some((_, list)) = self
            .lists
            .iter_mut()
            .find(|(guild, list)| *guild == update.guild_id && list.id == update.list_id)
        else {
            return;
        };
        // Discord invalidates the ranges it stops sending when the
        // subscription moves on; what was there is still the best guess
        // until the list is opened and synced again.
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

/// Applies Discord's edits to a list's rows, at Discord's indices.
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

/// How long the groups say the list is: a header for each group with anyone
/// in it, then its members. Rows past what's been synced are placeholders
/// until they are.
pub(super) fn list_len(groups: &[discord::MemberListGroup]) -> usize {
    groups
        .iter()
        .filter(|group| group.count > 0)
        .map(|group| group.count + 1)
        .sum()
}
