//! Call chrome: the sidebar's connected panel, the stage of participant tiles,
//! and the control bar under it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    spinner::Spinner,
    tag::Tag,
    v_flex,
};

use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::screens::home::voice::{StreamWatch, VoiceCall, VoiceParticipant, VoiceStatus};
use crate::ui::button::Button;
use crate::ui::depth::{self, radius};
use crate::voice;

/// Height of the call band shown above a DM conversation. A voice channel's
/// stage fills its pane instead, since there's no chat under it.
const DM_STAGE_HEIGHT: f32 = 280.;

/// Largest diameter of the avatar on a participant tile; smaller tiles get a
/// smaller one.
const TILE_AVATAR: f32 = 80.;

/// Space between tiles, and around them inside the stage.
const TILE_GAP: f32 = 8.;
const STAGE_PADDING: f32 = 16.;

/// Tiles are video-shaped, like Discord's, so a stream or camera can fill one.
const TILE_ASPECT: f32 = 16. / 9.;

/// The tiles under a channel's join button, which aren't fitted to anything.
const JOIN_TILE: Size<Pixels> = size(px(224.), px(126.));

/// What the camera button says. It's drawn because the call has a place for
/// it, and disabled because nothing behind it sends camera video yet.
const CAMERA_UNAVAILABLE: &str = "Camera isn't supported yet";

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

    /// The panel's line about the screen share: what's live, or why the last
    /// one stopped.
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
    fn share_button_enabled(&self) -> bool {
        self.screen_share.is_some() || self.can_share_screen()
    }

    fn share_tooltip(&self) -> &'static str {
        if self.screen_share.is_some() {
            "Stop Streaming"
        } else if crate::platform::capture::is_supported() {
            "Share Your Screen"
        } else {
            "Screen sharing isn't supported on this system"
        }
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

        // A stream being watched takes the stage over from the tiles.
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

    /// A watched stream, filling the stage: the picture once it's arrived,
    /// who it is, and the way out.
    fn stream_view(
        &self,
        watch: &StreamWatch,
        streamer: &VoiceParticipant,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let picture = match &watch.frame {
            // Raw pixels, so it never touches the image cache.
            Some(frame) => img(frame.clone())
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

    /// One participant: their avatar on their banner colour, and a badge with
    /// their name and what they've silenced. Ringed while they're talking.
    fn participant_tile(
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

    /// The tile a live participant's stream gets beside their own: the way to
    /// open it, or, for the user's own, what's going out and the way to stop.
    fn stream_tile(
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
            // Black rather than a theme colour, like the watched stream's
            // letterbox: it's where the picture goes.
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
                    CAMERA_UNAVAILABLE,
                    false,
                )
                .disabled(true),
            )
            .child(
                control(
                    "call-share",
                    "icons/screen-share.svg",
                    self.share_tooltip(),
                    self.screen_share.is_some(),
                )
                .disabled(!self.share_button_enabled())
                .on_click(cx.listener(|this, _, window, cx| this.toggle_screen_share(window, cx))),
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

/// A tile on the stage: a card with its contents centred, sized by the grid.
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

/// How the stage's tiles are laid out: how many to a row, and how big.
struct TileGrid {
    columns: usize,
    size: Size<Pixels>,
}

impl TileGrid {
    /// The width that wraps exactly `columns` tiles to a row, so a shorter
    /// last row centres under the others.
    fn row_width(&self) -> Pixels {
        self.size.width * self.columns as f32 + px(TILE_GAP) * (self.columns - 1) as f32
    }
}

/// The largest tiles that fit `count` of them into `stage`, trying every
/// column count and keeping whichever leaves them biggest.
fn fit_tiles(count: usize, stage: Size<Pixels>) -> TileGrid {
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

/// Where the call is, as the sidebar panel labels it: `Guild / channel` for a
/// voice channel, the other person's name for a DM call.
fn call_location(call: &VoiceCall) -> String {
    match &call.context {
        Some(guild) => format!("{guild} / {}", call.name),
        None => call.name.clone(),
    }
}
