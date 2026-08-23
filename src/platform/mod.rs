//! Host-facing plumbing the rest of the app builds on: the background Tokio
//! runtime, the HTTP client gpui loads remote images through, the on-disk
//! preferences file, and the self-updater.

pub mod http;
pub mod prefs;
pub mod runtime;
pub mod updater;
