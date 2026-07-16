mod channels;
mod data;
mod dm_data;
mod dm_sidebar;
mod messages;
mod rail;
mod sidebar;

use std::collections::HashSet;

use gpui::*;
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState},
    ActiveTheme as _,
};
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker},
    Id,
};

use crate::discord::{self, DirectMessage, Guild};

use channels::ChannelGroup;

/// Which list occupies the sidebar: a guild's channels, or the DM list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Guild,
    DirectMessages,
}

pub struct HomeScreen {
    view: View,
    guilds: Vec<Guild>,
    selected_guild: Option<Id<GuildMarker>>,
    loading: bool,
    error: Option<String>,
    channel_groups: Vec<ChannelGroup>,
    selected_channel: Option<Id<ChannelMarker>>,
    /// The user's DM conversations, loaded lazily the first time the DM view is
    /// opened. A selected DM reuses `selected_channel` and the message plumbing.
    dms: Vec<DirectMessage>,
    dms_loading: bool,
    dms_error: Option<String>,
    /// Set once the DM list has been fetched, so reopening the view doesn't
    /// refetch it every time.
    dms_loaded: bool,
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
    /// Owns the decoded bitmaps for the currently displayed messages' images.
    /// Cleared on channel switch so image memory doesn't grow without bound.
    image_cache: Entity<RetainAllImageCache>,
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
            view: View::Guild,
            guilds: Vec::new(),
            selected_guild: None,
            loading: true,
            error: None,
            channel_groups: Vec::new(),
            selected_channel: None,
            dms: Vec::new(),
            dms_loading: false,
            dms_error: None,
            dms_loaded: false,
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
            image_cache: RetainAllImageCache::new(cx),
        };
        this.load_guilds(window, cx);
        this
    }
}

impl Render for HomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = match self.view {
            View::DirectMessages => Some(self.render_dm_sidebar(cx).into_any_element()),
            View::Guild => (self.selected_guild.is_some() || self.loading)
                .then(|| self.render_channel_sidebar(cx).into_any_element()),
        };

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_server_rail(cx))
            .children(sidebar)
            .child(self.render_content(cx))
    }
}
