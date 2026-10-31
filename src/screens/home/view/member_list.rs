//! The member list down the right of a guild conversation, under its header.

use std::ops::Range;

use gpui_kit::component::{ActiveTheme as _, h_flex, skeleton::Skeleton, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::{
    ListMember, MemberGroupKind, MemberListGroup, MemberListItem, PresenceStatus,
};
use crate::screens::home::HomeScreen;
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{INSET, avatar, bot_tag};
use crate::ui::depth::radius;
use crate::ui::theme;

use super::profile::CARD_WIDTH;

pub(super) const MEMBER_LIST_WIDTH: f32 = 240.;

/// Every row's height, headers included: the virtual list needs one size.
const ROW_HEIGHT: f32 = 44.;

const SKELETON_WIDTHS: [f32; 6] = [96., 120., 72., 108., 84., 128.];

const OFFLINE_OPACITY: f32 = 0.35;

impl HomeScreen {
    pub(in crate::screens::home) fn render_member_list(
        &self,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        if self.member_list_hidden {
            return None;
        }
        let list = self.member_list.as_ref()?;
        let theme = cx.theme();

        let body = if list.rows.is_empty() {
            v_flex()
                .size_full()
                .pt_2()
                .px_2()
                .children((0..12).map(skeleton_row))
                .into_any_element()
        } else {
            uniform_list(
                "member-list",
                list.rows.len(),
                cx.processor(|this, range: Range<usize>, _, cx| {
                    this.track_member_list_range(range.clone());
                    this.render_member_rows(range, cx)
                }),
            )
            .track_scroll(&list.scroll)
            .size_full()
            .pb_2()
            .into_any_element()
        };

        Some(
            div()
                .w(px(MEMBER_LIST_WIDTH))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(theme.border)
                .child(body),
        )
    }

    fn render_member_rows(&self, range: Range<usize>, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(list) = &self.member_list else {
            return Vec::new();
        };
        range
            .map(|index| {
                let row = match list.rows.get(index).cloned().flatten() {
                    Some(MemberListItem::Group(group)) => self.render_group_header(&group, cx),
                    Some(MemberListItem::Member(member)) => self.render_member_row(&member, cx),
                    None => skeleton_row(index).into_any_element(),
                };
                // The list sizes items to content; spanning the list makes hover and click cover
                // the whole row.
                div().w_full().px_2().child(row).into_any_element()
            })
            .collect()
    }

    fn render_group_header(&self, group: &MemberListGroup, cx: &Context<Self>) -> AnyElement {
        let name = match &group.kind {
            MemberGroupKind::Online => "Online".to_owned(),
            MemberGroupKind::Offline => "Offline".to_owned(),
            MemberGroupKind::Role(role_id) => self
                .selected_guild
                .and_then(|guild_id| self.guild_data.roles.get(&guild_id)?.get(role_id))
                .map_or_else(|| "Role".to_owned(), |role| role.name.clone()),
        };
        // A header op carries only the group's id; the count comes from the update's group list.
        let count = self
            .member_list
            .as_ref()
            .and_then(|list| list.groups.iter().find(|known| known.kind == group.kind))
            .map_or(group.count, |known| known.count);
        h_flex()
            .h(px(ROW_HEIGHT))
            .w_full()
            .px_2()
            .pb_1()
            .items_end()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .truncate()
                    .child(format!("{} - {count}", name.to_uppercase())),
            )
            .into_any_element()
    }

    fn render_member_row(&self, member: &ListMember, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let user_id = member.user_id;
        let status = self.user_status(user_id);
        let offline = status == PresenceStatus::Invisible;
        let group: SharedString = format!("member-{user_id}").into();
        let hover_bg = theme.background.blend(theme.accent.opacity(0.4));

        let avatar =
            div()
                .relative()
                .flex_shrink_0()
                .child(avatar(
                    member.name.clone(),
                    member.avatar_url.clone(),
                    px(32.),
                ))
                // Discord shows no status for offline members.
                .when(!offline, |this| {
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

        let name_color = self
            .role_color(user_id)
            .map(|color| rgb(color).into())
            .unwrap_or(theme::secondary_text(cx));
        let name = h_flex()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(name_color)
                    .child(member.name.clone()),
            )
            .when(member.bot, |this| this.child(bot_tag(cx)));

        let name_and_activity = v_flex().flex_1().min_w_0().child(name).when_some(
            member.activity.clone().filter(|_| !offline),
            |this, line| {
                this.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(line),
                )
            },
        );

        // Opens left of the list, level with the row; beside the pointer it would cover the list.
        let name = member.name.clone();
        let avatar_url = member.avatar_url.clone();
        h_flex()
            .id(("member", user_id.get()))
            .group(group)
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_3()
            .items_center()
            .rounded(radius::ITEM)
            .cursor_pointer()
            .when(offline, |this| this.opacity(OFFLINE_OPACITY))
            .hover(|this| this.bg(hover_bg).opacity(1.))
            .child(avatar)
            .child(name_and_activity)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // Mouse-down, like a message's avatar, so the card's dismiss layer doesn't see
                    // the same press.
                    cx.stop_propagation();
                    let list_left =
                        window.viewport_size().width - px(INSET + 1. + MEMBER_LIST_WIDTH);
                    // The popout adds its own offset of 24 to the right.
                    let x = list_left - px(CARD_WIDTH + 8. + 24.);
                    this.open_profile(
                        user_id,
                        name.clone(),
                        avatar_url.clone(),
                        point(x, event.position.y),
                        cx,
                    );
                }),
            )
            .into_any_element()
    }
}

fn skeleton_row(index: usize) -> impl IntoElement {
    h_flex()
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_2()
        .gap_3()
        .items_center()
        .child(Skeleton::new().size(px(32.)).rounded_full())
        .child(
            Skeleton::new()
                .w(px(SKELETON_WIDTHS[index % SKELETON_WIDTHS.len()]))
                .h_3()
                .rounded_md(),
        )
}
