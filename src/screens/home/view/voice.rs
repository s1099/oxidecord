//! Call chrome: the sidebar's connected panel, the stage of participant tiles,
//! and the control bar under it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, h_flex, v_flex,
};

use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{VoiceCall, VoiceKind, VoiceParticipant, VoiceStatus};

/// Height of the call band shown above a DM conversation. A voice channel's
/// stage fills its pane instead, since there's no chat under it.
const DM_STAGE_HEIGHT: f32 = 280.;

/// Diameter of the avatar on a participant tile.
const TILE_AVATAR: f32 = 72.;

impl HomeScreen {
    /// The sidebar footer: the call panel when there's a call, the account
    /// panel always.
    pub(super) fn render_sidebar_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_shrink_0()
            .w_full()
            .children(self.render_voice_panel(cx))
            .child(self.render_user_panel(cx))
    }

    /// The strip that sits above the account panel while a call is up: where
    /// the call is, and the ways out of it.
    fn render_voice_panel(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let call = self.voice.as_ref()?;
        let theme = cx.theme();
        let connected = call.status == VoiceStatus::Connected;
        let status_color = if connected {
            theme.success
        } else {
            theme.muted_foreground
        };

        Some(
            v_flex()
                .w_full()
                .px_2()
                .py_2()
                .gap_2()
                .border_t_1()
                .border_color(theme.sidebar_border)
                .bg(theme.sidebar_accent.opacity(0.3))
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
                                        .child(call.status.label()),
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
                                .icon(Icon::default().path(if call.camera {
                                    "icons/video.svg"
                                } else {
                                    "icons/video-off.svg"
                                }))
                                .ghost()
                                .small()
                                .flex_1()
                                .selected(call.camera)
                                .tooltip("Turn Camera On/Off")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.toggle_voice_camera(cx)),
                                ),
                        )
                        .child(
                            Button::new("voice-panel-share")
                                .icon(Icon::default().path("icons/screen-share.svg"))
                                .ghost()
                                .small()
                                .flex_1()
                                .selected(call.screen_share)
                                .tooltip("Share Your Screen")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.toggle_screen_share(cx)),
                                ),
                        ),
                ),
        )
    }

    /// A voice channel's pane: the stage when the user is in it, an invitation
    /// to join when they aren't.
    pub(super) fn render_voice_stage(&self, cx: &Context<Self>) -> AnyElement {
        let Some(call) = self
            .voice
            .as_ref()
            .filter(|call| Some(call.channel_id) == self.selected_channel)
        else {
            return self.render_join_prompt(cx).into_any_element();
        };

        self.stage(call, cx).flex_1().into_any_element()
    }

    /// The call band above a DM conversation, shown only while that DM's call
    /// is up.
    pub(super) fn render_dm_stage(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let call = self
            .voice
            .as_ref()
            .filter(|call| call.kind == VoiceKind::Direct)
            .filter(|call| Some(call.channel_id) == self.selected_channel)?;

        Some(
            self.stage(call, cx)
                .h(px(DM_STAGE_HEIGHT))
                .flex_shrink_0()
                .border_b_1()
                .border_color(cx.theme().border),
        )
    }

    /// The tiles and the control bar under them, shared by both stages.
    fn stage(&self, call: &VoiceCall, cx: &Context<Self>) -> Div {
        v_flex()
            .w_full()
            .min_h_0()
            .bg(cx.theme().muted.opacity(0.4))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .children(
                        call.participants
                            .iter()
                            .map(|participant| self.participant_tile(participant, cx)),
                    ),
            )
            .child(self.render_call_controls(call, cx))
    }

    /// One participant: their avatar, their name, and what they've muted.
    fn participant_tile(&self, participant: &VoiceParticipant, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        // The signed-in user's own mute and deafen live on the screen, not on
        // the participant, so they survive leaving and rejoining.
        let (muted, deafened) = if participant.is_self {
            (self.voice_muted, self.voice_deafened)
        } else {
            (participant.muted, participant.deafened)
        };

        let mut avatar = Avatar::new()
            .name(participant.name.clone())
            .with_size(px(TILE_AVATAR));
        if let Some(url) = participant.avatar_url.clone() {
            avatar = avatar.src(url);
        }

        v_flex()
            .w(px(180.))
            .h(px(150.))
            .gap_2()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(theme.background.opacity(0.6))
            .when(participant.pending, |this| this.opacity(0.6))
            .child(avatar)
            .child(
                h_flex()
                    .max_w_full()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(participant.name.clone()),
                    )
                    .when(muted, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/mic-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    })
                    .when(deafened, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/headphone-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    }),
            )
            .when(participant.pending, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Ringing…"),
                )
            })
    }

    /// The row of call actions under the stage.
    fn render_call_controls(&self, call: &VoiceCall, cx: &Context<Self>) -> impl IntoElement {
        let camera = call.camera;
        let sharing = call.screen_share;

        h_flex()
            .flex_shrink_0()
            .w_full()
            .py_3()
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                control(
                    "call-mute",
                    if self.voice_muted {
                        "icons/mic-off.svg"
                    } else {
                        "icons/mic.svg"
                    },
                    if self.voice_muted { "Unmute" } else { "Mute" },
                    self.voice_muted,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_mute(cx))),
            )
            .child(
                control(
                    "call-deafen",
                    if self.voice_deafened {
                        "icons/headphone-off.svg"
                    } else {
                        "icons/headphones.svg"
                    },
                    if self.voice_deafened {
                        "Undeafen"
                    } else {
                        "Deafen"
                    },
                    self.voice_deafened,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_deafen(cx))),
            )
            .child(
                control(
                    "call-camera",
                    if camera {
                        "icons/video.svg"
                    } else {
                        "icons/video-off.svg"
                    },
                    if camera {
                        "Turn Off Camera"
                    } else {
                        "Turn On Camera"
                    },
                    camera,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_camera(cx))),
            )
            .child(
                control(
                    "call-share",
                    "icons/screen-share.svg",
                    if sharing {
                        "Stop Sharing"
                    } else {
                        "Share Your Screen"
                    },
                    sharing,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_screen_share(cx))),
            )
            .child(
                Button::new("call-hangup")
                    .icon(Icon::default().path("icons/phone-off.svg"))
                    .danger()
                    .rounded(px(8.))
                    .tooltip("Disconnect")
                    .on_click(cx.listener(|this, _, _, cx| this.leave_voice(cx))),
            )
    }

    /// The empty state for a voice channel the user hasn't joined.
    fn render_join_prompt(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = self.selected_channel;
        let name = self
            .selected_channel_info()
            .map(|channel| channel.name.clone())
            .unwrap_or_default();

        v_flex()
            .flex_1()
            .gap_3()
            .items_center()
            .justify_center()
            .bg(theme.muted.opacity(0.4))
            .child(
                Icon::default()
                    .path("icons/volume-2.svg")
                    .size(px(40.))
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("No one is in {name} yet.")),
            )
            .child(
                Button::new("voice-join")
                    .icon(Icon::default().path("icons/phone.svg"))
                    .label("Join Voice")
                    .primary()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(id) = channel_id {
                            this.join_voice_channel(id, cx);
                        }
                    })),
            )
    }
}

/// One round toggle on the control bar, lit while its thing is on.
fn control(id: &'static str, icon: &'static str, tooltip: &'static str, active: bool) -> Button {
    Button::new(id)
        .icon(Icon::default().path(icon))
        .ghost()
        .rounded(px(8.))
        .selected(active)
        .tooltip(tooltip)
}

/// Where the call is, as the sidebar panel labels it: `Guild / channel` for a
/// voice channel, the other person's name for a DM call.
fn call_location(call: &VoiceCall) -> String {
    match (&call.context, call.kind) {
        (Some(guild), VoiceKind::Channel) => format!("{guild} / {}", call.name),
        _ => call.name.clone(),
    }
}
