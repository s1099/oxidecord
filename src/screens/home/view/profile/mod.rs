//! The profile popout: the small card Discord shows when you click someone's
//! avatar, opened here from a message's avatar in the chat.

mod card;

pub(super) use card::CARD_WIDTH;

use gpui_kit::*;

use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Deferred so it paints above the panes it overlaps; opened from inside the
    /// message list, it would otherwise be clipped and occluded.
    pub(in crate::screens::home) fn render_profile_popup(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let popup = self.profile_popup.as_ref()?;

        let card = div()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_profile_card(popup, cx));

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    // Consuming the press means clicking the avatar again toggles
                    // rather than closing and reopening.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_profile(cx);
                        }),
                    )
                    .child(
                        anchored()
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
