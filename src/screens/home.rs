use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar,
    collapsible::Collapsible,
    divider::Divider,
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _,
};
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker},
    Id,
};

use crate::discord::{self, Channel, ChannelKind, Guild};

/// A category and its channels (or the uncategorized channels when
/// `category` is `None`), in Discord's display order.
struct ChannelGroup {
    category: Option<Channel>,
    channels: Vec<Channel>,
}

/// Groups a guild's channels for the sidebar in Discord's display order:
/// uncategorized channels first, then each category (by position) with its
/// children, text-like channels before voice channels in each group.
fn build_channel_groups(channels: Vec<Channel>) -> Vec<ChannelGroup> {
    let (mut categories, mut others): (Vec<_>, Vec<_>) = channels
        .into_iter()
        .partition(|channel| channel.kind == ChannelKind::Category);

    categories.sort_by_key(|channel| (channel.position, channel.id.get()));
    others.sort_by_key(|channel| (channel.kind.is_voice(), channel.position, channel.id.get()));

    let category_ids: HashSet<_> = categories.iter().map(|category| category.id).collect();

    let mut groups = Vec::new();
    // Channels with no (known) parent category sit above all categories.
    let uncategorized: Vec<_> = others
        .iter()
        .filter(|channel| channel.parent_id.is_none_or(|id| !category_ids.contains(&id)))
        .cloned()
        .collect();
    if !uncategorized.is_empty() {
        groups.push(ChannelGroup {
            category: None,
            channels: uncategorized,
        });
    }
    for category in categories {
        let channels = others
            .iter()
            .filter(|channel| channel.parent_id == Some(category.id))
            .cloned()
            .collect();
        groups.push(ChannelGroup {
            category: Some(category),
            channels,
        });
    }
    groups
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
    channel_groups: Vec<ChannelGroup>,
    selected_channel: Option<Id<ChannelMarker>>,
    collapsed_categories: HashSet<Id<ChannelMarker>>,
    channels_loading: bool,
    channels_error: Option<String>,
    messages: Vec<discord::Message>,
    messages_loading: bool,
    messages_error: Option<String>,
    /// An older page is currently being fetched (scrolled to the top).
    older_loading: bool,
    /// The start of the channel's history has been reached; stop fetching.
    reached_oldest: bool,
    send_error: Option<String>,
    message_input: Entity<InputState>,
    messages_list: ListState,
}

impl HomeScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Send a message"));

        cx.subscribe_in(
            &message_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.send_current_message(window, cx);
                }
            },
        )
        .detach();

        // Bottom-aligned like a chat log; items are measured lazily, and
        // splicing older items in at the front keeps the scroll position.
        let messages_list = ListState::new(0, ListAlignment::Bottom, px(512.));
        let weak = cx.entity().downgrade();
        messages_list.set_scroll_handler(move |event, _window, cx| {
            // Nearing the oldest loaded message; fetch the previous page.
            if event.visible_range.start <= 2 {
                let _ = weak.update(cx, |this, cx| this.load_older_messages(cx));
            }
        });

        let mut this = Self {
            guilds: Vec::new(),
            selected_guild: None,
            loading: true,
            error: None,
            channel_groups: Vec::new(),
            selected_channel: None,
            collapsed_categories: HashSet::new(),
            channels_loading: false,
            channels_error: None,
            messages: Vec::new(),
            messages_loading: false,
            messages_error: None,
            older_loading: false,
            reached_oldest: false,
            send_error: None,
            message_input,
            messages_list,
        };
        this.load_guilds(window, cx);
        this
    }

    fn load_guilds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, window, cx);
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

    fn select_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_guild == Some(guild_id) {
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_groups.clear();
        self.selected_channel = None;
        self.collapsed_categories.clear();
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

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                // The user may have clicked another guild while this request
                // was in flight; drop the stale response.
                if this.selected_guild != Some(guild_id) {
                    return;
                }
                match result {
                    Ok(channels) => {
                        this.channel_groups = build_channel_groups(channels);
                        // Default to the first text-like channel, like Discord.
                        let first = this
                            .channel_groups
                            .iter()
                            .flat_map(|group| &group.channels)
                            .find(|channel| !channel.kind.is_voice())
                            .map(|channel| channel.id);
                        if let Some(channel_id) = first {
                            this.select_channel(channel_id, window, cx);
                        }
                    }
                    Err(err) => this.channels_error = Some(err),
                }
                this.channels_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn select_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel == Some(channel_id) {
            return;
        }
        self.selected_channel = Some(channel_id);
        self.load_messages(window, cx);
    }

    fn load_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        self.messages.clear();
        self.messages_list.reset(0);
        self.messages_error = None;
        self.older_loading = false;
        self.reached_oldest = false;
        self.send_error = None;
        self.messages_loading = true;

        let placeholder = self
            .selected_channel_info()
            .map(|channel| format!("Message #{}", channel.name))
            .unwrap_or_else(|| "Send a message".into());
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.messages_loading = false;
            self.messages_error = Some("No token found in auth.json.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, None, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // The user may have clicked another channel while this request
                // was in flight; drop the stale response.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    Ok(messages) => {
                        this.reached_oldest = messages.len() < discord::MESSAGE_PAGE_SIZE;
                        this.messages = messages;
                        // Resetting also snaps the bottom-aligned list to the
                        // newest message.
                        this.messages_list.reset(this.messages.len());
                    }
                    Err(err) => this.messages_error = Some(err),
                }
                this.messages_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetches the page of messages older than the oldest loaded one and
    /// prepends it, keeping the current scroll position.
    fn load_older_messages(&mut self, cx: &mut Context<Self>) {
        if self.messages_loading || self.older_loading || self.reached_oldest {
            return;
        }
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(oldest_id) = self.messages.first().map(|message| message.id) else {
            return;
        };
        let Some(token) = discord::load_token() else {
            return;
        };

        self.older_loading = true;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, Some(oldest_id), move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the channel changed or the messages
                // were reloaded while this request was in flight.
                if this.selected_channel != Some(channel_id)
                    || this.messages.first().map(|message| message.id) != Some(oldest_id)
                {
                    return;
                }
                this.older_loading = false;
                match result {
                    Ok(older) => {
                        if older.len() < discord::MESSAGE_PAGE_SIZE {
                            this.reached_oldest = true;
                        }
                        let count = older.len();
                        if count > 0 {
                            this.messages.splice(0..0, older);
                            this.messages_list.splice(0..0, count);
                        }
                    }
                    // Keep the messages on screen; the fetch retries the next
                    // time the user scrolls near the top.
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let content = self.message_input.read(cx).value().trim().to_string();
        if content.is_empty() {
            return;
        }

        let Some(token) = discord::load_token() else {
            self.send_error = Some("No token found in auth.json.".into());
            cx.notify();
            return;
        };

        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_error = None;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::send_message(token, channel_id, content, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the user switched channels meanwhile.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    Ok(message) => {
                        let ix = this.messages.len();
                        this.messages.push(message);
                        this.messages_list.splice(ix..ix, 1);
                        // Past-the-end offsets clamp to the bottom.
                        this.messages_list.scroll_to(ListOffset {
                            item_ix: ix + 1,
                            offset_in_item: px(0.),
                        });
                    }
                    Err(err) => this.send_error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn selected_channel_info(&self) -> Option<&Channel> {
        let id = self.selected_channel?;
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == id)
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_guild(guild_id, window, cx);
                            }))
                    })),
            )
    }

    fn render_channel_row(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = channel.id;
        let is_selected = self.selected_channel == Some(channel_id);

        h_flex()
            .id(("channel", channel_id.get()))
            .px_2()
            .py(px(5.))
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_sm()
            .text_color(if is_selected {
                theme.sidebar_accent_foreground
            } else {
                theme.muted_foreground
            })
            .when(is_selected, |this| this.bg(theme.sidebar_accent))
            .hover(|this| this.bg(theme.sidebar_accent.opacity(0.5)))
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_4(),
            )
            .child(div().flex_1().truncate().child(channel.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_channel(channel_id, window, cx);
            }))
    }

    fn render_channel_group(&self, group: &ChannelGroup, cx: &Context<Self>) -> AnyElement {
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
        if collapsed {
            if let Some(selected) = group
                .channels
                .iter()
                .find(|channel| Some(channel.id) == self.selected_channel)
            {
                collapsible = collapsible.child(self.render_channel_row(selected, cx));
            }
        }

        collapsible.into_any_element()
    }

    fn render_channel_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sidebar_border = theme.sidebar_border;
        let muted = theme.muted_foreground;
        let danger = theme.danger;

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
            list = list.children(
                self.channel_groups
                    .iter()
                    .map(|group| self.render_channel_group(group, cx)),
            );
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

    fn render_channel_header(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        h_flex()
            .h(px(48.))
            .w_full()
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(channel.name.clone()),
            )
            .when_some(
                // show only the first line and let `truncate` elide the rest.
                channel
                    .topic
                    .as_deref()
                    .and_then(|topic| topic.lines().find(|line| !line.trim().is_empty()))
                    .map(str::to_owned),
                |this, topic| {
                    this.child(Divider::vertical().h(px(24.))).child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(topic),
                    )
                },
            )
    }

    fn render_message(
        &self,
        message: &discord::Message,
        show_header: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let content: AnyElement = if message.content.is_empty() {
            div()
                .italic()
                .text_color(theme.muted_foreground)
                .child("(no text content)")
                .into_any_element()
        } else {
            div().child(message.content.clone()).into_any_element()
        };

        // Consecutive messages from the same author share one header, like
        // Discord; align follow-ups with the content column (avatar + gap).
        if !show_header {
            return div()
                .id(("message", message.id.get()))
                .pl(px(52.))
                .py(px(1.))
                .text_sm()
                .child(content)
                .into_any_element();
        }

        let mut avatar = Avatar::new()
            .name(message.author_name.clone())
            .with_size(px(40.));
        if let Some(avatar_url) = message.author_avatar_url.clone() {
            avatar = avatar.src(avatar_url);
        }

        h_flex()
            .id(("message", message.id.get()))
            .mt_3()
            .gap_3()
            .items_start()
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(message.author_name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(message.timestamp.clone()),
                            ),
                    )
                    .child(div().text_sm().child(content)),
            )
            .into_any_element()
    }

    fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Loading messages...")
                .into_any_element();
        }

        if let Some(error) = &self.messages_error {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element();
        }

        let entity = cx.entity();
        let messages_list = list(self.messages_list.clone(), move |ix, _window, cx| {
            entity.update(cx, |this, cx| this.render_message_item(ix, cx))
        })
        .flex_1()
        .px_4()
        .py_2();

        let mut container = v_flex().flex_1().min_h_0().w_full();
        if self.older_loading {
            container = container.child(
                h_flex()
                    .w_full()
                    .py_1()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Loading older messages..."),
            );
        }
        container.child(messages_list).into_any_element()
    }

    fn render_message_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        // Consecutive messages from the same author share one header.
        let show_header = ix == 0
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        self.render_message(message, show_header, cx)
    }

    fn render_message_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .w_full()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_4()
            .gap_1()
            .when_some(self.send_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(Input::new(&self.message_input))
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

        let Some(channel) = self.selected_channel_info().cloned() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select a channel")
                .into_any_element();
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_channel_header(&channel, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(cx))
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
