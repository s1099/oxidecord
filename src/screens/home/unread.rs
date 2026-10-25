//! Which channels and guilds have messages the user hasn't read.
//!
//! Discord leaves this to the client: it sends how far the user has read each
//! channel (their acks) and how far each channel has got, and a channel is
//! unread while the second is past the first. [`Unreads`] keeps both, fed by
//! the gateway, the REST loads, and the user's own reading.
//!
//! Not every unread channel shows as one: a muted channel, category or guild
//! never does, and nor does one set to mark only pings unread — which is
//! where a "Only @mentions" notification level leaves it unless the user
//! picked an unread setting apart from it.
//!
//! Mentions are counted here too, for the red badges. Discord only reports
//! the count as each session starts and when it's acked; in between, the
//! client counts the pings that arrive itself, as the official one does.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{
    ChannelLatest, GuildUnreads, MuteSettings, ReadState, UnreadChannel, UnreadSetting,
};

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

/// What a guild's rail icon shows, worked out from all of its channels.
#[derive(Clone, Copy)]
struct GuildMark {
    unread: bool,
    mentions: u32,
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
    /// What marks each guild's channels unread for a user who hasn't set it.
    default_unreads: HashMap<GuildId, UnreadSetting>,
    /// What the user set to mark each guild's channels unread, where they did.
    guild_unreads: HashMap<GuildId, UnreadSetting>,
    /// Channels and categories with an unread setting of their own, by guild.
    channel_unreads: HashMap<GuildId, HashMap<ChannelId, UnreadSetting>>,
    /// Unread messages that ping the user, by channel. A channel with none
    /// has no entry.
    mentions: HashMap<ChannelId, u32>,
    /// Guilds whose `@everyone` pings don't count.
    suppress_everyone: HashSet<GuildId>,
    /// Guilds whose role pings don't count.
    suppress_roles: HashSet<GuildId>,
    /// Each guild's mark as last worked out. The rail asks for every guild's
    /// on every frame, and working one out walks all its channels, so it's
    /// kept until something in the guild changes.
    marks: RefCell<HashMap<GuildId, GuildMark>>,
}

impl Unreads {
    /// Replaces every channel's ack with a session's read states.
    pub(super) fn set_read_states(&mut self, states: Vec<ReadState>) {
        self.marks.get_mut().clear();
        self.mentions = states
            .iter()
            .filter(|state| state.mention_count > 0)
            .map(|state| (state.channel_id, state.mention_count))
            .collect();
        self.acked = states
            .into_iter()
            .map(|state| (state.channel_id, state.last_message_id))
            .collect();
    }

    pub(super) fn set_guild(&mut self, guild: GuildUnreads) {
        self.marks.get_mut().remove(&guild.guild_id);
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
        self.marks.get_mut().clear();
        for settings in settings {
            if let Some(guild_id) = settings.guild_id {
                for (set, on) in [
                    (&mut self.muted_guilds, settings.muted),
                    (&mut self.suppress_everyone, settings.suppress_everyone),
                    (&mut self.suppress_roles, settings.suppress_roles),
                ] {
                    if on {
                        set.insert(guild_id);
                    } else {
                        set.remove(&guild_id);
                    }
                }
                match settings.unreads {
                    Some(unreads) => self.guild_unreads.insert(guild_id, unreads),
                    None => self.guild_unreads.remove(&guild_id),
                };
                self.channel_unreads
                    .insert(guild_id, settings.channel_unreads.into_iter().collect());
            }
            self.muted_channels.insert(
                settings.guild_id,
                settings.muted_channels.into_iter().collect(),
            );
        }
    }

    pub(super) fn set_default_unreads(&mut self, guild_id: GuildId, unreads: UnreadSetting) {
        self.marks.get_mut().remove(&guild_id);
        self.default_unreads.insert(guild_id, unreads);
    }

    /// Records how far a channel has got, as a channel list reports it.
    pub(super) fn note_latest(&mut self, channel_id: ChannelId, message_id: MessageId) {
        let latest = self.latest.entry(channel_id).or_insert(message_id);
        *latest = (*latest).max(message_id);
        self.forget_mark(channel_id);
    }

    /// Drops the mark of the guild a channel is in, after the channel changed.
    fn forget_mark(&mut self, channel_id: ChannelId) {
        if let Some(guild_id) = self.channel_guild.get(&channel_id) {
            self.marks.get_mut().remove(guild_id);
        }
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

    /// Moves a channel's ack forward to `message_id`, which reads its
    /// mentions too. Returns whether it moved: an ack behind the one already
    /// held changes nothing.
    pub(super) fn ack(&mut self, channel_id: ChannelId, message_id: MessageId) -> bool {
        if self
            .acked(channel_id)
            .is_some_and(|acked| acked >= message_id)
        {
            return false;
        }
        self.acked.insert(channel_id, message_id);
        self.mentions.remove(&channel_id);
        self.forget_mark(channel_id);
        true
    }

    /// Sets a channel's ack wherever it lands, backwards included, along
    /// with the mentions left past it.
    pub(super) fn set_acked(
        &mut self,
        channel_id: ChannelId,
        message_id: MessageId,
        mentions: u32,
    ) {
        self.acked.insert(channel_id, message_id);
        self.set_mentions(channel_id, mentions);
    }

    pub(super) fn set_mentions(&mut self, channel_id: ChannelId, mentions: u32) {
        if mentions == 0 {
            self.mentions.remove(&channel_id);
        } else {
            self.mentions.insert(channel_id, mentions);
        }
        self.forget_mark(channel_id);
    }

    /// Counts a message that pings the user.
    pub(super) fn note_mention(&mut self, channel_id: ChannelId) {
        *self.mentions.entry(channel_id).or_default() += 1;
        self.forget_mark(channel_id);
    }

    pub(super) fn mentions(&self, channel_id: ChannelId) -> u32 {
        self.mentions.get(&channel_id).copied().unwrap_or(0)
    }

    /// A DM's mentions, none while it's muted. Every message in a DM counts as
    /// a ping, and the counts a session starts with take no notice of mutes,
    /// but Discord's client shows a muted conversation's as nothing.
    pub(super) fn dm_mentions(&self, channel_id: ChannelId) -> u32 {
        if self.is_muted(channel_id, None) {
            0
        } else {
            self.mentions(channel_id)
        }
    }

    /// A guild's mentions across its channels. Mutes don't hide them: a
    /// muted guild still shows how often it pinged the user, as Discord's
    /// does.
    pub(super) fn guild_mentions(&self, guild_id: GuildId) -> u32 {
        self.mark(guild_id).mentions
    }

    /// Whether `@everyone` and `@here`, and role mentions, ping the user in a
    /// guild.
    pub(super) fn pinged_by(&self, guild_id: GuildId) -> (bool, bool) {
        (
            !self.suppress_everyone.contains(&guild_id),
            !self.suppress_roles.contains(&guild_id),
        )
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

    /// What marks a channel unread: its own setting, else its category's,
    /// else its guild's. DMs mark every message.
    fn unread_setting(&self, channel_id: ChannelId, parent_id: Option<ChannelId>) -> UnreadSetting {
        let Some(guild_id) = self.channel_guild.get(&channel_id) else {
            return UnreadSetting::AllMessages;
        };
        self.channel_unreads
            .get(guild_id)
            .and_then(|channels| {
                channels
                    .get(&channel_id)
                    .or_else(|| channels.get(&parent_id?))
            })
            .or_else(|| self.guild_unreads.get(guild_id))
            .or_else(|| self.default_unreads.get(guild_id))
            .copied()
            .unwrap_or(UnreadSetting::AllMessages)
    }

    /// Whether a channel shows as unread: it has something new and isn't
    /// muted, and either marks every message or has pinged the user.
    pub(super) fn shows_unread(&self, channel_id: ChannelId, parent_id: Option<ChannelId>) -> bool {
        self.is_unread(channel_id)
            && !self.is_muted(channel_id, parent_id)
            && (self.mentions(channel_id) > 0
                || self.unread_setting(channel_id, parent_id) == UnreadSetting::AllMessages)
    }

    /// Whether a guild has a channel that shows as unread — a muted guild
    /// never shows its mark at all.
    pub(super) fn is_guild_unread(&self, guild_id: GuildId) -> bool {
        self.mark(guild_id).unread
    }

    fn mark(&self, guild_id: GuildId) -> GuildMark {
        if let Some(&mark) = self.marks.borrow().get(&guild_id) {
            return mark;
        }
        let channels = self
            .guilds
            .get(&guild_id)
            .map_or(&[][..], |guild| &guild.channels);
        let mark = GuildMark {
            unread: !self.muted_guilds.contains(&guild_id)
                && channels
                    .iter()
                    .any(|&(channel_id, parent_id)| self.shows_unread(channel_id, parent_id)),
            mentions: channels
                .iter()
                .map(|&(channel_id, _)| self.mentions(channel_id))
                .sum(),
        };
        self.marks.borrow_mut().insert(guild_id, mark);
        mark
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
            mention_count: 0,
        }]);
        assert!(unreads.is_unread(Id::new(10)));
        assert!(unreads.is_guild_unread(Id::new(1)));

        assert!(unreads.ack(Id::new(10), Id::new(50)));
        assert!(!unreads.is_unread(Id::new(10)));
        assert!(!unreads.is_guild_unread(Id::new(1)));
        // Acks only go forward unless set outright.
        assert!(!unreads.ack(Id::new(10), Id::new(45)));
        unreads.set_acked(Id::new(10), Id::new(45), 0);
        assert!(unreads.is_unread(Id::new(10)));
        assert!(unreads.is_guild_unread(Id::new(1)));
    }

    #[test]
    fn mentions_count_up_until_acked() {
        let mut unreads = Unreads::default();
        unreads.set_guild(guild(Some(1), &[(10, None, Some(50)), (11, None, None)]));
        unreads.set_read_states(vec![ReadState {
            channel_id: Id::new(10),
            last_message_id: Id::new(40),
            mention_count: 2,
        }]);
        unreads.note_message(Id::new(11), Id::new(60));
        unreads.note_mention(Id::new(11));
        assert_eq!(unreads.guild_mentions(Id::new(1)), 3);

        // Muting hides the unread mark, but not the mentions.
        unreads.set_mutes(vec![MuteSettings {
            guild_id: Some(Id::new(1)),
            muted: true,
            muted_channels: Vec::new(),
            unreads: None,
            channel_unreads: Vec::new(),
            suppress_everyone: false,
            suppress_roles: false,
        }]);
        assert!(!unreads.is_guild_unread(Id::new(1)));
        assert_eq!(unreads.guild_mentions(Id::new(1)), 3);

        unreads.ack(Id::new(10), Id::new(50));
        assert_eq!(unreads.mentions(Id::new(10)), 0);
        assert_eq!(unreads.guild_mentions(Id::new(1)), 1);
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
            unreads: None,
            channel_unreads: Vec::new(),
            suppress_everyone: false,
            suppress_roles: false,
        }]);
        assert!(unreads.is_muted(Id::new(10), Some(Id::new(5))));
        assert!(!unreads.is_guild_unread(Id::new(1)));
        // Muting doesn't make the channel itself read.
        assert!(unreads.is_unread(Id::new(10)));
    }

    #[test]
    fn mention_only_channels_need_a_ping_to_show() {
        let mut unreads = Unreads::default();
        unreads.set_guild(guild(
            Some(1),
            &[(10, Some(5), Some(50)), (11, None, Some(50))],
        ));
        unreads.set_default_unreads(Id::new(1), UnreadSetting::OnlyMentions);
        assert!(!unreads.is_guild_unread(Id::new(1)));

        // The user's setting for one category outranks the guild's default.
        unreads.set_mutes(vec![MuteSettings {
            guild_id: Some(Id::new(1)),
            muted: false,
            muted_channels: Vec::new(),
            unreads: None,
            channel_unreads: vec![(Id::new(5), UnreadSetting::AllMessages)],
            suppress_everyone: false,
            suppress_roles: false,
        }]);
        assert!(unreads.shows_unread(Id::new(10), Some(Id::new(5))));
        assert!(!unreads.shows_unread(Id::new(11), None));

        // A ping shows a channel that only marks pings.
        unreads.note_mention(Id::new(11));
        assert!(unreads.shows_unread(Id::new(11), None));
    }

    #[test]
    fn a_new_conversation_is_unread_from_its_first_message() {
        let mut unreads = Unreads::default();
        unreads.note_message(Id::new(10), Id::new(100));
        assert!(unreads.is_unread(Id::new(10)));
    }

    #[test]
    fn a_muted_dm_shows_no_mentions() {
        let mut unreads = Unreads::default();
        unreads.set_read_states(vec![ReadState {
            channel_id: Id::new(10),
            last_message_id: Id::new(40),
            mention_count: 3,
        }]);
        assert_eq!(unreads.dm_mentions(Id::new(10)), 3);

        unreads.set_mutes(vec![MuteSettings {
            guild_id: None,
            muted: false,
            muted_channels: vec![Id::new(10)],
            unreads: None,
            channel_unreads: Vec::new(),
            suppress_everyone: false,
            suppress_roles: false,
        }]);
        assert!(unreads.is_muted(Id::new(10), None));
        assert_eq!(unreads.dm_mentions(Id::new(10)), 0);
    }
}
