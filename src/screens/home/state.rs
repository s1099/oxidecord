//! [`HomeScreen`]'s state and the small types it holds.
//!
//! Fields are `pub(super)` so the [`data`](super::data) and [`view`](super::view)
//! modules — which extend the screen with inherent methods — can reach them,
//! while nothing outside the home screen can.

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
use super::selection::{Selection, TextCopies, TextRegions};
use super::unread::Unreads;
use super::voice::{PendingVoice, ScreenShare, StreamWatch, VoiceCall};
use crate::voice::VoiceEngine;
use crate::voice::stream::StreamEvent;

const COMPOSER_MAX_ROWS: usize = 12;

/// How many messages from either end of what's loaded the view has to reach
/// before the next page that way is fetched. A page takes a round trip to
/// arrive, and a scroll that hits the end before then has nowhere left to go.
const PAGE_PREFETCH: usize = 10;

/// Which list occupies the sidebar: a guild's channels, or the DM list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Guild,
    DirectMessages,
}

/// The roles each member of one guild holds.
pub(super) type MemberRoles = HashMap<Id<UserMarker>, Vec<Id<RoleMarker>>>;

/// The message a pending reply is aimed at, shown as a banner above the
/// message bar. Kept minimal: enough to label the banner and reference the
/// target when the reply is sent.
#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub message_id: Id<MessageMarker>,
    pub author_name: String,
}

/// Someone else typing in a conversation, shown above the message bar until
/// `until` passes or their message arrives.
pub(super) struct Typist {
    pub user_id: Id<UserMarker>,
    /// Their guild name, from the dispatch. A DM's typists are named from the
    /// conversation's messages instead.
    pub name: Option<String>,
    pub until: Instant,
}

/// The message being edited in place, and the input standing in for its text.
///
/// The input is made per edit rather than kept on the screen like the
/// composer's: it only exists while a message is open for editing, and a fresh
/// one comes with no leftover undo history from the last.
pub(super) struct EditingMessage {
    pub message_id: Id<MessageMarker>,
    pub input: Entity<TextareaState>,
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

/// The open pinned-messages popout for the open conversation.
pub(super) struct PinsPopup {
    pub channel_id: Id<ChannelMarker>,
    /// Window coordinates of the click on the header's pin button, which the
    /// popout hangs below.
    pub position: Point<Pixels>,
    /// Most recently pinned first. Empty while loading.
    pub messages: Vec<discord::Message>,
    /// Lays the pins out the way the message list lays out messages: each on
    /// its own, at the list's width. Nested in the popout's own flex layout,
    /// text in a pin is measured as if it had no width, and the pin keeps the
    /// height of all those wrapped lines as empty space.
    pub list: ListState,
    pub loading: bool,
    pub error: Option<String>,
}

/// The image open full size over the whole screen, as Discord's lightbox.
pub(super) struct ImageViewer {
    /// What's already on screen in the message, drawn stretched while the
    /// larger copy loads so the viewer never opens empty.
    pub preview_url: SharedString,
    /// The copy scaled to the window. Dropped from the image cache when the
    /// viewer closes, being far bigger than anything in the message list.
    pub full_url: SharedString,
    /// The original, for "Open in Browser".
    pub source_url: String,
    /// A copy at the original's resolution, fetched the first time the image
    /// is zoomed past what the window-sized one holds. Dropped on close too.
    pub zoomed_url: Option<SharedString>,
    /// Intrinsic dimensions, when Discord reported them.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether the larger copies have to be asked for as animated WebP to stay
    /// moving; see [`discord::animated_webp`].
    pub animated_webp: bool,
    /// How far the image is zoomed past fitting the window: 1 fits it.
    pub zoom: f32,
    /// How far the image's centre is panned from the window's.
    pub offset: Point<Pixels>,
    /// The pan under way: where the pointer went down, and the offset then.
    pub drag: Option<(Point<Pixels>, Point<Pixels>)>,
}

/// Where an emoji chosen from the picker goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmojiTarget {
    Composer,
    /// A reaction on this message, opened from its hover toolbar.
    Reaction(Id<MessageMarker>),
}

/// Who reacted to a message with one emoji, as far as it's been fetched.
#[derive(Default)]
pub(super) struct Reactors {
    pub users: Vec<discord::Reactor>,
    pub loading: bool,
    /// The last page came back short, so `users` is everyone.
    pub complete: bool,
    pub error: Option<String>,
    /// The fetch in flight. Held so forgetting the entry cancels it, and a
    /// page of a list that has since changed can't land in its replacement.
    pub _task: Option<Task<()>>,
}

pub(super) struct EmojiPicker {
    /// Window coordinates of the click that opened it, which the picker sits
    /// beside.
    pub position: Point<Pixels>,
    pub target: EmojiTarget,
    pub search: Entity<InputState>,
    /// The recently used as they were when the picker opened, so choosing one
    /// doesn't reshuffle the list under the pointer.
    pub recent: Vec<super::emoji::PickerEmoji>,
    /// The list as last laid out. Rebuilt when the search changes rather than
    /// every frame — the screen repaints for every video frame, and this is a
    /// couple of thousand emoji.
    pub layout: Rc<PickerLayout>,
    /// Sections folded down to their header, by the chevron beside it.
    pub collapsed: HashSet<SectionKind>,
    /// The emoji under the pointer, named in the footer.
    pub hovered: Option<super::emoji::PickerEmoji>,
    pub scroll: SmoothScroll,
    /// The category rail down the picker's left side.
    pub rail_scroll: ScrollHandle,
    pub _search_changed: Subscription,
}

/// The suggestions offered above the composer for the `@`, `#` or `:` word
/// being typed at the cursor.
pub(super) struct Autocomplete {
    pub trigger: Trigger,
    /// The word's byte range in the composer's text, trigger included: what
    /// accepting a suggestion replaces.
    pub range: Range<usize>,
    pub query: String,
    pub items: Vec<Suggestion>,
    pub selected: usize,
}

/// The open guild channel's member list, as far as Discord has synced it.
pub(super) struct MemberList {
    pub guild_id: Id<GuildMarker>,
    pub channel_id: Id<ChannelMarker>,
    /// The list Discord is sending, from its first sync. Kept across a switch
    /// to another channel in the guild, since channels the same people can
    /// see share a list and Discord carries on with it.
    pub id: Option<String>,
    pub groups: Vec<discord::MemberListGroup>,
    /// Every row of the list, headers included, at Discord's indices. `None`
    /// for a row in a range that isn't synced, drawn as a placeholder.
    pub rows: Vec<Option<discord::MemberListItem>>,
    /// The index ranges subscribed to, as last sent.
    pub ranges: Vec<[usize; 2]>,
    pub scroll: UniformListScrollHandle,
}

/// One video attachment or embed in the message list: which playback is
/// which, and a source of element ids unique across every message on screen.
///
/// A message id alone isn't enough — one message can carry several of either —
/// and the index is the message's own, so it stays stable as the list is
/// spliced at either end.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct MediaKey {
    pub message_id: u64,
    pub index: usize,
}

impl MediaKey {
    /// A gpui id for one of this attachment's elements. Both halves of the key
    /// go in: two messages can each hold a video at index 0, and sharing an id
    /// between them would share their interaction state too.
    pub fn element_id(self, role: &str) -> ElementId {
        ElementId::from(SharedString::from(format!(
            "{role}-{}-{}",
            self.message_id, self.index
        )))
    }

    /// The key a spoiler cover over this attachment is revealed by, in
    /// `revealed_spoilers`. `role` tells an image from a video, since each is
    /// indexed separately.
    pub fn spoiler_key(self, role: &str) -> SharedString {
        SharedString::from(format!(
            "attachment-{role}-spoiler-{}-{}",
            self.message_id, self.index
        ))
    }

    /// A hover group shared by the whole item.
    pub fn group(self) -> SharedString {
        SharedString::from(format!("media-{}-{}", self.message_id, self.index))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum PlaybackState {
    /// Until the first frame lands, which covers both the download and the
    /// decoder opening the file.
    Loading,
    Playing,
    /// The clip ran out, so the card offers to play it again rather than
    /// sitting on the last frame.
    Ended,
    /// Why playback stopped badly. Shown over the poster.
    Failed(String),
}

/// Only one plays at a time: decoding a second clip nobody is watching is
/// exactly the cost this feature exists to avoid, so starting one stops the
/// other.
pub(super) struct VideoPlayback {
    pub key: MediaKey,
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
    pub state: PlaybackState,
    /// The progress bar, which doubles as the scrubber. Its value is a
    /// fraction of the run time rather than a time, so it needs no resetting
    /// when the duration arrives after the first frames.
    pub scrubber: Entity<SliderState>,
}

/// The message list, drawn as a view of its own so the window can replay it
/// from the last frame. It redraws whenever the screen is notified, as it
/// would inline, but not when something animating in a view of its own (the
/// typing dots) redraws around it — which would otherwise rebuild every
/// visible message each frame.
pub(super) struct MessageListView {
    pub(super) screen: WeakEntity<HomeScreen>,
}

/// The typing indicator's pulsing dots. A view of their own, since an
/// animation redraws the view it's drawn in on every frame.
pub(super) struct TypingDots;

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
    /// The status the user picked, from settings-proto. `None` until that
    /// fetch resolves; the account panel shows online meanwhile, and nothing
    /// is sent to the gateway until it's known.
    pub(super) presence_status: Option<discord::PresenceStatus>,
    /// Other users' statuses, as far as the gateway has said. Anyone missing
    /// is shown offline: Discord only sends the presences of friends and
    /// people who share a guild, and never sends offline ones up front.
    pub(super) user_presences: HashMap<Id<UserMarker>, discord::PresenceStatus>,
    /// Folders the user has opened, by folder id. Folders start collapsed.
    pub(super) expanded_folders: HashSet<i64>,
    pub(super) selected_guild: Option<Id<GuildMarker>>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    /// Set through `set_channel_groups`, which keeps `channel_index` in step.
    pub(super) channel_groups: Vec<ChannelGroup>,
    /// Where each of `channel_groups`' channels sits in it, as (group,
    /// channel) indices: channels are looked up by id for every message drawn.
    pub(super) channel_index: HashMap<Id<ChannelMarker>, (usize, usize)>,
    /// Every guild's channels as far as they're known: from the gateway,
    /// which sends them all with `READY`, or from a fetch where it hasn't
    /// yet. Opening a guild draws its list from here at once.
    pub(super) guild_channels: HashMap<Id<GuildMarker>, Vec<discord::Channel>>,
    /// Guilds whose list in `guild_channels` has changed since it was sent.
    /// Opening one still draws what's there, and refetches it underneath.
    pub(super) stale_guild_channels: HashSet<Id<GuildMarker>>,
    /// Guilds whose channels are being fetched, so a refetch isn't sent twice.
    pub(super) fetching_guild_channels: HashSet<Id<GuildMarker>>,
    /// The channel last open in each guild, which going back to the guild
    /// reopens, as Discord's does.
    pub(super) last_guild_channel: HashMap<Id<GuildMarker>, Id<ChannelMarker>>,
    pub(super) selected_channel: Option<Id<ChannelMarker>>,
    /// The user's DM conversations, loaded lazily the first time the DM view is
    /// opened. A selected DM reuses `selected_channel` and the message plumbing.
    pub(super) dms: Vec<DirectMessage>,
    pub(super) dms_loading: bool,
    pub(super) dms_error: Option<String>,
    /// Set once the DM list has been fetched, so reopening the view doesn't
    /// refetch it every time.
    pub(super) dms_loaded: bool,
    /// A background refetch of the loaded DM list is in flight, after a
    /// message arrived in a conversation the list didn't have.
    pub(super) dms_refreshing: bool,
    pub(super) collapsed_categories: HashSet<Id<ChannelMarker>>,
    pub(super) unreads: Unreads,
    /// The last message that had been read when the open conversation was
    /// opened, which the "new" divider sits under. `None` when nothing was
    /// unread.
    pub(super) unread_marker: Option<Id<MessageMarker>>,
    /// The user marked a message in the open conversation unread, so it
    /// isn't acked again until they leave it.
    pub(super) marked_unread: bool,
    /// Acks waiting to be sent, by channel, and the timer that sends them.
    pub(super) pending_acks: HashMap<Id<ChannelMarker>, Id<MessageMarker>>,
    pub(super) ack_flush: Option<Task<()>>,
    pub(super) channels_loading: bool,
    pub(super) channels_error: Option<String>,
    pub(super) messages: Vec<discord::Message>,
    /// The conversation `messages` belong to, once they've arrived. What's
    /// cached when it's left.
    pub(super) messages_channel: Option<Id<ChannelMarker>>,
    /// Recently opened conversations, which reopen from here before their
    /// newest page is fetched.
    pub(super) history_cache: HistoryCache,
    pub(super) messages_loading: bool,
    pub(super) messages_error: Option<String>,
    pub(super) older_loading: bool,
    pub(super) reached_oldest: bool,
    /// A newer page is being fetched, scrolled down through history that
    /// was opened further back than the present.
    pub(super) newer_loading: bool,
    /// The loaded messages run up to the channel's newest. Only not so when
    /// the conversation opened at an unread run longer than a page, as
    /// Discord's does, until paging forward or jumping to the present.
    pub(super) reached_newest: bool,
    /// Whether the newest message is currently in view. Used to decide if a
    /// live message should snap the list to the bottom (following the
    /// conversation) or be appended silently (the user is reading history).
    pub(super) at_bottom: bool,
    pub(super) send_error: Option<String>,
    /// Set while composing a reply; drives the "Replying to …" banner and is
    /// cleared when the reply is sent, dismissed, or the channel changes.
    pub(super) replying_to: Option<ReplyTarget>,
    /// The message open for editing, if any. At most one, like Discord; it's
    /// closed when saved, cancelled, or the channel changes.
    pub(super) editing: Option<EditingMessage>,
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
    /// Who each ongoing DM call is ringing, by channel. Only calls the gateway
    /// has reported are here; one with nobody being rung has an empty list.
    pub(super) call_ringing: HashMap<Id<ChannelMarker>, Vec<Id<UserMarker>>>,
    /// The connection parameters for a join that's still in flight.
    pub(super) pending_voice: Option<PendingVoice>,
    /// Runs the call. Started with the gateway, since a call needs both.
    pub(super) voice_engine: Option<VoiceEngine>,
    /// The user's screen share into the call, from the moment it's announced
    /// to the gateway.
    pub(super) screen_share: Option<ScreenShare>,
    /// Where stream connections report back. One channel for every stream,
    /// each event tagged with the stream it's about.
    pub(super) stream_events: Option<futures::channel::mpsc::UnboundedSender<StreamEvent>>,
    /// Someone else's stream the user is watching, from the moment it's
    /// asked for.
    pub(super) watching: Option<StreamWatch>,
    /// Pictures from a watch that has ended, waiting to be handed back to
    /// gpui's sprite atlas on the next render — the one place a window is
    /// always to hand, whatever ended the watch.
    pub(super) retired_stream_frames: Vec<Arc<RenderImage>>,
    /// The call stage's tile area as last laid out. Tiles are sized to fill
    /// it, which takes knowing how big it is before they're built, so a
    /// change is caught at paint and lays the stage out again.
    pub(super) voice_stage_size: Size<Pixels>,
    /// The text chat beside a voice channel's stage is open. While it is, the
    /// open voice channel is the conversation every message path works on.
    /// Kept across voice channels, as Discord's is, until it's closed.
    pub(super) voice_chat_open: bool,
    /// Sends commands up the gateway — joining and leaving voice channels is
    /// done with a gateway command, not a REST call.
    pub(super) gateway: Option<discord::GatewaySender>,
    /// The signed-in user's id, from whichever of `READY` or `GET /users/@me`
    /// lands first. The voice connection can't identify itself without it.
    pub(super) self_user_id: Option<Id<UserMarker>>,
    /// Whether shift is currently held, tracked so the message toolbar can
    /// expand its hidden actions inline the way Discord's does.
    pub(super) shift_held: bool,
    /// The message row under the pointer and the message whose toolbar is,
    /// tracked apart because the toolbar overhangs the row above: moving onto
    /// it leaves its own row, which mustn't take the toolbar away.
    pub(super) hovered_message: Option<Id<MessageMarker>>,
    pub(super) hovered_toolbar: Option<Id<MessageMarker>>,
    /// The message whose toolbar "More" menu is open, which keeps the message
    /// lit while the pointer is off on the menu.
    pub(super) message_menu: Option<Id<MessageMarker>>,
    /// Focus for the screen as a whole, held by an empty element inside it and
    /// focused on startup: key and modifier events only travel the focus path,
    /// so without it the root's listeners never run.
    pub(super) focus_handle: FocusHandle,
    pub(super) profile_popup: Option<ProfilePopup>,
    /// Profiles already fetched this session, so reopening a card is instant
    /// and repeated clicks don't refetch.
    pub(super) profile_cache: HashMap<Id<UserMarker>, discord::UserProfile>,
    /// Who reacted with what, fetched as reaction pills are hovered or the
    /// reactions dialog asks. Each entry is dropped when its reaction changes
    /// and refetched by whatever is still showing it.
    pub(super) reactors: HashMap<(Id<MessageMarker>, discord::ReactionEmoji), Reactors>,
    /// Every guild's roles, from the gateway, so a role mention can be named
    /// and coloured.
    pub(super) guild_roles: HashMap<Id<GuildMarker>, HashMap<Id<RoleMarker>, discord::Role>>,
    /// The largest file each boosted guild accepts, from the gateway. A guild
    /// missing here allows only the default.
    pub(super) guild_upload_limits: HashMap<Id<GuildMarker>, u64>,
    /// Every guild's custom emoji, from the gateway, for the emoji picker.
    pub(super) guild_emojis: HashMap<Id<GuildMarker>, Vec<discord::GuildEmoji>>,
    pub(super) emoji_picker: Option<EmojiPicker>,
    pub(super) pins_popup: Option<PinsPopup>,
    pub(super) image_viewer: Option<ImageViewer>,
    /// The roles each member holds, by guild, as far as the gateway has told
    /// us. The signed-in user's decide whether a role mention pings them;
    /// everyone's colour their names.
    pub(super) member_roles: HashMap<Id<GuildMarker>, MemberRoles>,
    /// Members already asked of the gateway this session, so an author who
    /// isn't in the guild any more — or a webhook, which never was — isn't
    /// asked for again with every page.
    pub(super) requested_members: HashSet<(Id<GuildMarker>, Id<UserMarker>)>,
    pub(super) member_list: Option<MemberList>,
    /// The user closed the member list. Remembered between runs.
    pub(super) member_list_hidden: bool,
    /// The message search box in the conversation header. Not wired up yet.
    pub(super) search_input: Entity<InputState>,
    /// Spoilers clicked open in the open conversation, by the key the view
    /// gives each one. Cleared with the conversation, as Discord hides them
    /// again once you leave.
    pub(super) revealed_spoilers: HashSet<SharedString>,
    /// The mouse selection over the message list's text. Cleared with the
    /// conversation.
    pub(super) text_selection: Option<Selection>,
    /// Where the list's selectable texts were last laid out, for a press or a
    /// drag to land on.
    pub(super) text_regions: TextRegions,
    /// What each selectable text drawn in the conversation copies as. In a
    /// cell since drawing, which only borrows the screen, fills it.
    pub(super) text_copies: RefCell<TextCopies>,
    pub(super) message_input: Entity<TextareaState>,
    /// The conversation the typing indicator was last sent to, and when.
    pub(super) typing_sent: Option<(Id<ChannelMarker>, Instant)>,
    pub(super) typists: HashMap<Id<ChannelMarker>, Vec<Typist>>,
    pub(super) autocomplete: Option<Autocomplete>,
    /// Where the word the user dismissed suggestions for with escape starts,
    /// so typing on in it doesn't bring them straight back.
    pub(super) autocomplete_dismissed: Option<usize>,
    /// Mentions put in the composer by the autocomplete, as the text shown
    /// there and the markup it's sent as.
    pub(super) composer_mentions: Vec<(String, String)>,
    /// Guild members the gateway has named, by guild: from mention searches
    /// and from the requests for messages' authors.
    pub(super) guild_members:
        HashMap<Id<GuildMarker>, HashMap<Id<UserMarker>, discord::GuildMember>>,
    /// Member searches already sent this session, by guild and lowercased
    /// query, so retyping a name doesn't ask again.
    pub(super) member_searches: HashSet<(Id<GuildMarker>, String)>,
    pub(super) _composer_changed: Subscription,
    pub(super) messages_list: ListState,
    pub(super) message_list_view: Entity<MessageListView>,
    pub(super) typing_dots: Entity<TypingDots>,
    pub(super) video: Option<VideoPlayback>,
    /// Owns the decoded bitmaps for the currently displayed messages' images.
    /// Cleared on channel switch so image memory doesn't grow without bound.
    pub(super) image_cache: Entity<RetainAllImageCache>,
    /// The GIF clips decoded for the currently displayed messages, released
    /// alongside `image_cache`.
    pub(super) gifs: LoadedGifs,
    /// Wheel easing for each scrollable pane; see [`crate::ui::smooth_scroll`].
    pub(super) rail_scroll: SmoothScroll,
    pub(super) sidebar_scroll: SmoothScroll,
    pub(super) dm_scroll: SmoothScroll,
    pub(super) messages_scroll: SmoothScroll,
}

impl HomeScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Multi-line so shift-enter can start a new line. Enter itself is
        // rebound to send (see `SendMessage`); the input's `PressEnter` event
        // can't be used for that, since it fires for the newlines too.
        let message_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, COMPOSER_MAX_ROWS)
                .placeholder("Send a message")
        });

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        let composer_changed = cx.subscribe(&message_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.on_composer_changed(cx);
                this.refresh_autocomplete(cx);
            }
        });

        // Bottom-aligned like a chat log; items are measured lazily, and
        // splicing older items in at the front keeps the scroll position.
        let messages_list = ListState::new(0, ListAlignment::Bottom, px(512.));
        let weak = cx.entity().downgrade();
        messages_list.set_scroll_handler(move |event, _window, cx| {
            // Whether the newest message is in view is read off the list each
            // frame instead (`sync_at_bottom`): the range here is from before
            // this event moved it.
            let _ = weak.update(cx, |this, cx| {
                // Far enough ahead that a page usually lands before the scroll
                // reaches the top, so a continuous scroll up never stalls
                // against it.
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
            send_error: None,
            replying_to: None,
            editing: None,
            pending_attachments: Vec::new(),
            next_attachment_id: 0,
            voice: None,
            voice_muted: false,
            voice_deafened: false,
            voice_mute_before_deafen: false,
            voice_input_device: prefs::load().input_device,
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
            member_list_hidden: prefs::load().member_list_hidden,
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
        // Fetched up front, so the DM list is there the first time it's
        // opened rather than loading then.
        this.load_dms(cx);
        this.start_gateway(window, cx);
        // Seeds the focus path so the root element's key listeners are live
        // from the first frame, before anything has been clicked.
        this.focus_handle.focus(window, cx);
        this
    }
}
