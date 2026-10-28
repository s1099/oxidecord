//! Recently opened conversations' messages, drawn at once on reopen while the
//! newest page is fetched and merged in. The gateway keeps closed entries
//! current; the fetch covers what it couldn't, like time spent disconnected.

use std::collections::VecDeque;

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, Delivery, Message};

/// A parsed message is a few kilobytes, so this bounds the cache to a few megabytes.
const CACHED_CONVERSATIONS: usize = 24;

const CACHED_MESSAGES: usize = 2 * discord::MESSAGE_PAGE_SIZE;

pub(super) struct CachedHistory {
    /// Oldest first.
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
    /// Only a conversation loaded up to the present is worth keeping: reopening
    /// goes to the present, so history opened further back would leave a gap.
    pub(super) fn store(
        &mut self,
        channel_id: Id<ChannelMarker>,
        mut messages: Vec<Message>,
        mut reached_oldest: bool,
    ) {
        self.entries.retain(|(id, _)| *id != channel_id);
        // An in-flight send confirms into the open conversation only; its echo
        // would land here as a separate message.
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

    /// Removes the entry; it's stored again when the conversation is left.
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

    /// Copies only if kept, since most live messages are for uncached channels.
    pub(super) fn push(&mut self, channel_id: Id<ChannelMarker>, message: &Message) {
        let Some(history) = self.get_mut(channel_id) else {
            return;
        };
        // A repeated dispatch, or one that crossed with the fetch.
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
            for message in &mut history.messages {
                message.forget_deleted_reply(ids);
            }
        }
    }
}

/// Returns the index from which `shown` changed. The page replaces everything
/// from its oldest message on (catching missed edits and deletions); older
/// messages and live arrivals newer than the page are kept. `whole` means the
/// page came back short, so it is the entire history.
///
/// The page is handed back when it doesn't reach `shown`: there's a gap, so it
/// has to stand alone.
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
            Message::pending(Id::new(id), &author, String::new(), 0, None, None);
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
