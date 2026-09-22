//! Host-facing plumbing the rest of the app builds on: shared audio-device
//! helpers, the background Tokio runtime, the HTTP client gpui loads remote images through, the on-disk
//! preferences file, the self-updater, and the operating system's video
//! decoder.

pub mod audio;
pub mod http;
pub mod prefs;
pub mod runtime;
pub mod updater;
pub mod video;
