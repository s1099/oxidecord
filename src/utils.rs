use tokio::runtime::Handle;
use std::sync::OnceLock;

static RUNTIME_HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn init_runtime() {
    RUNTIME_HANDLE.get_or_init(|| {
        Handle::try_current().unwrap_or_else(|_| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create Tokio runtime");
                let handle = rt.handle().clone();
                tx.send(handle.clone()).unwrap();
                rt.block_on(async {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                    }
                });
            });
            rx.recv().unwrap()
        })
    });
}

pub fn get_runtime_handle() -> &'static Handle {
    RUNTIME_HANDLE.get().expect("Tokio runtime not initialized. Call init_runtime() first.")
}

