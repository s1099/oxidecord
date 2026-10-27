//! Host-facing plumbing: audio-device helpers, the Tokio runtime, the HTTP
//! client, prefs, the updater, and OS screen capture and video decoders.

pub mod audio;
pub mod capture;
pub mod h264;
pub mod http;
pub mod prefs;
pub mod runtime;
pub mod updater;
pub mod video;
