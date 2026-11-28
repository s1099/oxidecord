//! Opening the Friends page, and changing relationships: sending, accepting and cancelling
//! friend requests, unfriending, blocking, and friends' nicknames.
//!
//! Every change waits for Discord's `RELATIONSHIP_*` dispatch to show, as Discord's client
//! does; a refusal comes back as a toast.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    WindowExt as _, button::ButtonVariant, dialog::DialogButtonProps, h_flex, v_flex,
};
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, Relationship, RelationshipKind};
use crate::platform::notify::{Toast, ToastId};
use crate::screens::home::friends::{AddFriendResult, FriendsTab};
use crate::screens::home::{HomeScreen, View};
use crate::ui::button::Button;
use crate::ui::depth;

/// What a relationship action asks of Discord.
#[derive(Clone, Copy)]
pub(in crate::screens::home) enum RelationshipAction {
    /// Accepts their request, or sends one.
    Add,
    /// Unfriends, cancels or ignores a request, or unblocks.
    Remove,
    Block,
}

impl HomeScreen {
    /// The DM home with nothing open, which is where Discord lists friends.
    pub(in crate::screens::home) fn show_friends(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::DirectMessages && self.selected_channel.is_none() {
            return;
        }
        self.view = View::DirectMessages;
        self.selected_channel = None;
        // So coming back to the DM home lands here again, as in Discord.
        self.last_dm = None;
        self.reset_conversation(None, window, cx);
        cx.notify();
    }

    pub(in crate::screens::home) fn select_friends_tab(
        &mut self,
        tab: FriendsTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_friends(window, cx);
        if self.friends.tab == tab {
            return;
        }
        self.friends.tab = tab;
        self.friends.scroll.set_offset(Point::default());
        if tab == FriendsTab::AddFriend {
            self.friends
                .add_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn handle_relationship_add(
        &mut self,
        relationship: Relationship,
        notify: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if notify {
            self.notify_relationship(&relationship, window);
        }
        self.friends
            .relationships
            .insert(relationship.user_id, relationship);
        cx.notify();
    }

    /// A request received, or one the user sent accepted. Clicking the toast opens Pending
    /// or the friend's DM through [`Self::open_notification`], which reads a toast with no
    /// channel as this.
    fn notify_relationship(&self, relationship: &Relationship, window: &Window) {
        let body = match relationship.kind {
            RelationshipKind::Incoming if !relationship.spam => "Sent you a friend request.",
            RelationshipKind::Friend
                if self.friends.kind(relationship.user_id) == Some(RelationshipKind::Outgoing) =>
            {
                "Accepted your friend request."
            }
            _ => return,
        };
        let Some(notifier) = &self.notifier else {
            return;
        };
        if self.presence_status == Some(discord::PresenceStatus::DoNotDisturb)
            || window.is_window_active() && self.on_friends_page()
        {
            return;
        }
        notifier.show(Toast {
            id: ToastId {
                guild: None,
                channel: 0,
                message: relationship.user_id.get(),
            },
            title: relationship.display_name().to_owned(),
            body: body.to_owned(),
            avatar_url: relationship.avatar_url.clone(),
        });
    }

    /// A relationship toast's click: Pending for a request, else the friend's DM.
    pub(in crate::screens::home) fn open_relationship_toast(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.friends.kind(user_id) {
            Some(RelationshipKind::Friend) => self.message_user(user_id, window, cx),
            _ => self.select_friends_tab(FriendsTab::Pending, window, cx),
        }
    }

    /// Opens their DM and rings them there.
    pub(in crate::screens::home) fn call_user(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_dm(user_id, window, cx, |this, channel_id, window, cx| {
            this.open_dm(channel_id, window, cx);
            this.start_dm_call(cx);
        });
    }

    pub(in crate::screens::home) fn on_friends_page(&self) -> bool {
        self.view == View::DirectMessages && self.selected_channel.is_none()
    }

    /// The Add Friend tab's button and enter key.
    pub(in crate::screens::home) fn send_friend_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let username = self
            .friends
            .add_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        if username.is_empty() || self.friends.sending {
            return;
        }
        self.friends.sending = true;
        self.friends.add_result = None;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = discord::send_friend_request(username.clone()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.friends.sending = false;
                this.friends.add_result = Some(match result {
                    Ok(()) => {
                        // Cleared first: its change event drops the outcome.
                        this.friends
                            .add_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        AddFriendResult::Sent(username)
                    }
                    Err(err) => AddFriendResult::Failed(err),
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn change_relationship(
        &mut self,
        user_id: Id<UserMarker>,
        action: RelationshipAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = match action {
                RelationshipAction::Add => discord::add_friend(user_id).await,
                RelationshipAction::Remove => discord::remove_relationship(user_id).await,
                RelationshipAction::Block => discord::block_user(user_id).await,
            };
            let Err(err) = result else {
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                window.push_notification(Notification::error(err), cx);
            });
        })
        .detach();
    }

    /// Unfriending asks first, as Discord does.
    pub(in crate::screens::home) fn confirm_remove_friend(
        &mut self,
        user_id: Id<UserMarker>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_relationship(
            user_id,
            RelationshipAction::Remove,
            format!("Remove '{name}'"),
            format!("Are you sure you want to remove {name} from your friends?"),
            "Remove Friend",
            window,
            cx,
        );
    }

    pub(in crate::screens::home) fn confirm_block(
        &mut self,
        user_id: Id<UserMarker>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let friend = self.friends.kind(user_id) == Some(RelationshipKind::Friend);
        let detail = if friend {
            " Blocking will also remove them from your friends list."
        } else {
            ""
        };
        self.confirm_relationship(
            user_id,
            RelationshipAction::Block,
            format!("Block {name}?"),
            format!(
                "They won't be able to message you or send you friend requests, and their \
                 messages won't notify you.{detail}"
            ),
            "Block",
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_relationship(
        &mut self,
        user_id: Id<UserMarker>,
        action: RelationshipAction,
        title: String,
        body: String,
        ok: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let screen = screen.clone();
            dialog
                .confirm()
                .border_color(depth::ring(cx))
                .width(px(440.))
                .title(SharedString::from(title.clone()))
                .child(body.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok)
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |this, cx| {
                            this.change_relationship(user_id, action, window, cx);
                        });
                    }
                    true
                })
        });
    }

    /// Discord's "Edit Friend Nickname": a private name shown in place of theirs.
    pub(in crate::screens::home) fn edit_friend_nickname(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(relationship) = self.friends.relationships.get(&user_id) else {
            return;
        };
        let current = relationship.nickname.clone().unwrap_or_default();
        let placeholder = relationship.name.clone();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(placeholder)
                .validate(|text, _| text.chars().count() <= 32);
            state.set_value(current, window, cx);
            state
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let screen = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _, cx| {
            let save = {
                let (screen, input) = (screen.clone(), input.clone());
                move |window: &mut Window, cx: &mut App| {
                    let nickname = input.read(cx).value().trim().to_owned();
                    let nickname = (!nickname.is_empty()).then_some(nickname);
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |this, cx| {
                            this.save_friend_nickname(user_id, nickname, window, cx);
                        });
                    }
                    window.close_dialog(cx);
                }
            };
            let screen_reset = screen.clone();
            dialog
                .border_color(depth::ring(cx))
                .w(px(440.))
                .title("Edit Friend Nickname")
                .child(
                    v_flex()
                        .gap_2()
                        .child("Friend nicknames are only visible to you.")
                        .child(Input::new(&input)),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap_2()
                        .child(
                            Button::new("nickname-reset")
                                .ghost()
                                .label("Reset Nickname")
                                .on_click(move |_, window, cx| {
                                    if let Some(screen) = screen_reset.upgrade() {
                                        screen.update(cx, |this, cx| {
                                            this.save_friend_nickname(user_id, None, window, cx);
                                        });
                                    }
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("nickname-cancel")
                                        .ghost()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("nickname-save")
                                        .primary()
                                        .label("Save")
                                        .on_click({
                                            let save = save.clone();
                                            move |_, window, cx| save(window, cx)
                                        }),
                                ),
                        ),
                )
        });
    }

    fn save_friend_nickname(
        &mut self,
        user_id: Id<UserMarker>,
        nickname: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let Err(err) = discord::set_friend_nickname(user_id, nickname).await else {
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                window.push_notification(
                    Notification::error(format!("Couldn't change the nickname: {err}")),
                    cx,
                );
            });
        })
        .detach();
    }
}
