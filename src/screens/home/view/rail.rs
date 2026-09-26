//! The left-hand server rail: the DMs button, the guild icons, and the folders
//! grouping them.

use std::sync::{Arc, LazyLock};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, avatar::Avatar, divider::Divider, h_flex, v_flex,
};

use crate::assets::icons::DISCORD_ICON;
use crate::discord::Guild;
use crate::screens::home::folders::{RailEntry, RailFolder};
use crate::screens::home::{HomeScreen, View};
use crate::ui::depth::{Finish, Level, Lit as _};
use crate::ui::tooltip;

/// Corner radius of the pill behind the selected icon: the icon's own 16px
/// plus the pill's 4px of padding, so the two curves run parallel.
const PILL_RADIUS: f32 = 20.;

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
            .children(self.rail_entries.iter().map(|entry| match entry {
                RailEntry::Guild(guild) => self.render_rail_guild(guild, cx).into_any_element(),
                RailEntry::Folder(folder) => self.render_rail_folder(folder, cx).into_any_element(),
            }))
    }

    fn render_rail_guild(&self, guild: &Guild, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let guild_id = guild.id;
        let guild_name = SharedString::from(guild.name.clone());
        let is_selected = self.view == View::Guild && self.selected_guild == Some(guild_id);

        selection_pill(div().id(("guild", guild_id.get())), is_selected, cx)
            .child(guild_avatar(guild, px(48.)))
            .tooltip(tooltip::text(guild_name))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_guild(guild_id, window, cx);
            }))
    }

    /// A folder: a square that toggles between showing its guilds inline and
    /// standing in for them with a preview of the first few.
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

        v_flex()
            .items_center()
            .gap_2()
            // Expanded, the tint runs behind the column so its guilds read as
            // being inside the folder rather than loose in the rail.
            .when(expanded, |this| {
                this.p(px(4.))
                    .rounded(px(PILL_RADIUS))
                    .bg(accent.opacity(0.12))
            })
            .child(
                div()
                    .id(("guild-folder", folder_id as u64))
                    .cursor_pointer()
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
                            this.child(Icon::new(IconName::Folder).text_color(accent).size(px(24.)))
                        } else {
                            this.child(folder_preview(&folder.guilds))
                        }
                    })
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

/// The padded frame around a rail icon, which lifts off the rail as a raised
/// pill while its guild (or the DMs) is the one open.
fn selection_pill(frame: Stateful<Div>, selected: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    frame
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
