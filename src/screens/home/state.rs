//! [`HomeScreen`]'s state and the small types it holds.
//!
//! Fields are `pub(super)` so the [`data`](super::data) and [`view`](super::view)
//! modules — which extend the screen with inherent methods — can reach them,
//! while nothing outside the home screen can.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::slider::SliderState;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker},
};

use crate::discord::{self, DirectMessage, Guild};
use crate::platform::prefs;
use crate::platform::video::VideoPlayer;
use crate::ui::smooth_scroll::SmoothScroll;

use super::channels::ChannelGroup;
use super::data::attachments::PendingAttachment;
use super::folders::RailEntry;
use super::voice::{PendingVoice, VoiceCall};
use crate::voice::VoiceEngine;

/// How many messages from the oldest loaded one the view has to reach before
/// the next page is fetched. A page takes a round trip to arrive, and a scroll
/// that hits the top before then has nowhere left to go.
const OLDER_PAGE_PREFETCH: usize = 10;

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

/// Which video attachment a playback belongs to.
///
/// A message id alone isn't enough — one message can carry several videos —
/// and the index is the message's own, so it stays stable as the list is
/// spliced at either end.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct VideoKey {
    pub message_id: u64,
    pub index: usize,
}

impl VideoKey {
    /// A gpui id for one of this attachment's elements. Both halves of the key
    /// go in: two messages can each hold a video at index 0, and sharing an id
    /// between them would share their interaction state too.
    pub fn element_id(self, role: &str) -> ElementId {
        ElementId::from(SharedString::from(format!(
            "{role}-{}-{}",
            self.message_id, self.index
        )))
    }
}

/// The one video playing, if any.
///
/// Only ever one: decoding a second clip nobody is watching is exactly the
/// cost this feature exists to avoid, so starting one stops the other.
pub(super) struct VideoPlayback {
    pub key: VideoKey,
    /// Dropping this stops the decoder, closes the output device, and deletes
    /// the downloaded file.
    pub player: VideoPlayer,
    /// The frame currently on screen. Held so it can be handed back to gpui's
    /// sprite atlas when the next one replaces it — every frame takes a slot
    /// there, and at 30 a second an un-evicted one is a leak with a timer on
    /// it.
    pub frame: Option<Arc<RenderImage>>,
    /// Zero until the source reports one, and for a stream that never does.
    pub duration: Duration,
    pub position: Duration,
    /// Set until the first frame lands, which covers both the download and the
    /// decoder opening the file.
    pub loading: bool,
    /// Why playback stopped, when it stopped badly. Shown over the poster.
    pub error: Option<String>,
    /// Set when the clip runs out, so the card offers to play it again rather
    /// than sitting on the last frame.
    pub ended: bool,
    /// The progress bar, which doubles as the scrubber. Its value is a
    /// fraction of the run time rather than a time, so it needs no resetting
    /// when the duration arrives after the first frames.
    pub scrubber: Entity<SliderState>,
}

pub struct HomeScreen {
    pub(super) view: View,
    pub(super) guilds: Vec<Guild>,
    /// The signed-in user, shown in the sidebar account panel. `None` until the
    /// `GET /users/@me` fetch resolves (or if it fails).
    pub(super) current_user: Option<discord::CurrentUser>,
    /// The guild list laid out for the rail, folders and all.
    pub(super) rail_entries: Vec<RailEntry>,
    /// The user's folder settings. `None` until the settings-proto fetch
    /// resolves, or if it fails — the rail falls back to the plain guild order.
    pub(super) guild_folders: Option<discord::GuildFolders>,
    /// Folders the user has opened, by folder id. Folders start collapsed.
    pub(super) expanded_folders: HashSet<i64>,
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
    /// The call the user is in, if any. Drives the sidebar's voice panel and
    /// the call stage in the content pane.
    pub(super) voice: Option<VoiceCall>,
    /// Self-mute and self-deafen: one stops the microphone being sent, the
    /// other stops everyone else being played. Deafening silences the
    /// microphone too, so `voice_muted` can be on without the user having
    /// pressed mute. They sit on the screen rather than on the call because
    /// they're kept between calls: leave muted, rejoin muted.
    pub(super) voice_muted: bool,
    pub(super) voice_deafened: bool,
    /// The mute state to go back to when undeafening, so a user who was
    /// unmuted before gets their microphone back and one who was muted stays
    /// muted.
    pub(super) voice_mute_before_deafen: bool,
    /// Id of the chosen microphone, remembered between runs. `None` follows
    /// the system default.
    pub(super) voice_input_device: Option<String>,
    /// Everyone the gateway has reported in a voice channel, by user. This is
    /// what fills every participant list: the tiles on the call stage, and the
    /// names under each voice channel in the sidebar.
    pub(super) voice_states: HashMap<Id<UserMarker>, discord::VoiceUserState>,
    /// Who is transmitting in the open call, from the audio the driver
    /// receives rather than from the gateway.
    pub(super) voice_speaking: HashSet<Id<UserMarker>>,
    /// The connection parameters for a join that's still in flight.
    pub(super) pending_voice: Option<PendingVoice>,
    /// Runs the call. Started with the gateway, since a call needs both.
    pub(super) voice_engine: Option<VoiceEngine>,
    /// Sends commands up the gateway — joining and leaving voice channels is
    /// done with a gateway command, not a REST call.
    pub(super) gateway: Option<discord::GatewaySender>,
    /// The signed-in user's id, from whichever of `READY` or `GET /users/@me`
    /// lands first. The voice connection can't identify itself without it.
    pub(super) self_user_id: Option<Id<UserMarker>>,
    /// Whether shift is currently held, tracked so the message toolbar can
    /// expand its hidden actions inline the way Discord's does.
    pub(super) shift_held: bool,
    /// Focus for the screen as a whole, held by an empty element inside it and
    /// focused on startup: key and modifier events only travel the focus path,
    /// so without it the root's listeners never run.
    pub(super) focus_handle: FocusHandle,
    /// The profile card currently open over the app, if any.
    pub(super) profile_popup: Option<ProfilePopup>,
    /// Profiles already fetched this session, so reopening a card is instant
    /// and repeated clicks don't refetch.
    pub(super) profile_cache: HashMap<Id<UserMarker>, discord::UserProfile>,
    pub(super) message_input: Entity<InputState>,
    pub(super) messages_list: ListState,
    /// The video attachment currently playing, if any.
    pub(super) video: Option<VideoPlayback>,
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
                // Far enough ahead that a page usually lands before the scroll
                // reaches the top, so a continuous scroll up never stalls
                // against it.
                if event.visible_range.start <= OLDER_PAGE_PREFETCH {
                    this.load_older_messages(cx);
                }
            });
        });

        let mut this = Self {
            view: View::Guild,
            guilds: Vec::new(),
            rail_entries: Vec::new(),
            guild_folders: None,
            expanded_folders: HashSet::new(),
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
            voice: None,
            voice_muted: false,
            voice_deafened: false,
            voice_mute_before_deafen: false,
            voice_input_device: prefs::load().input_device,
            voice_states: HashMap::new(),
            voice_speaking: HashSet::new(),
            pending_voice: None,
            voice_engine: None,
            gateway: None,
            self_user_id: None,
            shift_held: false,
            focus_handle: cx.focus_handle(),
            profile_popup: None,
            profile_cache: HashMap::new(),
            video: None,
            message_input,
            messages_scroll: SmoothScroll::list(messages_list.clone()),
            messages_list,
            image_cache: RetainAllImageCache::new(cx),
            rail_scroll: SmoothScroll::div(),
            sidebar_scroll: SmoothScroll::div(),
            dm_scroll: SmoothScroll::div(),
        };
        this.load_guilds(window, cx);
        this.load_guild_folders(cx);
        this.load_current_user(cx);
        this.start_gateway(cx);
        // Seeds the focus path so the root element's key listeners are live
        // from the first frame, before anything has been clicked.
        this.focus_handle.focus(window);
        this
    }
}
