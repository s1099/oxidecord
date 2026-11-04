//! Host-facing plumbing: audio-device helpers, the Tokio runtime, the HTTP
//! client, prefs, the updater, OS screen capture and video decoders, and on
//! Windows the single-instance guard and tray icon.

pub mod audio;
pub mod capture;
pub mod clipboard;
pub mod h264;
pub mod http;
#[cfg(windows)]
pub mod instance;
pub mod prefs;
pub mod runtime;
#[cfg(windows)]
pub mod tray;
pub mod updater;
pub mod video;
