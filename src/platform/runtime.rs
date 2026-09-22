//! The app's background Tokio runtime.
//!
//! gpui has its own executor, but the crates the app talks to the network with
//! (twilight, reqwest) need to run inside a Tokio context, so a dedicated
//! runtime is kept alive on its own thread and shared by every caller.

use std::sync::OnceLock;

use tokio::runtime::Handle;
use tokio::task::JoinError;

static RUNTIME: OnceLock<Handle> = OnceLock::new();

pub fn handle() -> &'static Handle {
    RUNTIME.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            tx.send(rt.handle().clone())
                .expect("failed to send tokio runtime handle");
            rt.block_on(std::future::pending::<()>());
        });
        rx.recv().expect("failed to receive tokio runtime handle")
    })
}

/// Runs `future` on the runtime and resolves with its output. The returned
/// future can be awaited from any executor, gpui's included; it only fails if
/// the task panicked.
pub async fn run<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
) -> Result<T, JoinError> {
    handle().spawn(future).await
}
