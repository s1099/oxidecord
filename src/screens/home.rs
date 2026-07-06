use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar, divider::Divider, h_flex, tooltip::Tooltip, v_flex, ActiveTheme as _, Icon,
};
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker},
    Id,
};

use crate::discord::{self, Channel, ChannelKind, Guild};

/// One row in the channel sidebar: either a category header or a channel.
enum ChannelRow {
    Category(Channel),
    Channel(Channel),
}

/// Flattens a guild's channels into sidebar rows in Discord's display order:
/// uncategorized channels first, then each category (by position) followed by
/// its children, with text-like channels before voice channels in each group.
fn build_channel_rows(channels: Vec<Channel>) -> Vec<ChannelRow> {
    let (mut categories, mut others): (Vec<_>, Vec<_>) = channels
        .into_iter()
        .partition(|channel| channel.kind == ChannelKind::Category);

    categories.sort_by_key(|channel| (channel.position, channel.id.get()));
    others.sort_by_key(|channel| (channel.kind.is_voice(), channel.position, channel.id.get()));

    let category_ids: HashSet<_> = categories.iter().map(|category| category.id).collect();

    let mut rows = Vec::new();
    // Channels with no (known) parent category sit above all categories.
    for channel in &others {
        if channel.parent_id.is_none_or(|id| !category_ids.contains(&id)) {
            rows.push(ChannelRow::Channel(channel.clone()));
        }
    }
    for category in categories {
        let category_id = category.id;
        rows.push(ChannelRow::Category(category));
        for channel in &others {
            if channel.parent_id == Some(category_id) {
                rows.push(ChannelRow::Channel(channel.clone()));
            }
        }
    }
    rows
}

fn channel_icon_path(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Text => "icons/hash.svg",
        ChannelKind::Announcement => "icons/megaphone.svg",
        ChannelKind::Voice | ChannelKind::Stage => "icons/volume-2.svg",
        ChannelKind::Forum => "icons/messages-square.svg",
        ChannelKind::Category => "icons/chevron-down.svg",
    }
}

pub struct HomeScreen {
    guilds: Vec<Guild>,
    selected_guild: Option<Id<GuildMarker>>,
    loading: bool,
    error: Option<String>,
    channel_rows: Vec<ChannelRow>,
    selected_channel: Option<Id<ChannelMarker>>,
    channels_loading: bool,
    channels_error: Option<String>,
}

impl HomeScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            guilds: Vec::new(),
            selected_guild: None,
            loading: true,
            error: None,
            channel_rows: Vec::new(),
            selected_channel: None,
            channels_loading: false,
            channels_error: None,
        };
        this.load_guilds(cx);
        this
    }

    fn load_guilds(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            self.loading = false;
            self.error = Some("No token found in auth.json. Please log in first.".into());
            return;
        };

        // `discord::fetch_guilds` runs its callback on a background Tokio
        // thread, but gpui's entity/async handles are `!Send`. Bridge the two
        // with a plain, `Send`-safe channel and let gpui's own (non-Send)
        // foreground task pick up the result.
        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_guilds(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, cx);
                        }
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn select_guild(&mut self, guild_id: Id<GuildMarker>, cx: &mut Context<Self>) {
        if self.selected_guild == Some(guild_id) {
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_rows.clear();
        self.selected_channel = None;
        self.channels_error = None;
        self.channels_loading = true;
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.channels_loading = false;
            self.channels_error = Some("No token found in auth.json.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_channels(token, guild_id, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // The user may have clicked another guild while this request
                // was in flight; drop the stale response.
                if this.selected_guild != Some(guild_id) {
                    return;
                }
                match result {
                    Ok(channels) => {
                        this.channel_rows = build_channel_rows(channels);
                        // Default to the first text-like channel, like Discord.
                        this.selected_channel =
                            this.channel_rows.iter().find_map(|row| match row {
                                ChannelRow::Channel(channel) if !channel.kind.is_voice() => {
                                    Some(channel.id)
                                }
                                _ => None,
                            });
                    }
                    Err(err) => this.channels_error = Some(err),
                }
                this.channels_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_guild;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let rail_border = theme.sidebar_border;
        // Fixed dark badge: the white logo is baked into the SVG (img() can't
        // tint), and a dark background keeps it readable in either theme while
        // hiding the antialiased edge fringe from gpui's SVG rasterization.
        let logo_bg = rgb(0x313338);
        let selected_bg = theme.sidebar_accent;

        v_flex()
            .id("server-rail")
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .py_3()
            .gap_3()
            .bg(rail_bg)
            .border_r_1()
            .border_color(rail_border)
            .child(
                div()
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(logo_bg)
                    .child(
                        img(Arc::new(Image::from_bytes(
                            ImageFormat::Svg,
                            crate::constants::DISCORD_ICON.as_bytes().to_vec(),
                        )))
                        .size(px(28.)),
                    ),
            )
            .child(Divider::horizontal().w(px(32.)))
            .child(
                v_flex()
                    .id("guild-list")
                    .flex_1()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .overflow_y_scroll()
                    .children(self.guilds.iter().map(|guild| {
                        let guild_id = guild.id;
                        let guild_name = guild.name.clone();
                        let is_selected = selected == Some(guild_id);

                        let mut avatar = Avatar::new().name(guild_name.clone());
                        if let Some(icon_url) = guild.icon_url.clone() {
                            avatar = avatar.src(icon_url);
                        }

                        div()
                            .id(("guild", guild_id.get()))
                            .cursor_pointer()
                            .p(px(4.))
                            .rounded(px(16.))
                            .when(is_selected, |this| this.bg(selected_bg))
                            .child(avatar)
                            .tooltip(move |window, cx| {
                                Tooltip::new(guild_name.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_guild(guild_id, cx);
                            }))
                    })),
            )
    }

    fn render_channel_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sidebar_border = theme.sidebar_border;
        let muted = theme.muted_foreground;
        let selected_bg = theme.sidebar_accent;
        let selected_fg = theme.sidebar_accent_foreground;
        let hover_bg = theme.sidebar_accent.opacity(0.5);
        let danger = theme.danger;
        let selected_channel = self.selected_channel;

        let guild_name = self
            .selected_guild
            .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
            .map(|guild| guild.name.clone())
            .unwrap_or_default();

        let mut list = v_flex()
            .id("channel-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .gap(px(2.));

        if self.channels_loading {
            list = list.child(div().px_2().text_sm().text_color(muted).child("Loading..."));
        } else if let Some(error) = &self.channels_error {
            list = list.child(div().px_2().text_sm().text_color(danger).child(error.clone()));
        } else {
            list = list.children(self.channel_rows.iter().map(|row| match row {
                ChannelRow::Category(category) => h_flex()
                    .mt_2()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(Icon::default().path(channel_icon_path(category.kind)).size_3())
                    .child(category.name.to_uppercase())
                    .into_any_element(),
                ChannelRow::Channel(channel) => {
                    let channel_id = channel.id;
                    let is_selected = selected_channel == Some(channel_id);

                    h_flex()
                        .id(("channel", channel_id.get()))
                        .px_2()
                        .py(px(5.))
                        .gap_2()
                        .items_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_sm()
                        .text_color(if is_selected { selected_fg } else { muted })
                        .when(is_selected, |this| this.bg(selected_bg))
                        .hover(|this| this.bg(hover_bg))
                        .child(
                            Icon::default()
                                .path(channel_icon_path(channel.kind))
                                .size_4(),
                        )
                        .child(div().flex_1().truncate().child(channel.name.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_channel = Some(channel_id);
                            cx.notify();
                        }))
                        .into_any_element()
                }
            }));
        }

        v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(sidebar_border)
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .border_b_1()
                    .border_color(sidebar_border)
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(guild_name),
                    ),
            )
            .child(list)
    }

    fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Loading...")
                .into_any_element();
        }

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(theme.danger).child(error.clone()))
                .into_any_element();
        }

        let selected_channel = self.selected_channel.and_then(|id| {
            self.channel_rows.iter().find_map(|row| match row {
                ChannelRow::Channel(channel) if channel.id == id => Some(channel.clone()),
                _ => None,
            })
        });

        let title = match &selected_channel {
            Some(channel) => channel.name.clone(),
            None => self
                .selected_guild
                .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
                .map(|guild| guild.name.clone())
                .unwrap_or_else(|| "Select a server".into()),
        };

        v_flex()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when_some(selected_channel, |this, channel| {
                        this.child(Icon::default().path(channel_icon_path(channel.kind)).size_5())
                    })
                    .child(title),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("soonTM."),
            )
            .into_any_element()
    }
}

impl Render for HomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self
            .selected_guild
            .is_some()
            .then(|| self.render_channel_sidebar(cx).into_any_element());

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_server_rail(cx))
            .children(sidebar)
            .child(self.render_content(cx))
    }
}
