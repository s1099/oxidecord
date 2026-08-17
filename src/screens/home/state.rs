//! [`HomeScreen`]'s state and the small types it holds.
//!
//! Fields are `pub(super)` so the [`data`](super::data) and [`view`](super::view)
//! modules — which extend the screen with inherent methods — can reach them,
//! while nothing outside the home screen can.

use std::collections::{HashMap, HashSet};

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker},
};

use crate::discord::{self, DirectMessage, Guild};
use crate::ui::smooth_scroll::SmoothScroll;

use super::channels::ChannelGroup;
use super::data::attachments::PendingAttachment;

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

/// The open profile popout: which user it's for, where it's anchored, and the
/// placeholder shown from the message that opened it while the fetch is in
/// flight.
pub(super) struct ProfilePopup {
    pub user_id: Id<UserMarker>,
    /// Window coordinates the card is anchored to, taken from the click on the
    /// avatar.
    pub position: Point<Pixels>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// `None` until the profile fetch resolves; the card renders a skeleton in
    /// the meantime.
    pub profile: Option<discord::UserProfile>,
    pub error: Option<String>,
}

pub struct HomeScreen {
    pub(super) view: View,
    pub(super) guilds: Vec<Guild>,
    /// The signed-in user, shown in the sidebar account panel. `None` until the
    /// `GET /users/@me` fetch resolves (or if it fails).
    pub(super) current_user: Option<discord::CurrentUser>,
    pub(super) selected_guild: Option<Id<GuildMarker>>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) channel_groups: Vec<ChannelGroup>,
    pub(super) selected_channel: Option<Id<ChannelMarker>>,
    /// The user's DM conversations, loaded lazily the first time the DM view is
    /// opened. A selected DM reuses `selected_channel` and the message plumbing.
    pub(super) dms: Vec<DirectMessage>,
    pub(super) dms_loading: bool,
    pub(super) dms_error: Option<String>,
    /// Set once the DM list has been fetched, so reopening the view doesn't
    /// refetch it every time.
    pub(super) dms_loaded: bool,
    pub(super) collapsed_categories: HashSet<Id<ChannelMarker>>,
    pub(super) channels_loading: bool,
    pub(super) channels_error: Option<String>,
    pub(super) messages: Vec<discord::Message>,
    pub(super) messages_loading: bool,
    pub(super) messages_error: Option<String>,
    /// An older page is currently being fetched (scrolled to the top).
    pub(super) older_loading: bool,
    /// The start of the channel's history has been reached; stop fetching.
    pub(super) reached_oldest: bool,
    /// Whether the newest message is currently in view. Used to decide if a
    /// live message should snap the list to the bottom (following the
    /// conversation) or be appended silently (the user is reading history).
    pub(super) at_bottom: bool,
    pub(super) send_error: Option<String>,
    /// Set while composing a reply; drives the "Replying to …" banner and is
    /// cleared when the reply is sent, dismissed, or the channel changes.
    pub(super) replying_to: Option<ReplyTarget>,
    /// Images pasted into the composer, shown as removable thumbnails and
    /// uploaded when the message is sent. Cleared on send and on channel switch.
    pub(super) pending_attachments: Vec<PendingAttachment>,
    /// Monotonic id source for `pending_attachments`, so each thumbnail has a
    /// stable key even if the same image is pasted twice.
    pub(super) next_attachment_id: u64,
    /// The profile card currently open over the app, if any.
    pub(super) profile_popup: Option<ProfilePopup>,
    /// Profiles already fetched this session, so reopening a card is instant
    /// and repeated clicks don't refetch.
    pub(super) profile_cache: HashMap<Id<UserMarker>, discord::UserProfile>,
    pub(super) message_input: Entity<InputState>,
    pub(super) messages_list: ListState,
    /// Owns the decoded bitmaps for the currently displayed messages' images.
    /// Cleared on channel switch so image memory doesn't grow without bound.
    pub(super) image_cache: Entity<RetainAllImageCache>,
    /// Wheel easing for each scrollable pane; see [`crate::ui::smooth_scroll`].
    pub(super) rail_scroll: SmoothScroll,
    pub(super) sidebar_scroll: SmoothScroll,
    pub(super) dm_scroll: SmoothScroll,
    pub(super) messages_scroll: SmoothScroll,
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
            profile_popup: None,
            profile_cache: HashMap::new(),
            message_input,
            messages_scroll: SmoothScroll::list(messages_list.clone()),
            messages_list,
            image_cache: RetainAllImageCache::new(cx),
            rail_scroll: SmoothScroll::div(),
            sidebar_scroll: SmoothScroll::div(),
            dm_scroll: SmoothScroll::div(),
        };
        this.load_guilds(window, cx);
        this.load_current_user(cx);
        this.start_gateway(cx);
        this
    }
}
