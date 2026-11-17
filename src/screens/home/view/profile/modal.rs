//! The full profile: a modal with the identity column on the left (bio, member
//! since, roles, connections, note) and tabs on the right for their board, what
//! they're doing, and the friends and servers shared with them.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Textarea,
    menu::{DropdownMenu as _, PopupMenuItem},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use super::card::{render_badges, render_roles};
use crate::discord::{self, ActivityEmoji, ActivityKind, UserProfile};
use crate::screens::home::view::markdown::MarkdownOptions;
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{avatar, bot_tag};
use crate::screens::home::{HomeScreen, ProfileModal, ProfileTab};
use crate::ui::depth::{self, Level, radius};
use crate::ui::{scrollbar, tooltip};

const MAX_WIDTH: f32 = 1000.;
const MAX_HEIGHT: f32 = 760.;
/// Kept clear around the modal so the scrim still reads as clickable.
const MARGIN: f32 = 40.;
const LEFT_WIDTH: f32 = 360.;
const BANNER_HEIGHT: f32 = 140.;
const AVATAR_SIZE: f32 = 120.;
const AVATAR_RING: f32 = 7.;
const STATUS_SIZE: f32 = 24.;
const INSET: f32 = 20.;

const COVER_WIDTH: f32 = 84.;
/// Box art is 3:4.
const COVER_HEIGHT: f32 = 112.;
/// Discord's grid widgets show two rows of four until expanded.
const GRID_COLUMNS: usize = 4;
const GRID_ROWS: usize = 2;
const GRID_MAX_WIDTH: f32 = 500.;
const ACTIVITY_IMAGE: f32 = 72.;
const ROW_AVATAR: f32 = 36.;

impl HomeScreen {
    pub(in crate::screens::home) fn render_profile_modal(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let modal = self.profile_modal.as_ref()?;
        let theme = cx.theme();
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - MARGIN * 2.).min(MAX_WIDTH);
        let height = (f32::from(viewport.height) - MARGIN * 2.).min(MAX_HEIGHT);

        let close = Button::new("profile-modal-close")
            .icon(Icon::default().path("icons/x.svg"))
            .ghost()
            .small()
            .tooltip("Close (Esc)")
            .on_click(cx.listener(|this, _, _, cx| this.close_profile_modal(cx)));

        let panel = h_flex()
            .id("profile-modal")
            .occlude()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_modal_identity(modal, cx))
            .child(self.render_modal_tabs(modal, cx))
            .child(div().absolute().top(px(10.)).right(px(10.)).child(close));

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    // A scrim, not a theme colour: it must dim the screen in either theme.
                    .bg(black().opacity(0.7))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_profile_modal(cx);
                        }),
                    )
                    .child(panel),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_modal_identity(&self, modal: &ProfileModal, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let profile = modal.profile.as_ref();
        let is_self = Some(modal.user_id) == self.self_user_id;

        let banner_url = profile.and_then(|profile| profile.display_banner().cloned());
        let banner_radius = radius::CARD - px(1.);
        let banner_color: Hsla = profile
            .and_then(|profile| profile.accent_color)
            .map_or(theme.primary, |color| rgb(color).into());
        let avatar_url = profile
            .and_then(|profile| profile.display_avatar().cloned())
            .or_else(|| modal.avatar_url.clone());
        let name = profile
            .map(|profile| profile.display_name().to_owned())
            .unwrap_or_else(|| modal.name.clone());

        let status_inset =
            AVATAR_SIZE / 2. * (1. - std::f32::consts::FRAC_1_SQRT_2) - (STATUS_SIZE + 6.) / 2.;
        let avatar = div()
            .absolute()
            .left(px(INSET - AVATAR_RING))
            .top(px(BANNER_HEIGHT - AVATAR_SIZE / 2. - AVATAR_RING))
            .p(px(AVATAR_RING))
            .rounded_full()
            .bg(theme.popover)
            .child(
                div()
                    .relative()
                    .child(avatar(name.clone(), avatar_url, px(AVATAR_SIZE)))
                    .child(
                        div()
                            .absolute()
                            .right(px(status_inset))
                            .bottom(px(status_inset))
                            .child(status_indicator(
                                self.user_status(modal.user_id),
                                px(STATUS_SIZE),
                                theme.popover,
                                None,
                                theme,
                            )),
                    ),
            );

        let header = div()
            .relative()
            .w_full()
            .h(px(BANNER_HEIGHT + AVATAR_SIZE / 2. + 12.))
            .flex_shrink_0()
            .child(
                // The modal's corner; gpui clips to rectangles, so the banner rounds its own.
                div()
                    .w_full()
                    .h(px(BANNER_HEIGHT))
                    .overflow_hidden()
                    .rounded_tl(banner_radius)
                    .bg(banner_color)
                    .when_some(banner_url, |this, url| {
                        this.child(
                            img(url)
                                .image_cache(&crate::ui::avatar::cache(cx))
                                .size_full()
                                .rounded_tl(banner_radius)
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(avatar);

        let identity = v_flex()
            .gap(px(4.))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .child(name),
                    )
                    .when(profile.is_some_and(|profile| profile.bot), |this| {
                        this.child(bot_tag(cx))
                    }),
            )
            .children(profile.map(|profile| {
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .items_center()
                    .text_sm()
                    .child(profile.username.clone())
                    .children(profile.display_pronouns().map(|pronouns| {
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .text_color(theme.muted_foreground)
                            .child(div().size(px(4.)).rounded_full().bg(theme.muted_foreground))
                            .child(pronouns.to_owned())
                    }))
            }))
            .children(
                profile
                    .filter(|profile| !profile.badges.is_empty())
                    .map(|profile| div().pt(px(4.)).child(render_badges(&profile.badges, cx))),
            );

        let user_id = modal.user_id;
        let actions = (!is_self).then(|| {
            h_flex()
                .gap(px(8.))
                .child(
                    Button::new("profile-modal-message")
                        .primary()
                        .small()
                        .icon(Icon::default().path("icons/message-circle.svg"))
                        .label("Message")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.message_user(user_id, window, cx);
                        })),
                )
                .child(render_modal_menu(user_id, profile))
        });

        let body = match (profile, &modal.error) {
            (Some(profile), _) => Some(
                self.render_modal_sections(modal, profile, cx)
                    .into_any_element(),
            ),
            (None, Some(error)) => Some(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone())
                    .into_any_element(),
            ),
            (None, None) => None,
        };

        h_flex()
            .w(px(LEFT_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .id("profile-modal-identity")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&modal.identity_scroll)
                    .child(header)
                    .child(
                        v_flex()
                            .px(px(INSET))
                            .pb(px(INSET))
                            .gap(px(16.))
                            .child(identity)
                            .children(actions)
                            .children(body),
                    ),
            )
            .child(scrollbar::gutter(
                "profile-modal-identity-scrollbar",
                &modal.identity_scroll,
                cx,
            ))
    }

    fn render_modal_sections(
        &self,
        modal: &ProfileModal,
        profile: &UserProfile,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        let bio = profile.display_bio().map(|bio| {
            section(
                "About Me",
                div().text_sm().child(self.render_markdown(
                    bio,
                    MarkdownOptions::new(format!("profile-modal-bio-{}", modal.user_id), &[]),
                    cx,
                )),
                cx,
            )
        });

        // The server's join date sits beside Discord's, behind the server's icon.
        let guild = modal
            .guild_id
            .and_then(|guild_id| self.guilds.iter().find(|guild| guild.id == guild_id));
        let joined = guild.zip(
            profile
                .member
                .as_ref()
                .and_then(|member| member.joined_at.clone()),
        );
        let member_since = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .items_center()
            .text_sm()
            .child(
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        Icon::default()
                            .path("icons/calendar.svg")
                            .size(px(16.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(profile.created_at.clone()),
            )
            .children(joined.map(|(guild, joined_at)| {
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(div().size(px(4.)).rounded_full().bg(theme.muted_foreground))
                    .child(
                        div()
                            .id("profile-modal-joined")
                            .tooltip(tooltip::text(guild.name.clone()))
                            .child(avatar(guild.name.clone(), guild.icon_url.clone(), px(16.))),
                    )
                    .child(joined_at)
            }));

        let roles = modal
            .guild_id
            .zip(profile.member.as_ref())
            .map(|(guild_id, member)| self.member_roles_sorted(guild_id, &member.roles))
            .filter(|roles| !roles.is_empty())
            .map(|roles| section("Roles", render_roles(roles, cx), cx));

        let connections = (!profile.connections.is_empty()).then(|| {
            section(
                "Connections",
                render_connections(&profile.connections, cx),
                cx,
            )
        });

        let note = section(
            "Note (only visible to you)",
            // Drawn without the input's own chrome, so focusing it shows no ring.
            div()
                .px(px(10.))
                .py(px(6.))
                .rounded(radius::CONTROL)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_sm()
                .child(
                    Textarea::new(&modal.note)
                        .appearance(false)
                        .disabled(modal.saved_note.is_none()),
                ),
            cx,
        );

        v_flex()
            .gap(px(16.))
            .children(bio)
            .child(section("Member Since", member_since, cx))
            .children(
                profile.legacy_username.clone().map(|legacy| {
                    section("Originally Known As", div().text_sm().child(legacy), cx)
                }),
            )
            .children(roles)
            .children(connections)
            .child(note)
    }

    fn render_modal_tabs(&self, modal: &ProfileModal, cx: &Context<Self>) -> impl IntoElement {
        let profile = modal.profile.as_ref();
        let is_self = Some(modal.user_id) == self.self_user_id;

        let mut tabs = Vec::new();
        if profile.is_some_and(|profile| !profile.widgets.is_empty()) {
            tabs.push((ProfileTab::Board, "Board".to_owned()));
        }
        tabs.push((ProfileTab::Activity, "Activity".to_owned()));
        if !is_self && let Some(profile) = profile {
            let friends = profile.mutual_friends_count;
            let servers = profile.mutual_guilds.len();
            tabs.push((
                ProfileTab::MutualFriends,
                format!(
                    "{friends} Mutual Friend{}",
                    if friends == 1 { "" } else { "s" }
                ),
            ));
            tabs.push((
                ProfileTab::MutualServers,
                format!(
                    "{servers} Mutual Server{}",
                    if servers == 1 { "" } else { "s" }
                ),
            ));
        }
        // A tab picked before the profile loaded may not exist once it has.
        let selected = tabs
            .iter()
            .position(|(tab, _)| Some(*tab) == modal.tab)
            .unwrap_or(0);
        let tab = tabs[selected].0;

        let order: Vec<ProfileTab> = tabs.iter().map(|(tab, _)| *tab).collect();
        let screen = cx.entity().downgrade();
        let bar = TabBar::new("profile-modal-tabs")
            .underline()
            .selected_index(selected)
            .on_click(move |index: &usize, _, cx| {
                if let Some(&tab) = order.get(*index) {
                    let _ = screen.update(cx, |this, cx| this.select_profile_tab(tab, cx));
                }
            })
            .children(tabs.into_iter().map(|(_, label)| Tab::new().label(label)));

        let content = match (tab, profile) {
            (_, None) if modal.error.is_none() => loading(cx).into_any_element(),
            (_, None) => div().into_any_element(),
            (ProfileTab::Board, Some(profile)) => {
                self.render_board(modal, profile, cx).into_any_element()
            }
            (ProfileTab::Activity, _) => self.render_activities(modal, cx).into_any_element(),
            (ProfileTab::MutualFriends, Some(profile)) => self
                .render_mutual_friends(modal, profile, cx)
                .into_any_element(),
            (ProfileTab::MutualServers, Some(profile)) => {
                self.render_mutual_servers(profile, cx).into_any_element()
            }
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(div().pl(px(INSET)).pr(px(52.)).pt(px(14.)).child(bar))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id(("profile-modal-tab", tab as usize))
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&modal.tab_scroll)
                            .child(div().p(px(INSET)).child(content)),
                    )
                    .child(scrollbar::gutter(
                        "profile-modal-tab-scrollbar",
                        &modal.tab_scroll,
                        cx,
                    )),
            )
    }

    fn render_board(
        &self,
        modal: &ProfileModal,
        profile: &UserProfile,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap(px(12.))
            .children(profile.widgets.iter().enumerate().map(|(index, widget)| {
                let body = if widget.kind.detailed() {
                    v_flex()
                        .gap(px(12.))
                        .children(widget.games.iter().enumerate().map(|(game_index, game)| {
                            let info = self.games.get(&game.app_id);
                            h_flex()
                                .gap(px(14.))
                                .items_start()
                                .child(self.game_cover((index, game_index), game.app_id, false, cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(6.))
                                        .pt(px(4.))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(game_name(info)),
                                        )
                                        .children(game.comment.clone().map(|comment| {
                                            div()
                                                .text_sm()
                                                .italic()
                                                .text_color(theme.muted_foreground)
                                                .child(comment)
                                        }))
                                        .when(!game.tags.is_empty(), |this| {
                                            this.child(render_tags(&game.tags, cx))
                                        }),
                                )
                                .into_any_element()
                        }))
                        .into_any_element()
                } else {
                    let expanded = modal.expanded_widgets.contains(&index);
                    let shown = if expanded {
                        widget.games.len()
                    } else {
                        GRID_COLUMNS * GRID_ROWS
                    };
                    let overflow = widget.games.len() > GRID_COLUMNS * GRID_ROWS;
                    v_flex()
                        .gap(px(10.))
                        .child(
                            div()
                                .max_w(px(GRID_MAX_WIDTH))
                                .grid()
                                .grid_cols(GRID_COLUMNS as u16)
                                .gap(px(10.))
                                .children(widget.games.iter().take(shown).enumerate().map(
                                    |(game_index, game)| {
                                        self.game_cover((index, game_index), game.app_id, true, cx)
                                    },
                                )),
                        )
                        .when(overflow, |this| {
                            this.child(
                                div().child(
                                    Button::new(("profile-widget-more", index))
                                        .ghost()
                                        .small()
                                        .label(if expanded { "Show less" } else { "Show more" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_profile_widget(index, cx);
                                        })),
                                ),
                            )
                        })
                        .into_any_element()
                };
                v_flex()
                    .gap(px(10.))
                    .p(px(14.))
                    .rounded(radius::CARD)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(widget.kind.title()),
                    )
                    .child(body)
            }))
    }

    /// Box art, or the name on a plain tile while it loads or when there is none.
    /// In a grid it fills its column and the name moves to a tooltip.
    fn game_cover(
        &self,
        (widget, game): (usize, usize),
        app_id: u64,
        in_grid: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let info = self.games.get(&app_id);
        let name = game_name(info);
        let cover = info.and_then(|info| info.cover_url.clone());
        div()
            .id(("profile-game", widget * 100 + game))
            .map(|this| {
                if in_grid {
                    this.w_full().aspect_ratio(COVER_WIDTH / COVER_HEIGHT)
                } else {
                    this.w(px(COVER_WIDTH)).h(px(COVER_HEIGHT)).flex_shrink_0()
                }
            })
            .rounded(radius::ITEM)
            .overflow_hidden()
            .bg(theme.muted)
            .when(in_grid, |this| this.tooltip(tooltip::text(name.clone())))
            .map(|this| match cover {
                // gpui clips to rectangles, so the image rounds its own corners.
                Some(url) => this.child(
                    img(url)
                        .image_cache(&crate::ui::avatar::cache(cx))
                        .size_full()
                        .rounded(radius::ITEM)
                        .object_fit(ObjectFit::Cover),
                ),
                None => this.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(px(6.))
                        .text_xs()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(name),
                ),
            })
    }

    fn render_activities(&self, modal: &ProfileModal, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_self = Some(modal.user_id) == self.self_user_id;
        let activities = self
            .user_activities
            .get(&modal.user_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if activities.is_empty() {
            // The gateway never reports the user's own presence back.
            let text = if is_self {
                "Your activity isn't shown here yet."
            } else {
                "Nothing going on right now."
            };
            return empty_state(text, cx).into_any_element();
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);

        v_flex()
            .gap(px(12.))
            .children(activities.iter().enumerate().map(|(index, activity)| {
                if activity.kind == ActivityKind::Custom {
                    return render_custom_status(activity, cx).into_any_element();
                }
                let heading = match activity.kind {
                    ActivityKind::Listening | ActivityKind::Watching | ActivityKind::Streaming => {
                        format!("{} {}", activity.kind.heading(), activity.name)
                    }
                    kind => kind.heading().to_owned(),
                };
                // Music and video name the platform in the heading and lead
                // with what's playing.
                let title = match activity.kind {
                    ActivityKind::Playing | ActivityKind::Competing => Some(activity.name.clone()),
                    _ => activity.details.clone(),
                };
                let lines = match activity.kind {
                    ActivityKind::Playing | ActivityKind::Competing => {
                        vec![activity.details.clone(), activity.state.clone()]
                    }
                    _ => vec![activity.state.clone(), activity.large_text.clone()],
                };
                let timing = activity_timing(activity, now_ms);

                let image = activity.large_image.clone().map(|url| {
                    div()
                        .relative()
                        .flex_shrink_0()
                        .size(px(ACTIVITY_IMAGE))
                        .overflow_hidden()
                        .child(
                            img(url)
                                .image_cache(&crate::ui::avatar::cache(cx))
                                .size_full()
                                .rounded(radius::ITEM)
                                .object_fit(ObjectFit::Cover),
                        )
                        .children(activity.small_image.clone().map(|url| {
                            div()
                                .id(("activity-small-image", index))
                                .absolute()
                                .right(px(-4.))
                                .bottom(px(-4.))
                                .size(px(26.))
                                .p(px(2.))
                                .rounded_full()
                                .bg(theme.background)
                                .when_some(activity.small_text.clone(), |this, text| {
                                    this.tooltip(tooltip::text(text))
                                })
                                .child(
                                    img(url)
                                        .image_cache(&crate::ui::avatar::cache(cx))
                                        .size_full()
                                        .rounded_full(),
                                )
                        }))
                });
                let image = image.unwrap_or_else(|| {
                    div()
                        .flex_shrink_0()
                        .size(px(ACTIVITY_IMAGE))
                        .rounded(radius::ITEM)
                        .bg(theme.muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::default()
                                .path("icons/gamepad-2.svg")
                                .size(px(28.))
                                .text_color(theme.muted_foreground),
                        )
                });

                let url = activity.url.clone();
                v_flex()
                    .gap(px(10.))
                    .p(px(14.))
                    .rounded(radius::CARD)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(heading),
                    )
                    .child(
                        h_flex().gap(px(14.)).items_center().child(image).child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .text_sm()
                                .children(title.map(|title| {
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title)
                                }))
                                .children(lines.into_iter().flatten().map(|line| {
                                    div()
                                        .truncate()
                                        .text_color(theme.muted_foreground)
                                        .child(line)
                                }))
                                .children(timing.map(|timing| {
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(timing)
                                })),
                        ),
                    )
                    .children(
                        url.filter(|_| activity.kind == ActivityKind::Streaming)
                            .map(|url| {
                                Button::new(("activity-watch", index))
                                    .small()
                                    .label("Watch Stream")
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                            }),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    fn render_mutual_friends(
        &self,
        modal: &ProfileModal,
        profile: &UserProfile,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        if profile.mutual_friends.is_empty() {
            return empty_state("No friends in common.", cx).into_any_element();
        }
        let guild_id = modal.guild_id;
        v_flex()
            .children(profile.mutual_friends.iter().map(|friend| {
                let (id, name, avatar_url) =
                    (friend.id, friend.name.clone(), friend.avatar_url.clone());
                list_row(
                    ("mutual-friend", id.get()),
                    avatar(
                        friend.name.clone(),
                        friend.avatar_url.clone(),
                        px(ROW_AVATAR),
                    ),
                    friend.name.clone(),
                    Some(friend.username.clone()).filter(|username| *username != friend.name),
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_profile_modal(
                        id,
                        guild_id,
                        name.clone(),
                        avatar_url.clone(),
                        None,
                        window,
                        cx,
                    );
                }))
            }))
            .into_any_element()
    }

    fn render_mutual_servers(&self, profile: &UserProfile, cx: &Context<Self>) -> impl IntoElement {
        let guilds: Vec<_> = profile
            .mutual_guilds
            .iter()
            .filter_map(|mutual| {
                let guild = self.guilds.iter().find(|guild| guild.id == mutual.id)?;
                Some((guild, mutual.nick.clone()))
            })
            .collect();
        if guilds.is_empty() {
            return empty_state("No servers in common.", cx).into_any_element();
        }
        v_flex()
            .children(guilds.into_iter().map(|(guild, nick)| {
                let guild_id = guild.id;
                list_row(
                    ("mutual-server", guild_id.get()),
                    avatar(guild.name.clone(), guild.icon_url.clone(), px(ROW_AVATAR)),
                    guild.name.clone(),
                    nick,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_profile_modal(cx);
                    this.select_guild(guild_id, window, cx);
                }))
            }))
            .into_any_element()
    }
}

fn section(title: &'static str, body: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
        .child(body)
}

/// Discord's layout: the service's logo on a white tile, the account, and what
/// the service shares about it.
fn render_connections(connections: &[discord::ProfileConnection], cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap(px(12.))
        .children(connections.iter().enumerate().map(|(index, connection)| {
            let label = discord::connection_label(&connection.kind);
            // Logos are drawn black, so the tile stays white in either theme.
            let tile = div()
                .flex_shrink_0()
                .size(px(32.))
                .rounded(radius::ITEM)
                .bg(white())
                .flex()
                .items_center()
                .justify_center()
                .map(|this| match connection.icon_url.clone() {
                    Some(url) => this.child(
                        img(url)
                            .image_cache(&crate::ui::avatar::cache(cx))
                            .size(px(18.)),
                    ),
                    None => this.child(
                        Icon::default()
                            .path("icons/globe.svg")
                            .size(px(18.))
                            .text_color(black()),
                    ),
                });

            let name = h_flex()
                .id(("profile-connection", index))
                .min_w_0()
                .gap(px(4.))
                .items_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .tooltip(tooltip::text(label))
                .child(div().min_w_0().truncate().child(connection.name.clone()))
                .when(connection.verified, |this| {
                    this.child(
                        Icon::default()
                            .path("icons/badge-check.svg")
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    )
                })
                .when_some(connection.url.clone(), |this, url| {
                    this.cursor_pointer()
                        .hover(|style| style.underline())
                        .child(
                            Icon::default()
                                .path("icons/arrow-up-right.svg")
                                .size(px(14.))
                                .text_color(theme.muted_foreground),
                        )
                        .on_click(move |_, _, cx| cx.open_url(&url))
                });

            h_flex().gap(px(12.)).items_center().child(tile).child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(name)
                    .children(connection.created_at.clone().map(|date| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("Member since {date}"))
                    }))
                    .when(!connection.stats.is_empty(), |this| {
                        this.child(
                            h_flex()
                                .gap(px(10.))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .children(connection.stats.iter().cloned()),
                        )
                    }),
            )
        }))
}

fn render_modal_menu(user_id: Id<UserMarker>, profile: Option<&UserProfile>) -> impl IntoElement {
    let username = profile.map(|profile| profile.username.clone());
    Button::new("profile-modal-more")
        .icon(IconName::Ellipsis)
        .small()
        .tooltip("More")
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
            menu.when_some(username.clone(), |menu, username| {
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
        })
}

fn render_tags(tags: &[String], cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap(px(4.))
        .children(tags.iter().map(|tag| {
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .child(discord::widget_tag_label(tag))
        }))
}

fn render_custom_status(activity: &discord::Activity, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let emoji = activity.emoji.clone().map(|emoji| match emoji {
        ActivityEmoji::Custom(url) => img(url)
            .image_cache(&crate::ui::avatar::cache(cx))
            .size(px(20.))
            .into_any_element(),
        ActivityEmoji::Unicode(text) => div().text_size(px(18.)).child(text).into_any_element(),
    });
    h_flex()
        .gap(px(8.))
        .items_center()
        .p(px(14.))
        .rounded(radius::CARD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .children(emoji)
        .children(
            activity
                .state
                .clone()
                .map(|state| div().text_sm().child(state)),
        )
}

fn list_row(
    id: impl Into<ElementId>,
    icon: impl IntoElement,
    name: String,
    detail: Option<String>,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .gap(px(12.))
        .items_center()
        .px(px(8.))
        .py(px(6.))
        .rounded(radius::ITEM)
        .cursor_pointer()
        .hover(|style| style.bg(theme.muted))
        .child(icon)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                .children(detail.map(|detail| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail)
                })),
        )
}

fn empty_state(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .py(px(40.))
        .text_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

fn loading(cx: &App) -> impl IntoElement {
    empty_state("Loading…", cx)
}

fn game_name(info: Option<&discord::GameInfo>) -> String {
    info.map_or_else(|| "Unknown game".to_owned(), |info| info.name.clone())
}

/// "12:34 elapsed" for a game, "1:02 / 3:45" for a track with an end.
fn activity_timing(activity: &discord::Activity, now_ms: i64) -> Option<String> {
    let start = activity.started_at?;
    let elapsed = (now_ms - start).max(0) / 1000;
    match activity.ends_at {
        Some(end) => {
            let total = (end - start).max(0) / 1000;
            Some(format!("{} / {}", clock(elapsed.min(total)), clock(total)))
        }
        None => Some(format!("{} elapsed", clock(elapsed))),
    }
}

fn clock(seconds: i64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
