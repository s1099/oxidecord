//! Opening a conversation, paging its history, sending to it, editing and
//! deleting what's in it, and keeping it live over the gateway.

mod delete;
mod edit;
mod history;
mod live;
mod reactions;
mod send;

use std::ops::Range;

use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Tells the message list that `old_range` became `count` rows.
    ///
    /// The newest message carries the space under the conversation, so a
    /// change at the tail also re-lays out the row that was newest: it gains
    /// or loses that space, and the list would otherwise keep its old height.
    pub(super) fn splice_messages(&self, old_range: Range<usize>, count: usize) {
        if old_range.end == self.messages_list.item_count() && old_range.start > 0 {
            self.messages_list
                .splice(old_range.start - 1..old_range.end, count + 1);
        } else {
            self.messages_list.splice(old_range, count);
        }
    }
}
