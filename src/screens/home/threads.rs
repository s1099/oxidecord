//! Threads the client knows of: those the user is in, the active ones a sync
//! brought, and any opened from a link, a notice or the threads list.
//!
//! A thread opens as a conversation of its own, so each is kept alongside the
//! [`discord::Channel`] it's drawn and sent to as, which takes the parent's
//! permissions.

use std::collections::HashMap;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{self, Channel, Membership, Thread};

pub(super) struct KnownThread {
    pub thread: Thread,
    pub channel: Channel,
}

#[derive(Default)]
pub(super) struct Threads {
    known: HashMap<Id<ChannelMarker>, KnownThread>,
    /// Thread ids by parent channel: the sidebar asks for each channel's every frame,
    /// and `known` spans every guild.
    by_parent: HashMap<Id<ChannelMarker>, Vec<Id<ChannelMarker>>>,
    /// Each thread's newest message, previewed in the threads list.
    pub latest: HashMap<Id<ChannelMarker>, discord::MessageReference>,
}

impl Threads {
    pub fn get(&self, id: Id<ChannelMarker>) -> Option<&KnownThread> {
        self.known.get(&id)
    }

    pub fn thread(&self, id: Id<ChannelMarker>) -> Option<&Thread> {
        self.known.get(&id).map(|known| &known.thread)
    }

    /// Merged into what's known, which keeps membership an update leaves out.
    pub fn upsert(&mut self, thread: Thread, parent: Option<&Channel>) -> &KnownThread {
        let thread = match self.known.remove(&thread.id) {
            Some(mut known) => {
                self.unindex(&known.thread);
                known.thread.merge(thread);
                known.thread
            }
            None => thread,
        };
        if let Some(parent_id) = thread.parent_id {
            self.by_parent.entry(parent_id).or_default().push(thread.id);
        }
        let channel = thread.channel(parent);
        self.known
            .entry(thread.id)
            .insert_entry(KnownThread { thread, channel })
            .into_mut()
    }

    pub fn remove(&mut self, id: Id<ChannelMarker>) -> Option<KnownThread> {
        self.latest.remove(&id);
        let known = self.known.remove(&id)?;
        self.unindex(&known.thread);
        Some(known)
    }

    fn unindex(&mut self, thread: &Thread) {
        let Some(parent_id) = thread.parent_id else {
            return;
        };
        if let Some(siblings) = self.by_parent.get_mut(&parent_id) {
            siblings.retain(|&id| id != thread.id);
            if siblings.is_empty() {
                self.by_parent.remove(&parent_id);
            }
        }
    }

    /// Newest activity first.
    fn children(
        &self,
        parent_id: Id<ChannelMarker>,
        keep: impl Fn(&Thread) -> bool,
    ) -> Vec<&Thread> {
        let mut threads: Vec<&Thread> = self
            .by_parent
            .get(&parent_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.thread(*id))
            .filter(|thread| keep(thread))
            .collect();
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.last_active()));
        threads
    }

    /// Active threads in the guild (or in `channels` of it) that aren't in a sync
    /// replacing them, except `keep`, the one open. Archived ones stay: a sync
    /// only lists active threads, and one opened from the list is still there.
    pub fn stale_after_sync(
        &self,
        guild_id: Id<GuildMarker>,
        channels: Option<&[Id<ChannelMarker>]>,
        synced: &[Thread],
        keep: Option<Id<ChannelMarker>>,
    ) -> Vec<Id<ChannelMarker>> {
        self.known
            .values()
            .map(|known| &known.thread)
            .filter(|thread| {
                thread.guild_id == Some(guild_id)
                    && !thread.archived
                    && Some(thread.id) != keep
                    && channels.is_none_or(|channels| {
                        thread
                            .parent_id
                            .is_some_and(|parent| channels.contains(&parent))
                    })
                    && !synced.iter().any(|synced| synced.id == thread.id)
            })
            .map(|thread| thread.id)
            .collect()
    }

    /// The guild's permissions changed, which threads take from their parents.
    pub fn refresh_permissions(&mut self, guild_id: Id<GuildMarker>, channels: &[Channel]) {
        for known in self.known.values_mut() {
            if known.thread.guild_id != Some(guild_id) {
                continue;
            }
            let parent = channels
                .iter()
                .find(|channel| Some(channel.id) == known.thread.parent_id);
            known.channel = known.thread.channel(parent);
        }
    }

    /// Under a channel in the sidebar: the active ones the user is in, newest
    /// activity first.
    pub fn joined_in(&self, parent_id: Id<ChannelMarker>) -> Vec<&Thread> {
        self.children(parent_id, |thread| {
            !thread.archived && thread.membership.is_joined()
        })
    }

    /// Active threads under `parent_id`, newest activity first.
    pub fn active_in(&self, parent_id: Id<ChannelMarker>) -> Vec<&Thread> {
        self.children(parent_id, |thread| !thread.archived)
    }

    /// A message arrived in `channel_id`. Returns whether that's a known thread.
    pub fn note_message(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message: &discord::Message,
    ) -> bool {
        let Some(known) = self.known.get_mut(&channel_id) else {
            return false;
        };
        let thread = &mut known.thread;
        if thread.last_message_id.is_none_or(|last| last < message.id) {
            thread.last_message_id = Some(message.id);
            thread.message_count += 1;
            // A message to an archived thread reopens it.
            thread.archived = false;
            known.channel.last_message_id = Some(message.id);
            self.latest
                .insert(channel_id, discord::MessageReference::quoting(message));
        }
        true
    }

    pub fn set_membership(&mut self, id: Id<ChannelMarker>, membership: Membership) {
        if let Some(known) = self.known.get_mut(&id) {
            known.thread.membership = membership;
        }
    }

    /// Drops a deleted message's preview; the one before it isn't known.
    pub fn forget_latest(&mut self, channel_id: Id<ChannelMarker>, deleted: &[Id<MessageMarker>]) {
        if self
            .latest
            .get(&channel_id)
            .is_some_and(|latest| deleted.contains(&latest.id))
        {
            self.latest.remove(&channel_id);
        }
    }
}

/// The "New Thread" panel beside the conversation, as Discord's: a name, an
/// opening message, and for a thread on its own, whether it's private.
pub(super) struct ThreadDraft {
    pub parent_id: Id<ChannelMarker>,
    /// The message it branches from; drawn at the top of the panel.
    pub starter: Option<discord::Message>,
    pub name: Entity<InputState>,
    pub message: Entity<InputState>,
    pub private: bool,
    pub creating: bool,
    pub error: Option<String>,
    pub _subscriptions: Vec<Subscription>,
}

/// Discord's name for a thread off a message: its first line, cut to fit.
pub(super) fn default_thread_name(message: &discord::Message) -> String {
    const MAX_CHARS: usize = 40;
    let line = message
        .content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let name: String = line.chars().take(MAX_CHARS).collect();
    if name.is_empty() {
        "New Thread".into()
    } else {
        name
    }
}
