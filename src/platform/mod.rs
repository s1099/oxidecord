//! Host-facing plumbing the rest of the app builds on: the background Tokio
//! runtime, the HTTP client gpui loads remote images through, and the on-disk
//! preferences file.

pub mod http;
pub mod prefs;
pub mod runtime;
