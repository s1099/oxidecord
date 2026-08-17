//! The gateway websocket connection, which delivers live events.

use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_model::gateway::payload::incoming::MessageCreate;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::platform::runtime;

use super::model::{Message, convert_message};

/// A message received live over the gateway, tagged with the channel it
/// belongs to so the UI can decide whether it's for the open conversation.
pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    pub message: Message,
}

/// Opens a gateway websocket connection and invokes `on_message` for every
/// `MESSAGE_CREATE` dispatch. Returning `false` from it (the receiving end went
/// away) ends the shard loop and drops the socket.
///
/// The shard reconnects and resumes on its own, so transient errors are
/// skipped rather than treated as fatal.
pub fn connect_gateway(
    token: String,
    mut on_message: impl FnMut(IncomingMessage) -> bool + Send + 'static,
) {
    runtime::handle().spawn(async move {
        // Discord ignores the intents field for user tokens; a real user
        // client receives every event its account can see. Request all intents
        // so we mirror that and never filter events at this layer (the shard
        // still requires a value in the IDENTIFY payload).
        let mut shard = Shard::new(ShardId::ONE, token, Intents::all());

        while let Some(item) = shard.next_event(EventTypeFlags::MESSAGE_CREATE).await {
            let event = match item {
                Ok(event) => event,
                // Reconnects/resumes are handled by the shard internally; a
                // receive error just means skip this one and keep listening.
                Err(_) => continue,
            };

            if let Event::MessageCreate(message) = event {
                let MessageCreate { message, .. } = *message;
                let incoming = IncomingMessage {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                };
                if !on_message(incoming) {
                    break;
                }
            }
        }
    });
}
