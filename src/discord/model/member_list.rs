//! A channel's member list, as Discord streams it to a client subscribed to
//! it: everyone who can see the channel, grouped under hoisted roles and then
//! by whether they're online.
//!
//! The list is never sent whole. The client subscribes to index ranges of it,
//! and Discord answers with a `SYNC` of each range followed by edits —
//! inserts, updates, deletes — at positions in the full list, headers
//! included. None of this is in Discord's own documentation or the userdocs.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use super::guild::RawMember;
use super::settings::PresenceStatus;

pub struct MemberListUpdate {
    pub guild_id: Id<GuildMarker>,
    /// Which list this is: channels that the same people can see share one,
    /// and its id is a hash of their overwrites, or `everyone`.
    pub list_id: String,
    /// Every group in the list as it now stands, empty ones included.
    pub groups: Vec<MemberListGroup>,
    pub ops: Vec<MemberListOp>,
}

/// A section of the list and how many members it holds.
#[derive(Clone, PartialEq, Eq)]
pub struct MemberListGroup {
    pub kind: MemberGroupKind,
    pub count: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub enum MemberGroupKind {
    /// Online members whose highest hoisted role is this one.
    Role(Id<RoleMarker>),
    /// Online members with no hoisted role.
    Online,
    /// Everyone offline. Left out of large guilds' lists.
    Offline,
}

/// An edit to the list, at an index into it.
#[derive(Clone)]
pub enum MemberListOp {
    /// The items from `start` on, as they are now.
    Sync {
        start: usize,
        items: Vec<MemberListItem>,
    },
    Insert {
        index: usize,
        item: MemberListItem,
    },
    Update {
        index: usize,
        item: MemberListItem,
    },
    Delete {
        index: usize,
    },
    /// The range, inclusive, is no longer kept current — usually because the
    /// client stopped subscribing to it.
    Invalidate {
        start: usize,
        end: usize,
    },
}

#[derive(Clone)]
pub enum MemberListItem {
    /// A header. Discord leaves the count off these, so it reads zero; the
    /// count is in [`MemberListUpdate::groups`].
    Group(MemberListGroup),
    Member(ListMember),
}

#[derive(Clone)]
pub struct ListMember {
    pub user_id: Id<UserMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<Id<RoleMarker>>,
    pub status: PresenceStatus,
    pub bot: bool,
    /// Their custom status, or else what they're doing, as the line under
    /// their name reads.
    pub activity: Option<String>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawMemberListUpdate {
    guild_id: Id<GuildMarker>,
    id: String,
    #[serde(default)]
    groups: Vec<RawGroup>,
    #[serde(default)]
    ops: Vec<RawOp>,
}

#[derive(Deserialize)]
struct RawGroup {
    id: String,
    #[serde(default)]
    count: usize,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "UPPERCASE")]
enum RawOp {
    Sync {
        range: [usize; 2],
        #[serde(default)]
        items: Vec<RawItem>,
    },
    Insert {
        index: usize,
        item: RawItem,
    },
    Update {
        index: usize,
        item: RawItem,
    },
    Delete {
        index: usize,
    },
    Invalidate {
        range: [usize; 2],
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawItem {
    Group(RawGroup),
    Member(RawListMember),
}

#[derive(Deserialize)]
struct RawListMember {
    #[serde(flatten)]
    member: RawMember,
    #[serde(default)]
    presence: Option<RawListPresence>,
}

#[derive(Deserialize)]
struct RawListPresence {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    activities: Vec<RawActivity>,
}

#[derive(Deserialize)]
struct RawActivity {
    #[serde(rename = "type", default)]
    kind: u8,
    #[serde(default)]
    name: Option<String>,
    /// A custom status's text.
    #[serde(default)]
    state: Option<String>,
}

const CUSTOM_STATUS: u8 = 4;

pub(in crate::discord) fn convert_member_list_update(raw: RawMemberListUpdate) -> MemberListUpdate {
    MemberListUpdate {
        guild_id: raw.guild_id,
        list_id: raw.id,
        groups: raw.groups.into_iter().filter_map(convert_group).collect(),
        ops: raw.ops.into_iter().map(convert_op).collect(),
    }
}

fn convert_op(op: RawOp) -> MemberListOp {
    match op {
        RawOp::Sync { range, items } => MemberListOp::Sync {
            start: range[0],
            items: items.into_iter().filter_map(convert_item).collect(),
        },
        RawOp::Insert { index, item } => match convert_item(item) {
            Some(item) => MemberListOp::Insert { index, item },
            // Keeps the indices after it lined up; the row stays a
            // placeholder.
            None => MemberListOp::Invalidate {
                start: index,
                end: index,
            },
        },
        RawOp::Update { index, item } => match convert_item(item) {
            Some(item) => MemberListOp::Update { index, item },
            None => MemberListOp::Invalidate {
                start: index,
                end: index,
            },
        },
        RawOp::Delete { index } => MemberListOp::Delete { index },
        RawOp::Invalidate { range } => MemberListOp::Invalidate {
            start: range[0],
            end: range[1],
        },
    }
}

fn convert_item(item: RawItem) -> Option<MemberListItem> {
    Some(match item {
        RawItem::Group(group) => MemberListItem::Group(convert_group(group)?),
        RawItem::Member(member) => MemberListItem::Member(convert_member(member)?),
    })
}

fn convert_group(group: RawGroup) -> Option<MemberListGroup> {
    let kind = match group.id.as_str() {
        "online" => MemberGroupKind::Online,
        "offline" => MemberGroupKind::Offline,
        id => MemberGroupKind::Role(id.parse().ok()?),
    };
    Some(MemberListGroup {
        kind,
        count: group.count,
    })
}

fn convert_member(raw: RawListMember) -> Option<ListMember> {
    let member = raw.member;
    let user_id = member.user_id()?;
    let (status, activity) = match raw.presence {
        Some(presence) => (
            presence
                .status
                .as_deref()
                .and_then(PresenceStatus::from_presence)
                .unwrap_or(PresenceStatus::Invisible),
            activity_line(presence.activities),
        ),
        None => (PresenceStatus::Invisible, None),
    };
    Some(ListMember {
        user_id,
        name: member.display_name().unwrap_or_default(),
        avatar_url: member.avatar_url(),
        bot: member.is_bot(),
        roles: member.roles,
        status,
        activity,
    })
}

/// The line under a member's name: their custom status when they've set one,
/// as Discord prefers it, else the first thing they're doing.
fn activity_line(activities: Vec<RawActivity>) -> Option<String> {
    if let Some(custom) = activities
        .iter()
        .filter(|activity| activity.kind == CUSTOM_STATUS)
        .find_map(|activity| activity.state.clone().filter(|state| !state.is_empty()))
    {
        return Some(custom);
    }
    activities.into_iter().find_map(|activity| {
        let verb = match activity.kind {
            0 => "Playing",
            1 => "Streaming",
            2 => "Listening to",
            3 => "Watching",
            5 => "Competing in",
            _ => return None,
        };
        Some(format!("{verb} {}", activity.name?))
    })
}
