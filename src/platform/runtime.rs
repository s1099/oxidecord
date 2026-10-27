//! The app's background Tokio runtime.
//!
//! twilight and reqwest need a Tokio context that gpui's executor doesn't
//! provide, so one runtime lives on its own thread.

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

/// Runs `future` on the runtime; awaitable from any executor. Errs only if the task panicked.
pub async fn run<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
) -> Result<T, JoinError> {
    handle().spawn(future).await
}
