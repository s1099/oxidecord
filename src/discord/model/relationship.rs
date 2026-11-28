//! Friends, friend requests and blocks: the user's relationships with other users.

use serde::Deserialize;
use twilight_model::id::{Id, marker::UserMarker};

use super::cdn;
use super::user::parse_discriminator;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    Friend,
    Blocked,
    /// They asked to be friends.
    Incoming,
    /// The user asked them.
    Outgoing,
}

impl RelationshipKind {
    /// `None` for the kinds the app has nothing to show for (implicit, suggestions).
    fn from_raw(kind: u8) -> Option<Self> {
        Some(match kind {
            1 => Self::Friend,
            2 => Self::Blocked,
            3 => Self::Incoming,
            4 => Self::Outgoing,
            _ => return None,
        })
    }

    pub fn is_pending(self) -> bool {
        matches!(self, Self::Incoming | Self::Outgoing)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Relationship {
    pub user_id: Id<UserMarker>,
    pub kind: RelationshipKind,
    /// Global display name when set, else the username.
    pub name: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bot: bool,
    /// The user's private name for a friend, shown in place of theirs.
    pub nickname: Option<String>,
    /// Flagged by Discord as spam; such requests don't count toward the badge.
    pub spam: bool,
}

impl Relationship {
    pub fn display_name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.name)
    }
}

/// One of `READY`'s `relationships`, or a `RELATIONSHIP_*` dispatch. `id` is the other
/// user's. Only `RELATIONSHIP_ADD` and `READY` carry `user`; an update or removal names
/// the user by id alone.
#[derive(Deserialize)]
pub(in crate::discord) struct RawRelationship {
    pub(in crate::discord) id: Id<UserMarker>,
    #[serde(rename = "type", default)]
    kind: u8,
    #[serde(default)]
    pub(in crate::discord) user: Option<RawRelationshipUser>,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    is_spam_request: bool,
    #[serde(default)]
    pub(in crate::discord) should_notify: bool,
}

#[derive(Deserialize, Clone)]
pub(in crate::discord) struct RawRelationshipUser {
    pub(in crate::discord) id: Id<UserMarker>,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    discriminator: Option<String>,
    #[serde(default)]
    bot: bool,
}

impl RawRelationship {
    pub(in crate::discord) fn kind(&self) -> Option<RelationshipKind> {
        RelationshipKind::from_raw(self.kind)
    }

    pub(in crate::discord) fn nickname(&self) -> Option<String> {
        self.nickname
            .clone()
            .filter(|nickname| !nickname.is_empty())
    }
}

/// `None` for a kind the app ignores, or with no user to name.
pub(in crate::discord) fn convert_relationship(raw: RawRelationship) -> Option<Relationship> {
    let kind = raw.kind()?;
    let nickname = raw.nickname();
    let user = raw.user?;
    Some(Relationship {
        user_id: raw.id,
        kind,
        name: user
            .global_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| user.username.clone()),
        avatar_url: Some(cdn::small_user_avatar(
            user.id.get(),
            user.avatar.as_deref(),
            parse_discriminator(user.discriminator.as_deref()),
        )),
        username: user.username,
        bot: user.bot,
        nickname,
        spam: raw.is_spam_request,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friend_request_converts_and_unknown_kinds_drop() {
        let raw: RawRelationship = serde_json::from_value(serde_json::json!({
            "id": "1", "type": 3, "nickname": null, "is_spam_request": true,
            "user": {"id": "1", "username": "wumpus", "global_name": "Wumpus",
                     "avatar": null, "discriminator": "0"}
        }))
        .unwrap();
        let relationship = convert_relationship(raw).unwrap();
        assert_eq!(relationship.kind, RelationshipKind::Incoming);
        assert_eq!(relationship.display_name(), "Wumpus");
        assert!(relationship.spam);

        let implicit: RawRelationship = serde_json::from_value(serde_json::json!({
            "id": "1", "type": 5,
            "user": {"id": "1", "username": "wumpus"}
        }))
        .unwrap();
        assert!(convert_relationship(implicit).is_none());
    }
}
