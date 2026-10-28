//! [`HomeScreen`]'s state and the small types it holds.
//!
//! Fields are `pub(super)` so [`data`](super::data) and [`view`](super::view) can
//! reach them but nothing outside the home screen can.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::slider::SliderState;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use crate::discord::{self, DirectMessage, Guild};
use crate::platform::prefs;
use crate::platform::video::VideoPlayer;
use crate::ui::smooth_scroll::SmoothScroll;

use super::autocomplete::{Suggestion, Trigger};
use super::channels::ChannelGroup;
use super::data::attachments::PendingAttachment;
use super::data::gif::LoadedGifs;
use super::emoji::{PickerLayout, SectionKind};
use super::folders::RailEntry;
use super::history_cache::HistoryCache;
use super::member_list_cache::MemberListCache;
use super::selection::{Selection, TextCopies, TextRegions};
use super::unread::Unreads;
use super::voice::{PendingVoice, ScreenShare, StreamWatch, VoiceCall};
use crate::voice::VoiceEngine;
use crate::voice::stream::StreamEvent;

const COMPOSER_MAX_ROWS: usize = 12;

/// Messages from either loaded end at which the next page is fetched; it takes
/// a round trip, and a scroll reaching the end first would stall.
const PAGE_PREFETCH: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Guild,
    DirectMessages,
}

pub(super) type MemberRoles = HashMap<Id<UserMarker>, Vec<Id<RoleMarker>>>;

#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub message_id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    /// Whether the reply pings its author, as Discord's "@ ON" toggle.
    pub mention: bool,
}

/// Shown until `until` passes or their message arrives.
pub(super) struct Typist {
    pub user_id: Id<UserMarker>,
    /// From the dispatch; a DM's typists are named from its messages instead.
    pub name: Option<String>,
    pub until: Instant,
}

/// The input is made per edit so it starts with no undo history from the last.
pub(super) struct EditingMessage {
    pub message_id: Id<MessageMarker>,
    pub input: Entity<TextareaState>,
}

pub(super) struct ProfilePopup {
    pub user_id: Id<UserMarker>,
    /// Window coordinates of the avatar click.
    pub position: Point<Pixels>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// `None` until the fetch resolves; the card shows a skeleton.
    pub profile: Option<discord::UserProfile>,
    pub error: Option<String>,
}

pub(super) struct PinsPopup {
    pub channel_id: Id<ChannelMarker>,
    /// Window coordinates of the pin button click; the popout hangs below.
    pub position: Point<Pixels>,
    /// Most recently pinned first. Empty while loading.
    pub messages: Vec<discord::Message>,
    /// A list rather than nested flex: inside the popout's flex layout, pin text
    /// is measured as if it had no width and keeps the height of every wrapped
    /// line as empty space.
    pub list: ListState,
    pub loading: bool,
    pub error: Option<String>,
}

pub(super) struct ImageViewer {
    /// Already on screen; drawn stretched while the larger copy loads.
    pub preview_url: SharedString,
    /// Window-sized; dropped from the image cache on close, being far bigger than list images.
    pub full_url: SharedString,
    pub source_url: String,
    /// Original resolution, fetched on the first zoom past `full_url`. Dropped on close too.
    pub zoomed_url: Option<SharedString>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Larger copies must be requested as animated WebP to keep moving; see
    /// [`discord::animated_webp`].
    pub animated_webp: bool,
    /// 1 fits the window.
    pub zoom: f32,
    pub offset: Point<Pixels>,
    /// Pointer position and `offset` at the press.
    pub drag: Option<(Point<Pixels>, Point<Pixels>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmojiTarget {
    Composer,
    Reaction(Id<MessageMarker>),
}

#[derive(Default)]
pub(super) struct Reactors {
    pub users: Vec<discord::Reactor>,
    pub loading: bool,
    /// The last page came back short, so `users` is everyone.
    pub complete: bool,
    pub error: Option<String>,
    /// Held so forgetting the entry cancels the fetch and a stale page can't
    /// land in its replacement.
    pub _task: Option<Task<()>>,
}

pub(super) struct EmojiPicker {
    /// Window coordinates of the opening click.
    pub position: Point<Pixels>,
    pub target: EmojiTarget,
    pub search: Entity<InputState>,
    /// Snapshot from opening, so choosing one doesn't reshuffle under the pointer.
    pub recent: Vec<super::emoji::PickerEmoji>,
    /// Rebuilt on search change, not per frame: the screen repaints for every
    /// video frame and this is a couple of thousand emoji.
    pub layout: Rc<PickerLayout>,
    pub collapsed: HashSet<SectionKind>,
    pub hovered: Option<super::emoji::PickerEmoji>,
    pub scroll: SmoothScroll,
    pub rail_scroll: ScrollHandle,
    pub _search_changed: Subscription,
}

pub(super) struct Autocomplete {
    pub trigger: Trigger,
    /// Byte range of the word, trigger included; what accepting replaces.
    pub range: Range<usize>,
    pub query: String,
    pub items: Vec<Suggestion>,
    pub selected: usize,
}

pub(super) struct MemberList {
    pub guild_id: Id<GuildMarker>,
    pub channel_id: Id<ChannelMarker>,
    /// Kept across a channel switch within the guild: channels the same people
    /// see share a list and Discord carries on with it.
    pub id: Option<String>,
    pub groups: Vec<discord::MemberListGroup>,
    /// Headers included, at Discord's indices; `None` for unsynced rows (placeholders).
    pub rows: Vec<Option<discord::MemberListItem>>,
    pub ranges: Vec<[usize; 2]>,
    pub scroll: UniformListScrollHandle,
}

/// Identifies one video attachment or embed. A message id alone isn't enough
/// (a message can carry several), and the message's own index stays stable as
/// the list is spliced at either end.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct MediaKey {
    pub message_id: u64,
    pub index: usize,
}

impl MediaKey {
    /// Both halves go in: two messages can each hold a video at index 0, and a
    /// shared id would share interaction state.
    pub fn element_id(self, role: &str) -> ElementId {
        ElementId::from(SharedString::from(format!(
            "{role}-{}-{}",
            self.message_id, self.index
        )))
    }

    /// Key into `revealed_spoilers`. `role` tells an image from a video, which
    /// are indexed separately.
    pub fn spoiler_key(self, role: &str) -> SharedString {
        SharedString::from(format!(
            "attachment-{role}-spoiler-{}-{}",
            self.message_id, self.index
        ))
    }

    pub fn group(self) -> SharedString {
        SharedString::from(format!("media-{}-{}", self.message_id, self.index))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum PlaybackState {
    /// Until the first frame lands: covers the download and the decoder opening.
    Loading,
    Playing,
    Ended,
    Failed(String),
}

/// Only one plays at a time; starting one stops the other.
pub(super) struct VideoPlayback {
    pub key: MediaKey,
    /// Dropping it stops the decoder, closes the output device and deletes the file.
    pub player: VideoPlayer,
    /// Held to hand back to gpui's sprite atlas when the next replaces it; every
    /// frame takes a slot, so an un-evicted one is a leak.
    pub frame: Option<Arc<RenderImage>>,
    /// Zero until the source reports one.
    pub duration: Duration,
    pub position: Duration,
    pub state: PlaybackState,
    /// A fraction of the run time, not a time, so a duration arriving after
    /// the first frames needs no reset.
    pub scrubber: Entity<SliderState>,
}

/// A view of its own so the window can replay it from the last frame instead
/// of rebuilding every visible message when the typing dots animate.
pub(super) struct MessageListView {
    pub(super) screen: WeakEntity<HomeScreen>,
}

/// A view of its own, since an animation redraws its view every frame.
pub(super) struct TypingDots;

pub struct HomeScreen {
    pub(super) view: View,
    pub(super) guilds: Vec<Guild>,
    /// `None` until `GET /users/@me` resolves (or if it fails).
    pub(super) current_user: Option<discord::CurrentUser>,
    pub(super) rail_entries: Vec<RailEntry>,
    /// `None` until the settings-proto fetch resolves; the rail uses plain guild order.
    pub(super) guild_folders: Option<discord::GuildFolders>,
    /// From settings-proto. Until it resolves the panel shows online and
    /// nothing is sent to the gateway.
    pub(super) presence_status: Option<discord::PresenceStatus>,
    /// Anyone missing is shown offline: Discord sends presences only for friends
    /// and shared-guild users, and never offline ones up front.
    pub(super) user_presences: HashMap<Id<UserMarker>, discord::PresenceStatus>,
    /// By folder id; folders start collapsed.
    pub(super) expanded_folders: HashSet<i64>,
    pub(super) selected_guild: Option<Id<GuildMarker>>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    /// Set through `set_channel_groups`, which keeps `channel_index` in step.
    pub(super) channel_groups: Vec<ChannelGroup>,
    /// (group, channel) indices; channels are looked up by id for every message drawn.
    pub(super) channel_index: HashMap<Id<ChannelMarker>, (usize, usize)>,
    /// From `READY`, or a fetch where it hasn't arrived. Opening a guild draws
    /// from here at once.
    pub(super) guild_channels: HashMap<Id<GuildMarker>, Vec<discord::Channel>>,
    /// Changed since sent; opening one draws what's there and refetches underneath.
    pub(super) stale_guild_channels: HashSet<Id<GuildMarker>>,
    pub(super) fetching_guild_channels: HashSet<Id<GuildMarker>>,
    pub(super) last_guild_channel: HashMap<Id<GuildMarker>, Id<ChannelMarker>>,
    pub(super) selected_channel: Option<Id<ChannelMarker>>,
    /// A selected DM reuses `selected_channel` and the message plumbing.
    pub(super) dms: Vec<DirectMessage>,
    pub(super) dms_loading: bool,
    pub(super) dms_error: Option<String>,
    pub(super) dms_loaded: bool,
    /// Refetch after a message arrived in a conversation the list lacks.
    pub(super) dms_refreshing: bool,
    pub(super) collapsed_categories: HashSet<Id<ChannelMarker>>,
    pub(super) unreads: Unreads,
    /// Last message read when the conversation opened; the "new" divider sits under it.
    pub(super) unread_marker: Option<Id<MessageMarker>>,
    /// Suppresses acking until the conversation is left.
    pub(super) marked_unread: bool,
    /// Debounced acks by channel, and the timer that sends them.
    pub(super) pending_acks: HashMap<Id<ChannelMarker>, Id<MessageMarker>>,
    pub(super) ack_flush: Option<Task<()>>,
    pub(super) channels_loading: bool,
    pub(super) channels_error: Option<String>,
    pub(super) messages: Vec<discord::Message>,
    /// Set once `messages` arrive; this is what's cached on leaving.
    pub(super) messages_channel: Option<Id<ChannelMarker>>,
    pub(super) history_cache: HistoryCache,
    pub(super) messages_loading: bool,
    pub(super) messages_error: Option<String>,
    pub(super) older_loading: bool,
    pub(super) reached_oldest: bool,
    /// Scrolling down through history opened further back than the present.
    pub(super) newer_loading: bool,
    /// False only when the conversation opened at an unread run longer than a
    /// page, until paging forward or jumping to the present.
    pub(super) reached_newest: bool,
    /// Decides whether a live message snaps the list down or is appended silently.
    pub(super) at_bottom: bool,
    /// Bumped on wholesale replacement so a page fetched for the old messages can't land.
    pub(super) messages_generation: u64,
    /// A link's target, applied once its channel's conversation loads.
    pub(super) pending_jump: Option<(Id<ChannelMarker>, Id<MessageMarker>)>,
    /// Highlighted briefly after a jump, and the timer that clears it.
    pub(super) flashed_message: Option<Id<MessageMarker>>,
    pub(super) flash_timer: Option<Task<()>>,
    pub(super) send_error: Option<String>,
    pub(super) replying_to: Option<ReplyTarget>,
    /// At most one, like Discord.
    pub(super) editing: Option<EditingMessage>,
    pub(super) pending_attachments: Vec<PendingAttachment>,
    /// Stable keys for `pending_attachments`, even if the same image is pasted twice.
    pub(super) next_attachment_id: u64,
    pub(super) voice: Option<VoiceCall>,
    /// Deafening silences the microphone too, so `voice_muted` can be on without
    /// the user pressing mute. Kept here, not on the call, so they survive
    /// between calls.
    pub(super) voice_muted: bool,
    pub(super) voice_deafened: bool,
    /// Restored on undeafen.
    pub(super) voice_mute_before_deafen: bool,
    /// `None` follows the system default. Persisted in prefs.
    pub(super) voice_input_device: Option<String>,
    /// `None` follows the system default. Persisted in prefs.
    pub(super) voice_output_device: Option<String>,
    /// Source of every participant list: call stage tiles and sidebar names.
    pub(super) voice_states: HashMap<Id<UserMarker>, discord::VoiceUserState>,
    /// From received audio, not the gateway.
    pub(super) voice_speaking: HashSet<Id<UserMarker>>,
    /// Only calls the gateway has reported; nobody being rung is an empty list.
    pub(super) call_ringing: HashMap<Id<ChannelMarker>, Vec<Id<UserMarker>>>,
    pub(super) pending_voice: Option<PendingVoice>,
    pub(super) voice_engine: Option<VoiceEngine>,
    /// Present from the moment it's announced to the gateway.
    pub(super) screen_share: Option<ScreenShare>,
    /// One channel for all streams, each event tagged with its stream.
    pub(super) stream_events: Option<futures::channel::mpsc::UnboundedSender<StreamEvent>>,
    /// Present from the moment it's requested.
    pub(super) watching: Option<StreamWatch>,
    /// Returned to gpui's sprite atlas on the next render, the one place a window
    /// is always at hand whatever ended the watch.
    pub(super) retired_stream_frames: Vec<Arc<RenderImage>>,
    /// Tiles are sized to fill this area before they're built, so a change is
    /// caught at paint and the stage laid out again.
    pub(super) voice_stage_size: Size<Pixels>,
    /// While open, the voice channel is the conversation every message path
    /// works on. Kept across voice channels until closed.
    pub(super) voice_chat_open: bool,
    /// Voice join and leave are gateway commands, not REST calls.
    pub(super) gateway: Option<discord::GatewaySender>,
    /// From whichever of `READY` or `GET /users/@me` lands first; voice needs it to identify.
    pub(super) self_user_id: Option<Id<UserMarker>>,
    /// Expands the message toolbar's hidden actions inline, as in Discord.
    pub(super) shift_held: bool,
    /// Tracked apart because the toolbar overhangs the row above: moving onto it
    /// leaves its own row, which mustn't remove the toolbar.
    pub(super) hovered_message: Option<Id<MessageMarker>>,
    pub(super) hovered_toolbar: Option<Id<MessageMarker>>,
    /// Keeps the message lit while the pointer is on the menu.
    pub(super) message_menu: Option<Id<MessageMarker>>,
    /// Key and modifier events only travel the focus path, so without this the
    /// root's listeners never run.
    pub(super) focus_handle: FocusHandle,
    pub(super) profile_popup: Option<ProfilePopup>,
    pub(super) profile_cache: HashMap<Id<UserMarker>, discord::UserProfile>,
    /// Dropped when its reaction changes, then refetched by whatever still shows it.
    pub(super) reactors: HashMap<(Id<MessageMarker>, discord::ReactionEmoji), Reactors>,
    pub(super) guild_roles: HashMap<Id<GuildMarker>, HashMap<Id<RoleMarker>, discord::Role>>,
    /// A guild missing here allows only the default.
    pub(super) guild_upload_limits: HashMap<Id<GuildMarker>, u64>,
    pub(super) guild_emojis: HashMap<Id<GuildMarker>, Vec<discord::GuildEmoji>>,
    pub(super) emoji_picker: Option<EmojiPicker>,
    pub(super) pins_popup: Option<PinsPopup>,
    pub(super) image_viewer: Option<ImageViewer>,
    /// The signed-in user's decide whether a role mention pings them.
    pub(super) member_roles: HashMap<Id<GuildMarker>, MemberRoles>,
    /// Stops an author who left the guild, or a webhook, being re-requested with
    /// every page.
    pub(super) requested_members: HashSet<(Id<GuildMarker>, Id<UserMarker>)>,
    pub(super) member_list: Option<MemberList>,
    pub(super) member_list_cache: MemberListCache,
    /// Persisted in prefs.
    pub(super) member_list_hidden: bool,
    /// The channel/DM sidebar's dragged width, persisted in prefs.
    pub(super) sidebar_width: Pixels,
    /// The message search box in the conversation header. Not wired up yet.
    pub(super) search_input: Entity<InputState>,
    /// Cleared with the conversation, as Discord re-hides them on leaving.
    pub(super) revealed_spoilers: HashSet<SharedString>,
    pub(super) text_selection: Option<Selection>,
    pub(super) text_regions: TextRegions,
    /// In a cell because drawing only borrows the screen.
    pub(super) text_copies: RefCell<TextCopies>,
    pub(super) message_input: Entity<TextareaState>,
    pub(super) typing_sent: Option<(Id<ChannelMarker>, Instant)>,
    pub(super) typists: HashMap<Id<ChannelMarker>, Vec<Typist>>,
    pub(super) autocomplete: Option<Autocomplete>,
    /// Start of the word dismissed with escape, so typing on doesn't reopen suggestions.
    pub(super) autocomplete_dismissed: Option<usize>,
    /// (shown text, sent markup).
    pub(super) composer_mentions: Vec<(String, String)>,
    pub(super) guild_members:
        HashMap<Id<GuildMarker>, HashMap<Id<UserMarker>, discord::GuildMember>>,
    /// By guild and lowercased query, so retyping doesn't ask again.
    pub(super) member_searches: HashSet<(Id<GuildMarker>, String)>,
    pub(super) _composer_changed: Subscription,
    /// Refocuses the screen when a menu or edit box closes so typing reaches the composer.
    pub(super) _focus_lost: Subscription,
    pub(super) messages_list: ListState,
    pub(super) message_list_view: Entity<MessageListView>,
    pub(super) typing_dots: Entity<TypingDots>,
    pub(super) video: Option<VideoPlayback>,
    /// Cleared on channel switch so image memory doesn't grow without bound.
    pub(super) image_cache: Entity<RetainAllImageCache>,
    /// Released alongside `image_cache`.
    pub(super) gifs: LoadedGifs,
    // Wheel easing; see [`crate::ui::smooth_scroll`].
    pub(super) rail_scroll: SmoothScroll,
    pub(super) sidebar_scroll: SmoothScroll,
    pub(super) dm_scroll: SmoothScroll,
    pub(super) messages_scroll: SmoothScroll,
}

impl HomeScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Enter is rebound to send (see `SendMessage`) because the input's
        // `PressEnter` event also fires for newlines.
        let message_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, COMPOSER_MAX_ROWS)
                .placeholder("Send a message")
        });

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            this.focus_handle.focus(window, cx);
        });

        let composer_changed = cx.subscribe(&message_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.on_composer_changed(cx);
                this.refresh_autocomplete(cx);
            }
        });

        // Bottom-aligned, so splicing older items in at the front keeps the scroll position.
        let messages_list = ListState::new(0, ListAlignment::Bottom, px(512.));
        let weak = cx.entity().downgrade();
        messages_list.set_scroll_handler(move |event, _window, cx| {
            // `at_bottom` is read off the list each frame (`sync_at_bottom`):
            // the range here predates this event's scroll.
            let _ = weak.update(cx, |this, cx| {
                if event.visible_range.start <= PAGE_PREFETCH {
                    this.load_older_messages(cx);
                }
                if event.visible_range.end + PAGE_PREFETCH >= event.count {
                    this.load_newer_messages(cx);
                }
            });
        });

        let screen = cx.entity();
        let message_list_view = cx.new(|cx| {
            cx.observe(&screen, |_, _, cx| cx.notify()).detach();
            MessageListView {
                screen: screen.downgrade(),
            }
        });
        let typing_dots = cx.new(|_| TypingDots);

        let prefs = prefs::load();
        let mut this = Self {
            view: View::Guild,
            guilds: Vec::new(),
            rail_entries: Vec::new(),
            guild_folders: None,
            presence_status: None,
            user_presences: HashMap::new(),
            expanded_folders: HashSet::new(),
            current_user: None,
            selected_guild: None,
            loading: true,
            error: None,
            channel_groups: Vec::new(),
            channel_index: HashMap::new(),
            guild_channels: HashMap::new(),
            stale_guild_channels: HashSet::new(),
            fetching_guild_channels: HashSet::new(),
            last_guild_channel: HashMap::new(),
            selected_channel: None,
            dms: Vec::new(),
            dms_loading: false,
            dms_error: None,
            dms_loaded: false,
            dms_refreshing: false,
            collapsed_categories: HashSet::new(),
            unreads: Unreads::default(),
            unread_marker: None,
            marked_unread: false,
            pending_acks: HashMap::new(),
            ack_flush: None,
            channels_loading: false,
            channels_error: None,
            messages: Vec::new(),
            messages_channel: None,
            history_cache: HistoryCache::default(),
            messages_loading: false,
            messages_error: None,
            older_loading: false,
            reached_oldest: false,
            newer_loading: false,
            reached_newest: true,
            at_bottom: true,
            messages_generation: 0,
            pending_jump: None,
            flashed_message: None,
            flash_timer: None,
            send_error: None,
            replying_to: None,
            editing: None,
            pending_attachments: Vec::new(),
            next_attachment_id: 0,
            voice: None,
            voice_muted: false,
            voice_deafened: false,
            voice_mute_before_deafen: false,
            voice_input_device: prefs.input_device,
            voice_output_device: prefs.output_device,
            voice_states: HashMap::new(),
            voice_speaking: HashSet::new(),
            call_ringing: HashMap::new(),
            pending_voice: None,
            voice_engine: None,
            screen_share: None,
            stream_events: None,
            watching: None,
            retired_stream_frames: Vec::new(),
            voice_stage_size: Size::default(),
            voice_chat_open: false,
            gateway: None,
            self_user_id: None,
            shift_held: false,
            hovered_message: None,
            hovered_toolbar: None,
            message_menu: None,
            focus_handle: cx.focus_handle(),
            profile_popup: None,
            profile_cache: HashMap::new(),
            reactors: HashMap::new(),
            guild_roles: HashMap::new(),
            member_roles: HashMap::new(),
            requested_members: HashSet::new(),
            guild_upload_limits: HashMap::new(),
            guild_emojis: HashMap::new(),
            emoji_picker: None,
            pins_popup: None,
            image_viewer: None,
            member_list: None,
            member_list_cache: MemberListCache::default(),
            member_list_hidden: prefs.member_list_hidden,
            sidebar_width: super::view::sidebar_width(prefs.sidebar_width),
            search_input,
            revealed_spoilers: HashSet::new(),
            text_selection: None,
            text_regions: TextRegions::default(),
            text_copies: RefCell::default(),
            video: None,
            message_input,
            typing_sent: None,
            typists: HashMap::new(),
            autocomplete: None,
            autocomplete_dismissed: None,
            composer_mentions: Vec::new(),
            guild_members: HashMap::new(),
            member_searches: HashSet::new(),
            _composer_changed: composer_changed,
            _focus_lost: focus_lost,
            messages_scroll: SmoothScroll::list(messages_list.clone()),
            messages_list,
            message_list_view,
            typing_dots,
            image_cache: RetainAllImageCache::new(cx),
            gifs: LoadedGifs::default(),
            rail_scroll: SmoothScroll::div(),
            sidebar_scroll: SmoothScroll::div(),
            dm_scroll: SmoothScroll::div(),
        };
        this.load_guilds(window, cx);
        this.load_user_settings(cx);
        this.load_current_user(cx);
        this.load_dms(cx);
        this.start_gateway(window, cx);
        // Seeds the focus path so key listeners work before anything is clicked.
        this.focus_handle.focus(window, cx);
        this
    }
}
