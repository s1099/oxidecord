//! Finding, starting, joining and closing threads.

use std::collections::HashMap;
use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use super::request;
use crate::discord::model::{Membership, Thread, convert_thread};

/// Most a search page holds, and what Discord's client asks for.
pub const THREAD_PAGE_SIZE: usize = 25;

/// A fresh index answers 202 until it's built; this many tries is a few seconds' wait.
const INDEXING_RETRIES: usize = 3;
const MAX_INDEXING_WAIT: Duration = Duration::from_secs(5);

/// Discord's default: three days without a message.
const AUTO_ARCHIVE_MINUTES: u32 = 4320;

pub struct ThreadPage {
    /// Membership is known for each: the page says which the user is in.
    pub threads: Vec<Thread>,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct ThreadSearch {
    /// `None` searches both.
    pub archived: Option<bool>,
    pub name: Option<String>,
    pub offset: usize,
}

/// One body for both answers: a 202 carries `retry_after` and no threads.
#[derive(Deserialize)]
struct RawThreadSearch {
    #[serde(default)]
    threads: Vec<serde_json::Value>,
    #[serde(default)]
    members: Vec<RawMember>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    retry_after: Option<f64>,
}

#[derive(Deserialize)]
struct RawMember {
    #[serde(default)]
    id: Option<Id<ChannelMarker>>,
    join_timestamp: twilight_model::util::Timestamp,
}

/// `GET /channels/{id}/threads/search`, the user-account way to list a channel's
/// threads, most recently active first, as Discord's threads popout does.
pub async fn search_threads(
    channel_id: Id<ChannelMarker>,
    search: ThreadSearch,
) -> Result<ThreadPage, String> {
    request(move |client| async move {
        let mut query = format!(
            "sort_by=last_message_time&sort_order=desc&limit={THREAD_PAGE_SIZE}&offset={}",
            search.offset
        );
        if let Some(archived) = search.archived {
            query.push_str(&format!("&archived={archived}"));
        }
        if let Some(name) = search.name.as_deref().filter(|name| !name.is_empty()) {
            query.push_str(&format!(
                "&name={}",
                utf8_percent_encode(name, NON_ALPHANUMERIC)
            ));
        }
        let mut tries = 0;
        let raw = loop {
            let request = RequestBuilder::raw(
                Method::Get,
                format!("channels/{channel_id}/threads/search?{query}"),
            )
            .build()?;
            let raw = client
                .request::<RawThreadSearch>(request)
                .await?
                .model()
                .await?;
            match raw.retry_after {
                Some(wait) if tries < INDEXING_RETRIES => {
                    tries += 1;
                    let wait = Duration::from_secs_f64(wait.max(0.5)).min(MAX_INDEXING_WAIT);
                    tokio::time::sleep(wait).await;
                }
                _ => break raw,
            }
        };
        let joined: HashMap<_, _> = raw
            .members
            .iter()
            .filter_map(|member| Some((member.id?, member.join_timestamp.as_secs())))
            .collect();
        let threads = raw
            .threads
            .into_iter()
            .filter_map(|thread| serde_json::from_value(thread).ok())
            .filter_map(convert_thread)
            .map(|mut thread| {
                thread.membership = joined
                    .get(&thread.id)
                    .map_or(Membership::NotJoined, |&at| Membership::Joined(at));
                thread
            })
            .collect();
        Ok(ThreadPage {
            threads,
            has_more: raw.has_more,
        })
    })
    .await
}

/// `GET /channels/{id}`: a thread the client hasn't been sent, such as an
/// archived one a link or a notice points at. `None` if it isn't a thread.
pub async fn fetch_thread(thread_id: Id<ChannelMarker>) -> Result<Option<Thread>, String> {
    request(move |client| async move {
        let channel = client.channel(thread_id).await?.model().await?;
        Ok(convert_thread(channel))
    })
    .await
}

/// `POST /channels/{id}/messages/{id}/threads`. The thread takes the message's id,
/// so a message can start only one.
pub async fn start_thread_from_message(
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    name: String,
) -> Result<Thread, String> {
    let path = format!("channels/{channel_id}/messages/{message_id}/threads");
    create(
        path,
        serde_json::json!({
            "name": name,
            "auto_archive_duration": AUTO_ARCHIVE_MINUTES,
            "location": "Message",
        }),
    )
    .await
}

/// `POST /channels/{id}/threads`: a thread with no message behind it.
pub async fn start_thread(
    channel_id: Id<ChannelMarker>,
    name: String,
    private: bool,
) -> Result<Thread, String> {
    let path = format!("channels/{channel_id}/threads");
    create(
        path,
        serde_json::json!({
            "name": name,
            // Public and private threads.
            "type": if private { 12 } else { 11 },
            "auto_archive_duration": AUTO_ARCHIVE_MINUTES,
            "location": "Thread Browser Toolbar",
        }),
    )
    .await
}

async fn create(path: String, body: serde_json::Value) -> Result<Thread, String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, path)
            .json(&body)
            .build()?;
        let channel = client
            .request::<twilight_model::channel::Channel>(request)
            .await?
            .model()
            .await?;
        let mut thread = convert_thread(channel)
            .ok_or_else(|| anyhow::anyhow!("Discord didn't answer with a thread"))?;
        // The creator is added; the response doesn't always say so.
        if thread.membership == Membership::Unknown {
            thread.membership = Membership::Joined(thread.created_at());
        }
        Ok(thread)
    })
    .await
}

/// `PUT` or `DELETE /channels/{id}/thread-members/@me`.
pub async fn set_thread_joined(thread_id: Id<ChannelMarker>, joined: bool) -> Result<(), String> {
    request(move |client| async move {
        let method = if joined { Method::Put } else { Method::Delete };
        let request =
            RequestBuilder::raw(method, format!("channels/{thread_id}/thread-members/@me"))
                .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// `PATCH /channels/{id}`. Archiving takes ownership or `MANAGE_THREADS`;
/// reopening one that isn't locked only takes being able to send in it.
pub async fn set_thread_archived(
    thread_id: Id<ChannelMarker>,
    archived: bool,
) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Patch, format!("channels/{thread_id}"))
            .json(&serde_json::json!({ "archived": archived }))
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_202_body_carries_no_threads() {
        let raw: RawThreadSearch = serde_json::from_str(
            r#"{"message":"Index not yet available. Try again later","code":110000,"documents_indexed":0,"retry_after":2}"#,
        )
        .unwrap();
        assert!(raw.threads.is_empty());
        assert_eq!(raw.retry_after, Some(2.));
    }
}
