//! The user's friends, friend requests and blocks, and the Friends page that lists them.
//!
//! Relationships arrive whole with `READY` and change only through `RELATIONSHIP_*`
//! dispatches, which also answer every change made from here; nothing is fetched.

use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, Relationship, RelationshipKind};

use super::HomeScreen;

/// Discord's username limit.
const USERNAME_MAX_LEN: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FriendsTab {
    Online,
    All,
    Pending,
    Blocked,
    AddFriend,
}

impl FriendsTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::All => "All",
            Self::Pending => "Pending",
            Self::Blocked => "Blocked",
            Self::AddFriend => "Add Friend",
        }
    }
}

/// What came of the last friend request sent from the Add Friend tab.
pub(super) enum AddFriendResult {
    /// The username it went to.
    Sent(String),
    Failed(String),
}

pub(super) struct Friends {
    pub relationships: HashMap<Id<UserMarker>, Relationship>,
    pub tab: FriendsTab,
    /// Filters the list tabs.
    pub search: Entity<InputState>,
    pub add_input: Entity<InputState>,
    pub add_result: Option<AddFriendResult>,
    pub sending: bool,
    pub scroll: ScrollHandle,
    /// Whether the window is wide enough for the Active Now column, as Discord hides it
    /// below that. Set each frame, where the window is at hand.
    pub wide: bool,
    _subscriptions: [Subscription; 2],
}

impl Friends {
    pub fn new(window: &mut Window, cx: &mut Context<HomeScreen>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let add_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("You can add friends with their Discord username.")
                .validate(|text, _| {
                    text.chars().count() <= USERNAME_MAX_LEN && !text.contains(char::is_whitespace)
                })
        });
        let searched = cx.observe(&search, |_, _, cx| cx.notify());
        let add_changed = cx.subscribe_in(
            &add_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.send_friend_request(window, cx),
                // A new name makes the last outcome stale, as in Discord.
                InputEvent::Change => {
                    if this.friends.add_result.take().is_some() {
                        cx.notify();
                    }
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );
        Self {
            relationships: HashMap::new(),
            tab: FriendsTab::Online,
            search,
            add_input,
            add_result: None,
            sending: false,
            scroll: ScrollHandle::new(),
            wide: true,
            _subscriptions: [searched, add_changed],
        }
    }

    pub fn kind(&self, user_id: Id<UserMarker>) -> Option<RelationshipKind> {
        self.relationships
            .get(&user_id)
            .map(|relationship| relationship.kind)
    }

    pub fn is_blocked(&self, user_id: Id<UserMarker>) -> bool {
        self.kind(user_id) == Some(RelationshipKind::Blocked)
    }

    /// The badge on the Friends row and the home icon: requests waiting on the user,
    /// leaving out those Discord flagged as spam.
    pub fn incoming_count(&self) -> usize {
        self.relationships
            .values()
            .filter(|relationship| {
                relationship.kind == RelationshipKind::Incoming && !relationship.spam
            })
            .count()
    }

    pub fn of_kind(&self, pick: impl Fn(RelationshipKind) -> bool) -> Vec<&Relationship> {
        let mut list: Vec<&Relationship> = self
            .relationships
            .values()
            .filter(|relationship| pick(relationship.kind))
            .collect();
        list.sort_by_cached_key(|relationship| relationship.display_name().to_lowercase());
        list
    }

    pub fn replace(&mut self, relationships: Vec<Relationship>) {
        self.relationships = relationships
            .into_iter()
            .map(|relationship| (relationship.user_id, relationship))
            .collect();
    }

    pub fn update(
        &mut self,
        user_id: Id<UserMarker>,
        kind: Option<RelationshipKind>,
        nickname: Option<String>,
    ) {
        if let Some(relationship) = self.relationships.get_mut(&user_id) {
            if let Some(kind) = kind {
                relationship.kind = kind;
            }
            relationship.nickname = nickname;
        }
    }
}

/// Whether `relationship` matches what's typed in the search box: its shown name or
/// username, ignoring case.
pub(super) fn matches_search(relationship: &Relationship, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || relationship.display_name().to_lowercase().contains(&query)
        || relationship.username.to_lowercase().contains(&query)
        || relationship.name.to_lowercase().contains(&query)
}

/// A status as the Friends list words it under a name.
pub(super) fn status_label(status: discord::PresenceStatus) -> &'static str {
    match status {
        discord::PresenceStatus::Online => "Online",
        discord::PresenceStatus::Idle => "Idle",
        discord::PresenceStatus::DoNotDisturb => "Do Not Disturb",
        discord::PresenceStatus::Invisible => "Offline",
    }
}
