//! Inline video playback state, and the key that names one playable attachment
//! or embed.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::slider::SliderState;
use gpui_kit::*;

use crate::platform::video::VideoPlayer;

/// Identifies one video attachment or embed. A message id alone isn't enough
/// (a message can carry several), and the message's own index stays stable as
/// the list is spliced at either end.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct MediaKey {
    pub message_id: u64,
    /// Attachments and embeds are indexed separately, and both can play video.
    pub embed: bool,
    pub index: usize,
}

impl MediaKey {
    pub fn attachment(message_id: u64, index: usize) -> Self {
        Self {
            message_id,
            embed: false,
            index,
        }
    }

    /// Both halves go in: two messages can each hold a video at index 0, and a
    /// shared id would share interaction state.
    pub fn element_id(self, role: &str) -> ElementId {
        ElementId::from(SharedString::from(format!(
            "{role}-{}-{}{}",
            self.message_id,
            self.kind(),
            self.index
        )))
    }

    /// Key into `revealed_spoilers`. `role` tells an image from a video, which
    /// are indexed separately.
    pub fn spoiler_key(self, role: &str) -> SharedString {
        SharedString::from(format!(
            "attachment-{role}-spoiler-{}-{}{}",
            self.message_id,
            self.kind(),
            self.index
        ))
    }

    pub fn group(self) -> SharedString {
        SharedString::from(format!(
            "media-{}-{}{}",
            self.message_id,
            self.kind(),
            self.index
        ))
    }

    fn kind(self) -> char {
        if self.embed { 'e' } else { 'a' }
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
