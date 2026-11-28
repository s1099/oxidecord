//! The Friends page, shown on the DM home with no conversation open: the Online, All,
//! Pending and Blocked lists, the Add Friend form, and the Active Now column. Also the
//! friend actions other views reuse — the profile card's and full profile's friend button,
//! and the relationship items in their menus.

use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, ActivityKind, PresenceStatus, Relationship, RelationshipKind};
use crate::screens::home::HomeScreen;
use crate::screens::home::data::friends::RelationshipAction;
use crate::screens::home::friends::{AddFriendResult, FriendsTab, matches_search, status_label};
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{avatar, bot_tag, mention_badge};
use crate::ui::button::Button;
use crate::ui::depth::radius;
use crate::ui::theme;

use super::content::{header, header_actions, header_content};

const LIST_PADDING_X: f32 = 30.;
const ROW_HEIGHT: f32 = 62.;
const ACTION_SIZE: f32 = 36.;
const ACTIVE_NOW_WIDTH: f32 = 360.;
/// Below this window width the Active Now column gives its room to the list, as in Discord.
pub(super) const ACTIVE_NOW_MIN_WINDOW: f32 = 1200.;

impl HomeScreen {
    pub(super) fn render_friends(&self, cx: &Context<Self>) -> AnyElement {
        let tab = self.friends.tab;
        let body = if tab == FriendsTab::AddFriend {
            self.render_add_friend(cx).into_any_element()
        } else {
            self.render_friend_list(tab, cx).into_any_element()
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_friends_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(v_flex().flex_1().h_full().min_w_0().child(body))
                    .when(self.friends.wide, |this| {
                        this.child(self.render_active_now(cx))
                    }),
            )
            .into_any_element()
    }

    fn render_friends_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tabs = [
            FriendsTab::Online,
            FriendsTab::All,
            FriendsTab::Pending,
            FriendsTab::Blocked,
        ];
        let pending = self.friends.incoming_count();

        header(
            header_content()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::default()
                                .path("icons/friends.svg")
                                .size_5()
                                .text_color(theme.muted_foreground),
                        )
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Friends")),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .size(px(4.))
                        .mx_1()
                        .rounded_full()
                        .bg(theme.muted_foreground),
                )
                .child(
                    header_actions()
                        .gap_1()
                        .children(tabs.into_iter().map(|tab| {
                            let badge = (tab == FriendsTab::Pending)
                                .then(|| mention_badge(pending as u32, cx))
                                .flatten();
                            self.render_friends_tab(tab, badge, cx)
                        }))
                        .child(self.render_add_friend_tab(cx)),
                )
                .child(div().flex_1()),
            cx,
        )
    }

    fn render_friends_tab(
        &self,
        tab: FriendsTab,
        badge: Option<Div>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.friends.tab == tab;
        h_flex()
            .id(SharedString::from(format!("friends-tab-{}", tab.label())))
            .px(px(8.))
            .h(px(26.))
            .gap_1p5()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if selected {
                theme.foreground
            } else {
                theme::secondary_text(cx)
            })
            .when(selected, |this| this.bg(theme.accent))
            .hover(|this| this.bg(theme.accent).text_color(theme.foreground))
            .child(tab.label())
            .children(badge)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_friends_tab(tab, window, cx);
            }))
    }

    /// Filled green until chosen, then green text on nothing, as Discord draws it.
    fn render_add_friend_tab(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.friends.tab == FriendsTab::AddFriend;
        h_flex()
            .id("friends-tab-add")
            .ml_2()
            .px(px(8.))
            .h(px(26.))
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .map(|this| {
                if selected {
                    this.text_color(theme.success)
                        .bg(theme.success.opacity(0.12))
                } else {
                    this.bg(theme.success)
                        .text_color(theme.success_foreground)
                        .hover(|this| this.opacity(0.9))
                }
            })
            .child(FriendsTab::AddFriend.label())
            .on_click(cx.listener(|this, _, window, cx| {
                this.select_friends_tab(FriendsTab::AddFriend, window, cx);
            }))
    }

    fn render_friend_list(&self, tab: FriendsTab, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let query = self.friends.search.read(cx).value().to_string();
        let mut list: Vec<&Relationship> = match tab {
            FriendsTab::Online => self
                .friends
                .of_kind(|kind| kind == RelationshipKind::Friend),
            FriendsTab::All => self
                .friends
                .of_kind(|kind| kind == RelationshipKind::Friend),
            FriendsTab::Pending => self.friends.of_kind(RelationshipKind::is_pending),
            FriendsTab::Blocked | FriendsTab::AddFriend => self
                .friends
                .of_kind(|kind| kind == RelationshipKind::Blocked),
        };
        if tab == FriendsTab::Online {
            list.retain(|friend| self.user_status(friend.user_id) != PresenceStatus::Invisible);
        }
        // Requests waiting on the user come first.
        if tab == FriendsTab::Pending {
            list.sort_by_key(|request| request.kind != RelationshipKind::Incoming);
        }
        let total = list.len();
        list.retain(|relationship| matches_search(relationship, &query));

        let heading = match tab {
            FriendsTab::Online => "Online",
            FriendsTab::All => "All friends",
            FriendsTab::Pending => "Pending",
            _ => "Blocked",
        };

        let content = if total == 0 {
            empty_state(empty_text(tab), cx).into_any_element()
        } else if list.is_empty() {
            empty_state(
                "Wumpus looked, but couldn't find anyone with that name.",
                cx,
            )
            .into_any_element()
        } else {
            v_flex()
                .id("friend-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.friends.scroll)
                .px(px(LIST_PADDING_X - 10.))
                .pb_4()
                .child(
                    div()
                        .px(px(10.))
                        .pt(px(16.))
                        .pb(px(8.))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(format!("{heading} — {}", list.len())),
                )
                .children(
                    list.into_iter()
                        .map(|relationship| self.render_friend_row(relationship, tab, cx)),
                )
                .into_any_element()
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div().px(px(LIST_PADDING_X)).pt(px(16.)).child(
                    Input::new(&self.friends.search).cleanable(true).prefix(
                        Icon::new(IconName::Search)
                            .small()
                            .text_color(theme.muted_foreground),
                    ),
                ),
            )
            .child(content)
    }

    fn render_friend_row(
        &self,
        relationship: &Relationship,
        tab: FriendsTab,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let user_id = relationship.user_id;
        let kind = relationship.kind;
        let name = relationship.display_name().to_owned();
        let group: SharedString = format!("friend-{user_id}").into();
        let hover_bg = theme.accent;
        let status = self.user_status(user_id);
        let show_status = kind == RelationshipKind::Friend;

        let subtitle = match kind {
            RelationshipKind::Friend => self.friend_subtitle(user_id),
            RelationshipKind::Incoming => "Incoming Friend Request".to_owned(),
            RelationshipKind::Outgoing => "Outgoing Friend Request".to_owned(),
            RelationshipKind::Blocked => "Blocked".to_owned(),
        };

        let face =
            div()
                .relative()
                .flex_shrink_0()
                .child(avatar(
                    relationship.name.clone(),
                    relationship.avatar_url.clone(),
                    px(32.),
                ))
                .when(show_status, |this| {
                    this.child(div().absolute().right(px(-3.)).bottom(px(-3.)).child(
                        status_indicator(
                            status,
                            px(10.),
                            theme.background,
                            Some((group.clone(), hover_bg)),
                            theme,
                        ),
                    ))
                });

        let identity = v_flex()
            .flex_1()
            .min_w_0()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(name.clone()),
                    )
                    .when(relationship.bot, |this| this.child(bot_tag(cx)))
                    // The username shows on hover, or always where there's no presence to show.
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .when(show_status, |this| {
                                this.invisible()
                                    .group_hover(group.clone(), |style| style.visible())
                            })
                            .child(relationship.username.clone()),
                    ),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(subtitle),
            );

        let actions = self.render_friend_actions(relationship, cx);
        let screen = cx.entity().downgrade();
        let menu_name = name.clone();
        let (avatar_url, card_name) = (relationship.avatar_url.clone(), relationship.name.clone());
        let (modal_name, modal_avatar) = (card_name.clone(), avatar_url.clone());

        h_flex()
            .id(("friend-row", user_id.get()))
            .group(group)
            .h(px(ROW_HEIGHT))
            .px(px(10.))
            .gap_3()
            .items_center()
            .rounded(radius::ITEM)
            .border_t_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg).border_color(transparent_black()))
            .child(face)
            .child(identity)
            .child(actions)
            .on_click(
                cx.listener(move |this, event: &ClickEvent, window, cx| match kind {
                    RelationshipKind::Friend => this.message_user(user_id, window, cx),
                    _ => this.open_profile(
                        user_id,
                        card_name.clone(),
                        avatar_url.clone(),
                        false,
                        event.position(),
                        window,
                        cx,
                    ),
                }),
            )
            .context_menu(move |menu, _, _| {
                let menu = menu.item(PopupMenuItem::new("Profile").on_click({
                    let screen = screen.clone();
                    let (name, avatar_url) = (modal_name.clone(), modal_avatar.clone());
                    move |_, window, cx| {
                        let _ = screen.update(cx, |this, cx| {
                            this.open_profile_modal(
                                user_id,
                                None,
                                name.clone(),
                                avatar_url.clone(),
                                None,
                                window,
                                cx,
                            );
                        });
                    }
                }));
                let menu = if kind == RelationshipKind::Friend {
                    menu.item(PopupMenuItem::new("Message").on_click({
                        let screen = screen.clone();
                        move |_, window, cx| {
                            let _ = screen.update(cx, |this, cx| {
                                this.message_user(user_id, window, cx);
                            });
                        }
                    }))
                } else {
                    menu
                };
                relationship_items(
                    menu.separator(),
                    screen.clone(),
                    user_id,
                    menu_name.clone(),
                    Some(kind),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Copy User ID").on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                    }),
                )
            })
            .when(tab == FriendsTab::Blocked, |this| this.opacity(0.8))
    }

    /// The round buttons at a row's end.
    fn render_friend_actions(
        &self,
        relationship: &Relationship,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let user_id = relationship.user_id;
        let name = relationship.display_name().to_owned();
        let theme = cx.theme();
        let action = |id: &'static str, icon: Icon, tip: &'static str| {
            Button::new((id, user_id.get()))
                .icon(icon)
                .size(px(ACTION_SIZE))
                .rounded_full()
                .tooltip(tip)
        };

        let buttons: Vec<AnyElement> = match relationship.kind {
            RelationshipKind::Friend => {
                let screen = cx.entity().downgrade();
                vec![
                    action(
                        "friend-message",
                        Icon::default().path("icons/message-circle.svg"),
                        "Message",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.message_user(user_id, window, cx);
                    }))
                    .into_any_element(),
                    action("friend-more", Icon::new(IconName::EllipsisVertical), "More")
                        .dropdown_menu(move |menu, _, _| {
                            let call_screen = screen.clone();
                            menu.item(
                                PopupMenuItem::new("Start Voice Call")
                                    .icon(Icon::default().path("icons/phone.svg"))
                                    .on_click(move |_, window, cx| {
                                        let _ = call_screen.update(cx, |this, cx| {
                                            this.call_user(user_id, window, cx);
                                        });
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("Remove Friend")
                                    .icon(Icon::default().path("icons/user-minus.svg"))
                                    .on_click({
                                        let screen = screen.clone();
                                        let name = name.clone();
                                        move |_, window, cx| {
                                            let _ = screen.update(cx, |this, cx| {
                                                this.confirm_remove_friend(
                                                    user_id,
                                                    name.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                        })
                        .into_any_element(),
                ]
            }
            RelationshipKind::Incoming => vec![
                action("friend-accept", Icon::new(IconName::Check), "Accept")
                    .text_color(theme.success)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change_relationship(user_id, RelationshipAction::Add, window, cx);
                    }))
                    .into_any_element(),
                action("friend-ignore", Icon::new(IconName::Close), "Ignore")
                    .text_color(theme.danger)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change_relationship(user_id, RelationshipAction::Remove, window, cx);
                    }))
                    .into_any_element(),
            ],
            RelationshipKind::Outgoing => vec![
                action("friend-cancel", Icon::new(IconName::Close), "Cancel")
                    .text_color(theme.danger)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change_relationship(user_id, RelationshipAction::Remove, window, cx);
                    }))
                    .into_any_element(),
            ],
            RelationshipKind::Blocked => vec![
                action(
                    "friend-unblock",
                    Icon::default().path("icons/user-x.svg"),
                    "Unblock",
                )
                .text_color(theme.danger)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.change_relationship(user_id, RelationshipAction::Remove, window, cx);
                }))
                .into_any_element(),
            ],
        };

        h_flex()
            .flex_shrink_0()
            .gap(px(10.))
            // Keeps a press on a button from also opening the row.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(buttons)
    }

    /// What a friend's row says under their name: their custom status, else what they're
    /// doing, else their status.
    fn friend_subtitle(&self, user_id: Id<UserMarker>) -> String {
        let status = self.user_status(user_id);
        if status == PresenceStatus::Invisible {
            return status_label(status).to_owned();
        }
        let activities = self.user_activities.get(&user_id);
        let custom = activities.and_then(|activities| {
            activities
                .iter()
                .find(|activity| activity.kind == ActivityKind::Custom)
                .and_then(|activity| activity.state.clone())
        });
        custom
            .or_else(|| activities.and_then(|activities| activities.iter().find_map(activity_line)))
            .unwrap_or_else(|| status_label(status).to_owned())
    }

    fn render_add_friend(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let empty = self.friends.add_input.read(cx).value().trim().is_empty();
        let sending = self.friends.sending;

        let result = self.friends.add_result.as_ref().map(|result| match result {
            AddFriendResult::Sent(username) => div().text_sm().text_color(theme.success).child(
                format!("Success! Your friend request to {username} was sent."),
            ),
            AddFriendResult::Failed(error) => div()
                .text_sm()
                .text_color(theme.danger)
                .child(error.clone()),
        });

        let send = Button::new("send-friend-request")
            .primary()
            .small()
            .label("Send Friend Request")
            .loading(sending)
            .disabled(empty || sending)
            .on_click(cx.listener(|this, _, window, cx| this.send_friend_request(window, cx)));

        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .px(px(LIST_PADDING_X))
                    .py(px(20.))
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child("Add Friend"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("You can add friends with their Discord username."),
                    )
                    .child(
                        div().pt_2().child(
                            Input::new(&self.friends.add_input)
                                .large()
                                .disabled(sending)
                                .suffix(send),
                        ),
                    )
                    .children(result),
            )
            .child(empty_state(
                "Wumpus is waiting on friends. You don't have to though!",
                cx,
            ))
    }

    /// Friends playing, listening or watching something, with what it is.
    fn render_active_now(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut active: Vec<(&Relationship, &discord::Activity)> = self
            .friends
            .of_kind(|kind| kind == RelationshipKind::Friend)
            .into_iter()
            .filter(|friend| self.user_status(friend.user_id) != PresenceStatus::Invisible)
            .filter_map(|friend| {
                let activity = self
                    .user_activities
                    .get(&friend.user_id)?
                    .iter()
                    .find(|activity| activity.kind != ActivityKind::Custom)?;
                Some((friend, activity))
            })
            .collect();
        active.sort_by_key(|(_, activity)| std::cmp::Reverse(activity.started_at));

        let body = if active.is_empty() {
            v_flex()
                .pt_4()
                .gap_1()
                .items_center()
                .text_center()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("It's quiet for now..."),
                )
                .child(div().text_sm().text_color(theme.muted_foreground).child(
                    "When a friend starts an activity—like playing a game or hanging out on \
                     voice—we'll show it here!",
                ))
                .into_any_element()
        } else {
            v_flex()
                .gap_2()
                .children(
                    active
                        .into_iter()
                        .map(|(friend, activity)| self.render_active_card(friend, activity, cx)),
                )
                .into_any_element()
        };

        v_flex()
            .id("active-now")
            .w(px(ACTIVE_NOW_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child("Active Now"),
            )
            .child(body)
    }

    fn render_active_card(
        &self,
        friend: &Relationship,
        activity: &discord::Activity,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let user_id = friend.user_id;
        let detail = activity.details.clone().or_else(|| activity.state.clone());
        let (name, avatar_url) = (friend.name.clone(), friend.avatar_url.clone());

        v_flex()
            .id(("active-now", user_id.get()))
            .p_4()
            .gap_3()
            .rounded(radius::CARD)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(|this| this.bg(theme.accent))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .relative()
                            .flex_shrink_0()
                            .child(avatar(
                                friend.name.clone(),
                                friend.avatar_url.clone(),
                                px(32.),
                            ))
                            .child(div().absolute().right(px(-3.)).bottom(px(-3.)).child(
                                status_indicator(
                                    self.user_status(user_id),
                                    px(10.),
                                    theme.background,
                                    None,
                                    theme,
                                ),
                            )),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(friend.display_name().to_owned()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(activity_line(activity).unwrap_or_default()),
                            ),
                    )
                    .when_some(activity.large_image.clone(), |this, url| {
                        this.child(
                            img(url)
                                .image_cache(&crate::ui::avatar::cache(cx))
                                .flex_shrink_0()
                                .size(px(40.))
                                .rounded(px(8.)),
                        )
                    }),
            )
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .truncate()
                        .text_sm()
                        .text_color(theme::secondary_text(cx))
                        .child(detail),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.open_profile(
                    user_id,
                    name.clone(),
                    avatar_url.clone(),
                    false,
                    event.position(),
                    window,
                    cx,
                );
            }))
    }
}

/// "Playing Minecraft", as a row or card words an activity; `None` for a custom status.
fn activity_line(activity: &discord::Activity) -> Option<String> {
    let verb = match activity.kind {
        ActivityKind::Custom => return None,
        ActivityKind::Streaming => "Streaming",
        kind => kind.heading(),
    };
    Some(format!("{verb} {}", activity.name))
}

fn empty_text(tab: FriendsTab) -> &'static str {
    match tab {
        FriendsTab::Online => "No one's around to play with Wumpus.",
        FriendsTab::All => "Wumpus is waiting on friends. You don't have to though!",
        FriendsTab::Pending => "There are no pending friend requests. Here's Wumpus for now.",
        FriendsTab::Blocked | FriendsTab::AddFriend => "You can't unblock the Wumpus.",
    }
}

fn empty_state(text: &'static str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .px_8()
        .child(
            Icon::default()
                .path("icons/friends.svg")
                .size(px(96.))
                .text_color(theme.muted_foreground.opacity(0.4)),
        )
        .child(
            div()
                .text_sm()
                .text_center()
                .text_color(theme.muted_foreground)
                .child(text),
        )
}

/// The friend and block items for a user's menu, by where the two stand. `None` when the
/// relationship is unknown, which offers the same as no relationship at all.
pub(super) fn relationship_items(
    menu: PopupMenu,
    screen: WeakEntity<HomeScreen>,
    user_id: Id<UserMarker>,
    name: String,
    kind: Option<RelationshipKind>,
) -> PopupMenu {
    let act = |label: &'static str, icon: &'static str, action: RelationshipAction| {
        let screen = screen.clone();
        PopupMenuItem::new(label)
            .icon(Icon::default().path(icon))
            .on_click(move |_, window, cx| {
                let _ = screen.update(cx, |this, cx| {
                    this.change_relationship(user_id, action, window, cx);
                });
            })
    };
    let menu = match kind {
        None => menu.item(act(
            "Add Friend",
            "icons/user-plus.svg",
            RelationshipAction::Add,
        )),
        Some(RelationshipKind::Incoming) => menu
            .item(act(
                "Accept Friend Request",
                "icons/user-check.svg",
                RelationshipAction::Add,
            ))
            .item(act(
                "Ignore Friend Request",
                "icons/user-x.svg",
                RelationshipAction::Remove,
            )),
        Some(RelationshipKind::Outgoing) => menu.item(act(
            "Cancel Friend Request",
            "icons/user-x.svg",
            RelationshipAction::Remove,
        )),
        Some(RelationshipKind::Friend) => {
            let (nick_screen, remove_screen, name) = (screen.clone(), screen.clone(), name.clone());
            menu.item(
                PopupMenuItem::new("Edit Friend Nickname")
                    .icon(Icon::default().path("icons/pencil.svg"))
                    .on_click(move |_, window, cx| {
                        let _ = nick_screen.update(cx, |this, cx| {
                            this.edit_friend_nickname(user_id, window, cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("Remove Friend")
                    .icon(Icon::default().path("icons/user-minus.svg"))
                    .on_click(move |_, window, cx| {
                        let _ = remove_screen.update(cx, |this, cx| {
                            this.confirm_remove_friend(user_id, name.clone(), window, cx);
                        });
                    }),
            )
        }
        Some(RelationshipKind::Blocked) => menu,
    };
    if kind == Some(RelationshipKind::Blocked) {
        menu.item(act("Unblock", "icons/ban.svg", RelationshipAction::Remove))
    } else {
        let screen = screen.clone();
        menu.item(
            PopupMenuItem::new("Block")
                .icon(Icon::default().path("icons/ban.svg"))
                .on_click(move |_, window, cx| {
                    let _ = screen.update(cx, |this, cx| {
                        this.confirm_block(user_id, name.clone(), window, cx);
                    });
                }),
        )
    }
}

/// The profile card's and full profile's friend button: send a request, accept one, or
/// show that one's waiting. `None` for a friend or a blocked user, who get none.
pub(super) fn friend_button(
    id: &'static str,
    user_id: Id<UserMarker>,
    kind: Option<RelationshipKind>,
    labelled: bool,
    screen: WeakEntity<HomeScreen>,
) -> Option<Button> {
    let (label, icon, action) = match kind {
        None => (
            "Add Friend",
            "icons/user-plus.svg",
            Some(RelationshipAction::Add),
        ),
        Some(RelationshipKind::Incoming) => (
            "Accept Friend Request",
            "icons/user-check.svg",
            Some(RelationshipAction::Add),
        ),
        Some(RelationshipKind::Outgoing) => ("Friend Request Sent", "icons/clock.svg", None),
        Some(RelationshipKind::Friend | RelationshipKind::Blocked) => return None,
    };
    let button = Button::new(id)
        .icon(Icon::default().path(icon))
        .small()
        .disabled(action.is_none())
        .map(|button| {
            if labelled {
                button.label(label)
            } else {
                button.tooltip(label)
            }
        });
    Some(match action {
        Some(action) => button.on_click(move |_, window, cx| {
            let _ = screen.update(cx, |this, cx| {
                this.change_relationship(user_id, action, window, cx);
            });
        }),
        None => button,
    })
}
