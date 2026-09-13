//! The left-hand server rail: the DMs button, the guild icons, and the folders
//! grouping them.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, avatar::Avatar, divider::Divider, h_flex,
    tooltip::Tooltip, v_flex,
};

use crate::assets::icons::DISCORD_ICON;
use crate::discord::Guild;
use crate::screens::home::folders::{RailEntry, RailFolder};
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(super) fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_dms = self.view == View::DirectMessages;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let rail_border = theme.sidebar_border;
        let logo_bg = rgb(0x313338);
        let selected_bg = theme.sidebar_accent;

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
            .border_r_1()
            .border_color(rail_border)
            .overflow_y_scroll()
            .track_scroll(self.rail_scroll.handle())
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.rail_scroll.absorb(event, window)),
            )
            .child(
                div()
                    .id("home-dms")
                    .p(px(4.))
                    .rounded(px(16.))
                    .cursor_pointer()
                    .when(in_dms, |this| this.bg(selected_bg))
                    .child(
                        div()
                            .size(px(48.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(logo_bg)
                            .child(
                                img(Arc::new(Image::from_bytes(
                                    ImageFormat::Svg,
                                    DISCORD_ICON.as_bytes().to_vec(),
                                )))
                                .size(px(28.)),
                            ),
                    )
                    .tooltip(|window, cx| Tooltip::new("Direct Messages").build(window, cx))
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
        let selected_bg = cx.theme().sidebar_accent;

        div()
            .id(("guild", guild_id.get()))
            .cursor_pointer()
            .p(px(4.))
            .rounded(px(16.))
            .when(is_selected, |this| this.bg(selected_bg))
            .child(guild_avatar(guild, px(48.)))
            .tooltip({
                let guild_name = guild_name.clone();
                move |window, cx| Tooltip::new(guild_name.clone()).build(window, cx)
            })
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
        // Discord's own default for an uncoloured folder is the accent colour.
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
            // Expanded, the tint runs behind the whole column so its guilds
            // read as being inside the folder rather than loose in the rail.
            .when(expanded, |this| {
                this.p(px(4.)).rounded(px(20.)).bg(accent.opacity(0.12))
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
                    // Collapsed, the folder is the only thing standing in for
                    // the selected guild, so it takes the highlight.
                    .when(holds_selected && !expanded, |this| this.bg(selected_bg))
                    .map(|this| {
                        if expanded {
                            this.child(Icon::new(IconName::Folder).text_color(accent).size(px(24.)))
                        } else {
                            this.child(folder_preview(&folder.guilds))
                        }
                    })
                    .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
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

fn guild_avatar(guild: &Guild, size: Pixels) -> Avatar {
    let avatar = Avatar::new().name(guild.name.clone()).with_size(size);
    match guild.icon_url.clone() {
        Some(icon_url) => avatar.src(icon_url),
        None => avatar,
    }
}
