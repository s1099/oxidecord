//! Slash commands: fetching the indexes that list them, and running them as
//! interactions.

use std::time::Duration;

use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use super::request;
use crate::discord::model::{AppCommand, CommandIndex, RawCommandIndex, convert_command_index};

/// Which index to fetch: a guild's, a bot DM's, or the user's own installed apps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CommandIndexScope {
    Guild(Id<GuildMarker>),
    Channel(Id<ChannelMarker>),
    User,
}

/// Discord answers 202 while it builds an index, and its client asks again
/// after five seconds, three times at most.
const INDEX_ATTEMPTS: usize = 3;
const INDEX_RETRY: Duration = Duration::from_secs(5);

pub async fn fetch_command_index(scope: CommandIndexScope) -> Result<CommandIndex, String> {
    request(move |client| async move {
        let path = match scope {
            CommandIndexScope::Guild(guild_id) => {
                format!("guilds/{guild_id}/application-command-index")
            }
            CommandIndexScope::Channel(channel_id) => {
                format!("channels/{channel_id}/application-command-index")
            }
            CommandIndexScope::User => "users/@me/application-command-index".to_owned(),
        };
        for _ in 0..INDEX_ATTEMPTS {
            let request = RequestBuilder::raw(Method::Get, path.clone()).build()?;
            let response = client.request::<RawCommandIndex>(request).await?;
            if response.status().get() == 202 {
                tokio::time::sleep(INDEX_RETRY).await;
                continue;
            }
            return Ok(convert_command_index(response.model().await?));
        }
        anyhow::bail!("Discord is still preparing this list of commands")
    })
    .await
}

/// An option's value as sent: a string, number or bool, or an id as a string.
pub struct CommandArgument {
    pub name: String,
    pub kind: u8,
    pub value: serde_json::Value,
    pub focused: bool,
}

pub struct CommandInvocation {
    pub command: AppCommand,
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Id<ChannelMarker>,
    /// The gateway session's; Discord delivers ephemeral replies to it alone.
    pub session_id: String,
    pub nonce: u64,
    pub arguments: Vec<CommandArgument>,
}

/// Runs a command (`POST /interactions`, type 2). Discord answers 204 and the
/// outcome follows on the gateway, matched by `nonce`.
pub async fn run_command(invocation: CommandInvocation) -> Result<(), String> {
    post_interaction(invocation, 2).await
}

/// Asks the app for choices for the focused option (type 4); they arrive as an
/// `APPLICATION_COMMAND_AUTOCOMPLETE_RESPONSE` with the same nonce.
pub async fn request_autocomplete(invocation: CommandInvocation) -> Result<(), String> {
    post_interaction(invocation, 4).await
}

async fn post_interaction(invocation: CommandInvocation, kind: u8) -> Result<(), String> {
    let body = interaction_body(&invocation, kind);
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, "interactions".to_owned())
            .json(&body)
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

fn interaction_body(invocation: &CommandInvocation, kind: u8) -> serde_json::Value {
    let command = &invocation.command;
    let mut options: Vec<_> = invocation
        .arguments
        .iter()
        .map(|argument| {
            let mut option = serde_json::json!({
                "type": argument.kind,
                "name": argument.name,
                "value": argument.value,
            });
            if argument.focused {
                option["focused"] = true.into();
            }
            option
        })
        .collect();
    // Wrapped from the innermost subcommand out.
    for (name, kind) in command.path.iter().rev() {
        options = vec![serde_json::json!({
            "type": *kind as u8,
            "name": name,
            "options": options,
        })];
    }
    let mut data = serde_json::json!({
        "version": command.version,
        "id": command.id,
        "name": command.root_name,
        "type": 1,
        "options": options,
        "application_command": *command.raw,
        "attachments": [],
    });
    if let Some(guild_id) = command.guild_id {
        data["guild_id"] = guild_id.to_string().into();
    }
    let mut body = serde_json::json!({
        "type": kind,
        "application_id": command.app.id,
        "channel_id": invocation.channel_id,
        "session_id": invocation.session_id,
        "data": data,
        "nonce": invocation.nonce.to_string(),
    });
    if let Some(guild_id) = invocation.guild_id {
        body["guild_id"] = guild_id.to_string().into();
    }
    if kind == 2 {
        body["analytics_location"] = "slash_ui".into();
    }
    body
}
