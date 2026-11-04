//! The left-hand server rail: the DMs button, the guild icons, and the folders
//! grouping them.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, separator::Separator, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::{DirectMessage, Guild};
use crate::screens::home::folders::{RailEntry, RailFolder};
use crate::screens::home::{HomeScreen, View};
use crate::ui::avatar::CachedAvatar;
use crate::ui::depth::{Finish, Level, Lit as _};
use crate::ui::tooltip;

use super::{mention_badge, unread_pill};

/// Corner radius of the selected pill: the icon's 16px plus the pill's 4px padding, so the curves
/// run parallel.
const PILL_RADIUS: f32 = 20.;

/// Space between an icon's frame and the rail's edge, where the unread mark hangs: (72px rail -
/// 56px frame) / 2.
const RAIL_GUTTER: Pixels = px(8.);

/// Room left between the rail's edge and an icon the "NEW" pill scrolls to.
const NEW_PILL_CLEARANCE: Pixels = px(40.);

impl HomeScreen {
    pub(super) fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_dms = self.view == View::DirectMessages;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let logo_bg = rgb(0x313338);
        let unread_dms: Vec<&DirectMessage> = self
            .dms
            .list
            .iter()
            .filter(|dm| self.unreads.dm_mentions(dm.id) > 0)
            .collect();

        // Scroll children: the DMs icon, the unread DMs, the separator, then the rail entries.
        let first_entry = 1 + unread_dms.len() + 1;
        let pinged = (1..=unread_dms.len()).chain(
            self.rail_entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.rail_entry_mentions(entry) > 0)
                .map(|(ix, _)| first_entry + ix),
        );
        let (above, below) = offscreen_pings(self.rail_scroll.handle(), pinged);

        // The DMs icon, separator and guild list scroll as one column.
        let rail = v_flex()
            .id("server-rail")
            .size_full()
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
                            // `svg` not `img`: an image is rasterized once at twice the SVG's size
                            // and shrunk on the
                            // GPU, which has no mipmaps, so it came out jagged.
                            .child(
                                svg()
                                    .path("icons/discord.svg")
                                    .size(px(28.))
                                    .text_color(white()),
                            ),
                    )
                    .tooltip(tooltip::text("Direct Messages"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_direct_messages(window, cx);
                    })),
            )
            // Newest first, as in Discord; muted conversations never ping, so never show.
            .children(unread_dms.iter().map(|dm| self.render_rail_dm(dm, cx)))
            .child(Separator::horizontal().w(px(32.)))
            .children(self.rail_entries.iter().map(|entry| match entry {
                RailEntry::Guild(guild) => self.render_rail_guild(guild, cx).into_any_element(),
                RailEntry::Folder(folder) => self.render_rail_folder(folder, cx).into_any_element(),
            }));

        div()
            .relative()
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .child(rail)
            .when_some(above, |this, ix| {
                this.child(self.render_new_pill(ix, true, cx).top(px(8.)))
            })
            .when_some(below, |this, ix| {
                this.child(self.render_new_pill(ix, false, cx).bottom(px(8.)))
            })
    }

    fn rail_entry_mentions(&self, entry: &RailEntry) -> u32 {
        match entry {
            RailEntry::Guild(guild) => self.unreads.guild_mentions(guild.id),
            RailEntry::Folder(folder) => folder
                .guilds
                .iter()
                .map(|guild| self.unreads.guild_mentions(guild.id))
                .sum(),
        }
    }

    /// Floats over the rail's edge while a pinged icon is scrolled past it; a click brings the
    /// nearest one into view.
    fn render_new_pill(&self, ix: usize, above: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        h_flex()
            .id(if above {
                "rail-new-above"
            } else {
                "rail-new-below"
            })
            .absolute()
            .left(px(8.))
            .w(px(56.))
            .h(px(24.))
            .justify_center()
            .rounded_full()
            .bg(theme.danger)
            .text_color(theme.danger_foreground)
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .shadow_md()
            .cursor_pointer()
            .child("NEW")
            .on_click(cx.listener(move |this, _, _, cx| {
                let handle = this.rail_scroll.handle();
                let (Some(item), viewport) = (handle.bounds_for_item(ix), handle.bounds()) else {
                    return;
                };
                // Clear of the pill, which would otherwise sit on top of it.
                let target = if above {
                    viewport.top() + NEW_PILL_CLEARANCE - item.top()
                } else {
                    viewport.bottom() - NEW_PILL_CLEARANCE - item.bottom()
                };
                this.rail_scroll.glide_to(target);
                cx.notify();
            }))
    }

    /// An unread DM's icon. Never the open one: opening reads it and it leaves the rail.
    fn render_rail_dm(
        &self,
        dm: &DirectMessage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let channel_id = dm.id;
        selection_pill(div().id(("rail-dm", channel_id.get())), false, cx)
            .child(super::avatar(
                dm.name.clone(),
                dm.avatar_url.clone(),
                px(48.),
            ))
            .child(unread_pill(RAIL_GUTTER, cx))
            .children(rail_badge(self.unreads.dm_mentions(channel_id), cx))
            .tooltip(tooltip::text(SharedString::from(dm.name.clone())))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_dm(channel_id, window, cx);
            }))
    }

    fn render_rail_guild(&self, guild: &Guild, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let guild_id = guild.id;
        let guild_name = SharedString::from(guild.name.clone());
        let is_selected = self.view == View::Guild && self.selected_guild == Some(guild_id);
        let unread = !is_selected && self.unreads.is_guild_unread(guild_id);
        let mentions = self.unreads.guild_mentions(guild_id);

        selection_pill(div().id(("guild", guild_id.get())), is_selected, cx)
            .child(guild_avatar(guild, px(48.)))
            .when(unread, |this| this.child(unread_pill(RAIL_GUTTER, cx)))
            .children(rail_badge(mentions, cx))
            .tooltip(tooltip::text(guild_name))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_guild(guild_id, window, cx);
            }))
    }

    fn render_rail_folder(
        &self,
        folder: &RailFolder,
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
        let mentions = if expanded {
            0
        } else {
            folder
                .guilds
                .iter()
                .map(|guild| self.unreads.guild_mentions(guild.id))
                .sum()
        };

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
                    .children(rail_badge(mentions, cx))
                    .tooltip(tooltip::text(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_folders.remove(&folder_id) {
                            this.expanded_folders.insert(folder_id);
                        }
                        cx.notify();
                    })),
            )
            .when(expanded, |this| {
                this.children(
                    folder
                        .guilds
                        .iter()
                        .map(|guild| self.render_rail_guild(guild, cx)),
                )
            })
    }
}

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

/// Ringed in the rail's colour so it reads as cut out of the icon.
fn rail_badge(mentions: u32, cx: &App) -> Option<Div> {
    let badge = mention_badge(mentions, cx)?;
    Some(
        div()
            .absolute()
            // Inside the frame's 4px padding, so the ring overlaps the icon's
            // corner as Discord's does.
            .right(px(1.))
            .bottom(px(1.))
            .rounded_full()
            .border(px(3.))
            .border_color(cx.theme().sidebar)
            .child(badge),
    )
}

/// The nearest pinged child scrolled past each edge, from last frame's layout. A child counts as
/// past once its centre is, as its badge sits low on the icon.
fn offscreen_pings(
    handle: &ScrollHandle,
    pinged: impl Iterator<Item = usize>,
) -> (Option<usize>, Option<usize>) {
    let viewport = handle.bounds();
    // Child bounds are laid out unscrolled.
    let scroll = handle.offset().y;
    let (mut above, mut below) = (None, None);
    for ix in pinged {
        let Some(bounds) = handle.bounds_for_item(ix) else {
            continue;
        };
        let centre = bounds.center().y + scroll;
        if centre < viewport.top() {
            above = Some(ix);
        } else if centre > viewport.bottom() && below.is_none() {
            below = Some(ix);
        }
    }
    (above, below)
}

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

/// Rounded squares, as in Discord: radius a third of the side (48px gets 16px).
fn guild_avatar(guild: &Guild, size: Pixels) -> CachedAvatar {
    super::avatar(guild.name.clone(), guild.icon_url.clone(), size).rounded(size / 3.)
}
