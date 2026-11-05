//! The activities button under the stage, and its menu: the activity the user
//! is in, the ones running in the call, and the shelf to start one from.

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::ActivityApp;
use crate::screens::home::HomeScreen;
use crate::screens::home::activities::ShelfState;
use crate::ui::button::Button;

const ICON: &str = "icons/rocket.svg";
const ICON_SIZE: f32 = 20.;

impl HomeScreen {
    pub(super) fn render_activities_control(&self, cx: &Context<Self>) -> AnyElement {
        let active = self.activity.is_some();
        let button = Button::new("call-activities")
            .icon(Icon::default().path(ICON))
            .outline()
            .large()
            .selected(active);
        if !active && !self.can_use_activities() {
            return button
                .disabled(true)
                .tooltip("You don't have permission to use activities here")
                .into_any_element();
        }
        let screen = cx.entity().downgrade();
        button
            .tooltip("Activities")
            .dropdown_menu_with_anchor(Anchor::BottomCenter, move |menu, _, cx| {
                match screen.upgrade() {
                    Some(screen) => activities_menu(menu, &screen, cx),
                    None => menu,
                }
            })
            .into_any_element()
    }
}

fn activities_menu(
    menu: PopupMenu,
    screen: &Entity<HomeScreen>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let home = screen.read(cx);
    let Some(call) = &home.voice else {
        return menu;
    };
    let guild_id = call.guild_id;
    let mut menu = menu.min_w(px(240.)).max_h(px(420.)).scrollable(true);

    let current = home.activity.as_ref().map(|session| session.app.id);
    if let Some(session) = &home.activity {
        menu = menu.label(session.app.name.clone());
        menu = if session.window.is_some() {
            menu.item(on_screen(PopupMenuItem::new("Open"), screen, |this, cx| {
                this.show_activity(cx)
            }))
        } else {
            menu.item(PopupMenuItem::new("Starting…").disabled(true))
        };
        let leave = if session.window.is_some() {
            "Leave"
        } else {
            "Cancel"
        };
        menu = menu
            .item(on_screen(PopupMenuItem::new(leave), screen, |this, cx| {
                this.end_activity(cx)
            }))
            .separator();
    }

    // Each running app once, however many instances, with everyone in them.
    let mut seen = HashSet::new();
    let running: Vec<(ActivityApp, usize)> = home
        .running_activities(call.channel_id)
        .iter()
        .filter(|instance| Some(instance.app_id) != current && seen.insert(instance.app_id))
        .map(|instance| {
            let people: HashSet<_> = home
                .running_activities(call.channel_id)
                .iter()
                .filter(|other| other.app_id == instance.app_id)
                .flat_map(|other| other.participants.iter().map(|p| p.user_id))
                .collect();
            let app = home
                .activity_app(guild_id, instance.app_id)
                .cloned()
                .unwrap_or_else(|| ActivityApp {
                    id: instance.app_id,
                    name: "Activity".into(),
                    icon_url: None,
                });
            (app, people.len())
        })
        .collect();
    if !running.is_empty() {
        menu = menu.label("In this call");
        for (app, people) in running {
            let detail = if people == 1 {
                "1 person".to_owned()
            } else {
                format!("{people} people")
            };
            menu = menu.item(app_item(app, Some(detail), screen));
        }
        menu = menu.separator();
    }

    if let Some(error) = &home.activity_error {
        menu = menu.item(PopupMenuItem::new(error.clone()).disabled(true));
    }
    menu = menu.label("Activities");
    match home.activity_shelf.get(&guild_id) {
        Some(ShelfState::Loaded(apps)) if apps.is_empty() => {
            menu.item(PopupMenuItem::new("No activities available").disabled(true))
        }
        Some(ShelfState::Loaded(apps)) => {
            let apps = apps.clone();
            apps.iter()
                .filter(|app| Some(app.id) != current)
                .fold(menu, |menu, app| {
                    menu.item(app_item(app.clone(), None, screen))
                })
        }
        Some(ShelfState::Failed) => {
            menu.item(PopupMenuItem::new("Couldn't load activities").disabled(true))
        }
        Some(ShelfState::Loading) | None => {
            menu.item(PopupMenuItem::new("Loading…").disabled(true))
        }
    }
}

fn app_item(
    app: ActivityApp,
    detail: Option<String>,
    screen: &Entity<HomeScreen>,
) -> PopupMenuItem {
    let name = app.name.clone();
    let icon_url = app.icon_url.clone();
    let item = PopupMenuItem::element(move |_, cx| {
        let theme = cx.theme();
        let icon = match &icon_url {
            Some(url) => img(url.clone())
                .size(px(ICON_SIZE))
                .rounded(px(4.))
                .into_any_element(),
            None => Icon::default()
                .path(ICON)
                .size(px(ICON_SIZE))
                .text_color(theme.muted_foreground)
                .into_any_element(),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(icon)
            .child(div().flex_1().truncate().child(name.clone()))
            .when_some(detail.clone(), |this, detail| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
            })
    });
    on_screen(item, screen, move |this, cx| {
        this.launch_activity(app.clone(), cx)
    })
}

fn on_screen(
    item: PopupMenuItem,
    screen: &Entity<HomeScreen>,
    action: impl Fn(&mut HomeScreen, &mut Context<HomeScreen>) + 'static,
) -> PopupMenuItem {
    let screen = screen.downgrade();
    item.on_click(move |_, _, cx| {
        let _ = screen.update(cx, |this, cx| action(this, cx));
    })
}
