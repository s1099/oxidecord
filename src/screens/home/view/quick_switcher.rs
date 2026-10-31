//! The quick switcher dialog, opened with Ctrl+K: gpui-component's command
//! palette, fed by [`HomeScreen::quick_switcher_sections`] rather than its
//! own filter.

use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{ActiveTheme as _, Icon, IndexPath, WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::channels::channel_icon_path;
use crate::screens::home::quick_switcher::{RowIcon, SwitcherRow, Target};
use crate::screens::home::view::{avatar, mention_badge};
use crate::screens::home::{HomeScreen, ToggleQuickSwitcher};
use crate::ui::depth;

const WIDTH: f32 = 570.;
const LIST_HEIGHT: f32 = 400.;
const ICON_SIZE: f32 = 20.;

pub(in crate::screens::home) struct QuickSwitcher {
    screen: WeakEntity<HomeScreen>,
    command: Entity<CommandState>,
    /// What each item opens, by the section and row the palette reports.
    targets: Vec<Vec<Target>>,
}

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_quick_switcher(
        &mut self,
        _: &ToggleQuickSwitcher,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Another dialog has the keyboard; the switcher's own handler closes it.
        if window.has_active_dialog(cx) {
            return;
        }
        let screen = cx.entity().downgrade();
        let switcher = cx.new(|cx| QuickSwitcher {
            screen,
            command: cx.new(|cx| CommandState::new(window, cx)),
            targets: Vec::new(),
        });
        let command = switcher.read(cx).command.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .close_button(false)
                .overlay_closable(true)
                .border_color(depth::ring(cx))
                .w(px(WIDTH))
                .child(switcher.clone())
        });
        // Opening focuses the dialog itself.
        window.defer(cx, move |window, cx| {
            command.update(cx, |command, cx| command.focus(window, cx))
        });
    }
}

impl QuickSwitcher {
    fn confirm(&mut self, index: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&target) = self
            .targets
            .get(index.section)
            .and_then(|rows| rows.get(index.row))
        else {
            return;
        };
        window.close_dialog(cx);
        self.screen
            .update(cx, |screen, cx| screen.switch_to(target, window, cx))
            .ok();
    }
}

impl Render for QuickSwitcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.command.read(cx).query(cx);
        let sections = self
            .screen
            .upgrade()
            .map(|screen| screen.read(cx).quick_switcher_sections(&query))
            .unwrap_or_default();
        self.targets = sections
            .iter()
            .map(|section| section.rows.iter().map(|row| row.target).collect())
            .collect();

        let this = cx.entity().downgrade();
        let on_confirm = move |index: IndexPath, window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| this.confirm(index, window, cx))
                .ok();
        };
        let this = cx.entity().downgrade();
        let on_query = move |_: &str, _: &mut Window, cx: &mut App| {
            this.update(cx, |_, cx| cx.notify()).ok();
        };

        let mut command = Command::new(&self.command)
            .filterable(false)
            .bordered(false)
            .max_h(px(LIST_HEIGHT))
            .placeholder("Where would you like to go?")
            .on_query(on_query)
            .on_confirm(on_confirm)
            .empty(|_, _, cx| {
                div()
                    .py_6()
                    .text_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Can't find what you're looking for?")
            })
            .footer(|_, _, cx| footer(cx));
        for section in sections {
            let mut group = CommandGroup::new();
            if let Some(heading) = section.heading {
                group = group.label(heading);
            }
            command = command.group(group.items(section.rows.into_iter().map(|row| {
                CommandItem::new()
                    .label(row.name.clone())
                    .child(move |_, cx| row_content(&row, cx))
            })));
        }

        div()
            .on_action(cx.listener(|_, _: &ToggleQuickSwitcher, window, cx| {
                window.close_dialog(cx);
            }))
            .child(command)
    }
}

fn row_content(row: &SwitcherRow, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let icon = match &row.icon {
        RowIcon::Channel(kind) => Icon::default()
            .path(channel_icon_path(*kind))
            .size(px(ICON_SIZE))
            .text_color(theme.muted_foreground)
            .into_any_element(),
        RowIcon::Image { name, url } => {
            avatar(name.clone(), url.clone(), px(ICON_SIZE)).into_any_element()
        }
    };
    h_flex()
        .flex_1()
        .min_w_0()
        .h(px(22.))
        .gap_2()
        .child(icon)
        .child(
            div()
                .min_w_0()
                .truncate()
                .when(row.unread, |this| this.font_weight(FontWeight::SEMIBOLD))
                .child(row.name.clone()),
        )
        .children(row.detail.clone().map(|category| {
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(category)
        }))
        .child(div().flex_1())
        .children(mention_badge(row.mentions, cx))
        .children(row.context.clone().map(|guild| {
            div()
                .flex_shrink_0()
                .max_w(px(160.))
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(guild)
        }))
}

fn footer(cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let prefix = |key: &'static str| {
        div()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.foreground)
            .child(key)
    };
    h_flex()
        .pt_3()
        .gap_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child("Start searches with")
        .child(prefix("@"))
        .child(prefix("#"))
        .child(prefix("!"))
        .child(prefix("*"))
        .child("to narrow results to DMs, text channels, voice channels or servers.")
}
