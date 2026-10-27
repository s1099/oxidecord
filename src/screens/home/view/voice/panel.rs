//! The call panel above the account panel in the sidebar.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{VoiceCall, VoiceStatus};
use crate::ui::button::Button;
use crate::ui::depth::radius;

use super::CAMERA_UNAVAILABLE;

impl HomeScreen {
    /// The card under the rail and sidebar: the call strip when in a call, the account row always.
    /// One surface floating over both columns, with a margin of the window colour at its sides
    /// and bottom. None above, so the lists end where it starts.
    pub(in crate::screens::home) fn render_sidebar_footer(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        div().flex_shrink_0().w_full().px_2().pb_2().child(
            v_flex()
                .w_full()
                .overflow_hidden()
                .rounded(radius::CONTROL)
                .bg(theme.secondary)
                .text_color(theme.sidebar_foreground)
                .children(self.render_voice_panel(cx))
                .child(self.render_user_panel(cx)),
        )
    }

    fn render_voice_panel(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let call = self.voice.as_ref()?;
        let theme = cx.theme();
        let connected = call.status == VoiceStatus::Connected;
        let status_color = if call.error.is_some() {
            theme.danger
        } else if connected {
            theme.success
        } else {
            theme.muted_foreground
        };

        Some(
            div().child(
                div()
                    .w_full()
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .p_2()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::default()
                                    .path("icons/signal.svg")
                                    .size_4()
                                    .text_color(status_color),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(status_color)
                                            .child(match &call.error {
                                                Some(error) => SharedString::from(error.clone()),
                                                None => SharedString::from(call.status.label()),
                                            }),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(call_location(call)),
                                    ),
                            )
                            .child(
                                Button::new("voice-disconnect")
                                    .icon(Icon::default().path("icons/phone-off.svg"))
                                    .ghost()
                                    .small()
                                    .tooltip("Disconnect")
                                    .on_click(cx.listener(|this, _, _, cx| this.leave_voice(cx))),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("voice-panel-camera")
                                    .icon(Icon::default().path("icons/video-off.svg"))
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .disabled(true)
                                    .tooltip(CAMERA_UNAVAILABLE),
                            )
                            .child(
                                Button::new("voice-panel-share")
                                    .icon(Icon::default().path("icons/screen-share.svg"))
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .selected(self.screen_share.is_some())
                                    .disabled(!self.share_button_enabled())
                                    .tooltip(self.share_tooltip())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_screen_share(window, cx)
                                    })),
                            ),
                    )
                    .children(self.render_share_status(call, cx)),
            ),
        )
    }

    fn render_share_status(
        &self,
        call: &VoiceCall,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let (label, detail, color) = match (&self.screen_share, &call.share_error) {
            (Some(share), _) if share.live => ("Live", share.source_name.clone(), theme.danger),
            (Some(share), _) => (
                "Starting",
                share.source_name.clone(),
                theme.muted_foreground,
            ),
            (None, Some(error)) => ("Stream stopped", error.clone(), theme.danger),
            (None, None) => match &call.watch_error {
                Some(error) => ("Couldn't watch", error.clone(), theme.danger),
                None => return None,
            },
        };

        Some(
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .child(
                    div()
                        .flex_shrink_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                ),
        )
    }

    /// The share button works while the call is up, and always works to stop
    /// a stream that's running.
    pub(super) fn share_button_enabled(&self) -> bool {
        self.screen_share.is_some() || self.can_share_screen()
    }

    pub(super) fn share_tooltip(&self) -> &'static str {
        if self.screen_share.is_some() {
            "Stop Streaming"
        } else if crate::platform::capture::is_supported() {
            "Share Your Screen"
        } else {
            "Screen sharing isn't supported on this system"
        }
    }
}

/// Where the call is, as the sidebar panel labels it: `Guild / channel` for a
/// voice channel, the other person's name for a DM call.
fn call_location(call: &VoiceCall) -> String {
    match &call.context {
        Some(guild) => format!("{guild} / {}", call.name),
        None => call.name.clone(),
    }
}
