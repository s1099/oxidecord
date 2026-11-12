//! Host-facing plumbing: audio-device helpers, the Tokio runtime, the HTTP
//! client, prefs, the updater, desktop notifications, OS screen capture and
//! video decoders, a Lottie rasterizer, embedded webview setup, and on Windows the single-instance guard, tray icon and
//! taskbar call buttons.

pub mod audio;
pub mod capture;
pub mod clipboard;
pub mod h264;
pub mod http;
#[cfg(windows)]
pub mod instance;
pub mod lottie;
pub mod notify;
pub mod prefs;
pub mod runtime;
#[cfg(windows)]
pub mod taskbar;
#[cfg(windows)]
pub mod tray;
pub mod updater;
pub mod video;
pub mod webview;
