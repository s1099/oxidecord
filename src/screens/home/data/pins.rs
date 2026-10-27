//! Opening the pinned-messages popout and loading the pins behind it.

use gpui_kit::*;

use crate::discord;
use crate::screens::home::{HomeScreen, PinsPopup};

impl HomeScreen {
    /// Fetched fresh on every opening: nothing tells the client when someone else
    /// pins or unpins.
    pub(in crate::screens::home) fn toggle_pins(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.pins_popup.is_some() {
            return self.close_pins(cx);
        }
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        self.pins_popup = Some(PinsPopup {
            channel_id,
            position,
            messages: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(256.)),
            loading: true,
            error: None,
        });
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_pins(channel_id).await;
            let _ = this.update(cx, |this, cx| {
                // Closed, or reopened somewhere else, while in flight.
                let Some(popup) = this
                    .pins_popup
                    .as_mut()
                    .filter(|popup| popup.channel_id == channel_id && popup.loading)
                else {
                    return;
                };
                popup.loading = false;
                match result {
                    Ok(messages) => {
                        popup.list.reset(messages.len());
                        popup.messages = messages;
                    }
                    Err(err) => popup.error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn close_pins(&mut self, cx: &mut Context<Self>) {
        if self.pins_popup.take().is_some() {
            cx.notify();
        }
    }
}
