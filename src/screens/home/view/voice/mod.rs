//! Call chrome: the sidebar's connected panel, the stage of participant tiles,
//! the bubbles above a DM's conversation, and the control bar under either.

mod controls;
mod dm_call;
mod panel;
mod stage;
mod tiles;

/// The camera button is drawn because the call has a place for it, and disabled because nothing
/// sends camera video yet.
const CAMERA_UNAVAILABLE: &str = "Camera isn't supported yet";
