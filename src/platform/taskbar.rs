//! Mute, deafen and disconnect buttons under the taskbar button's thumbnail while
//! in a call, as Discord's client puts there.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use resvg::{tiny_skia, usvg};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, THB_FLAGS, THB_ICON,
    THB_TOOLTIP, THBF_ENABLED, THBF_HIDDEN, THBN_CLICKED, THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, DestroyIcon, GetSystemMetrics, HICON, RegisterWindowMessageW, SM_CXSMICON,
    WM_COMMAND, WM_SETTINGCHANGE,
};
use windows::core::{PCWSTR, w};

use crate::assets::icon_svg;

#[derive(Clone, Copy, Debug)]
pub enum Click {
    Mute,
    Deafen,
    Disconnect,
}

const IDS: [(u32, Click); 3] = [(1, Click::Mute), (2, Click::Deafen), (3, Click::Disconnect)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CallState {
    pub muted: bool,
    pub deafened: bool,
}

/// Removes the buttons' hook from the window when dropped.
pub struct ThumbBar(Rc<Inner>);

struct Inner {
    hwnd: HWND,
    taskbar: ITaskbarList3,
    button_created: u32,
    clicks: UnboundedSender<Click>,
    call: Cell<Option<CallState>>,
    /// Buttons can be added once per taskbar button and never removed, so
    /// leaving a call hides them instead.
    added: Cell<bool>,
    icons: RefCell<Icons>,
}

impl ThumbBar {
    pub fn install(window: &Window) -> Option<(Self, UnboundedReceiver<Click>)> {
        let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let hwnd = HWND(handle.hwnd.get() as _);
        let taskbar = unsafe {
            // Already initialized on gpui's thread; this only makes sure.
            _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let taskbar: ITaskbarList3 =
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok()?;
            taskbar.HrInit().ok()?;
            taskbar
        };
        let (clicks, rx) = unbounded();
        let inner = Rc::new(Inner {
            hwnd,
            taskbar,
            button_created: unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) },
            clicks,
            call: Cell::new(None),
            added: Cell::new(false),
            icons: RefCell::new(Icons::new()),
        });
        let data = Rc::into_raw(inner.clone()) as usize;
        if !unsafe { SetWindowSubclass(hwnd, Some(subclass), 0, data) }.as_bool() {
            drop(unsafe { Rc::from_raw(data as *const Inner) });
            return None;
        }
        Some((Self(inner), rx))
    }

    /// `None` hides the buttons.
    pub fn set(&self, call: Option<CallState>) {
        if self.0.call.replace(call) != call || !self.0.added.get() {
            self.0.apply();
        }
    }
}

impl Drop for ThumbBar {
    fn drop(&mut self) {
        self.0.call.set(None);
        self.0.apply();
        unsafe {
            if RemoveWindowSubclass(self.0.hwnd, Some(subclass), 0).as_bool() {
                drop(Rc::from_raw(Rc::as_ptr(&self.0)));
            }
        }
    }
}

impl Inner {
    fn apply(&self) {
        let call = self.call.get();
        // Nothing to hide on a taskbar button that never had them.
        if call.is_none() && !self.added.get() {
            return;
        }
        let buttons = {
            let icons = self.icons.borrow();
            let state = call.unwrap_or(CallState {
                muted: false,
                deafened: false,
            });
            IDS.map(|(id, click)| {
                let (icon, tip) = match click {
                    Click::Mute if state.muted => (icons.mic_off, "Unmute"),
                    Click::Mute => (icons.mic, "Mute"),
                    Click::Deafen if state.deafened => (icons.headphones_off, "Undeafen"),
                    Click::Deafen => (icons.headphones, "Deafen"),
                    Click::Disconnect => (icons.disconnect, "Disconnect"),
                };
                let mut button = THUMBBUTTON {
                    dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                    iId: id,
                    iBitmap: 0,
                    hIcon: icon,
                    szTip: [0; 260],
                    dwFlags: if call.is_some() {
                        THBF_ENABLED
                    } else {
                        THBF_HIDDEN
                    },
                };
                for (slot, unit) in button.szTip.iter_mut().zip(tip.encode_utf16()) {
                    *slot = unit;
                }
                button
            })
        };
        unsafe {
            if self.added.get() {
                _ = self.taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons);
            } else if self.taskbar.ThumbBarAddButtons(self.hwnd, &buttons).is_ok() {
                // Fails until the taskbar button exists; `TaskbarButtonCreated` retries.
                self.added.set(true);
            }
        }
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let inner = unsafe { &*(data as *const Inner) };
    if msg == WM_COMMAND && (wparam.0 >> 16) & 0xffff == THBN_CLICKED as usize {
        let id = (wparam.0 & 0xffff) as u32;
        if let Some(&(_, click)) = IDS.iter().find(|(i, _)| *i == id) {
            _ = inner.clicks.unbounded_send(click);
            return LRESULT(0);
        }
    } else if msg == inner.button_created {
        // Hiding to the tray drops the taskbar button; showing makes a new one.
        inner.added.set(false);
        inner.apply();
    } else if msg == WM_SETTINGCHANGE
        && lparam.0 != 0
        && unsafe { PCWSTR(lparam.0 as _).to_string() }.is_ok_and(|s| s == "ImmersiveColorSet")
    {
        let old = inner.icons.replace(Icons::new());
        inner.apply();
        drop(old);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

struct Icons {
    mic: HICON,
    mic_off: HICON,
    headphones: HICON,
    headphones_off: HICON,
    disconnect: HICON,
}

impl Icons {
    fn new() -> Self {
        // The thumbnail flyout follows the taskbar's theme, not the app's.
        let fg = if system_light_theme() {
            "#1f1f1f"
        } else {
            "#ffffff"
        };
        let red = "#f23f42";
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
        let icon = |path, color| render(path, color, size);
        Self {
            mic: icon("icons/mic.svg", fg),
            mic_off: icon("icons/mic-off.svg", red),
            headphones: icon("icons/headphones.svg", fg),
            headphones_off: icon("icons/headphone-off.svg", red),
            disconnect: icon("icons/phone-off.svg", red),
        }
    }
}

impl Drop for Icons {
    fn drop(&mut self) {
        for icon in [
            self.mic,
            self.mic_off,
            self.headphones,
            self.headphones_off,
            self.disconnect,
        ] {
            if !icon.is_invalid() {
                _ = unsafe { DestroyIcon(icon) };
            }
        }
    }
}

/// A null icon on failure, which leaves the button blank but clickable.
fn render(path: &str, color: &str, size: u32) -> HICON {
    let pixels = || -> Option<Vec<u8>> {
        let svg = icon_svg(path)?.replace("currentColor", color);
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
        let scale = size as f32 / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        // Icons take straight alpha in BGRA order.
        Some(
            pixmap
                .pixels()
                .iter()
                .flat_map(|p| {
                    let c = p.demultiply();
                    [c.blue(), c.green(), c.red(), c.alpha()]
                })
                .collect(),
        )
    };
    let Some(bgra) = pixels() else {
        return HICON::default();
    };
    // The AND mask's rows are padded to 16 bits; alpha makes its contents moot.
    let mask = vec![0u8; size.div_ceil(16) as usize * 2 * size as usize];
    unsafe {
        CreateIcon(
            None,
            size as i32,
            size as i32,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
    .unwrap_or_default()
}

fn system_light_theme() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the buffer and its size describe `value`.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    }
    .is_ok()
        && value != 0
}
