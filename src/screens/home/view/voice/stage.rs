//! The stage: where a call is shown, or the way into one.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, spinner::Spinner, tag::Tag, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{StreamWatch, VoiceCall, VoiceParticipant};
use crate::ui::button::Button;
use crate::ui::depth::radius;

use super::tiles::{STAGE_PADDING, TILE_GAP, fit_tiles};

const JOIN_TILE: Size<Pixels> = size(px(224.), px(126.));

impl HomeScreen {
    pub(in crate::screens::home) fn render_voice_stage(&self, cx: &Context<Self>) -> AnyElement {
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
            .map(|this| self.round_stage_corners(this))
            .into_any_element()
    }

    /// Rounds the stage into the panel's bottom corners (only the left while chat sits to its
    /// right).
    ///
    /// Also lets it shrink below its content: the tile grid is sized from the last measured width,
    /// which would otherwise push the chat out of the pane.
    fn round_stage_corners(&self, stage: Div) -> Div {
        let stage = stage.min_w_0();
        let corner = radius::CARD - px(1.);
        if self.voice_chat_open {
            stage.rounded_bl(corner)
        } else {
            stage.rounded_b(corner)
        }
    }

    pub(super) fn stage(&self, call: &VoiceCall, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let participants = self.voice_participants(call.channel_id);
        // Until the voice state for the join comes back there's nobody to
        // draw — say what's happening rather than show an empty stage.
        let waiting = participants.is_empty();

        let watched = self.watching.as_ref().and_then(|watch| {
            participants
                .iter()
                .find(|participant| participant.user_id == watch.streamer_id)
                .map(|streamer| (watch, streamer))
        });
        if let Some((watch, streamer)) = watched {
            return v_flex()
                .w_full()
                .min_h_0()
                .bg(theme.muted.opacity(0.4))
                .child(self.stream_view(watch, streamer, cx))
                .child(self.render_call_controls(cx));
        }

        // Discord gives a stream a tile of its own, ahead of everyone's
        // faces, so it reads as something to open rather than a badge.
        let tiles = participants.len() + participants.iter().filter(|p| p.streaming).count();
        let grid = fit_tiles(tiles, self.voice_stage_size);
        let screen = cx.entity().downgrade();

        v_flex()
            .w_full()
            .min_h_0()
            .bg(theme.muted.opacity(0.4))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .p(px(STAGE_PADDING))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                screen
                                    .update(cx, |this, cx| {
                                        if this.voice_stage_size != bounds.size {
                                            this.voice_stage_size = bounds.size;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .when(waiting, |this| {
                        this.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(call.status.label()),
                        )
                    })
                    .when(!waiting, |this| {
                        this.child(
                            h_flex()
                                .w(grid.row_width())
                                .flex_wrap()
                                .justify_center()
                                .gap(px(TILE_GAP))
                                .children(
                                    participants
                                        .iter()
                                        .filter(|participant| participant.streaming)
                                        .map(|participant| {
                                            self.stream_tile(participant, grid.size, cx)
                                        }),
                                )
                                .children(participants.iter().map(|participant| {
                                    self.participant_tile(participant, grid.size, cx)
                                })),
                        )
                    }),
            )
            .child(self.render_call_controls(cx))
    }

    fn stream_view(
        &self,
        watch: &StreamWatch,
        streamer: &VoiceParticipant,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let picture = match &watch.frame {
            // Raw pixels, so it never touches the image cache. Pinned to the box rather than laid
            // out in
            // it: in flow, `img` sized from the box's width alone via the picture's aspect ratio,
            // so a
            // wide stage cut the bottom off.
            Some(frame) => img(frame.clone())
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => v_flex()
                .absolute()
                .inset_0()
                .gap_2()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(white().opacity(0.7))
                .child(Spinner::new().large().color(white()))
                .child(format!("Joining {}'s stream…", streamer.name))
                .into_any_element(),
        };

        div().flex_1().min_h_0().p_4().flex().child(
            div()
                .relative()
                .size_full()
                .rounded(radius::CARD)
                .overflow_hidden()
                // Black rather than a theme colour: it's the letterbox around
                // the picture, and reads as part of it in either theme.
                .bg(black())
                .child(picture)
                .child(
                    h_flex()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .p_2()
                        .gap_2()
                        .items_center()
                        .child(Tag::danger().xsmall().child("LIVE"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(white())
                                .child(streamer.name.clone()),
                        )
                        .child(
                            Button::new("stream-stop-watching")
                                .icon(IconName::Close)
                                .ghost()
                                .small()
                                .tooltip("Stop Watching")
                                .on_click(cx.listener(|this, _, _, cx| this.stop_watching(cx))),
                        ),
                ),
        )
    }

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
            .map(|this| self.round_stage_corners(this))
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
                            participants.iter().map(|participant| {
                                self.participant_tile(participant, JOIN_TILE, cx)
                            }),
                        ),
                )
            })
            .child(
                Button::new("voice-join")
                    .icon(Icon::default().path("icons/phone.svg"))
                    .label("Join Voice")
                    .primary()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(id) = channel_id {
                            this.join_voice_channel(id, window, cx);
                        }
                    })),
            )
    }
}
