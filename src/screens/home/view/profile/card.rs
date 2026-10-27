//! The card itself: banner, avatar, identity line, and the details box.

use gpui_kit::component::{
    ActiveTheme as _, h_flex, separator::Separator, skeleton::Skeleton, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::UserProfile;
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{avatar, bot_tag};
use crate::screens::home::{HomeScreen, ProfilePopup};
use crate::ui::depth::{self, Level, radius};

pub(in crate::screens::home::view) const CARD_WIDTH: f32 = 300.;

/// The shorter height is the accent-colour strip for users without a banner.
const BANNER_HEIGHT: f32 = 105.;
const ACCENT_BANNER_HEIGHT: f32 = 60.;

const AVATAR_SIZE: f32 = 80.;

const STATUS_SIZE: f32 = 16.;

const AVATAR_RING: f32 = 6.;

const AVATAR_INSET: f32 = 10.;

/// Keeps a wall-of-text bio from stretching the card down the window.
const MAX_BIO_LINES: usize = 3;

impl HomeScreen {
    pub(super) fn render_profile_card(
        &self,
        popup: &ProfilePopup,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let profile = popup.profile.as_ref();

        // Accent colour and banner image only arrive with the fetched profile.
        let banner_url = profile.and_then(|profile| profile.banner_url.clone());
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
            .and_then(|profile| profile.avatar_url.clone())
            .or_else(|| popup.avatar_url.clone());
        let status_inset =
            AVATAR_SIZE / 2. * (1. - std::f32::consts::FRAC_1_SQRT_2) - (STATUS_SIZE + 4.) / 2.;
        let avatar = div()
            .relative()
            .child(avatar(popup.name.clone(), avatar_url, px(AVATAR_SIZE)))
            .child(
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
            );

        // gpui clips to rectangles, so the banner rounds its own corners.
        let banner_radius = radius::CARD - px(1.);

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
                                .size_full()
                                .rounded_t(banner_radius)
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(AVATAR_INSET - AVATAR_RING))
                    .top(px(banner_height - AVATAR_SIZE / 2. - AVATAR_RING))
                    .p(px(AVATAR_RING))
                    .rounded_full()
                    .bg(theme.popover)
                    .child(avatar),
            )
            .child(
                v_flex()
                    .px(px(AVATAR_INSET))
                    .pb(px(AVATAR_INSET))
                    .pt(px(AVATAR_SIZE / 2. + AVATAR_RING + 6.))
                    .gap(px(10.))
                    .child(render_identity(popup, cx))
                    .child(match (profile, &popup.error) {
                        (Some(profile), _) => render_details(profile, cx),
                        (None, Some(error)) => div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(error.clone())
                            .into_any_element(),
                        (None, None) => render_skeleton().into_any_element(),
                    }),
            )
    }
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
                                .map(|profile| profile.name.clone())
                                .unwrap_or_else(|| popup.name.clone()),
                        ),
                )
                .when(profile.is_some_and(|profile| profile.bot), |this| {
                    this.child(bot_tag(cx))
                }),
        )
        .child(
            h_flex()
                .gap(px(8.))
                .items_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .children(
                    profile
                        .map(|profile| div().min_w_0().truncate().child(profile.username.clone())),
                )
                .children(
                    profile
                        .and_then(|profile| profile.pronouns.clone())
                        .map(|pronouns| {
                            h_flex()
                                .gap(px(8.))
                                .items_center()
                                .child(div().size(px(4.)).rounded_full().bg(theme.muted_foreground))
                                .child(div().child(pronouns))
                        }),
                ),
        )
}

fn render_details(profile: &UserProfile, cx: &App) -> AnyElement {
    v_flex()
        .p(px(8.))
        .gap(px(8.))
        .rounded(radius::CONTROL)
        .bg(cx.theme().background)
        .when_some(profile.bio.clone(), |this, bio| {
            this.child(section(
                "ABOUT ME",
                div()
                    .line_clamp(MAX_BIO_LINES)
                    .text_ellipsis()
                    .child(clamp_lines(&bio, MAX_BIO_LINES))
                    .into_any_element(),
                cx,
            ))
            .child(Separator::horizontal())
        })
        .child(section(
            "MEMBER SINCE",
            div().child(profile.created_at.clone()).into_any_element(),
            cx,
        ))
        .into_any_element()
}

/// gpui's own clamp estimates how much fits in `max` full-width lines, so its
/// ellipsis never fires when explicit line breaks run out first; this covers
/// that and leaves wrapping overflow to the clamp.
fn clamp_lines(text: &str, max: usize) -> String {
    let mut lines = text.lines();
    let mut kept = lines.by_ref().take(max).collect::<Vec<_>>().join(
        "
",
    );
    if lines.next().is_some() {
        kept.push('…');
    }
    kept
}

fn section(header: &'static str, content: AnyElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap(px(5.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().foreground)
                .child(header),
        )
        .child(div().text_sm().child(content))
}

/// Keeps the card from resizing much once the real content lands.
fn render_skeleton() -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .child(Skeleton::new().w(px(100.)).h_3().rounded_md())
        .child(Skeleton::new().w_full().h_4().rounded_md())
        .child(Skeleton::new().w(px(180.)).h_4().rounded_md())
}
