//! Rendering for [`HomeScreen`](super::HomeScreen), one module per region of
//! the screen.

mod composer;
mod content;
mod dm_sidebar;
mod message;
mod rail;
mod sidebar;
mod text;

/// Horizontal padding, in pixels, on either side of the message list.
const MESSAGE_PADDING_X: f32 = 16.;

/// Vertical gap, in pixels, between two consecutive author groups. Split evenly
/// between the bottom of the group above and the top of the group below so each
/// message's hover highlight extends symmetrically into the gap.
const GROUP_GAP: f32 = 16.;
