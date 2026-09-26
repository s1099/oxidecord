//! Call chrome: the sidebar's connected panel, the stage of participant tiles,
//! and the control bar under it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    v_flex,
};

use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::screens::home::voice::{VoiceCall, VoiceParticipant, VoiceStatus};
use crate::ui::button::Button;
use crate::ui::depth::{self, radius};
use crate::voice;

/// Height of the call band shown above a DM conversation. A voice channel's
/// stage fills its pane instead, since there's no chat under it.
const DM_STAGE_HEIGHT: f32 = 280.;

/// Diameter of the avatar on a participant tile.
const TILE_AVATAR: f32 = 72.;

/// What the camera and screen-share buttons say. Both are drawn because the
/// call has a place for them, and disabled because nothing behind them sends
/// video yet.
const VIDEO_UNAVAILABLE: &str = "Video isn't supported yet";

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
        let status_color = if call.error.is_some() {
            theme.danger
        } else if connected {
            theme.success
        } else {
            theme.muted_foreground
        };

        // A card of its own above the account panel, so a live call stands out
        // from the sidebar it sits in.
        Some(
            div().px_2().child(
                depth::card(radius::CONTROL, cx)
                    .w_full()
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
                                    .tooltip(VIDEO_UNAVAILABLE),
                            )
                            .child(
                                Button::new("voice-panel-share")
                                    .icon(Icon::default().path("icons/screen-share.svg"))
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .disabled(true)
                                    .tooltip(VIDEO_UNAVAILABLE),
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

        // Fills the pane down to the panel's rounded corners, which gpui
        // won't clip to.
        self.stage(call, cx)
            .flex_1()
            .rounded_b(radius::CARD - px(1.))
            .into_any_element()
    }

    /// The call band above a DM conversation, shown only while that DM's call
    /// is up.
    pub(super) fn render_dm_stage(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let call = self
            .voice
            .as_ref()
            .filter(|call| call.guild_id.is_none())
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
        let theme = cx.theme();
        let participants = self.voice_participants(call.channel_id);
        // Until the voice state for the join comes back there's nobody to
        // draw — say what's happening rather than show an empty stage.
        let waiting = participants.is_empty();

        v_flex()
            .w_full()
            .min_h_0()
            .bg(theme.muted.opacity(0.4))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .when(waiting, |this| {
                        this.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(call.status.label()),
                        )
                    })
                    .children(
                        participants
                            .iter()
                            .map(|participant| self.participant_tile(participant, cx)),
                    ),
            )
            .child(self.render_call_controls(cx))
    }

    /// One participant: their avatar, their name, and what they've silenced.
    fn participant_tile(&self, participant: &VoiceParticipant, cx: &Context<Self>) -> Div {
        let theme = cx.theme();

        let avatar = avatar(
            participant.name.clone(),
            participant.avatar_url.clone(),
            px(TILE_AVATAR),
        );

        depth::card(radius::CARD, cx)
            .flex()
            .flex_col()
            .w(px(180.))
            .h(px(150.))
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                // The speaking ring goes on a wrapper rather than the avatar,
                // so appearing and disappearing doesn't nudge the layout.
                div()
                    .rounded_full()
                    .border_2()
                    .p(px(2.))
                    .border_color(if participant.speaking {
                        theme.success
                    } else {
                        transparent_black()
                    })
                    .child(avatar),
            )
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
                    .when(participant.muted, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/mic-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    })
                    .when(participant.deafened, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/headphone-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    }),
            )
    }

    /// The row of call actions under the stage.
    fn render_call_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .w_full()
            .py_3()
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                self.with_mic_menu(
                    "call-mic-menu",
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
                    cx,
                ),
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
                    "icons/video-off.svg",
                    VIDEO_UNAVAILABLE,
                    false,
                )
                .disabled(true),
            )
            .child(
                control(
                    "call-share",
                    "icons/screen-share.svg",
                    VIDEO_UNAVAILABLE,
                    false,
                )
                .disabled(true),
            )
            .child(
                Button::new("call-hangup")
                    .icon(Icon::default().path("icons/phone-off.svg"))
                    .danger()
                    .large()
                    .tooltip("Disconnect")
                    .on_click(cx.listener(|this, _, _, cx| this.leave_voice(cx))),
            )
    }

    /// The empty state for a voice channel the user hasn't joined: whoever is
    /// already in there, and the way in.
    fn render_join_prompt(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = self.selected_channel;
        let name = self
            .selected_channel_info()
            .map(|channel| channel.name.clone())
            .unwrap_or_default();
        let participants = channel_id
            .map(|id| self.voice_participants(id))
            .unwrap_or_default();

        v_flex()
            .flex_1()
            .gap_3()
            .items_center()
            .justify_center()
            .bg(theme.muted.opacity(0.4))
            .rounded_b(radius::CARD - px(1.))
            .when(participants.is_empty(), |this| {
                this.child(
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
            })
            .when(!participants.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap_4()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .children(
                            participants
                                .iter()
                                .map(|participant| self.participant_tile(participant, cx)),
                        ),
                )
            })
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

/// Puts the microphone picker behind a right-click on `button`.
///
/// The menu wraps the button rather than being hung off it: a `Button` with
/// children lays itself out as a labelled button, which would stretch an icon
/// one out of shape.
impl HomeScreen {
    pub(super) fn with_mic_menu(
        &self,
        id: &'static str,
        button: impl IntoElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let screen = cx.entity().downgrade();

        div().id(id).child(button).context_menu(move |menu, _, cx| {
            let Some(screen) = screen.upgrade() else {
                return menu;
            };
            let chosen = screen.read(cx).voice_input_device.clone();

            // Enumerated as the menu opens, so a microphone plugged in since
            // the app started is in the list.
            let devices: Vec<voice::InputDevice> = voice::input_devices();
            let menu = menu.label("Input Device").item(device_item(
                "System Default",
                None,
                chosen.is_none(),
                &screen,
            ));

            devices.into_iter().fold(menu, |menu, device| {
                let selected = chosen.as_deref() == Some(device.id.as_str());
                menu.item(device_item(device.name, Some(device.id), selected, &screen))
            })
        })
    }
}

/// One microphone in the picker, ticked when it's the one in use.
fn device_item(
    label: impl Into<SharedString>,
    id: Option<String>,
    selected: bool,
    screen: &Entity<HomeScreen>,
) -> PopupMenuItem {
    let screen = screen.downgrade();
    let item = PopupMenuItem::new(label.into()).on_click(move |_, _, cx| {
        if let Some(screen) = screen.upgrade() {
            screen.update(cx, |this, cx| this.set_input_device(id.clone(), cx));
        }
    });

    if selected {
        item.icon(Icon::new(IconName::Check))
    } else {
        item
    }
}

/// One square toggle on the control bar, pushed in while its thing is on.
fn control(id: &'static str, icon: &'static str, tooltip: &'static str, active: bool) -> Button {
    Button::new(id)
        .icon(Icon::default().path(icon))
        .outline()
        .large()
        .selected(active)
        .tooltip(tooltip)
}

/// Where the call is, as the sidebar panel labels it: `Guild / channel` for a
/// voice channel, the other person's name for a DM call.
fn call_location(call: &VoiceCall) -> String {
    match &call.context {
        Some(guild) => format!("{guild} / {}", call.name),
        None => call.name.clone(),
    }
}
