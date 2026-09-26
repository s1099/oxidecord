//! The gateway connection that keeps the open conversation live, and the call
//! events that ride alongside it.

use gpui::*;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::voice::{VoiceEngine, VoiceEvent};

impl HomeScreen {
    /// Opens the gateway connection and pumps its events onto the gpui
    /// foreground, where they update the open conversation and the call.
    ///
    /// The call engine is started here too: a call is opened with a gateway
    /// command and answered with gateway dispatches, so neither half is of any
    /// use without the other.
    pub(in crate::screens::home) fn start_gateway(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::mpsc::unbounded::<discord::GatewayEvent>();
        let gateway = discord::connect_gateway(token, move |event| {
            // Returns whether the foreground receiver is still around; once it
            // isn't (the screen was dropped), the gateway loop stops.
            tx.unbounded_send(event).is_ok()
        });
        self.gateway = Some(gateway);

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_gateway_event(event, cx))
                    .is_err()
                {
                    // The entity is gone; stop draining so the sender closes.
                    break;
                }
            }
        })
        .detach();

        let (tx, rx) = futures::channel::mpsc::unbounded::<VoiceEvent>();
        let engine = VoiceEngine::start(tx);
        // Hand over the remembered microphone before any call can be made.
        engine.set_input_device(self.voice_input_device.clone());
        self.voice_engine = Some(engine);

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_voice_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_gateway_event(&mut self, event: discord::GatewayEvent, cx: &mut Context<Self>) {
        match event {
            discord::GatewayEvent::Ready { user_id } => {
                self.self_user_id = Some(user_id);
            }
            discord::GatewayEvent::Message(incoming) => self.handle_incoming_message(incoming, cx),
            discord::GatewayEvent::MessageUpdate(incoming) => {
                self.handle_message_update(incoming, cx)
            }
            discord::GatewayEvent::VoiceState(state) => self.handle_voice_state(state, cx),
            discord::GatewayEvent::VoiceServer(server) => self.handle_voice_server(server, cx),
        }
    }

    /// Appends a live message to the open conversation, if it belongs there.
    fn handle_incoming_message(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(incoming.channel_id) {
            return;
        }
        // The history for this channel is still loading and will replace the
        // whole list when it lands (and include this message), so skip it now
        // to avoid a desync between `messages` and the list state.
        if self.messages_loading {
            return;
        }
        // Ignore duplicates: the echo of a message we just sent ourselves, or a
        // repeated dispatch.
        if self
            .messages
            .iter()
            .any(|message| message.id == incoming.message.id)
        {
            return;
        }

        let ix = self.messages.len();
        self.messages.push(incoming.message);
        self.messages_list.splice(ix..ix, 1);
        // Follow the conversation only when the newest message was already in
        // view; if the user has scrolled up to read history, leave them there.
        if self.at_bottom {
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }
}
