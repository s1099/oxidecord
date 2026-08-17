//! Guild channels and direct-message conversations.

use twilight_model::channel::ChannelType;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::cdn;
use super::user::small_avatar_url;

/// The subset of Discord channel types the app knows how to display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Announcement,
    Voice,
    Stage,
    Forum,
    Category,
}

impl ChannelKind {
    pub fn is_voice(self) -> bool {
        matches!(self, Self::Voice | Self::Stage)
    }
}

#[derive(Clone)]
pub struct Channel {
    pub id: Id<ChannelMarker>,
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<Id<ChannelMarker>>,
    pub position: i32,
    pub topic: Option<String>,
}

/// A 1:1 or group direct-message conversation from the user's DM list.
#[derive(Clone)]
pub struct DirectMessage {
    pub id: Id<ChannelMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// Snowflake of the last message, used only to order conversations by
    /// recency like Discord does. `None` (no messages yet) sorts last.
    last_message_id: Option<u64>,
}

/// Converts a guild channel, dropping the types the UI can't display (threads,
/// directories, ...).
pub(in crate::discord) fn convert_channel(
    channel: twilight_model::channel::Channel,
) -> Option<Channel> {
    let kind = match channel.kind {
        ChannelType::GuildText => ChannelKind::Text,
        ChannelType::GuildAnnouncement => ChannelKind::Announcement,
        ChannelType::GuildVoice => ChannelKind::Voice,
        ChannelType::GuildStageVoice => ChannelKind::Stage,
        ChannelType::GuildForum => ChannelKind::Forum,
        ChannelType::GuildCategory => ChannelKind::Category,
        _ => return None,
    };

    Some(Channel {
        id: channel.id,
        name: channel.name.unwrap_or_default(),
        kind,
        parent_id: channel.parent_id,
        position: channel.position.unwrap_or(0),
        topic: channel.topic.filter(|topic| !topic.is_empty()),
    })
}

/// Converts the raw private-channel list into DM conversations, ordered
/// most-recently-active first.
pub(in crate::discord) fn convert_dms(
    channels: Vec<twilight_model::channel::Channel>,
) -> Vec<DirectMessage> {
    let mut dms: Vec<DirectMessage> = channels.into_iter().filter_map(convert_dm).collect();
    dms.sort_by_key(|dm| std::cmp::Reverse(dm.last_message_id));
    dms
}

fn convert_dm(channel: twilight_model::channel::Channel) -> Option<DirectMessage> {
    let last_message_id = channel.last_message_id.map(|id| id.get());
    match channel.kind {
        ChannelType::Private => {
            let user = channel.recipients?.into_iter().next()?;
            Some(DirectMessage {
                id: channel.id,
                avatar_url: small_avatar_url(&user),
                name: user.global_name.unwrap_or(user.name),
                last_message_id,
            })
        }
        ChannelType::Group => Some(DirectMessage {
            id: channel.id,
            avatar_url: channel
                .icon
                .map(|hash| cdn::group_dm_icon_url(channel.id, &hash.to_string())),
            name: group_dm_name(channel.name, channel.recipients.as_deref()),
            last_message_id,
        }),
        _ => None,
    }
}

/// A group DM's own title, or the recipients' names joined the way Discord
/// labels an unnamed group.
fn group_dm_name(
    name: Option<String>,
    recipients: Option<&[twilight_model::user::User]>,
) -> String {
    name.filter(|name| !name.is_empty())
        .or_else(|| {
            let joined = recipients?
                .iter()
                .map(|user| {
                    user.global_name
                        .clone()
                        .unwrap_or_else(|| user.name.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            (!joined.is_empty()).then_some(joined)
        })
        .unwrap_or_else(|| "Group DM".to_string())
}
