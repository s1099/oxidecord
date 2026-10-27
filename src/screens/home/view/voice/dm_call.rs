//! The call band above a DM conversation: everyone in the call as a bubble,
//! ringed while they talk, with the call's controls under them — or the way
//! in, when the call is going on without the user.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::screens::home::voice::VoiceParticipant;
use crate::ui::button::Button;
use crate::ui::tooltip;

/// Share of the conversation's height, like Discord's.
const BAND_SHARE: f32 = 0.5;
const BAND_MIN_HEIGHT: f32 = 240.;

const BUBBLE: f32 = 96.;
const RING_GAP: f32 = 3.;

struct Bubble {
    user_id: Id<UserMarker>,
    name: String,
    avatar_url: Option<String>,
    speaking: bool,
    muted: bool,
    deafened: bool,
    /// Rung but not yet in the call, so drawn faded.
    ringing: bool,
}

impl From<&VoiceParticipant> for Bubble {
    fn from(participant: &VoiceParticipant) -> Self {
        Self {
            user_id: participant.user_id,
            name: participant.name.clone(),
            avatar_url: participant.avatar_url.clone(),
            speaking: participant.speaking,
            muted: participant.muted,
            deafened: participant.deafened,
            ringing: false,
        }
    }
}

impl HomeScreen {
    pub(in crate::screens::home) fn render_dm_stage(
        &self,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let dm = self.selected_dm_info()?;
        let call = self.voice.as_ref().filter(|call| call.channel_id == dm.id);
        let participants = self.voice_participants(dm.id);
        if call.is_none() && participants.is_empty() {
            return None;
        }

        let band = match call {
            // Streams need the room tiles give them, so a call with one in it
            // falls back to the guild stage's layout.
            Some(call)
                if self.watching.is_some()
                    || participants.iter().any(|participant| participant.streaming) =>
            {
                self.stage(call, cx)
            }
            Some(call) => {
                let status = participants.is_empty().then(|| call.status.label());
                self.bubble_stage(dm, &participants, status, cx)
                    .child(self.render_call_controls(cx))
            }
            None => self
                .bubble_stage(dm, &participants, None, cx)
                .child(self.render_join_dm_call(dm, cx)),
        };

        Some(
            band.flex_shrink_0()
                .h(relative(BAND_SHARE))
                .min_h(px(BAND_MIN_HEIGHT))
                // Black in every theme, as Discord's is.
                .bg(black())
                .border_b_1()
                .border_color(cx.theme().border),
        )
    }

    fn bubble_stage(
        &self,
        dm: &DirectMessage,
        participants: &[VoiceParticipant],
        status: Option<&'static str>,
        cx: &App,
    ) -> Div {
        let mut bubbles: Vec<Bubble> = participants.iter().map(Bubble::from).collect();
        bubbles.extend(
            self.call_ringing(dm.id)
                .into_iter()
                .filter_map(|user_id| self.rung_bubble(dm, user_id)),
        );

        v_flex().w_full().min_h_0().child(
            div()
                .flex_1()
                .min_h_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .when_some(status, |this, status| {
                    this.child(div().text_color(white().opacity(0.7)).child(status))
                })
                .when(status.is_none(), |this| {
                    this.child(
                        h_flex()
                            .flex_wrap()
                            .justify_center()
                            .gap_6()
                            .children(bubbles.into_iter().map(|b| bubble(b, cx))),
                    )
                }),
        )
    }

    /// Only people the app can name get one: the other side of a 1:1 DM, the user, and anyone whose
    /// profile has been opened.
    fn rung_bubble(&self, dm: &DirectMessage, user_id: Id<UserMarker>) -> Option<Bubble> {
        let (name, avatar_url) = if dm.recipient_id == Some(user_id) {
            (dm.name.clone(), dm.avatar_url.clone())
        } else if Some(user_id) == self.self_user_id {
            let user = self.current_user.as_ref()?;
            (user.name.clone(), user.avatar_url.clone())
        } else {
            let profile = self.profile_cache.get(&user_id)?;
            (profile.name.clone(), profile.avatar_url.clone())
        };

        Some(Bubble {
            user_id,
            name,
            avatar_url,
            speaking: false,
            muted: false,
            deafened: false,
            ringing: true,
        })
    }

    fn render_join_dm_call(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let rung = self
            .self_user_id
            .is_some_and(|self_id| self.call_ringing(dm.id).contains(&self_id));

        h_flex()
            .flex_shrink_0()
            .w_full()
            .py_3()
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                Button::new("dm-call-join")
                    .icon(Icon::default().path("icons/phone.svg"))
                    .label("Join Call")
                    .primary()
                    .large()
                    .on_click(cx.listener(|this, _, _, cx| this.start_dm_call(cx))),
            )
            .when(rung, |this| {
                this.child(
                    Button::new("dm-call-decline")
                        .icon(Icon::default().path("icons/phone-off.svg"))
                        .label("Decline")
                        .danger()
                        .large()
                        .on_click(cx.listener(|this, _, _, cx| this.decline_dm_call(cx))),
                )
            })
    }
}

fn bubble(bubble: Bubble, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let ring = px(BUBBLE + (RING_GAP + 2.) * 2.);
    let silenced = if bubble.deafened {
        Some("icons/headphone-off.svg")
    } else if bubble.muted {
        Some("icons/mic-off.svg")
    } else {
        None
    };

    div()
        .id(SharedString::from(format!(
            "dm-call-bubble-{}",
            bubble.user_id
        )))
        .relative()
        .flex_shrink_0()
        .size(ring)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        // Always drawn, transparent when quiet, so starting to talk doesn't
        // shift the bubble.
        .border_2()
        .border_color(if bubble.speaking {
            theme.success
        } else {
            transparent_black()
        })
        .when(bubble.ringing, |this| this.opacity(0.4))
        .tooltip(tooltip::text(bubble.name.clone()))
        .child(avatar(bubble.name, bubble.avatar_url, px(BUBBLE)))
        .when_some(silenced, |this, icon| {
            this.child(
                div()
                    .absolute()
                    .bottom(px(RING_GAP))
                    .right(px(RING_GAP))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(black())
                    .child(Icon::default().path(icon).size_4().text_color(theme.danger)),
            )
        })
}
