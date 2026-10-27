//! The tiles on the stage, one per participant and one per stream.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, tag::Tag, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::screens::home::voice::VoiceParticipant;
use crate::ui::button::Button;
use crate::ui::depth::{self, radius};

const TILE_AVATAR: f32 = 80.;

pub(super) const TILE_GAP: f32 = 8.;
pub(super) const STAGE_PADDING: f32 = 16.;

/// Tiles are video-shaped, like Discord's, so a stream or camera can fill one.
const TILE_ASPECT: f32 = 16. / 9.;

impl HomeScreen {
    pub(super) fn participant_tile(
        &self,
        participant: &VoiceParticipant,
        size: Size<Pixels>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        let user_id = participant.user_id;
        let avatar_size = (size.height * 0.4).clamp(px(32.), px(TILE_AVATAR));

        let badge = name_badge(None, participant.name.clone())
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
            });

        tile(format!("voice-tile-{user_id}"), size, cx)
            .when_some(participant.accent_color, |this, color| this.bg(rgb(color)))
            .child(avatar(
                participant.name.clone(),
                participant.avatar_url.clone(),
                avatar_size,
            ))
            .child(badge)
            // Drawn over the card's own edge rather than as a thicker one, so
            // starting to talk doesn't shift anything inside the tile.
            .when(participant.speaking, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(radius::CARD - px(1.))
                        .border_2()
                        .border_color(theme.success),
                )
            })
    }

    pub(super) fn stream_tile(
        &self,
        participant: &VoiceParticipant,
        size: Size<Pixels>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let user_id = participant.user_id;
        let watchable = self.can_watch(user_id);
        let caption = |text: String| {
            div()
                .max_w_full()
                .px_4()
                .truncate()
                .text_sm()
                .text_color(white().opacity(0.7))
                .child(text)
        };

        let middle = if participant.is_self {
            let streaming = match &self.screen_share {
                Some(share) if !share.source_name.is_empty() => {
                    format!("You're streaming {}", share.source_name)
                }
                _ => "You're streaming".into(),
            };
            v_flex()
                .max_w_full()
                .gap_2()
                .items_center()
                .child(caption(streaming))
                .when(self.screen_share.is_some(), |this| {
                    this.child(
                        Button::new("stream-tile-stop")
                            .label("Stop Streaming")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.stop_screen_share(cx))),
                    )
                })
                .into_any_element()
        } else if watchable {
            Button::new(SharedString::from(format!("stream-watch-{user_id}")))
                .label("Watch Stream")
                .on_click(
                    cx.listener(move |this, _, window, cx| this.watch_stream(user_id, window, cx)),
                )
                .into_any_element()
        } else {
            caption("Connecting…".into()).into_any_element()
        };

        let badge = name_badge(
            Some(Icon::default().path("icons/screen-share.svg").size_3p5()),
            participant.name.clone(),
        );

        tile(format!("voice-stream-{user_id}"), size, cx)
            // Black, not a theme colour, like the watched stream's letterbox.
            .bg(black())
            .child(middle)
            .child(
                div()
                    .absolute()
                    .top_2()
                    .right_2()
                    .child(Tag::danger().xsmall().child("LIVE")),
            )
            .child(badge)
            .when(watchable, |this| {
                let hover = cx.theme().danger;
                this.cursor_pointer()
                    .hover(move |style| style.border_color(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.watch_stream(user_id, window, cx)
                    }))
            })
    }
}

fn tile(id: String, size: Size<Pixels>, cx: &App) -> Stateful<Div> {
    depth::card(radius::CARD, cx)
        .id(SharedString::from(id))
        .relative()
        .flex_shrink_0()
        .w(size.width)
        .h(size.height)
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
}

/// The name in a tile's bottom-left corner. White on translucent black rather
/// than theme colours, since it sits on whatever the tile shows — a banner
/// colour, or a picture.
fn name_badge(icon: Option<Icon>, name: String) -> Div {
    h_flex()
        .absolute()
        .bottom_2()
        .left_2()
        .max_w(relative(0.8))
        .px_2()
        .py_1()
        .gap_1()
        .items_center()
        .rounded(radius::ITEM)
        .bg(black().opacity(0.6))
        .text_sm()
        .text_color(white())
        .children(icon)
        .child(div().min_w_0().truncate().child(name))
}

pub(super) struct TileGrid {
    columns: usize,
    pub(super) size: Size<Pixels>,
}

impl TileGrid {
    /// The width that wraps exactly `columns` tiles to a row, so a shorter
    /// last row centres under the others.
    pub(super) fn row_width(&self) -> Pixels {
        self.size.width * self.columns as f32 + px(TILE_GAP) * (self.columns - 1) as f32
    }
}

pub(super) fn fit_tiles(count: usize, stage: Size<Pixels>) -> TileGrid {
    let count = count.max(1);
    let width = f32::from(stage.width) - STAGE_PADDING * 2.;
    let height = f32::from(stage.height) - STAGE_PADDING * 2.;
    // Not laid out yet: something sensible for the one frame before it is.
    if width <= 0. || height <= 0. {
        return TileGrid {
            columns: count.min(3),
            size: size(px(240.), px(240. / TILE_ASPECT)),
        };
    }

    let (columns, tile_width) = (1..=count)
        .map(|columns| {
            let rows = count.div_ceil(columns);
            let by_width = (width - TILE_GAP * (columns - 1) as f32) / columns as f32;
            let by_height = (height - TILE_GAP * (rows - 1) as f32) / rows as f32 * TILE_ASPECT;
            (columns, by_width.min(by_height))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((1, width));

    // Floored so rounding can never push a row's last tile onto the next.
    let tile_width = tile_width.max(1.).floor();
    TileGrid {
        columns,
        size: size(px(tile_width), px((tile_width / TILE_ASPECT).floor())),
    }
}
