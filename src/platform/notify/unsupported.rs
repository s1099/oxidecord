//! Fallback for platforms without a notification backend: toasts are dropped.

use std::sync::mpsc::Receiver;

use futures::channel::mpsc::UnboundedSender;

use super::{Request, ToastId};

pub(super) fn spawn(requests: Receiver<Request>, _clicks: UnboundedSender<ToastId>) {
    drop(requests);
}
