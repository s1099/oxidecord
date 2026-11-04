//! Closing the main window hides it behind a notification-area icon instead of
//! quitting, as Discord's client does, so the session and any call stay up. The
//! icon, its menu and a relaunch bring the window back; only the menu quits.

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui_kit::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSMICON, SW_HIDE, SW_SHOW, ShowWindow,
};

enum Request {
    Show,
    Quit,
}

/// Puts the icon up and makes closing `window` hide it. `activations` come from
/// later launches of the app and show the window too.
pub fn install(window: AnyWindowHandle, activations: UnboundedReceiver<()>, cx: &mut App) {
    let (tx, requests) = unbounded();

    match build(tx) {
        Ok(tray) => {
            // Without the icon there'd be no way back short of relaunching, so
            // closing only hides once it's up.
            _ = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, |window, _| {
                    set_visible(window, false);
                    false
                });
            });
            // Taking the icon down on quit; left up, it lingers until hovered.
            let mut tray = Some(tray);
            cx.on_app_quit(move |_| {
                tray.take();
                async {}
            })
            .detach();
        }
        Err(err) => eprintln!("tray icon failed: {err}"),
    }

    let mut requests = futures::stream::select(requests, activations.map(|()| Request::Show));
    cx.spawn(async move |cx| {
        while let Some(request) = requests.next().await {
            match request {
                Request::Show => {
                    _ = window.update(cx, |_, window, _| {
                        set_visible(window, true);
                        window.activate_window();
                    });
                }
                Request::Quit => cx.update(|cx| cx.quit()),
            }
        }
    })
    .detach();
}

fn build(tx: futures::channel::mpsc::UnboundedSender<Request>) -> anyhow::Result<TrayIcon> {
    let open = MenuItem::new("Open Oxidecord", true, None);
    let quit = MenuItem::new("Quit Oxidecord", true, None);
    let menu = Menu::with_items(&[&open, &PredefinedMenuItem::separator(), &quit])?;

    // Resource 1 is the exe's icon (`assets/icon.rc`); asking for the small-icon
    // size picks the right frame instead of scaling down the 32px one.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let icon = Icon::from_resource(1, Some((size, size)))?;

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("Oxidecord")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;

    let clicks = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            _ = clicks.unbounded_send(Request::Show);
        }
    }));
    let (open, quit) = (open.id().clone(), quit.id().clone());
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let request = if event.id == open {
            Request::Show
        } else if event.id == quit {
            Request::Quit
        } else {
            return;
        };
        _ = tx.unbounded_send(request);
    }));

    Ok(tray)
}

/// gpui has no hide, so this goes to the OS. Hiding also drops the taskbar button,
/// and the maximized state survives it.
fn set_visible(window: &Window, visible: bool) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let command = if visible { SW_SHOW } else { SW_HIDE };
    unsafe {
        _ = ShowWindow(HWND(handle.hwnd.get() as _), command);
    }
}
