//! Message and channel links ("Copy Message Link", "Copy Channel Link"), opened
//! in the app rather than the browser.

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageLink {
    /// `None` for a direct message, whose links say `@me` in its place.
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Id<ChannelMarker>,
    /// `None` for a link to the channel itself.
    pub message_id: Option<Id<MessageMarker>>,
}

impl MessageLink {
    /// The link on discord.com, for the browser when the app can't open it;
    /// a `discord://` one would launch another client instead.
    pub fn web_url(&self) -> String {
        let guild = self
            .guild_id
            .map_or_else(|| "@me".to_string(), |id| id.to_string());
        let message = self
            .message_id
            .map_or_else(String::new, |id| format!("/{id}"));
        format!(
            "https://discord.com/channels/{guild}/{}{message}",
            self.channel_id
        )
    }

    /// Reads `https://discord.com/channels/<guild or @me>/<channel>[/<message>]`
    /// on any host Discord's clients have produced links with, and the
    /// `discord://` deep links apps use, which would otherwise launch
    /// whatever client the OS has registered for them.
    pub fn parse(url: &str) -> Option<Self> {
        let path = if let Some(rest) = url.strip_prefix("discord://") {
            // `discord://-/channels/…`, `discord:///channels/…` or with a host.
            let (_, path) = rest.split_once('/')?;
            path
        } else {
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
            path
        };
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
        let message_id = match parts.next() {
            Some(message) => Some(Id::new_checked(message.parse().ok()?)?),
            None => None,
        };
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
                message_id: Some(Id::new(3)),
            })
        );
        assert_eq!(
            MessageLink::parse("https://canary.discord.com/channels/@me/2/3?x=1"),
            Some(MessageLink {
                guild_id: None,
                channel_id: Id::new(2),
                message_id: Some(Id::new(3)),
            })
        );
    }

    #[test]
    fn channel_links() {
        assert_eq!(
            MessageLink::parse("https://discord.com/channels/1/2/"),
            Some(MessageLink {
                guild_id: Some(Id::new(1)),
                channel_id: Id::new(2),
                message_id: None,
            })
        );
        assert_eq!(
            MessageLink::parse("discord://-/channels/@me/2"),
            Some(MessageLink {
                guild_id: None,
                channel_id: Id::new(2),
                message_id: None,
            })
        );
    }

    #[test]
    fn deep_links() {
        let link = Some(MessageLink {
            guild_id: Some(Id::new(1)),
            channel_id: Id::new(2),
            message_id: Some(Id::new(3)),
        });
        assert_eq!(MessageLink::parse("discord://-/channels/1/2/3"), link);
        assert_eq!(MessageLink::parse("discord:///channels/1/2/3"), link);
        assert_eq!(
            MessageLink::parse("discord://discord.com/channels/1/2/3"),
            link
        );
    }

    #[test]
    fn not_message_links() {
        assert_eq!(MessageLink::parse("https://discord.com/channels/1"), None);
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
