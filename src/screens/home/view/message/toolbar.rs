//! The floating action toolbar revealed while a message is hovered.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::Button,
    button::ButtonVariants as _,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::{HomeScreen, ReplyTarget};

impl HomeScreen {
    /// Sits at the top-right of the message, shown only while `group_name` —
    /// the message row's hover group — is hovered.
    ///
    /// Holding shift flattens the overflow menu into the bar, in Discord's
    /// order — copy link, reply, delete — and drops the "More" button, so the
    /// actions behind it are one click away.
    pub(super) fn render_message_toolbar(
        &self,
        message: &discord::Message,
        group_name: &SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let message_id = message.id;
        let can_delete = self.can_delete_message(message);
        let expanded = self.shift_held;

        div()
            .absolute()
            .top(px(-16.))
            .right(px(12.))
            .invisible()
            .group_hover(group_name.clone(), |this| this.visible())
            .child(
                h_flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.))
                    .shadow_md()
                    .when(expanded, |this| {
                        this.child(
                            Button::new(("message-copy-link", message_id.get()))
                                .icon(Icon::default().path("icons/link.svg"))
                                .ghost()
                                .small()
                                .tooltip("Copy Message Link")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_message_link(message_id, cx);
                                })),
                        )
                    })
                    .child(self.reply_button(message, cx))
                    .when(expanded && can_delete, |this| {
                        this.child(
                            Button::new(("message-delete", message_id.get()))
                                .icon(Icon::default().path("icons/trash-2.svg"))
                                .ghost()
                                .small()
                                .text_color(theme.danger)
                                .tooltip("Delete Message")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete_message(message_id, cx);
                                })),
                        )
                    })
                    .when(!expanded, |this| {
                        this.child(self.more_menu(message_id, can_delete, cx))
                    }),
            )
    }

    fn reply_button(&self, message: &discord::Message, cx: &Context<Self>) -> impl IntoElement {
        Button::new(("message-reply", message.id.get()))
            .icon(IconName::Undo2)
            .ghost()
            .small()
            .tooltip("Reply")
            .on_click(cx.listener({
                let target = ReplyTarget {
                    message_id: message.id,
                    author_name: message.author_name.clone(),
                };
                move |this, _, window, cx| {
                    this.replying_to = Some(target.clone());
                    // Jump straight to composing, like Discord.
                    this.message_input.focus_handle(cx).focus(window);
                    cx.notify();
                }
            }))
    }

    /// The overflow menu, holding the actions shift exposes inline.
    fn more_menu(
        &self,
        message_id: Id<MessageMarker>,
        can_delete: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let danger = cx.theme().danger;
        let screen = cx.entity().downgrade();

        Button::new(("message-more", message_id.get()))
            .icon(IconName::Ellipsis)
            .ghost()
            .small()
            .tooltip("More")
            .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                let menu = menu.item(
                    PopupMenuItem::new("Copy Message Link")
                        .icon(Icon::default().path("icons/link.svg"))
                        .on_click({
                            let screen = screen.clone();
                            move |_, _, cx| {
                                if let Some(screen) = screen.upgrade() {
                                    screen.update(cx, |this, cx| {
                                        this.copy_message_link(message_id, cx)
                                    });
                                }
                            }
                        }),
                );
                if !can_delete {
                    return menu;
                }

                menu.separator().item(
                    PopupMenuItem::element(move |_, _| {
                        div().text_color(danger).child("Delete Message")
                    })
                    .icon(Icon::default().path("icons/trash-2.svg").text_color(danger))
                    .on_click({
                        let screen = screen.clone();
                        move |_, _, cx| {
                            if let Some(screen) = screen.upgrade() {
                                screen.update(cx, |this, cx| this.delete_message(message_id, cx));
                            }
                        }
                    }),
                )
            })
    }
}
