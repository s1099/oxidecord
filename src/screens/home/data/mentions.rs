//! Unread mention counts: the red badges on servers, channels, and DMs.
//!
//! Discord keeps the count per channel server-side and reports it in `READY`'s
//! read states; while a session runs, the client carries it itself — raising
//! it when a message pings the user, clearing it when the conversation is
//! read. A guild's badge is its channels' sum, which needs to know which guild
//! each counted channel is in; every guild's own unread report names them.

use std::collections::HashMap;

// A glob here would drag gpui's own `test` macro into the `tests` child
// module, shadowing the built-in `#[test]` and recursing rustc into a
// stack overflow during expansion.
use gpui::Context;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::discord::{self, IncomingMessage};
use crate::screens::home::HomeScreen;

/// Whether a message pings the user by name or with `@everyone`/`@here`.
/// Role pings follow per-guild notification settings the app doesn't model
/// yet, so they don't count.
fn message_mentions_user(message: &discord::Message, self_user_id: Option<Id<UserMarker>>) -> bool {
    match self_user_id {
        Some(self_id) => {
            message.mentions.iter().any(|user| user.id == self_id) || message.mention_everyone
        }
        None => message.mention_everyone,
    }
}

/// Unread mentions by channel, and where each channel lives.
#[derive(Default)]
pub(in crate::screens::home) struct Mentions {
    /// A DM conversation badges for every message: the user is a
    /// participant, so Discord marks it whatever the text says.
    counts: HashMap<Id<ChannelMarker>, u32>,
    /// The guild each counted channel is in, as its guild's unread report
    /// names it. A channel missing here is a DM, which has no guild to sum
    /// into.
    channel_guild: HashMap<Id<ChannelMarker>, Id<GuildMarker>>,
}

impl Mentions {
    /// Replaces the counts wholesale with a session's, from `READY`. The
    /// guild bindings stay: they are rebuilt by each guild's unread report,
    /// which follows in the same fan-out.
    fn set_counts(&mut self, counts: Vec<(Id<ChannelMarker>, u32)>) {
        self.counts = counts.into_iter().collect();
    }

    fn note_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        channels: impl IntoIterator<Item = Id<ChannelMarker>>,
    ) {
        self.channel_guild
            .extend(channels.into_iter().map(|id| (id, guild_id)));
    }

    fn note_message(
        &mut self,
        channel_id: Id<ChannelMarker>,
        guild_id: Option<Id<GuildMarker>>,
        mentioned: bool,
    ) {
        if let Some(guild_id) = guild_id {
            self.channel_guild.insert(channel_id, guild_id);
        }
        if mentioned {
            let count = self.counts.entry(channel_id).or_default();
            *count = count.saturating_add(1);
        }
    }

    fn clear(&mut self, channel_id: Id<ChannelMarker>) {
        self.counts.remove(&channel_id);
    }

    /// A channel's badge, zero when it has none.
    pub(in crate::screens::home) fn count(&self, channel_id: Id<ChannelMarker>) -> u32 {
        self.counts.get(&channel_id).copied().unwrap_or(0)
    }

    /// The badge a guild's rail icon carries.
    pub(in crate::screens::home) fn guild_count(&self, guild_id: Id<GuildMarker>) -> u32 {
        self.counts
            .iter()
            .filter(|&(channel_id, _)| self.channel_guild.get(channel_id) == Some(&guild_id))
            .map(|(_, &count)| count)
            .sum()
    }
}

impl HomeScreen {
    /// Takes a session's mention counts. The conversation open when the
    /// session came up is being read, and its badge stays down.
    pub(in crate::screens::home) fn mentions_on_ready(
        &mut self,
        counts: Vec<(Id<ChannelMarker>, u32)>,
        cx: &mut Context<Self>,
    ) {
        self.mentions.set_counts(counts);
        if let Some(channel_id) = self.selected_channel {
            self.mentions.clear(channel_id);
        }
        cx.notify();
    }

    /// Raises a channel's badge when a live message pings the user. Guild
    /// messages count direct mentions and `@everyone`/`@here`; role pings
    /// follow per-guild notification settings the app doesn't model yet, so
    /// they don't. The open conversation is being read, and Discord acks it
    /// as its messages arrive, so it never counts.
    pub(in crate::screens::home) fn mentions_on_message(
        &mut self,
        incoming: &IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if Some(incoming.message.author_id) == self.self_user_id
            || self.selected_channel == Some(incoming.channel_id)
        {
            return;
        }
        let mentioned = incoming.guild_id.is_none()
            || message_mentions_user(&incoming.message, self.self_user_id);
        if !mentioned {
            return;
        }
        self.mentions
            .note_message(incoming.channel_id, incoming.guild_id, true);
        cx.notify();
    }

    /// Rebinds a guild's channels after its unread report lands, so the
    /// guild badge sums the right channels even as they move or the session
    /// reconnects.
    pub(in crate::screens::home) fn mentions_on_guild_unreads(
        &mut self,
        guild: &discord::GuildUnreads,
    ) {
        self.mentions.note_guild_channels(
            guild.guild_id,
            guild.channels.iter().map(|channel| channel.id),
        );
    }

    /// Drops a channel's badge once the user has read it. A no-op without a
    /// count, so the read paths can call it freely.
    pub(in crate::screens::home) fn mentions_clear(
        &mut self,
        channel_id: Id<ChannelMarker>,
        cx: &mut Context<Self>,
    ) {
        if self.mentions.count(channel_id) == 0 {
            return;
        }
        self.mentions.clear(channel_id);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_start_from_ready_and_sum_per_guild() {
        let mut mentions = Mentions::default();
        mentions.note_guild_channels(Id::new(1), [Id::new(10), Id::new(11)]);
        mentions.set_counts(vec![(Id::new(10), 2), (Id::new(20), 5)]);
        assert_eq!(mentions.count(Id::new(10)), 2);
        assert_eq!(mentions.guild_count(Id::new(1)), 2);
        // A DM's badge is its own count; no guild sums it.
        assert_eq!(mentions.count(Id::new(20)), 5);
        assert_eq!(mentions.guild_count(Id::new(2)), 0);
    }

    #[test]
    fn live_mentions_rise_and_clear_when_read() {
        let mut mentions = Mentions::default();
        mentions.note_message(Id::new(10), Some(Id::new(1)), true);
        mentions.note_message(Id::new(10), Some(Id::new(1)), true);
        mentions.note_message(Id::new(20), None, true);
        assert_eq!(mentions.count(Id::new(10)), 2);
        assert_eq!(mentions.guild_count(Id::new(1)), 2);

        // An unmention in a guild channel still binds it to the guild, but
        // leaves no count behind.
        mentions.note_message(Id::new(11), Some(Id::new(1)), false);
        assert_eq!(mentions.count(Id::new(11)), 0);

        mentions.clear(Id::new(10));
        assert_eq!(mentions.count(Id::new(10)), 0);
        assert_eq!(mentions.guild_count(Id::new(1)), 0);
        // Clearing one channel leaves the rest, DMs included.
        assert_eq!(mentions.count(Id::new(20)), 1);
    }

    #[test]
    fn a_count_saturates_instead_of_overflowing() {
        let mut mentions = Mentions::default();
        mentions.set_counts(vec![(Id::new(10), u32::MAX)]);
        mentions.note_message(Id::new(10), None, true);
        assert_eq!(mentions.count(Id::new(10)), u32::MAX);
    }
}
