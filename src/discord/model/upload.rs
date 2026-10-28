//! File size limits. The API doesn't report them, so these are the numbers from
//! Discord's File Attachments FAQ (as of August 2026: free 20 MB, Nitro 1 GB)
//! and Server Boosting FAQ. A file may be as large as the higher of the
//! sender's Nitro limit and the guild's boost limit.
//!
//! Also the progress of a send's upload, shared between the request and the view.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::Notify;
use twilight_model::user::PremiumType;

const MIB: u64 = 1024 * 1024;

pub const DEFAULT_UPLOAD_LIMIT: u64 = 20 * MIB;

/// Cap on one message's attachments combined, since they're held in memory until
/// sent, and on any one file: the most cloud storage takes in a single upload.
pub const MAX_UPLOAD_TOTAL: u64 = 500 * MIB;

pub(super) fn premium_upload_limit(tier: PremiumType) -> u64 {
    match tier {
        PremiumType::Nitro => 1024 * MIB,
        PremiumType::NitroClassic | PremiumType::NitroBasic => 50 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}

/// Level 1 doesn't raise the limit.
pub(in crate::discord) fn boost_upload_limit(premium_tier: u8) -> u64 {
    match premium_tier {
        2 => 50 * MIB,
        3.. => 100 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}

/// A pending message's files, drawn as one card with a progress bar until the
/// upload is done.
#[derive(Clone)]
pub struct Upload {
    pub filenames: Vec<String>,
    pub progress: Arc<UploadProgress>,
}

impl PartialEq for Upload {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.progress, &other.progress)
    }
}

impl Eq for Upload {}

/// Atomics, since the upload runs on the background runtime while the view reads it.
pub struct UploadProgress {
    total: u64,
    sent: AtomicU64,
    cancelled: AtomicBool,
    cancel: Notify,
}

impl UploadProgress {
    pub fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total,
            sent: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            cancel: Notify::new(),
        })
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed).min(self.total)
    }

    pub(in crate::discord) fn advance(&self, bytes: u64) {
        self.sent.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Resolves once [`cancel`](Self::cancel) is called, even if it already was.
    pub(in crate::discord) async fn cancelled(&self) {
        loop {
            // Registered before the check, so a cancel in between still wakes it.
            let notified = self.cancel.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}
