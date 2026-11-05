//! Threads: conversations branching off a guild channel, or off one of its
//! messages. Discord sends them as channels; only what the client draws or
//! decides with is kept.

use twilight_model::channel::ChannelType;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker},
};

/// Milliseconds from the Unix epoch to Discord's.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

#[derive(Clone)]
pub struct Thread {
    pub id: Id<ChannelMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    pub parent_id: Option<Id<ChannelMarker>>,
    pub name: String,
    pub owner_id: Option<Id<UserMarker>>,
    pub private: bool,
    pub archived: bool,
    /// Only those with `MANAGE_THREADS` can send or unarchive.
    pub locked: bool,
    /// Excludes the starter message. Threads from before mid-2022 stop at 50.
    pub message_count: u32,
    /// Approximate; Discord stops counting at 50.
    pub member_count: u32,
    pub last_message_id: Option<Id<MessageMarker>>,
    pub membership: Membership,
}

/// Whether the signed-in user is in the thread. Most updates don't say, which
/// leaves what was known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Membership {
    Unknown,
    /// Unix seconds.
    Joined(i64),
    NotJoined,
}

impl Membership {
    pub fn is_joined(self) -> bool {
        matches!(self, Self::Joined(_))
    }
}

impl Thread {
    /// Unix seconds. A thread's id is minted when it's created.
    pub fn created_at(&self) -> i64 {
        snowflake_secs(self.id.get())
    }

    /// Unix seconds of the newest message, or of creation when it has none.
    pub fn last_active(&self) -> i64 {
        self.last_message_id
            .map_or_else(|| self.created_at(), |id| snowflake_secs(id.get()))
    }

    /// Keeps what `newer` doesn't say: most updates leave out membership, and
    /// `THREAD_UPDATE` isn't sent when only the newest message changes.
    pub fn merge(&mut self, newer: Thread) {
        let membership = match newer.membership {
            Membership::Unknown => self.membership,
            known => known,
        };
        let last_message_id = self.last_message_id.max(newer.last_message_id);
        *self = Thread {
            membership,
            last_message_id,
            ..newer
        };
    }
}

fn snowflake_secs(id: u64) -> i64 {
    (((id >> 22) + DISCORD_EPOCH_MS) / 1000) as i64
}

/// `None` for any channel that isn't a thread.
pub(in crate::discord) fn convert_thread(
    channel: twilight_model::channel::Channel,
) -> Option<Thread> {
    let private = match channel.kind {
        ChannelType::PublicThread | ChannelType::AnnouncementThread => false,
        ChannelType::PrivateThread => true,
        _ => return None,
    };
    let metadata = channel.thread_metadata.as_ref();
    Some(Thread {
        id: channel.id,
        guild_id: channel.guild_id,
        parent_id: channel.parent_id,
        name: channel.name.unwrap_or_default(),
        owner_id: channel.owner_id,
        private,
        archived: metadata.is_some_and(|metadata| metadata.archived),
        locked: metadata.is_some_and(|metadata| metadata.locked),
        message_count: channel.message_count.unwrap_or(0),
        member_count: channel.member_count.map_or(0, |count| count.max(0) as u32),
        last_message_id: channel.last_message_id.map(Id::cast),
        membership: match channel.member {
            Some(member) => Membership::Joined(member.join_timestamp.as_secs()),
            None => Membership::Unknown,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(json: &str) -> Option<Thread> {
        convert_thread(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn threads_convert_and_other_channels_dont() {
        let public = thread(
            r#"{"id": "100", "type": 11, "guild_id": "1", "parent_id": "10", "name": "talk",
                "owner_id": "5", "message_count": 3, "member_count": 2,
                "thread_metadata": {"archived": true, "locked": false,
                    "auto_archive_duration": 1440,
                    "archive_timestamp": "2024-01-01T00:00:00+00:00"},
                "member": {"flags": 0, "join_timestamp": "2024-01-01T00:00:00+00:00"}}"#,
        )
        .unwrap();
        assert!(!public.private && public.archived && !public.locked);
        assert_eq!((public.message_count, public.member_count), (3, 2));
        assert!(public.membership.is_joined());

        let private = thread(r#"{"id": "101", "type": 12, "name": "secret"}"#).unwrap();
        assert!(private.private);
        assert_eq!(private.membership, Membership::Unknown);

        assert!(thread(r#"{"id": "102", "type": 0, "name": "general"}"#).is_none());
    }

    #[test]
    fn merging_keeps_what_an_update_leaves_out() {
        let mut known = thread(
            r#"{"id": "100", "type": 11, "name": "talk", "message_count": 9,
                "last_message_id": "500",
                "member": {"flags": 0, "join_timestamp": "2024-01-01T00:00:00+00:00"}}"#,
        )
        .unwrap();
        let update =
            thread(r#"{"id": "100", "type": 11, "name": "renamed", "message_count": 4}"#).unwrap();
        known.merge(update);
        assert_eq!(known.name, "renamed");
        assert_eq!(known.message_count, 4);
        assert_eq!(known.last_message_id, Id::new_checked(500));
        assert!(known.membership.is_joined());
    }
}
