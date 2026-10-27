//! Message links ("Copy Message Link"), opened in the app rather than the browser.

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageLink {
    /// `None` for a direct message, whose links say `@me` in its place.
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Id<ChannelMarker>,
    pub message_id: Id<MessageMarker>,
}

impl MessageLink {
    /// Reads `https://discord.com/channels/<guild or @me>/<channel>/<message>`
    /// on any host Discord's clients have produced links with.
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let (host, path) = rest.split_once('/')?;
        let host = host.strip_prefix("www.").unwrap_or(host);
        let host = host
            .strip_prefix("ptb.")
            .or_else(|| host.strip_prefix("canary."))
            .unwrap_or(host);
        if host != "discord.com" && host != "discordapp.com" {
            return None;
        }
        let path = path.split(['?', '#']).next()?;
        let mut parts = path.trim_end_matches('/').split('/');
        if parts.next()? != "channels" {
            return None;
        }
        let guild_id = match parts.next()? {
            "@me" => None,
            guild => Some(Id::new_checked(guild.parse().ok()?)?),
        };
        let channel_id = Id::new_checked(parts.next()?.parse().ok()?)?;
        let message_id = Id::new_checked(parts.next()?.parse().ok()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            guild_id,
            channel_id,
            message_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guild_and_dm_links() {
        assert_eq!(
            MessageLink::parse("https://discord.com/channels/1/2/3"),
            Some(MessageLink {
                guild_id: Some(Id::new(1)),
                channel_id: Id::new(2),
                message_id: Id::new(3),
            })
        );
        assert_eq!(
            MessageLink::parse("https://canary.discord.com/channels/@me/2/3?x=1"),
            Some(MessageLink {
                guild_id: None,
                channel_id: Id::new(2),
                message_id: Id::new(3),
            })
        );
    }

    #[test]
    fn not_message_links() {
        assert_eq!(MessageLink::parse("https://discord.com/channels/1/2"), None);
        assert_eq!(
            MessageLink::parse("https://example.com/channels/1/2/3"),
            None
        );
        assert_eq!(
            MessageLink::parse("https://discord.com/channels/1/2/3/4"),
            None
        );
        assert_eq!(
            MessageLink::parse("https://discord.com/channels/1/0/3"),
            None
        );
    }
}
