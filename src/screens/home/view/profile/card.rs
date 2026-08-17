//! The card itself: banner, avatar, identity line, and the details box.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, avatar::Avatar, divider::Divider, h_flex, skeleton::Skeleton,
    v_flex,
};

use crate::discord::UserProfile;
use crate::screens::home::{HomeScreen, ProfilePopup};

/// Card width. Discord's small popout is 300px wide.
const CARD_WIDTH: f32 = 300.;

/// Banner height with and without a banner image. The shorter one is the strip
/// of accent colour Discord draws for users who haven't set a banner.
const BANNER_HEIGHT: f32 = 105.;
const ACCENT_BANNER_HEIGHT: f32 = 60.;

const AVATAR_SIZE: f32 = 80.;

/// Thickness of the ring cut out of the card around the avatar, which is what
/// makes it read as sitting in front of the banner.
const AVATAR_RING: f32 = 6.;

/// How far the avatar's left edge sits from the card's.
const AVATAR_INSET: f32 = 16.;

/// Longest the "About Me" text gets before it's clipped, so a wall-of-text bio
/// can't stretch the card down the whole window.
const MAX_BIO_HEIGHT: f32 = 120.;

impl HomeScreen {
    pub(super) fn render_profile_card(
        &self,
        popup: &ProfilePopup,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let profile = popup.profile.as_ref();

        // Until the fetch resolves, the banner is a plain strip: the accent
        // colour and banner image only arrive with the profile.
        let banner_url = profile.and_then(|profile| profile.banner_url.clone());
        let banner_height = if banner_url.is_some() {
            BANNER_HEIGHT
        } else {
            ACCENT_BANNER_HEIGHT
        };
        let banner_color: Hsla = profile
            .and_then(|profile| profile.accent_color)
            .map_or(theme.primary, |color| rgb(color).into());

        // The message that opened the card supplies the avatar and name, so
        // both are already right while the profile is loading.
        let avatar_url = profile
            .and_then(|profile| profile.avatar_url.clone())
            .or_else(|| popup.avatar_url.clone());
        let mut avatar = Avatar::new()
            .name(popup.name.clone())
            .with_size(px(AVATAR_SIZE));
        if let Some(avatar_url) = avatar_url {
            avatar = avatar.src(avatar_url);
        }

        div()
            .relative()
            .w(px(CARD_WIDTH))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .text_color(theme.popover_foreground)
            .child(
                div()
                    .w_full()
                    .h(px(banner_height))
                    .bg(banner_color)
                    .when_some(banner_url, |this, url| {
                        this.child(img(url).size_full().object_fit(ObjectFit::Cover))
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
                    // Clears the half of the avatar hanging below the banner.
                    .pt(px(AVATAR_SIZE / 2. + AVATAR_RING + 8.))
                    .gap(px(12.))
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

/// The display name, `@handle`, and bot tag directly under the avatar.
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
                    this.child(
                        div()
                            .flex_shrink_0()
                            .px(px(4.))
                            .rounded(px(4.))
                            .bg(theme.primary)
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.primary_foreground)
                            .child("BOT"),
                    )
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

/// The inner box holding the "About Me" and "Member Since" sections.
fn render_details(profile: &UserProfile, cx: &App) -> AnyElement {
    v_flex()
        .p(px(12.))
        .gap(px(12.))
        .rounded(px(8.))
        .bg(cx.theme().background)
        .when_some(profile.bio.clone(), |this, bio| {
            this.child(section(
                "ABOUT ME",
                div()
                    .max_h(px(MAX_BIO_HEIGHT))
                    .overflow_hidden()
                    .child(bio)
                    .into_any_element(),
                cx,
            ))
            .child(Divider::horizontal())
        })
        .child(section(
            "MEMBER SINCE",
            div().child(profile.created_at.clone()).into_any_element(),
            cx,
        ))
        .into_any_element()
}

/// A labelled block inside the card's inner box — an uppercase header over its
/// content, the way Discord sections the popout.
fn section(header: &'static str, content: AnyElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().foreground)
                .child(header),
        )
        .child(div().text_sm().child(content))
}

/// Stand-in for the details box while the profile is being fetched, keeping the
/// card from resizing much once the real content lands.
fn render_skeleton() -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .child(Skeleton::new().w(px(100.)).h_3().rounded_md())
        .child(Skeleton::new().w_full().h_4().rounded_md())
        .child(Skeleton::new().w(px(180.)).h_4().rounded_md())
}
