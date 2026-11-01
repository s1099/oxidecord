//! Message search: the filters a search sends and the page of hits it gets back.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker},
};

use super::message::{Message, convert_message};

/// Discord's page size, and the most it returns at once.
pub const SEARCH_PAGE_SIZE: usize = 25;

/// Discord refuses offsets past this; later hits are out of reach by offset.
pub const SEARCH_MAX_OFFSET: usize = 9975;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchScope {
    Guild(Id<GuildMarker>),
    /// A DM or group DM.
    Channel(Id<ChannelMarker>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SearchSort {
    #[default]
    Newest,
    Oldest,
    Relevance,
}

/// What `has:` can ask for, under the names Discord's search box uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchHas {
    Link,
    Embed,
    File,
    Image,
    Video,
    Sound,
    Sticker,
    Poll,
    Forward,
}

impl SearchHas {
    pub const ALL: [Self; 9] = [
        Self::Link,
        Self::Embed,
        Self::Poll,
        Self::File,
        Self::Video,
        Self::Image,
        Self::Sound,
        Self::Sticker,
        Self::Forward,
    ];

    /// As typed after `has:`.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Embed => "embed",
            Self::File => "file",
            Self::Image => "image",
            Self::Video => "video",
            Self::Sound => "sound",
            Self::Sticker => "sticker",
            Self::Poll => "poll",
            Self::Forward => "forward",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|has| has.keyword().eq_ignore_ascii_case(keyword))
    }

    /// The API's name, which for a forward is the snapshot it carries.
    fn param(self) -> &'static str {
        match self {
            Self::Forward => "snapshot",
            other => other.keyword(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthorType {
    User,
    Bot,
    Webhook,
}

impl AuthorType {
    pub const ALL: [Self; 3] = [Self::User, Self::Bot, Self::Webhook];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Bot => "bot",
            Self::Webhook => "webhook",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.keyword().eq_ignore_ascii_case(keyword))
    }
}

/// A search's filters, every name already resolved to an id.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SearchFilters {
    pub content: String,
    pub authors: Vec<Id<UserMarker>>,
    pub mentions: Vec<Id<UserMarker>>,
    pub channels: Vec<Id<ChannelMarker>>,
    pub has: Vec<SearchHas>,
    pub author_types: Vec<AuthorType>,
    /// Snowflakes standing for a time: hits fall strictly between.
    pub min_id: Option<Id<MessageMarker>>,
    pub max_id: Option<Id<MessageMarker>>,
    pub pinned: Option<bool>,
    pub sort: SearchSort,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        let Self {
            content,
            authors,
            mentions,
            channels,
            has,
            author_types,
            min_id,
            max_id,
            pinned,
            sort: _,
        } = self;
        content.trim().is_empty()
            && authors.is_empty()
            && mentions.is_empty()
            && channels.is_empty()
            && has.is_empty()
            && author_types.is_empty()
            && min_id.is_none()
            && max_id.is_none()
            && pinned.is_none()
    }

    /// The query string, arrays as repeated keys. `encode` percent-encodes a value.
    pub(in crate::discord) fn query(
        &self,
        offset: usize,
        encode: impl Fn(&str) -> String,
    ) -> String {
        let mut params = vec![format!("limit={SEARCH_PAGE_SIZE}")];
        if offset > 0 {
            params.push(format!("offset={offset}"));
        }
        let content = self.content.trim();
        if !content.is_empty() {
            params.push(format!("content={}", encode(content)));
        }
        params.extend(self.authors.iter().map(|id| format!("author_id={id}")));
        params.extend(self.mentions.iter().map(|id| format!("mentions={id}")));
        params.extend(self.channels.iter().map(|id| format!("channel_id={id}")));
        params.extend(self.has.iter().map(|has| format!("has={}", has.param())));
        params.extend(
            self.author_types
                .iter()
                .map(|kind| format!("author_type={}", kind.keyword())),
        );
        params.extend(self.min_id.map(|id| format!("min_id={id}")));
        params.extend(self.max_id.map(|id| format!("max_id={id}")));
        params.extend(self.pinned.map(|pinned| format!("pinned={pinned}")));
        params.push(
            match self.sort {
                SearchSort::Newest => "sort_by=timestamp&sort_order=desc",
                SearchSort::Oldest => "sort_by=timestamp&sort_order=asc",
                SearchSort::Relevance => "sort_by=relevance",
            }
            .to_string(),
        );
        // Discord's own client leaves NSFW channels in; the server still drops
        // them for users it doesn't let see them.
        params.push("include_nsfw=true".to_string());
        params.join("&")
    }
}

pub struct SearchHit {
    pub channel_id: Id<ChannelMarker>,
    pub message: Message,
}

pub enum SearchResponse {
    Results {
        total: usize,
        hits: Vec<SearchHit>,
    },
    /// Discord is still indexing what was searched (a 202); try again after this
    /// many seconds.
    Indexing {
        retry_after: f64,
    },
}

/// One body for both answers: a 202 carries `retry_after` and no messages.
#[derive(Deserialize)]
pub(in crate::discord) struct RawSearch {
    #[serde(default)]
    total_results: usize,
    /// Each hit in an array of its own, once padded with context messages.
    #[serde(default)]
    messages: Option<Vec<Vec<twilight_model::channel::Message>>>,
    #[serde(default)]
    retry_after: Option<f64>,
}

pub(in crate::discord) fn convert_search(raw: RawSearch) -> SearchResponse {
    match raw.messages {
        Some(messages) => SearchResponse::Results {
            total: raw.total_results,
            hits: messages
                .into_iter()
                .filter_map(|group| group.into_iter().next())
                .map(|message| SearchHit {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                })
                .collect(),
        },
        None => SearchResponse::Indexing {
            retry_after: raw.retry_after.unwrap_or(0.),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_query_string() {
        let filters = SearchFilters {
            content: "hello world".into(),
            authors: vec![Id::new(1), Id::new(2)],
            has: vec![SearchHas::Image, SearchHas::Forward],
            pinned: Some(true),
            sort: SearchSort::Oldest,
            ..Default::default()
        };
        let query = filters.query(25, |value| value.replace(' ', "%20"));
        assert_eq!(
            query,
            "limit=25&offset=25&content=hello%20world&author_id=1&author_id=2\
             &has=image&has=snapshot&pinned=true&sort_by=timestamp&sort_order=asc\
             &include_nsfw=true"
        );
    }

    #[test]
    fn a_202_body_is_indexing() {
        let raw: RawSearch = serde_json::from_str(
            r#"{"message":"Index not yet available. Try again later","code":110000,"documents_indexed":0,"retry_after":2}"#,
        )
        .unwrap();
        assert!(matches!(
            convert_search(raw),
            SearchResponse::Indexing { retry_after } if retry_after == 2.
        ));
    }
}
