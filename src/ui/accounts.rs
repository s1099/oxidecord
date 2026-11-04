//! A saved account as the switchers draw it: picture, display name, and username.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable, Sizable as _, menu::DropdownMenu, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::Account;
use crate::ui::avatar::{CachedAvatar, avatar};

pub fn account_avatar(account: &Account, size: Pixels) -> CachedAvatar {
    avatar(account.display_name(), account.avatar_url.clone(), size)
}

/// Display name over `@username`.
pub fn account_label(account: &Account, cx: &App) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(account.display_name()),
        )
        .when(!account.username.is_empty(), |this| {
            this.child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("@{}", account.username)),
            )
        })
}

/// A saved account drawn as a small card, with the up-down chevrons that mark it as a switcher.
/// [`Selectable`] so it can show its menu is open.
#[derive(IntoElement)]
pub struct AccountCard {
    base: Stateful<Div>,
    account: Account,
    selected: bool,
}

impl AccountCard {
    pub fn new(id: impl Into<ElementId>, account: Account) -> Self {
        Self {
            base: div().id(id),
            account,
            selected: false,
        }
    }
}

impl Selectable for AccountCard {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Styled for AccountCard {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for AccountCard {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl DropdownMenu for AccountCard {}

impl RenderOnce for AccountCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let hover = theme.secondary_hover;
        self.base
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .cursor_pointer()
            .when(self.selected, |this| this.bg(hover))
            .hover(move |this| this.bg(hover))
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .xsmall()
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground),
            )
            .child(account_avatar(&self.account, px(24.)))
            .child(account_label(&self.account, cx))
    }
}
