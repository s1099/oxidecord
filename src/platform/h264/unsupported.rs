//! Fallback for platforms without a decoder backend yet. VideoToolbox on
//! macOS and GStreamer on Linux would fit the same shape.

use std::sync::mpsc::Receiver;

use super::{Control, DecoderEvent};

pub(super) fn run(
    _control: &Control,
    _input: &Receiver<Vec<u8>>,
    _emit: &mut dyn FnMut(DecoderEvent) -> bool,
) -> Result<(), String> {
    Err("Watching streams isn't supported on this platform yet".into())
}
