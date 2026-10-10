//! The account panel pinned below the sidebar, like Discord's user area.
//! Shared by the channel sidebar and the DM sidebar. Clicking the user opens
//! the status picker.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable, Sizable as _, Theme, h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};

use crate::discord::PresenceStatus;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::ui::button::Button;
use crate::ui::settings;

impl HomeScreen {
    pub(super) fn render_user_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let (name, username, avatar_src) = match &self.current_user {
            Some(user) => (
                user.name.clone(),
                format!("@{}", user.username),
                user.avatar_url.clone(),
            ),
            None => (String::new(), String::new(), None),
        };

        let status = self.presence_status.unwrap_or_default();
        let avatar = div()
            .relative()
            .flex_shrink_0()
            .child(avatar(name.clone(), avatar_src, px(32.)))
            .child(
                div()
                    .absolute()
                    .right(px(-3.))
                    .bottom(px(-3.))
                    .child(status_indicator(status, px(10.), theme.sidebar, theme)),
            );

        let screen = cx.entity().downgrade();
        let account = AccountButton::new("account-status")
            .flex_1()
            .min_w_0()
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(username),
                    ),
            )
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, cx| {
                let popover = cx.theme().popover;
                PresenceStatus::ALL.into_iter().fold(menu, |menu, option| {
                    let screen = screen.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            let theme = cx.theme();
                            h_flex()
                                .gap_2()
                                .child(status_indicator(option, px(10.), popover, theme))
                                .child(v_flex().child(status_label(option)).children(
                                    status_description(option).map(|description| {
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(description)
                                    }),
                                ))
                        })
                        .checked(option == status)
                        .on_click(move |_, _, cx| {
                            if let Some(screen) = screen.upgrade() {
                                screen.update(cx, |this, cx| this.set_presence_status(option, cx));
                            }
                        }),
                    )
                })
            });

        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(px(52.))
            .px_1()
            .gap_1()
            .items_center()
            .child(account)
            .child(
                self.with_mic_menu(
                    "panel-mic-menu",
                    Button::new("self-mute")
                        .icon(Icon::default().path(if self.voice_muted {
                            "icons/mic-off.svg"
                        } else {
                            "icons/mic.svg"
                        }))
                        .ghost()
                        .small()
                        .selected(self.voice_muted)
                        .tooltip(if self.voice_muted { "Unmute" } else { "Mute" })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_mute(cx))),
                    cx,
                ),
            )
            .child(
                Button::new("self-deafen")
                    .icon(Icon::default().path(if self.voice_deafened {
                        "icons/headphone-off.svg"
                    } else {
                        "icons/headphones.svg"
                    }))
                    .ghost()
                    .small()
                    .selected(self.voice_deafened)
                    .tooltip(if self.voice_deafened {
                        "Undeafen"
                    } else {
                        "Deafen"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_deafen(cx))),
            )
            .child(
                Button::new("user-settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .small()
                    .tooltip("User Settings")
                    .on_click({
                        let home = cx.weak_entity();
                        move |_, window, cx| {
                            let home = home.clone();
                            settings::open(window, cx, move |_, cx| {
                                // Gone already if the session ended while the
                                // popup was open.
                                let _ = home.update(cx, |home, cx| home.log_out(cx));
                            })
                        }
                    }),
            )
    }
}

fn status_label(status: PresenceStatus) -> &'static str {
    match status {
        PresenceStatus::Online => "Online",
        PresenceStatus::Idle => "Idle",
        PresenceStatus::DoNotDisturb => "Do Not Disturb",
        PresenceStatus::Invisible => "Invisible",
    }
}

/// The line Discord's own picker puts under a status whose effect isn't
/// obvious from its name.
fn status_description(status: PresenceStatus) -> Option<&'static str> {
    match status {
        PresenceStatus::DoNotDisturb => Some("You will not receive desktop notifications"),
        PresenceStatus::Invisible => Some("You will appear offline"),
        PresenceStatus::Online | PresenceStatus::Idle => None,
    }
}

/// A status as Discord draws it, so it reads without colour: a dot for
/// online, a crescent for idle, a dash for do not disturb, a ring for
/// invisible. `backdrop` is what it sits on: it's ringed in it, so it cuts
/// cleanly into an avatar, and the cut-outs are painted in it.
fn status_indicator(
    status: PresenceStatus,
    size: Pixels,
    backdrop: Hsla,
    theme: &Theme,
) -> impl IntoElement {
    let color = match status {
        PresenceStatus::Online => theme.success,
        PresenceStatus::Idle => theme.warning,
        PresenceStatus::DoNotDisturb => theme.danger,
        PresenceStatus::Invisible => theme.muted_foreground,
    };
    let cutout = || div().absolute().bg(backdrop).rounded_full();

    div().p(px(2.)).rounded_full().bg(backdrop).child(
        div()
            .relative()
            .overflow_hidden()
            .size(size)
            .rounded_full()
            .bg(color)
            .map(|dot| match status {
                PresenceStatus::Online => dot,
                PresenceStatus::Idle => dot.child(
                    cutout()
                        .top(-size * 0.15)
                        .left(-size * 0.15)
                        .size(size * 0.65),
                ),
                PresenceStatus::DoNotDisturb => dot.child(
                    cutout()
                        .top(size * 0.375)
                        .left(size * 0.2)
                        .w(size * 0.6)
                        .h(size * 0.25),
                ),
                PresenceStatus::Invisible => {
                    dot.child(cutout().top(size * 0.3).left(size * 0.3).size(size * 0.4))
                }
            }),
    )
}

/// The avatar and name, clickable as one. The status menu's trigger, which
/// has to be [`Selectable`] so it can show the menu is open.
#[derive(IntoElement)]
struct AccountButton {
    base: Stateful<Div>,
    selected: bool,
}

impl AccountButton {
    fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            selected: false,
        }
    }
}

impl Selectable for AccountButton {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Styled for AccountButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for AccountButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl ParentElement for AccountButton {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements)
    }
}

impl DropdownMenu for AccountButton {}

impl RenderOnce for AccountButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        self.base
            .flex()
            .items_center()
            .gap_2()
            .p_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(self.selected, |this| this.bg(theme.secondary_hover))
            .hover(|this| this.bg(theme.secondary_hover))
    }
}
