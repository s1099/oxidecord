//! A message's menu of actions, opened from its toolbar's "More" button or by
//! right-clicking the message.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::{EmojiTarget, HomeScreen, ReplyTarget};

#[derive(Clone)]
pub(super) struct MessageMenu {
    pub message_id: Id<MessageMarker>,
    pub reply: ReplyTarget,
    pub can_edit: bool,
    pub can_reply: bool,
    pub can_delete: bool,
    pub has_reactions: bool,
    /// The raw content, so a link hidden behind its GIF still copies.
    pub text: Option<SharedString>,
}

impl MessageMenu {
    pub(super) fn new(screen: &HomeScreen, message: &discord::Message) -> Self {
        Self {
            message_id: message.id,
            reply: ReplyTarget {
                message_id: message.id,
                author_id: message.author_id,
                author_name: screen
                    .author_name(message.author_id, &message.author_name)
                    .to_owned(),
                mention: true,
            },
            // Already open for editing, the action would do nothing.
            can_edit: screen.can_edit_message(message)
                && screen
                    .editing
                    .as_ref()
                    .is_none_or(|editing| editing.message_id != message.id),
            can_reply: message.system.is_none(),
            can_delete: screen.can_delete_message(message),
            has_reactions: !message.reactions.is_empty(),
            text: (message.system.is_none() && !message.content.trim().is_empty())
                .then(|| message.content.clone().into()),
        }
    }

    /// Fills `menu` in Discord's order. `full` adds react and reply, which the right-click menu
    /// needs but the toolbar already has buttons for.
    pub(super) fn build(
        &self,
        menu: PopupMenu,
        full: bool,
        screen: WeakEntity<HomeScreen>,
        cx: &App,
    ) -> PopupMenu {
        let message_id = self.message_id;
        let danger = cx.theme().danger;
        let action =
            |run: fn(&mut HomeScreen, Id<MessageMarker>, &mut Window, &mut Context<HomeScreen>)| {
                let screen = screen.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |this, cx| run(this, message_id, window, cx));
                    }
                }
            };

        let menu = menu
            .when(full, |menu| {
                menu.item(
                    PopupMenuItem::new("Add Reaction")
                        .icon(Icon::default().path("icons/smile-plus.svg"))
                        .on_click(action(|this, message_id, window, cx| {
                            // Opens beside the picked item, where the pointer still is.
                            this.toggle_emoji_picker(
                                window.mouse_position(),
                                EmojiTarget::Reaction(message_id),
                                window,
                                cx,
                            );
                        })),
                )
            })
            .when(self.has_reactions, |menu| {
                menu.item(
                    PopupMenuItem::new("View Reactions")
                        .icon(Icon::default().path("icons/smile.svg"))
                        .on_click(action(|this, message_id, window, cx| {
                            this.view_reactions(message_id, None, window, cx)
                        })),
                )
            })
            .when(full && self.has_reactions, |menu| menu.separator())
            .when(self.can_edit, |menu| {
                menu.item(
                    PopupMenuItem::new("Edit Message")
                        .icon(Icon::default().path("icons/pencil.svg"))
                        .on_click(action(|this, message_id, window, cx| {
                            this.start_editing(message_id, window, cx)
                        })),
                )
            })
            .when(full && self.can_reply, |menu| {
                let screen = screen.clone();
                let reply = self.reply.clone();
                menu.item(PopupMenuItem::new("Reply").icon(IconName::Undo2).on_click(
                    move |_, window, cx| {
                        if let Some(screen) = screen.upgrade() {
                            screen
                                .update(cx, |this, cx| this.start_reply(reply.clone(), window, cx));
                        }
                    },
                ))
            })
            .when_some(self.text.clone(), |menu, text| {
                menu.item(
                    PopupMenuItem::new("Copy Text")
                        .icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()))
                        }),
                )
            })
            .item(
                PopupMenuItem::new("Mark Unread")
                    .icon(Icon::default().path("icons/mail.svg"))
                    .on_click(action(|this, message_id, _, cx| {
                        this.mark_unread(message_id, cx)
                    })),
            )
            .item(
                PopupMenuItem::new("Copy Message Link")
                    .icon(Icon::default().path("icons/link.svg"))
                    .on_click(action(|this, message_id, _, cx| {
                        this.copy_message_link(message_id, cx)
                    })),
            );
        if !self.can_delete {
            return menu;
        }

        menu.separator().item(
            PopupMenuItem::element(move |_, _| div().text_color(danger).child("Delete Message"))
                .icon(Icon::default().path("icons/trash-2.svg").text_color(danger))
                .on_click(action(|this, message_id, _, cx| {
                    this.delete_message(message_id, cx)
                })),
        )
    }
}

impl HomeScreen {
    pub(super) fn start_reply(
        &mut self,
        target: ReplyTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replying_to = Some(target);
        self.message_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Keeps the message lit while the menu is up, as the toolbar's "More" menu does.
    pub(super) fn message_context_menu(
        &self,
        row: Stateful<Div>,
        message_id: Id<MessageMarker>,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        use gpui_kit::component::menu::ContextMenuExt as _;

        let screen = cx.entity().downgrade();
        row.context_menu(move |popup, _, cx| {
            // Decided when the menu opens, not per row render, which also catches later changes.
            let Some(entity) = screen.upgrade() else {
                return popup;
            };
            let this = entity.read(cx);
            let Some(menu) = this
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .map(|message| MessageMenu::new(this, message))
            else {
                return popup;
            };
            entity.update(cx, |this, cx| {
                this.message_menu = Some(message_id);
                cx.notify();
            });
            cx.subscribe_self(move |_, _: &DismissEvent, cx| {
                entity.update(cx, |this, cx| {
                    if this.message_menu == Some(message_id) {
                        this.message_menu = None;
                        cx.notify();
                    }
                });
            })
            .detach();
            menu.build(popup, true, screen.clone(), cx)
        })
    }
}
