//! Desktop notifications for incoming messages.
//!
//! Windows shows them as toasts (`windows.rs`); elsewhere they're dropped
//! (`unsupported.rs`). Toasts are posted from a thread of their own, since
//! WinRT wants an apartment gpui's foreground thread doesn't set up, and a
//! click comes back as the toast's [`ToastId`].

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::path::PathBuf;
use std::sync::mpsc;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use sha2::{Digest as _, Sha256};

use super::{http, runtime};

/// Raw ids, which keep this module free of Discord types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastId {
    pub guild: Option<u64>,
    pub channel: u64,
    pub message: u64,
}

pub struct Toast {
    pub id: ToastId,
    pub title: String,
    pub body: String,
    pub avatar_url: Option<String>,
}

enum Request {
    Show {
        toast: Toast,
        image: Option<PathBuf>,
    },
    /// Takes a channel's toasts down once it's been read.
    Clear { channel: u64 },
}

pub struct Notifier {
    requests: mpsc::Sender<Request>,
}

/// Starts the notification thread. The receiver yields each toast clicked.
pub fn start() -> (Notifier, UnboundedReceiver<ToastId>) {
    let (requests, rx) = mpsc::channel();
    let (clicks, clicked) = unbounded();
    backend::spawn(rx, clicks);
    (Notifier { requests }, clicked)
}

impl Notifier {
    /// The avatar is fetched first; a toast can only show a local image.
    pub fn show(&self, toast: Toast) {
        let requests = self.requests.clone();
        runtime::handle().spawn(async move {
            let image = match &toast.avatar_url {
                Some(url) => cached_avatar(url).await,
                None => None,
            };
            _ = requests.send(Request::Show { toast, image });
        });
    }

    pub fn clear(&self, channel: u64) {
        _ = self.requests.send(Request::Clear { channel });
    }
}

/// Re-encoded as PNG, since toasts don't take the CDN's WebP. Kept in the temp
/// directory by URL, which changes with the avatar, so a regular sender's
/// isn't fetched again.
async fn cached_avatar(url: &str) -> Option<PathBuf> {
    let hash = Sha256::digest(url.as_bytes());
    let name: String = hash[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    let dir = std::env::temp_dir().join("oxidecord-avatars");
    let path = dir.join(format!("{name}.png"));
    if path.exists() {
        return Some(path);
    }

    let bytes = http::client()
        .get(url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .ok()?;
    Some(path)
}
