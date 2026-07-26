mod channels;
mod data;
mod dm_data;
mod dm_sidebar;
mod messages;
mod rail;
mod sidebar;

use std::collections::HashSet;
use std::sync::Arc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, h_flex,
    input::{InputEvent, InputState},
};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{self, DirectMessage, Guild};

use channels::ChannelGroup;

actions!(
    oxidecord,
    [
        /// Paste an image from the clipboard into the message composer as an
        /// attachment. Bound to the paste shortcut so it runs ahead of the text
        /// input's own paste, which only handles text.
        PasteAttachment
    ]
);

/// Which list occupies the sidebar: a guild's channels, or the DM list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Guild,
    DirectMessages,
}

/// The message a pending reply is aimed at, shown as a banner above the
/// message bar. Kept minimal: enough to label the banner and reference the
/// target when the reply is sent.
#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub message_id: Id<MessageMarker>,
    pub author_name: String,
}

/// A file staged for sending, picked from disk or pasted from the clipboard
pub(super) struct PendingAttachment {
    /// Unique within the composer; keys the card element and targets removal.
    pub id: u64,
    /// The upload filename: the file's own name, or one derived from the image
    /// format for a pasted image (e.g. `image-1.png`).
    pub filename: String,
    /// The file's contents, rendered as a preview and uploaded on send.
    pub data: AttachmentData,
}

/// The contents of a staged attachment. An image keeps the decoded [`Image`], so
/// the same bytes back both its thumbnail and the upload; any other file is held
/// as raw bytes and previewed as a file card instead.
pub(super) enum AttachmentData {
    Image(Arc<Image>),
    File(Arc<Vec<u8>>),
}

impl AttachmentData {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Image(image) => &image.bytes,
            Self::File(bytes) => bytes,
        }
    }

    pub fn image(&self) -> Option<Arc<Image>> {
        match self {
            Self::Image(image) => Some(image.clone()),
            Self::File(_) => None,
        }
    }
}

pub(super) fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    const MB: f64 = 1024. * KB;

    let bytes = bytes as f64;
    if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

pub struct HomeScreen {
    view: View,
    guilds: Vec<Guild>,
    /// The signed-in user, shown in the sidebar account panel. `None` until the
    /// `GET /users/@me` fetch resolves (or if it fails).
    current_user: Option<discord::CurrentUser>,
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
    /// Whether the newest message is currently in view. Used to decide if a
    /// live message should snap the list to the bottom (following the
    /// conversation) or be appended silently (the user is reading history).
    at_bottom: bool,
    send_error: Option<String>,
    /// Set while composing a reply; drives the "Replying to …" banner and is
    /// cleared when the reply is sent, dismissed, or the channel changes.
    replying_to: Option<ReplyTarget>,
    /// Images pasted into the composer, shown as removable thumbnails and
    /// uploaded when the message is sent. Cleared on send and on channel switch.
    pending_attachments: Vec<PendingAttachment>,
    /// Monotonic id source for `pending_attachments`, so each thumbnail has a
    /// stable key even if the same image is pasted twice.
    next_attachment_id: u64,
    message_input: Entity<InputState>,
    messages_list: ListState,
    /// Owns the decoded bitmaps for the currently displayed messages' images.
    /// Cleared on channel switch so image memory doesn't grow without bound.
    image_cache: Entity<RetainAllImageCache>,
}

impl HomeScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message_input = cx.new(|cx| InputState::new(window, cx).placeholder("Send a message"));

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
            let _ = weak.update(cx, |this, cx| {
                // Track whether the newest message is on screen so live
                // messages only auto-scroll when the user is at the bottom.
                this.at_bottom = event.visible_range.end >= event.count;
                // Nearing the oldest loaded message; fetch the previous page.
                if event.visible_range.start <= 2 {
                    this.load_older_messages(cx);
                }
            });
        });

        let mut this = Self {
            view: View::Guild,
            guilds: Vec::new(),
            current_user: None,
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
            at_bottom: true,
            send_error: None,
            replying_to: None,
            pending_attachments: Vec::new(),
            next_attachment_id: 0,
            message_input,
            messages_list,
            image_cache: RetainAllImageCache::new(cx),
        };
        this.load_guilds(window, cx);
        this.load_current_user(cx);
        this.start_gateway(cx);
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
            .on_action(cx.listener(Self::on_paste_attachment))
            .child(self.render_server_rail(cx))
            .children(sidebar)
            .child(self.render_content(cx))
    }
}
