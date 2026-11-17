//! The card itself: banner, avatar, identity, mutuals, bio, roles, and the box
//! for messaging them.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    skeleton::Skeleton,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::{self, UserProfile};
use crate::screens::home::view::markdown::MarkdownOptions;
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{avatar, bot_tag};
use crate::screens::home::{HomeScreen, ProfilePopup, ProfileTab};
use crate::ui::depth::{self, Level, radius};
use crate::ui::tooltip;

pub(in crate::screens::home::view) const CARD_WIDTH: f32 = 300.;

/// The shorter height is the accent-colour strip for users without a banner.
const BANNER_HEIGHT: f32 = 105.;
const ACCENT_BANNER_HEIGHT: f32 = 60.;

const AVATAR_SIZE: f32 = 80.;

const STATUS_SIZE: f32 = 16.;

const AVATAR_RING: f32 = 6.;

const INSET: f32 = 16.;

const BADGE_SIZE: f32 = 18.;

const MUTUAL_AVATAR_SIZE: f32 = 16.;

/// Discord stacks no more than this many mutual friends' faces.
const MAX_MUTUAL_AVATARS: usize = 3;

impl HomeScreen {
    pub(super) fn render_profile_card(
        &self,
        popup: &ProfilePopup,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let profile = popup.profile.as_ref();
        let is_self = Some(popup.user_id) == self.self_user_id;

        // Accent colour and banner image only arrive with the fetched profile.
        let banner_url = profile.and_then(|profile| profile.display_banner().cloned());
        let banner_height = if banner_url.is_some() {
            BANNER_HEIGHT
        } else {
            ACCENT_BANNER_HEIGHT
        };
        let banner_color: Hsla = profile
            .and_then(|profile| profile.accent_color)
            .map_or(theme.primary, |color| rgb(color).into());

        // The opening message supplies avatar and name while the profile loads.
        let avatar_url = profile
            .and_then(|profile| profile.display_avatar().cloned())
            .or_else(|| popup.avatar_url.clone());
        let status_inset =
            AVATAR_SIZE / 2. * (1. - std::f32::consts::FRAC_1_SQRT_2) - (STATUS_SIZE + 4.) / 2.;
        let avatar = div()
            .id("profile-card-avatar")
            .relative()
            .when(!popup.webhook, |this| {
                this.cursor_pointer()
                    .tooltip(tooltip::text("View Full Profile"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.expand_profile(None, window, cx);
                    }))
            })
            .child(avatar(popup.name.clone(), avatar_url, px(AVATAR_SIZE)))
            // Webhooks have no presence.
            .when(!popup.webhook, |this| {
                this.child(
                    div()
                        .absolute()
                        .right(px(status_inset))
                        .bottom(px(status_inset))
                        .child(status_indicator(
                            self.user_status(popup.user_id),
                            px(STATUS_SIZE),
                            theme.popover,
                            None,
                            theme,
                        )),
                )
            });

        // gpui clips to rectangles, so the banner rounds its own corners.
        let banner_radius = radius::CARD - px(1.);

        let body = match (profile, &popup.error) {
            _ if popup.webhook => None,
            (Some(profile), _) => Some(self.render_details(popup, profile, is_self, cx)),
            (None, Some(error)) => Some(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone())
                    .into_any_element(),
            ),
            (None, None) => Some(render_skeleton().into_any_element()),
        };

        div()
            .relative()
            .w(px(CARD_WIDTH))
            .bg(theme.popover)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .text_color(theme.popover_foreground)
            .child(
                div()
                    .w_full()
                    .h(px(banner_height))
                    .rounded_t(banner_radius)
                    .bg(banner_color)
                    .when_some(banner_url, |this, url| {
                        this.child(
                            img(url)
                                .image_cache(&crate::ui::avatar::cache(cx))
                                .size_full()
                                .rounded_t(banner_radius)
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .when(!popup.webhook, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(10.))
                        .right(px(10.))
                        .child(render_more_menu(
                            popup,
                            profile,
                            cx.entity().downgrade(),
                            cx,
                        )),
                )
            })
            .child(
                div()
                    .absolute()
                    .left(px(INSET - AVATAR_RING))
                    .top(px(banner_height - AVATAR_SIZE / 2. - AVATAR_RING))
                    .p(px(AVATAR_RING))
                    .rounded_full()
                    .bg(theme.popover)
                    .child(avatar),
            )
            .child(
                v_flex()
                    .px(px(INSET))
                    .pb(px(INSET))
                    .pt(px(AVATAR_SIZE / 2. + AVATAR_RING + 8.))
                    .gap(px(12.))
                    .child(render_identity(popup, cx))
                    .children(body)
                    .when(!popup.webhook && !is_self, |this| {
                        this.child(Input::new(&popup.message).focus_bordered(false))
                    }),
            )
    }

    fn render_details(
        &self,
        popup: &ProfilePopup,
        profile: &UserProfile,
        is_self: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let bio = profile.display_bio().map(|bio| {
            div().text_sm().child(self.render_markdown(
                bio,
                MarkdownOptions::new(format!("profile-bio-{}", popup.user_id), &[]),
                cx,
            ))
        });
        let roles = popup
            .guild_id
            .zip(profile.member.as_ref())
            .map(|(guild_id, member)| self.member_roles_sorted(guild_id, &member.roles))
            .filter(|roles| !roles.is_empty())
            .map(|roles| render_roles(roles, cx));

        v_flex()
            .gap(px(12.))
            .when(!is_self, |this| {
                this.children(self.render_mutuals(profile, cx))
            })
            .children(bio)
            .children(roles)
            .into_any_element()
    }

    /// Highest first, as Discord lists them; `@everyone` is everyone's and never shown.
    pub(super) fn member_roles_sorted(
        &self,
        guild_id: Id<GuildMarker>,
        ids: &[Id<twilight_model::id::marker::RoleMarker>],
    ) -> Vec<discord::Role> {
        let Some(known) = self.guild_data.roles.get(&guild_id) else {
            return Vec::new();
        };
        let mut roles: Vec<discord::Role> = ids
            .iter()
            .filter(|id| id.get() != guild_id.get())
            .filter_map(|id| known.get(id).cloned())
            .collect();
        roles.sort_by_key(|role| std::cmp::Reverse(role.position));
        roles
    }
}

fn render_more_menu(
    popup: &ProfilePopup,
    profile: Option<&UserProfile>,
    screen: WeakEntity<HomeScreen>,
    cx: &App,
) -> impl IntoElement {
    let user_id = popup.user_id;
    let username = profile.map(|profile| profile.username.clone());
    let created_at = profile.map(|profile| profile.created_at.clone());

    // Dimmed so it reads over any banner.
    div()
        .rounded_full()
        .bg(black().opacity(0.45))
        .child(
            Button::new("profile-more")
                .icon(IconName::Ellipsis)
                .ghost()
                .small()
                .rounded_full()
                .text_color(white())
                .tooltip("More")
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let screen = screen.clone();
                    menu.when_some(created_at.clone(), |menu, created_at| {
                        menu.label(format!("Member since {created_at}")).separator()
                    })
                    .item(
                        PopupMenuItem::new("View Full Profile").on_click(move |_, window, cx| {
                            let _ =
                                screen.update(cx, |this, cx| this.expand_profile(None, window, cx));
                        }),
                    )
                    .when_some(username.clone(), |menu, username| {
                        menu.item(
                            PopupMenuItem::new("Copy Username").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(username.clone()));
                            }),
                        )
                    })
                    .item(
                        PopupMenuItem::new("Copy User ID").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                        }),
                    )
                }),
        )
        .text_color(cx.theme().foreground)
}

fn render_identity(popup: &ProfilePopup, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let profile = popup.profile.as_ref();

    v_flex()
        .gap(px(2.))
        .child(
            h_flex()
                .gap(px(6.))
                .items_center()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(20.))
                        .font_weight(FontWeight::BOLD)
                        .child(
                            profile
                                .map(|profile| profile.display_name().to_owned())
                                .unwrap_or_else(|| popup.name.clone()),
                        ),
                )
                .when(
                    popup.webhook || profile.is_some_and(|profile| profile.bot),
                    |this| this.child(bot_tag(cx)),
                ),
        )
        .child(
            h_flex()
                .gap(px(8.))
                .items_center()
                .text_sm()
                .text_color(theme.foreground)
                .children(
                    profile
                        .map(|profile| div().min_w_0().truncate().child(profile.username.clone())),
                )
                .children(
                    profile
                        .and_then(|profile| profile.display_pronouns())
                        .map(|pronouns| {
                            h_flex()
                                .gap(px(8.))
                                .items_center()
                                .text_color(theme.muted_foreground)
                                .child(div().size(px(4.)).rounded_full().bg(theme.muted_foreground))
                                .child(div().child(pronouns.to_owned()))
                        }),
                )
                .children(
                    profile
                        .filter(|profile| !profile.badges.is_empty())
                        .map(|profile| render_badges(&profile.badges, cx)),
                ),
        )
}

pub(super) fn render_badges(badges: &[discord::ProfileBadge], cx: &App) -> impl IntoElement {
    h_flex()
        .flex_shrink_0()
        .gap(px(2.))
        .p(px(2.))
        .rounded(radius::CONTROL)
        .border_1()
        .border_color(cx.theme().border)
        .children(badges.iter().enumerate().map(|(index, badge)| {
            let link = badge.link.clone();
            div()
                .id(("profile-badge", index))
                .size(px(BADGE_SIZE))
                .tooltip(tooltip::text(badge.description.clone()))
                .when_some(link, |this, link| {
                    this.cursor_pointer()
                        .on_click(move |_, _, cx| cx.open_url(&link))
                })
                .child(
                    img(badge.icon_url.clone())
                        .image_cache(&crate::ui::avatar::cache(cx))
                        .size_full(),
                )
        }))
}

impl HomeScreen {
    /// "3 Mutual Friends • 1 Mutual Server", with the first friends' faces in front.
    fn render_mutuals(
        &self,
        profile: &UserProfile,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let friends = profile.mutual_friends_count;
        let servers = profile.mutual_guilds.len();
        if friends == 0 && servers == 0 {
            return None;
        }
        let plural = |count: usize, noun: &str| {
            format!("{count} Mutual {noun}{}", if count == 1 { "" } else { "s" })
        };

        let faces = h_flex().children(
            profile
                .mutual_friends
                .iter()
                .take(MAX_MUTUAL_AVATARS)
                .enumerate()
                .map(|(index, friend)| {
                    div()
                        .when(index > 0, |this| this.ml(px(-4.)))
                        .rounded_full()
                        .border_2()
                        .border_color(theme.popover)
                        .child(avatar(
                            friend.name.clone(),
                            friend.avatar_url.clone(),
                            px(MUTUAL_AVATAR_SIZE),
                        ))
                }),
        );

        let mut parts = Vec::new();
        if friends > 0 {
            parts.push(plural(friends, "Friend"));
        }
        if servers > 0 {
            parts.push(plural(servers, "Server"));
        }

        // Opens the full profile on whichever list has anyone in it.
        let tab = if friends > 0 {
            ProfileTab::MutualFriends
        } else {
            ProfileTab::MutualServers
        };
        Some(
            h_flex()
                .id("profile-mutuals")
                .cursor_pointer()
                .hover(|style| style.text_color(theme.foreground))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.expand_profile(Some(tab), window, cx);
                }))
                .gap(px(6.))
                .items_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .when(!profile.mutual_friends.is_empty(), |this| this.child(faces))
                .child(div().min_w_0().truncate().child(parts.join(" • "))),
        )
    }
}

pub(super) fn render_roles(roles: Vec<discord::Role>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap(px(4.))
        .children(roles.into_iter().map(|role| {
            let color: Hsla = role
                .color
                .map_or(theme.muted_foreground, |color| rgb(color).into());
            h_flex()
                .max_w_full()
                .gap(px(5.))
                .items_center()
                .px(px(6.))
                .py(px(2.))
                .rounded(radius::CONTROL)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_xs()
                .child(div().flex_shrink_0().size(px(10.)).rounded_full().bg(color))
                .child(div().min_w_0().truncate().child(role.name))
        }))
}

/// Keeps the card from resizing much once the real content lands.
fn render_skeleton() -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .child(Skeleton::new().w(px(100.)).h_3().rounded_md())
        .child(Skeleton::new().w_full().h_4().rounded_md())
        .child(Skeleton::new().w(px(180.)).h_4().rounded_md())
}
