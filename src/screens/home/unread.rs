//! Which channels and guilds have messages the user hasn't read.
//!
//! Discord leaves this to the client: it sends how far the user has read each
//! channel (their acks) and how far each channel has got, and a channel is
//! unread while the second is past the first. [`Unreads`] keeps both, fed by
//! the gateway, the REST loads, and the user's own reading.

use std::collections::{HashMap, HashSet};

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{ChannelLatest, GuildUnreads, MuteSettings, ReadState, UnreadChannel};

type ChannelId = Id<ChannelMarker>;
type GuildId = Id<GuildMarker>;
type MessageId = Id<MessageMarker>;

/// A guild's channels that count toward its unread mark.
struct GuildChannels {
    /// When the user joined, which a channel they've never opened counts as
    /// read up to.
    joined: Option<MessageId>,
    /// Each channel and its category, whose mute it inherits.
    channels: Vec<(ChannelId, Option<ChannelId>)>,
}

#[derive(Default)]
pub(super) struct Unreads {
    /// The last message the user has read in each channel.
    acked: HashMap<ChannelId, MessageId>,
    latest: HashMap<ChannelId, MessageId>,
    guilds: HashMap<GuildId, GuildChannels>,
    /// Which guild each of `guilds`' channels is in.
    channel_guild: HashMap<ChannelId, GuildId>,
    muted_guilds: HashSet<GuildId>,
    /// Muted channels and categories, by guild — `None` for DMs — so one
    /// guild's settings can be replaced without touching the rest.
    muted_channels: HashMap<Option<GuildId>, HashSet<ChannelId>>,
}

impl Unreads {
    /// Replaces every channel's ack with a session's read states.
    pub(super) fn set_read_states(&mut self, states: Vec<ReadState>) {
        self.acked = states
            .into_iter()
            .map(|state| (state.channel_id, state.last_message_id))
            .collect();
    }

    pub(super) fn set_guild(&mut self, guild: GuildUnreads) {
        if let Some(old) = self.guilds.remove(&guild.guild_id) {
            for (id, _) in old.channels {
                self.channel_guild.remove(&id);
            }
        }
        let mut channels = Vec::with_capacity(guild.channels.len());
        for UnreadChannel {
            id,
            parent_id,
            last_message_id,
        } in guild.channels
        {
            if let Some(latest) = last_message_id {
                self.note_latest(id, latest);
            }
            self.channel_guild.insert(id, guild.guild_id);
            channels.push((id, parent_id));
        }
        self.guilds.insert(
            guild.guild_id,
            GuildChannels {
                joined: guild.joined,
                channels,
            },
        );
    }

    pub(super) fn set_latest(&mut self, channels: Vec<ChannelLatest>) {
        for channel in channels {
            self.note_latest(channel.channel_id, channel.last_message_id);
        }
    }

    pub(super) fn set_mutes(&mut self, settings: Vec<MuteSettings>) {
        for settings in settings {
            if let Some(guild_id) = settings.guild_id {
                if settings.muted {
                    self.muted_guilds.insert(guild_id);
                } else {
                    self.muted_guilds.remove(&guild_id);
                }
            }
            self.muted_channels.insert(
                settings.guild_id,
                settings.muted_channels.into_iter().collect(),
            );
        }
    }

    /// Records how far a channel has got, as a channel list reports it.
    pub(super) fn note_latest(&mut self, channel_id: ChannelId, message_id: MessageId) {
        let latest = self.latest.entry(channel_id).or_insert(message_id);
        *latest = (*latest).max(message_id);
    }

    /// Records a message arriving live. A channel with nothing to count from
    /// — a DM that has never had a read state — starts from just before it,
    /// so a brand-new conversation shows as unread.
    pub(super) fn note_message(&mut self, channel_id: ChannelId, message_id: MessageId) {
        if self.acked(channel_id).is_none() {
            let before = self
                .latest
                .get(&channel_id)
                .copied()
                .filter(|&latest| latest < message_id)
                .or_else(|| Id::new_checked(message_id.get() - 1));
            if let Some(before) = before {
                self.acked.insert(channel_id, before);
            }
        }
        self.note_latest(channel_id, message_id);
    }

    /// The last message read in a channel. A guild channel the user has never
    /// opened counts as read up to when they joined; anything else with no
    /// read state is `None`, and counts as read.
    pub(super) fn acked(&self, channel_id: ChannelId) -> Option<MessageId> {
        self.acked.get(&channel_id).copied().or_else(|| {
            let guild_id = self.channel_guild.get(&channel_id)?;
            self.guilds.get(guild_id)?.joined
        })
    }

    pub(super) fn latest(&self, channel_id: ChannelId) -> Option<MessageId> {
        self.latest.get(&channel_id).copied()
    }

    /// Moves a channel's ack forward to `message_id`. Returns whether it
    /// moved: an ack behind the one already held changes nothing.
    pub(super) fn ack(&mut self, channel_id: ChannelId, message_id: MessageId) -> bool {
        if self
            .acked(channel_id)
            .is_some_and(|acked| acked >= message_id)
        {
            return false;
        }
        self.acked.insert(channel_id, message_id);
        true
    }

    /// Sets a channel's ack wherever it lands, backwards included.
    pub(super) fn set_acked(&mut self, channel_id: ChannelId, message_id: MessageId) {
        self.acked.insert(channel_id, message_id);
    }

    pub(super) fn is_unread(&self, channel_id: ChannelId) -> bool {
        match (self.latest(channel_id), self.acked(channel_id)) {
            (Some(latest), Some(acked)) => latest > acked,
            _ => false,
        }
    }

    /// Whether a channel is muted, itself or through its category.
    pub(super) fn is_muted(&self, channel_id: ChannelId, parent_id: Option<ChannelId>) -> bool {
        let guild_id = self.channel_guild.get(&channel_id).copied();
        self.muted_channels.get(&guild_id).is_some_and(|muted| {
            muted.contains(&channel_id) || parent_id.is_some_and(|parent| muted.contains(&parent))
        })
    }

    /// Whether a whole guild is muted, which silences its unread mark and,
    /// with it, its mention badges.
    pub(super) fn is_guild_muted(&self, guild_id: GuildId) -> bool {
        self.muted_guilds.contains(&guild_id)
    }

    /// Whether a guild has a channel with something unread in it, leaving out
    /// what's muted — a muted guild never shows its mark at all.
    pub(super) fn is_guild_unread(&self, guild_id: GuildId) -> bool {
        if self.muted_guilds.contains(&guild_id) {
            return false;
        }
        self.guilds.get(&guild_id).is_some_and(|guild| {
            guild.channels.iter().any(|&(channel_id, parent_id)| {
                !self.is_muted(channel_id, parent_id) && self.is_unread(channel_id)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guild(joined: Option<u64>, channels: &[(u64, Option<u64>, Option<u64>)]) -> GuildUnreads {
        GuildUnreads {
            guild_id: Id::new(1),
            joined: joined.map(Id::new),
            channels: channels
                .iter()
                .map(|&(id, parent, latest)| UnreadChannel {
                    id: Id::new(id),
                    parent_id: parent.map(Id::new),
                    last_message_id: latest.map(Id::new),
                })
                .collect(),
        }
    }

    #[test]
    fn a_channel_is_unread_past_its_ack() {
        let mut unreads = Unreads::default();
        unreads.set_guild(guild(None, &[(10, None, Some(50))]));
        unreads.set_read_states(vec![ReadState {
            channel_id: Id::new(10),
            last_message_id: Id::new(40),
        }]);
        assert!(unreads.is_unread(Id::new(10)));
        assert!(unreads.is_guild_unread(Id::new(1)));

        assert!(unreads.ack(Id::new(10), Id::new(50)));
        assert!(!unreads.is_unread(Id::new(10)));
        // Acks only go forward unless set outright.
        assert!(!unreads.ack(Id::new(10), Id::new(45)));
        unreads.set_acked(Id::new(10), Id::new(45));
        assert!(unreads.is_unread(Id::new(10)));
    }

    #[test]
    fn a_channel_never_opened_counts_from_joining() {
        let mut unreads = Unreads::default();
        unreads.set_guild(guild(
            Some(20),
            &[(10, None, Some(15)), (11, None, Some(25))],
        ));
        assert!(!unreads.is_unread(Id::new(10)));
        assert!(unreads.is_unread(Id::new(11)));
    }

    #[test]
    fn mutes_hide_a_guilds_mark() {
        let mut unreads = Unreads::default();
        unreads.set_guild(guild(Some(1), &[(10, Some(5), Some(50))]));
        assert!(unreads.is_guild_unread(Id::new(1)));

        unreads.set_mutes(vec![MuteSettings {
            guild_id: Some(Id::new(1)),
            muted: false,
            muted_channels: vec![Id::new(5)],
        }]);
        assert!(unreads.is_muted(Id::new(10), Some(Id::new(5))));
        assert!(!unreads.is_guild_unread(Id::new(1)));
        // Muting doesn't make the channel itself read.
        assert!(unreads.is_unread(Id::new(10)));
    }

    #[test]
    fn a_new_conversation_is_unread_from_its_first_message() {
        let mut unreads = Unreads::default();
        unreads.note_message(Id::new(10), Id::new(100));
        assert!(unreads.is_unread(Id::new(10)));
    }
}
