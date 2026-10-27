//! Discord's notices (joins, pins, boosts, calls, …), which the client draws as
//! one line in place of a message. Their wording is the client's, not Discord's
//! data: the message carries only who, and sometimes a name or a count.

use twilight_model::channel::message::MessageType;
use twilight_model::id::{
    Id,
    marker::{MessageMarker, UserMarker},
};

#[derive(Clone)]
pub enum SystemEvent {
    /// To a group DM or thread; `target` is the added user.
    RecipientAdd {
        target: Id<UserMarker>,
    },
    /// `target` is the author themself when they left.
    RecipientRemove {
        target: Id<UserMarker>,
    },
    Call {
        participants: Vec<Id<UserMarker>>,
        /// `None` while the call is still going. Unix seconds.
        ended: Option<i64>,
    },
    /// An empty name means the custom name was removed.
    ChannelNameChange {
        name: String,
    },
    ChannelIconChange,
    Pinned {
        message: Option<Id<MessageMarker>>,
    },
    /// `welcome` picks one of the client's greetings.
    Join {
        welcome: usize,
    },
    Boost {
        count: Option<u32>,
        level: Option<u8>,
    },
    ChannelFollow {
        channel: String,
    },
    ThreadCreated {
        name: String,
    },
    StageStart {
        topic: String,
    },
    StageEnd {
        topic: String,
    },
    StageSpeaker,
    StageRaiseHand,
    StageTopic {
        topic: String,
    },
    RoleSubscription {
        tier: String,
        months: u16,
        renewal: bool,
    },
    InvitesPaused,
    InvitesResumed,
    RaidReported,
    RaidFalseAlarm,
    /// A notice with nothing in it but the client's own text.
    Notice(&'static str),
}

/// The client's greetings for a member joining, in its order.
pub const WELCOMES: [(&str, &str); 13] = [
    ("", " joined the party."),
    ("", " is here."),
    ("Welcome, ", ". We hope you brought pizza."),
    ("A wild ", " appeared."),
    ("", " just landed."),
    ("", " just slid into the server."),
    ("", " just showed up!"),
    ("Welcome ", ". Say hi!"),
    ("", " hopped into the server."),
    ("Everyone welcome ", "!"),
    ("Glad you're here, ", "."),
    ("Good to see you, ", "."),
    ("Yay you made it, ", "!"),
];

/// Discord's epoch, in Unix milliseconds.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// `None` for a regular message, or a type the client draws as one (replies,
/// commands, polls, AutoMod's embed) or that's unknown here.
pub(super) fn convert_system(message: &twilight_model::channel::Message) -> Option<SystemEvent> {
    let target = || message.mentions.first().map(|user| user.id);
    let content = || message.content.clone();
    Some(match message.kind {
        MessageType::RecipientAdd => SystemEvent::RecipientAdd { target: target()? },
        MessageType::RecipientRemove => SystemEvent::RecipientRemove {
            target: target().unwrap_or(message.author.id),
        },
        MessageType::Call => {
            let call = message.call.as_ref();
            SystemEvent::Call {
                participants: call
                    .map(|call| call.participants.clone())
                    .unwrap_or_default(),
                ended: call
                    .and_then(|call| call.ended_timestamp)
                    .map(|ended| ended.as_secs()),
            }
        }
        MessageType::ChannelNameChange => SystemEvent::ChannelNameChange { name: content() },
        MessageType::ChannelIconChange => SystemEvent::ChannelIconChange,
        MessageType::ChannelMessagePinned => SystemEvent::Pinned {
            message: message
                .reference
                .as_ref()
                .and_then(|reference| reference.message_id),
        },
        // The client picks by the send time in milliseconds, which only the id
        // keeps; `timestamp` here is whole seconds.
        MessageType::UserJoin => SystemEvent::Join {
            welcome: (((message.id.get() >> 22) + DISCORD_EPOCH_MS) % WELCOMES.len() as u64)
                as usize,
        },
        MessageType::GuildBoost => boost(message, None),
        MessageType::GuildBoostTier1 => boost(message, Some(1)),
        MessageType::GuildBoostTier2 => boost(message, Some(2)),
        MessageType::GuildBoostTier3 => boost(message, Some(3)),
        MessageType::ChannelFollowAdd => SystemEvent::ChannelFollow { channel: content() },
        MessageType::ThreadCreated => SystemEvent::ThreadCreated { name: content() },
        MessageType::StageStart => SystemEvent::StageStart { topic: content() },
        MessageType::StageEnd => SystemEvent::StageEnd { topic: content() },
        MessageType::StageSpeaker => SystemEvent::StageSpeaker,
        MessageType::StageTopic => SystemEvent::StageTopic { topic: content() },
        MessageType::Unknown(30) => SystemEvent::StageRaiseHand,
        MessageType::RoleSubscriptionPurchase => {
            let data = message.role_subscription_data.as_ref()?;
            SystemEvent::RoleSubscription {
                tier: data.tier_name.clone(),
                months: data.total_months_subscribed,
                renewal: data.is_renewal,
            }
        }
        MessageType::GuildIncidentAlertModeEnabled => SystemEvent::InvitesPaused,
        MessageType::GuildIncidentAlertModeDisabled => SystemEvent::InvitesResumed,
        MessageType::GuildIncidentReportRaid => SystemEvent::RaidReported,
        MessageType::GuildIncidentReportRaidFalseAlarm => SystemEvent::RaidFalseAlarm,
        MessageType::GuildDiscoveryDisqualified => SystemEvent::Notice(
            "This server has been removed from Server Discovery because it no longer passes \
             all the requirements. Check Server Settings for more details.",
        ),
        MessageType::GuildDiscoveryRequalified => SystemEvent::Notice(
            "This server is eligible for Server Discovery again and has been automatically \
             relisted!",
        ),
        MessageType::GuildDiscoveryGracePeriodInitialWarning => SystemEvent::Notice(
            "This server has failed Discovery activity requirements for 1 week. If this server \
             fails for 4 weeks in a row, it will be automatically removed from Discovery.",
        ),
        MessageType::GuildDiscoveryGracePeriodFinalWarning => SystemEvent::Notice(
            "This server has failed Discovery activity requirements for 3 weeks in a row. If \
             this server fails for 1 more week, it will be removed from Discovery.",
        ),
        MessageType::GuildInviteReminder => SystemEvent::Notice(
            "Wondering who to invite? Start by inviting anyone who can help you build the server!",
        ),
        _ => return None,
    })
}

/// `content` holds the number of boosts at once, when more than one.
fn boost(message: &twilight_model::channel::Message, level: Option<u8>) -> SystemEvent {
    SystemEvent::Boost {
        count: message.content.parse().ok().filter(|&count| count > 1),
        level,
    }
}

/// A call's length in the client's words.
pub fn call_duration(seconds: i64) -> String {
    let minutes = seconds / 60;
    let hours = (seconds as f64 / 3600.).round() as i64;
    match seconds {
        ..45 => "a few seconds".into(),
        45..90 => "a minute".into(),
        _ if minutes < 45 => format!("{} minutes", (seconds as f64 / 60.).round() as i64),
        _ if minutes < 90 => "an hour".into(),
        _ => format!("{hours} hours"),
    }
}

#[cfg(test)]
mod tests {
    use super::call_duration;

    #[test]
    fn call_lengths_read_like_the_client() {
        assert_eq!(call_duration(10), "a few seconds");
        assert_eq!(call_duration(60), "a minute");
        assert_eq!(call_duration(5 * 60), "5 minutes");
        assert_eq!(call_duration(60 * 60), "an hour");
        assert_eq!(call_duration(3 * 3600), "3 hours");
    }
}
