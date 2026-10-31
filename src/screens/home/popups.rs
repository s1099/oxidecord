//! The state behind the screen's popouts and overlays: profile cards, pins,
//! the image viewer, the emoji picker, and who reacted.

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use crate::discord;
use crate::ui::smooth_scroll::SmoothScroll;

use super::emoji::{PickerEmoji, PickerLayout, SectionKind};

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

pub(super) struct EmojiPicker {
    /// Window coordinates of the opening click.
    pub position: Point<Pixels>,
    pub target: EmojiTarget,
    pub search: Entity<InputState>,
    /// Snapshot from opening, so choosing one doesn't reshuffle under the pointer.
    pub recent: Vec<PickerEmoji>,
    /// Rebuilt on search change, not per frame: the screen repaints for every
    /// video frame and this is a couple of thousand emoji.
    pub layout: Rc<PickerLayout>,
    pub collapsed: HashSet<SectionKind>,
    pub hovered: Option<PickerEmoji>,
    pub scroll: SmoothScroll,
    pub rail_scroll: ScrollHandle,
    pub _search_changed: Subscription,
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
