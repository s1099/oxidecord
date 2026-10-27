//! The floating action toolbar revealed while a message is hovered.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, menu::PopupMenu,
    popover::Popover,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use twilight_model::id::{Id, marker::MessageMarker};

use super::menu::MessageMenu;
use crate::discord;
use crate::screens::home::{EmojiTarget, HomeScreen, ReplyTarget};
use crate::ui::button::Button;
use crate::ui::depth::{self, Finish, Level, Lit as _, radius};

#[derive(Clone, Copy)]
pub(super) enum Hovered {
    Message,
    Toolbar,
}

impl HomeScreen {
    /// Built only while [`toolbar_shown`](Self::toolbar_shown) holds for it.
    ///
    /// Shift flattens the overflow menu into the bar (copy link, reply, delete) and drops "More".
    pub(super) fn render_message_toolbar(
        &self,
        message: &discord::Message,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let message_id = message.id;
        let menu = MessageMenu::new(self, message);
        let can_delete = menu.can_delete;
        let can_edit = menu.can_edit;
        let expanded = self.shift_held;
        let reacting = self.reacting_to(message_id);

        div()
            .id(("message-toolbar", message_id.get()))
            .absolute()
            .top(px(-16.))
            .right(px(12.))
            // Belongs to its own message: the row it overhangs mustn't light up or take the
            // pointer.
            .occlude()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_hovered(Hovered::Toolbar, message_id, *hovered, cx);
            }))
            .child(
                h_flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .bg(theme.popover)
                    .border_1()
                    .border_color(depth::ring(cx))
                    .lit(
                        Level::Overlay,
                        Finish::Subtle,
                        px(34.),
                        radius::CONTROL - px(1.),
                        cx,
                    )
                    .rounded(radius::CONTROL)
                    .child(
                        Button::new(("message-react", message_id.get()))
                            .icon(Icon::default().path("icons/smile-plus.svg"))
                            .ghost()
                            .small()
                            .selected(reacting)
                            .tooltip("Add Reaction")
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.toggle_emoji_picker(
                                    event.position(),
                                    EmojiTarget::Reaction(message_id),
                                    window,
                                    cx,
                                );
                            })),
                    )
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
                    .when(can_edit, |this| {
                        this.child(
                            Button::new(("message-edit", message_id.get()))
                                .icon(Icon::default().path("icons/pencil.svg"))
                                .ghost()
                                .small()
                                .tooltip("Edit")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_editing(message_id, window, cx);
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
                    .when(!expanded, |this| this.child(self.more_menu(menu, cx))),
            )
    }

    /// The pointer is on the row or toolbar, or a picker or menu opened from the toolbar is
    /// (the pointer leaves the message to reach those).
    pub(super) fn toolbar_shown(&self, message_id: Id<MessageMarker>) -> bool {
        self.hovered_message == Some(message_id)
            || self.hovered_toolbar == Some(message_id)
            || self.message_menu == Some(message_id)
            || self.reacting_to(message_id)
    }

    /// Only a toolbar going up or down needs a redraw, which avoids redrawing the whole screen
    /// on every hover event.
    pub(super) fn set_hovered(
        &mut self,
        part: Hovered,
        message_id: Id<MessageMarker>,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let slot = match part {
            Hovered::Message => self.hovered_message,
            Hovered::Toolbar => self.hovered_toolbar,
        };
        let affected = [slot, Some(message_id)];
        let shown = |this: &Self| affected.map(|id| id.is_some_and(|id| this.toolbar_shown(id)));
        let before = shown(self);

        let slot = match part {
            Hovered::Message => &mut self.hovered_message,
            Hovered::Toolbar => &mut self.hovered_toolbar,
        };
        if hovered {
            *slot = Some(message_id);
        } else if *slot == Some(message_id) {
            *slot = None;
        }

        if shown(self) != before {
            cx.notify();
        }
    }

    fn reacting_to(&self, message_id: Id<MessageMarker>) -> bool {
        self.emoji_picker
            .as_ref()
            .is_some_and(|picker| picker.target == EmojiTarget::Reaction(message_id))
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
                    author_name: self
                        .author_name(message.author_id, &message.author_name)
                        .to_owned(),
                };
                move |this, _, window, cx| this.start_reply(target.clone(), window, cx)
            }))
    }

    fn more_menu(&self, menu: MessageMenu, cx: &Context<Self>) -> impl IntoElement {
        let message_id = menu.message_id;
        let screen = cx.entity().downgrade();
        let screen_for_open = screen.clone();
        let build =
            Rc::new(move |popup: PopupMenu, cx: &App| menu.build(popup, false, screen.clone(), cx));

        // Like gpui-component's `DropdownMenu` popover, but it reports opening and closing: the
        // toolbar has to stay up while the menu is.
        Popover::new(("message-more-menu", message_id.get()))
            .appearance(false)
            .overlay_closable(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new(("message-more", message_id.get()))
                    .icon(IconName::Ellipsis)
                    .ghost()
                    .small()
                    .tooltip("More"),
            )
            .content(move |_, window, cx| {
                // Built once per opening rather than every render, so the
                // menu keeps its focus and highlighted item.
                let menu_state =
                    window.use_keyed_state(("message-more-popup", message_id.get()), cx, |_, _| {
                        None::<Entity<PopupMenu>>
                    });
                if let Some(menu) = menu_state.read(cx).clone() {
                    return menu;
                }
                let build = build.clone();
                let menu = PopupMenu::build(window, cx, move |menu, _, cx| build(menu, cx));
                menu_state.update(cx, |state, _| *state = Some(menu.clone()));
                menu.focus_handle(cx).focus(window, cx);
                let popover = cx.entity();
                window
                    .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                        menu_state.update(cx, |state, _| *state = None);
                    })
                    .detach();
                menu
            })
            .on_open_change(move |open, _, cx| {
                if let Some(screen) = screen_for_open.upgrade() {
                    screen.update(cx, |this, cx| {
                        if *open {
                            this.message_menu = Some(message_id);
                        } else if this.message_menu == Some(message_id) {
                            this.message_menu = None;
                        }
                        cx.notify();
                    });
                }
            })
    }
}
