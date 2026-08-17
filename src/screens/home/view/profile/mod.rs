//! The profile popout: the small card Discord shows when you click someone's
//! avatar, opened here from a message's avatar in the chat.

mod card;

use gpui::*;

use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// The open profile card, rendered over the whole app: a transparent layer
    /// that dismisses on a click outside, with the card anchored where the
    /// avatar was clicked.
    ///
    /// Deferred so it paints above the panes it overlaps — the card is opened
    /// from inside the message list, which would otherwise clip and occlude it.
    pub(in crate::screens::home) fn render_profile_popup(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let popup = self.profile_popup.as_ref()?;

        let card = div()
            .occlude()
            // Clicks inside the card are for the card, not the dismiss layer
            // it sits on.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_profile_card(popup, cx));

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    // The layer sees a press before whatever is under it, and
                    // consuming it means a click outside only dismisses the
                    // card — so clicking the avatar again reads as a toggle
                    // rather than a close-and-reopen.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_profile(cx);
                        }),
                    )
                    .child(
                        anchored()
                            // Offset clear of the avatar so the card opens
                            // beside it rather than under the cursor.
                            .position(popup.position + point(px(24.), px(-12.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
