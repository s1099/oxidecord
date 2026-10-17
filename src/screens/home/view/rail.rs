//! The left-hand server rail: the DMs button, the guild icons, and the folders
//! grouping them.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, avatar::Avatar, divider::Divider, h_flex, v_flex,
};
use twilight_model::id::{Id, marker::GuildMarker};

use crate::assets::icons::DISCORD_ICON;
use crate::discord::Guild;
use crate::screens::home::folders::{RailEntry, RailFolder};
use crate::screens::home::voice::VoiceParticipant;
use crate::screens::home::{HomeScreen, View};
use crate::ui::depth::{Finish, Level, Lit as _, radius};
use crate::ui::tooltip;

use super::unread_pill;

/// Corner radius of the pill behind the selected icon: the icon's own 16px
/// plus the pill's 4px of padding, so the two curves run parallel.
const PILL_RADIUS: f32 = 20.;

/// Space between an icon's frame and the rail's edge, which the unread mark
/// hangs into: the 72px rail less the 56px frame, halved.
const RAIL_GUTTER: Pixels = px(8.);

/// Built once: `Image::from_bytes` hashes the whole buffer, and the rail
/// renders on every frame of a scroll glide or a playing video.
static DISCORD_LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        DISCORD_ICON.as_bytes().to_vec(),
    ))
});

impl HomeScreen {
    pub(super) fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_dms = self.view == View::DirectMessages;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let logo_bg = rgb(0x313338);

        // The rail renders every frame of a scroll glide, so the badges ask
        // only which guilds have anyone in voice, in one pass without clones;
        // the tooltip reads the participants themselves, and only while open.
        let voice_guilds: HashSet<Id<GuildMarker>> = self
            .voice_states
            .values()
            .filter_map(|state| {
                state
                    .channel_id
                    .is_some()
                    .then_some(state.guild_id)
                    .flatten()
            })
            .collect();

        // The whole rail — the DMs icon, its separator, and the guild list —
        // scrolls as one column, so the icon isn't pinned above the list.
        v_flex()
            .id("server-rail")
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .py_3()
            .gap_2()
            .bg(rail_bg)
            .overflow_y_scroll()
            .track_scroll(self.rail_scroll.handle())
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.rail_scroll.absorb(event, window)),
            )
            .child(
                selection_pill(div().id("home-dms"), in_dms, cx)
                    .child(
                        div()
                            .size(px(48.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(16.))
                            .bg(logo_bg)
                            .child(img(DISCORD_LOGO.clone()).size(px(28.))),
                    )
                    .tooltip(tooltip::text("Direct Messages"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_direct_messages(window, cx);
                    })),
            )
            .child(Divider::horizontal().w(px(32.)))
            .children(self.rail_entries.iter().map(|entry| {
                match entry {
                    RailEntry::Guild(guild) => {
                        let in_voice = voice_guilds.contains(&guild.id);
                        self.render_rail_guild(guild, in_voice, cx)
                            .into_any_element()
                    }
                    RailEntry::Folder(folder) => self
                        .render_rail_folder(folder, &voice_guilds, cx)
                        .into_any_element(),
                }
            }))
    }

    fn render_rail_guild(
        &self,
        guild: &Guild,
        in_voice: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let guild_id = guild.id;
        let guild_name = SharedString::from(guild.name.clone());
        let is_selected = self.view == View::Guild && self.selected_guild == Some(guild_id);
        let unread = !is_selected && self.unreads.is_guild_unread(guild_id);

        selection_pill(div().id(("guild", guild_id.get())), is_selected, cx)
            .child(guild_avatar(guild, px(48.)))
            .when(in_voice, |this| this.child(voice_badge(cx)))
            .when(unread, |this| this.child(unread_pill(RAIL_GUTTER, cx)))
            .when(in_voice, |this| {
                this.hoverable_tooltip(voice_tooltip(guild_id, guild_name.clone(), cx))
            })
            .when(!in_voice, |this| this.tooltip(tooltip::text(guild_name)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_guild(guild_id, window, cx);
            }))
    }

    /// A folder: a square that toggles between showing its guilds inline and
    /// standing in for them with a preview of the first few.
    fn render_rail_folder(
        &self,
        folder: &RailFolder,
        voice_guilds: &HashSet<Id<GuildMarker>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let accent: Hsla = folder
            .color
            .map_or(theme.primary, |color| rgb(color).into());
        let selected_bg = theme.sidebar_accent;

        let folder_id = folder.id;
        let expanded = self.expanded_folders.contains(&folder_id);
        let holds_selected = self.view == View::Guild
            && self
                .selected_guild
                .is_some_and(|id| folder.guilds.iter().any(|guild| guild.id == id));
        let label = SharedString::from(folder.name.clone().unwrap_or_else(|| "Folder".to_owned()));
        // Open, the folder's guilds carry their own marks.
        let unread = !expanded
            && folder
                .guilds
                .iter()
                .any(|guild| self.unreads.is_guild_unread(guild.id));

        v_flex()
            .items_center()
            .gap_2()
            // No padding: the icons' own pill frames inset the tint.
            .when(expanded, |this| {
                this.rounded(px(PILL_RADIUS)).bg(accent.opacity(0.12))
            })
            .child(
                div()
                    .id(("guild-folder", folder_id as u64))
                    .relative()
                    .p(px(4.))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(48.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(16.))
                            .bg(accent.opacity(0.24))
                            // Collapsed, the folder stands in for the guild inside it.
                            .when(holds_selected && !expanded, |this| this.bg(selected_bg))
                            .map(|this| {
                                if expanded {
                                    this.child(
                                        Icon::new(IconName::Folder)
                                            .text_color(accent)
                                            .size(px(24.)),
                                    )
                                } else {
                                    this.child(folder_preview(&folder.guilds))
                                }
                            }),
                    )
                    .when(unread, |this| this.child(unread_pill(RAIL_GUTTER, cx)))
                    .tooltip(tooltip::text(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_folders.remove(&folder_id) {
                            this.expanded_folders.insert(folder_id);
                        }
                        cx.notify();
                    })),
            )
            // Open, the folder's guilds carry their own marks, voice badges
            // included. Collapsed, it stands for them without one: folding
            // several servers' presence into one icon reads as the folder's
            // own call, and Discord doesn't badge folders either.
            .when(expanded, |this| {
                this.children(folder.guilds.iter().map(|guild| {
                    self.render_rail_guild(guild, voice_guilds.contains(&guild.id), cx)
                }))
            })
    }
}

/// The padded frame around a rail icon, which lifts off the rail as a raised
/// pill while its guild (or the DMs) is the one open.
fn selection_pill(frame: Stateful<Div>, selected: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    frame
        .relative()
        .p(px(4.))
        .rounded(px(PILL_RADIUS))
        .cursor_pointer()
        .when(selected, |this| {
            this.bg(theme.sidebar_accent).lit(
                Level::Raised,
                Finish::Subtle,
                px(56.),
                px(PILL_RADIUS),
                cx,
            )
        })
}

/// The first few of a collapsed folder's guilds, tiled inside its square.
fn folder_preview(guilds: &[Guild]) -> impl IntoElement + use<> {
    h_flex()
        .flex_wrap()
        .w(px(34.))
        .gap(px(2.))
        .items_center()
        .justify_center()
        .children(
            guilds
                .iter()
                .take(4)
                .map(|guild| guild_avatar(guild, px(15.))),
        )
}

/// Guild icons are rounded squares rather than circles, as in Discord: a
/// third of the side, so 48px gets its 16px radius and previews scale down.
fn guild_avatar(guild: &Guild, size: Pixels) -> Avatar {
    super::avatar(guild.name.clone(), guild.icon_url.clone(), size).rounded(size / 3.)
}

/// The speaker mark in a guild icon's upper-right corner while anyone is in voice.
fn voice_badge(cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    div()
        .absolute()
        .top_0()
        .right_0()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.background)
        .text_color(theme.foreground)
        .child(Icon::default().path("icons/volume-2.svg").size(px(12.)))
}

/// The tooltip builder for a guild with people in voice, in place of the
/// plain name one. The view reads the screen's voice states as it renders,
/// so one left open through a join or a move re-renders rather than going
/// stale.
fn voice_tooltip(
    guild_id: Id<GuildMarker>,
    guild_name: SharedString,
    cx: &Context<HomeScreen>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + use<> {
    let screen = cx.entity();
    move |_, cx| {
        let (screen, guild_name) = (screen.clone(), guild_name.clone());
        cx.new(|cx| GuildVoiceTooltip::new(screen, guild_id, guild_name, cx))
            .into()
    }
}

/// How many participants the tooltip previews before the rest fold into the
/// "+N" bubble, as in Discord.
const TOOLTIP_AVATARS: usize = 5;

/// Roughly how tall the voice tooltip is, for its lighting.
const TOOLTIP_HEIGHT: f32 = 72.;

/// The avatar row's disc size, ring included.
const TOOLTIP_AVATAR_SIZE: f32 = 28.;

struct GuildVoiceTooltip {
    screen: Entity<HomeScreen>,
    guild_id: Id<GuildMarker>,
    guild_name: SharedString,
    _screen_changed: Subscription,
}

impl GuildVoiceTooltip {
    fn new(
        screen: Entity<HomeScreen>,
        guild_id: Id<GuildMarker>,
        guild_name: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        let _screen_changed = cx.observe(&screen, |_, _, cx| cx.notify());
        Self {
            screen,
            guild_id,
            guild_name,
            _screen_changed,
        }
    }
}

impl Render for GuildVoiceTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (participants, total) = self
            .screen
            .read(cx)
            .guild_voice_presence(self.guild_id, TOOLTIP_AVATARS);
        let extra = total - participants.len();
        // Wrapped, so the margin that keeps it clear of the cursor isn't part
        // of the lit box — as `ui::tooltip` does.
        div().child(
            v_flex()
                .m_3()
                .max_w(px(240.))
                .gap_2()
                .bg(theme.popover)
                .lit(
                    Level::Overlay,
                    Finish::Matte,
                    px(TOOLTIP_HEIGHT),
                    radius::ITEM,
                    cx,
                )
                .px_3()
                .py_2()
                .font_family(theme.font_family.clone())
                .text_size(px(14.))
                .text_color(theme.popover_foreground)
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(self.guild_name.clone()),
                )
                .when(!participants.is_empty(), |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::default()
                                    .path("icons/volume-2.svg")
                                    .size(px(20.))
                                    .flex_shrink_0(),
                            )
                            .child(voice_avatar_row(&participants, theme.popover))
                            .when(extra > 0, |this| {
                                this.child(overflow_bubble(extra, theme.popover_foreground))
                            }),
                    )
                }),
        )
    }
}

/// The tooltip's overlapping avatars, cut out of each other by a ring of the
/// tooltip's own fill.
fn voice_avatar_row(participants: &[VoiceParticipant], ring: Hsla) -> impl IntoElement + use<> {
    h_flex().children(participants.iter().enumerate().map(|(ix, participant)| {
        div()
            .id(("voice-preview-user", participant.user_id.get()))
            .flex_shrink_0()
            .rounded_full()
            .border_2()
            .border_color(ring)
            .when(ix > 0, |this| this.ml(px(-8.)))
            .child(
                super::avatar(
                    participant.name.clone(),
                    participant.avatar_url.clone(),
                    px(TOOLTIP_AVATAR_SIZE - 4.),
                )
                .rounded_full()
                .border_0(),
            )
            .tooltip(tooltip::text(participant.name.clone()))
    }))
}

/// The "+N" disc folding the participants past the preview into the row.
fn overflow_bubble(extra: usize, text: Hsla) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(TOOLTIP_AVATAR_SIZE))
        .flex_shrink_0()
        .rounded_full()
        .bg(text.opacity(0.2))
        .child(format!("+{extra}"))
}
