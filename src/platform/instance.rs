//! Keeps one copy of the app running per Windows session. A second launch hands
//! off to the running one, which brings its window back, instead of signing in twice.

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::w;

/// The named event both claims the instance and carries "show yourself" requests.
/// `None` when it couldn't be created, so the app runs unguarded rather than not at all.
pub struct Instance(Option<HANDLE>);

/// Claims this session's instance, or signals the running one and returns `None`.
pub fn claim() -> Option<Instance> {
    // Auto-reset, so each launch wakes the listener once.
    let event = match unsafe { CreateEventW(None, false, false, w!("Local\\Oxidecord.Activate")) } {
        Ok(event) => event,
        Err(err) => {
            eprintln!("single-instance event failed: {err}");
            return Some(Instance(None));
        }
    };
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return Some(Instance(Some(event)));
    }
    unsafe {
        // The new launch holds foreground rights; lend them so the running
        // instance's window may come to the front.
        _ = AllowSetForegroundWindow(ASFW_ANY);
        _ = SetEvent(event);
    }
    None
}

impl Instance {
    /// Yields once per later launch. The event stays open for the life of the
    /// process, as that's what keeps later launches out.
    pub fn activations(self) -> UnboundedReceiver<()> {
        let (tx, rx) = unbounded();
        let Some(event) = self.0 else {
            return rx;
        };
        // HANDLE isn't Send.
        let event = event.0 as usize;
        std::thread::Builder::new()
            .name("instance-activate".into())
            .spawn(move || {
                let event = HANDLE(event as _);
                while unsafe { WaitForSingleObject(event, INFINITE) } == WAIT_OBJECT_0 {
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn the instance listener");
        rx
    }
}
