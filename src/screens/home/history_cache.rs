//! Recently opened conversations' messages, kept so that going back to one
//! draws it at once.
//!
//! Discord's own client does the same: it shows what it already has of a
//! channel straight away and fetches the newest page underneath, merging it in
//! when it lands. Cached conversations are kept current from the gateway while
//! they're closed, so the fetch usually only confirms what's there; it's what
//! covers what the gateway couldn't, like a stretch spent disconnected.

use std::collections::VecDeque;

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, Delivery, Message};

/// How many conversations are kept. A message is a few kilobytes once its
/// markdown is parsed, so this bounds the cache to a few megabytes.
const CACHED_CONVERSATIONS: usize = 24;

/// How many of a conversation's newest messages are kept: what reopening it
/// shows before anything is fetched. Older history pages in again as it's
/// scrolled, as it would have the first time.
const CACHED_MESSAGES: usize = 2 * discord::MESSAGE_PAGE_SIZE;

pub(super) struct CachedHistory {
    /// Oldest first, running up to the channel's newest as far as is known.
    pub messages: Vec<Message>,
    /// `messages` is the channel's whole history.
    pub reached_oldest: bool,
}

/// Most recently left first.
#[derive(Default)]
pub(super) struct HistoryCache {
    entries: VecDeque<(Id<ChannelMarker>, CachedHistory)>,
}

impl HistoryCache {
    /// Keeps a conversation that's being left. Only one loaded up to the
    /// present is worth keeping: reopening goes to the present, so history
    /// opened further back would leave a gap above it.
    pub(super) fn store(
        &mut self,
        channel_id: Id<ChannelMarker>,
        mut messages: Vec<Message>,
        mut reached_oldest: bool,
    ) {
        self.entries.retain(|(id, _)| *id != channel_id);
        // A send still in flight confirms into the open conversation only; its
        // echo lands in the cache as a message of its own.
        messages.retain(|message| message.delivery == Delivery::Sent);
        if messages.len() > CACHED_MESSAGES {
            messages.drain(..messages.len() - CACHED_MESSAGES);
            reached_oldest = false;
        }
        if messages.is_empty() && !reached_oldest {
            return;
        }
        self.entries.push_front((
            channel_id,
            CachedHistory {
                messages,
                reached_oldest,
            },
        ));
        self.entries.truncate(CACHED_CONVERSATIONS);
    }

    /// Takes a conversation that's being opened out of the cache. It goes
    /// back in when it's left again.
    pub(super) fn take(&mut self, channel_id: Id<ChannelMarker>) -> Option<CachedHistory> {
        let ix = self.entries.iter().position(|(id, _)| *id == channel_id)?;
        self.entries.remove(ix).map(|(_, history)| history)
    }

    fn get_mut(&mut self, channel_id: Id<ChannelMarker>) -> Option<&mut CachedHistory> {
        self.entries
            .iter_mut()
            .find(|(id, _)| *id == channel_id)
            .map(|(_, history)| history)
    }

    /// A message that arrived live in a cached conversation. It's copied only
    /// if it's kept, since most live messages are for channels not cached.
    pub(super) fn push(&mut self, channel_id: Id<ChannelMarker>, message: &Message) {
        let Some(history) = self.get_mut(channel_id) else {
            return;
        };
        // A repeated dispatch, or one that crossed with the fetch it was
        // cached from.
        if history
            .messages
            .last()
            .is_some_and(|last| last.id >= message.id)
        {
            return;
        }
        history.messages.push(message.clone());
        if history.messages.len() > CACHED_MESSAGES {
            history.messages.remove(0);
            history.reached_oldest = false;
        }
    }

    pub(super) fn message_mut(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
    ) -> Option<&mut Message> {
        self.get_mut(channel_id)?
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
    }

    pub(super) fn edit(&mut self, channel_id: Id<ChannelMarker>, message: &Message) {
        if let Some(cached) = self.message_mut(channel_id, message.id) {
            cached.apply_edit(message.clone());
        }
    }

    pub(super) fn delete(&mut self, channel_id: Id<ChannelMarker>, ids: &[Id<MessageMarker>]) {
        if let Some(history) = self.get_mut(channel_id) {
            history
                .messages
                .retain(|message| !ids.contains(&message.id));
        }
    }
}

/// Merges the newest page of a conversation into what was shown of it from the
/// cache, returning the index from which `shown` changed. The page replaces
/// everything from its oldest message on — catching the edits and deletions
/// the cache missed — while what's older is kept, as is anything newer that
/// arrived live while the page was in flight. `whole` when the page came back
/// short, so it's the channel's entire history and nothing older is kept.
///
/// The page comes back when it doesn't reach back to what was shown: there's
/// a gap between them, and it has to stand on its own.
pub(super) fn merge_latest(
    shown: &mut Vec<Message>,
    page: Vec<Message>,
    whole: bool,
) -> Result<usize, Vec<Message>> {
    let sent = |message: &&Message| message.delivery == Delivery::Sent;
    let ix = match page.first() {
        _ if whole => 0,
        Some(first) => {
            if shown
                .iter()
                .rev()
                .find(sent)
                .is_none_or(|newest| newest.id < first.id)
            {
                return Err(page);
            }
            shown
                .iter()
                .position(|message| message.id >= first.id)
                .unwrap_or(shown.len())
        }
        None => 0,
    };
    let last = page.last().map(|message| message.id);
    let after: Vec<Message> = shown
        .drain(ix..)
        .filter(|message| {
            message.delivery != Delivery::Sent || last.is_some_and(|last| message.id > last)
        })
        .collect();
    shown.extend(page);
    shown.extend(after);
    Ok(ix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(messages: &[Message]) -> Vec<u64> {
        messages.iter().map(|message| message.id.get()).collect()
    }

    fn message(id: u64) -> Message {
        let author = discord::CurrentUser {
            id: Id::new(1),
            name: String::new(),
            username: String::new(),
            avatar_url: None,
            premium: false,
            upload_limit: 0,
        };
        let mut message =
            Message::pending(Id::new(id), &author, String::new(), 0, None, Vec::new());
        message.delivery = Delivery::Sent;
        message
    }

    #[test]
    fn the_page_replaces_the_tail_it_overlaps() {
        let mut shown: Vec<_> = [1, 2, 3, 4].map(message).into();
        // 3 was deleted, 5 arrived; 6 came in live after the page was cut.
        shown.push(message(6));
        let ix = merge_latest(&mut shown, [2, 4, 5].map(message).into(), false);
        assert_eq!(ix.ok(), Some(1));
        assert_eq!(ids(&shown), [1, 2, 4, 5, 6]);
    }

    #[test]
    fn a_page_past_the_cache_leaves_a_gap() {
        let mut shown: Vec<_> = [1, 2].map(message).into();
        assert!(merge_latest(&mut shown, [5, 6].map(message).into(), false).is_err());
        assert_eq!(ids(&shown), [1, 2]);
    }

    #[test]
    fn a_whole_history_replaces_everything() {
        let mut shown: Vec<_> = [1, 2, 3].map(message).into();
        assert_eq!(
            merge_latest(&mut shown, [2, 3].map(message).into(), true).ok(),
            Some(0)
        );
        assert_eq!(ids(&shown), [2, 3]);
    }
}
