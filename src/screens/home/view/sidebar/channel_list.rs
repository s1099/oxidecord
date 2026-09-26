//! The channel list itself: one row per channel, grouped under its category.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, collapsible::Collapsible, h_flex, v_flex};

use twilight_model::id::{Id, marker::ChannelMarker};

use crate::discord::Channel;
use crate::screens::home::HomeScreen;
use crate::screens::home::channels::{ChannelGroup, channel_icon_path};
use crate::screens::home::view::avatar;
use crate::ui::depth::{Finish, Level, Lit as _, radius};
use crate::ui::theme;

impl HomeScreen {
    pub(super) fn render_channel_row(
        &self,
        channel: &Channel,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        // A voice channel carries the people in it under its own row, the way
        // Discord nests them — whether or not the user is in there too.
        v_flex()
            .gap(px(2.))
            .child(self.channel_row(channel, cx))
            .when(channel.kind.is_voice(), |this| {
                this.children(self.voice_members(channel.id, cx))
            })
    }

    fn channel_row(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = channel.id;
        let is_voice = channel.kind.is_voice();
        // A voice channel is highlighted by being *in* it, not by being the
        // pane on screen.
        let is_selected = if is_voice {
            self.in_voice_channel(channel_id)
        } else {
            self.selected_channel == Some(channel_id)
        };

        h_flex()
            .id(("channel", channel_id.get()))
            .px_2()
            .py(px(5.))
            .gap_2()
            .items_center()
            .rounded(radius::ITEM)
            .cursor_pointer()
            .text_sm()
            .text_color(if is_selected {
                theme.sidebar_accent_foreground
            } else {
                theme::secondary_text(cx)
            })
            .hover(|this| {
                this.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(is_selected, |this| {
                this.bg(theme.sidebar_accent)
                    .font_weight(FontWeight::MEDIUM)
                    .lit(Level::Raised, Finish::Subtle, px(30.), radius::ITEM, cx)
            })
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_4(),
            )
            .child(div().flex_1().truncate().child(channel.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if is_voice {
                    this.join_voice_channel(channel_id, cx);
                } else {
                    this.select_channel(channel_id, window, cx);
                }
            }))
    }

    /// Who's in a voice channel, listed under it.
    fn voice_members(
        &self,
        channel_id: Id<ChannelMarker>,
        cx: &Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let theme = cx.theme();

        self.voice_participants(channel_id)
            .into_iter()
            .map(|participant| {
                let avatar = avatar(
                    participant.name.clone(),
                    participant.avatar_url.clone(),
                    px(20.),
                );

                h_flex()
                    .id(("voice-member", participant.user_id.get()))
                    .pl(px(28.))
                    .pr_2()
                    .py(px(3.))
                    .gap_2()
                    .items_center()
                    .rounded(radius::ITEM)
                    .text_sm()
                    .text_color(if participant.speaking {
                        theme.sidebar_accent_foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(
                        // Mirrors the ring on the call stage, so the sidebar
                        // shows who's talking without opening the channel.
                        div()
                            .rounded_full()
                            .border_1()
                            .border_color(if participant.speaking {
                                theme.success
                            } else {
                                transparent_black()
                            })
                            .child(avatar),
                    )
                    .child(div().flex_1().truncate().child(participant.name.clone()))
                    .when(participant.muted, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/mic-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    })
                    .when(participant.deafened, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/headphone-off.svg")
                                .size_3()
                                .text_color(theme.danger),
                        )
                    })
            })
            .collect()
    }

    pub(super) fn render_channel_group(
        &self,
        group: &ChannelGroup,
        cx: &Context<Self>,
    ) -> AnyElement {
        let Some(category) = &group.category else {
            return v_flex()
                .gap(px(2.))
                .children(
                    group
                        .channels
                        .iter()
                        .map(|channel| self.render_channel_row(channel, cx)),
                )
                .into_any_element();
        };

        let theme = cx.theme();
        let category_id = category.id;
        let collapsed = self.collapsed_categories.contains(&category_id);

        let header = h_flex()
            .id(("category", category_id.get()))
            .mt_2()
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .hover(|this| this.text_color(theme.sidebar_foreground))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size_3(),
            )
            .child(category.name.to_uppercase())
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed_categories.insert(category_id) {
                    this.collapsed_categories.remove(&category_id);
                }
                cx.notify();
            }));

        let mut collapsible = Collapsible::new()
            .open(!collapsed)
            .gap(px(2.))
            .child(header)
            .content(
                v_flex().gap(px(2.)).children(
                    group
                        .channels
                        .iter()
                        .map(|channel| self.render_channel_row(channel, cx)),
                ),
            );

        // Like Discord, keep the selected channel visible when its category
        // is collapsed.
        if collapsed
            && let Some(selected) = group
                .channels
                .iter()
                .find(|channel| Some(channel.id) == self.selected_channel)
        {
            collapsible = collapsible.child(self.render_channel_row(selected, cx));
        }

        collapsible.into_any_element()
    }
}
