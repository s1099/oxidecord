//! Toasts through WinRT's `ToastNotificationManager`.
//!
//! An unpackaged app gets toasts by registering an AppUserModelID under
//! `HKCU\Software\Classes\AppUserModelId`, which gives them a name and icon.
//! The process keeps its own taskbar identity; only the toasts carry the id.
//! Clicks arrive through each toast's `Activated` event, which fires only
//! while the app runs; the toasts are held so their handlers stay alive.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::core::{HSTRING, IInspectable, PCWSTR, w};

use super::{Request, Toast, ToastId};

const APP_ID: &str = "Oxidecord";
/// As Discord's desktop client: past this, a channel's oldest toast is taken
/// down, so a busy conversation can't bury the rest.
const TOASTS_PER_CHANNEL: usize = 20;
/// Discord throttles its message sound to one a second; later toasts in a burst
/// arrive silent.
const SOUND_INTERVAL: Duration = Duration::from_secs(1);
const BODY_LIMIT: usize = 300;

pub(super) fn spawn(requests: Receiver<Request>, clicks: UnboundedSender<ToastId>) {
    std::thread::Builder::new()
        .name("notifications".into())
        .spawn(move || run(requests, clicks))
        .expect("failed to spawn the notification thread");
}

fn run(requests: Receiver<Request>, clicks: UnboundedSender<ToastId>) {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    if let Err(err) = register_app_id() {
        eprintln!("notification registration failed: {err}");
    }
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
    {
        Ok(notifier) => notifier,
        Err(err) => {
            eprintln!("notifications unavailable: {err}");
            return;
        }
    };

    // Held so their click handlers stay alive, by channel.
    let mut shown: HashMap<u64, VecDeque<ToastNotification>> = HashMap::new();
    let mut last_sound: Option<Instant> = None;
    for request in requests {
        match request {
            Request::Show { toast, image } => {
                let now = Instant::now();
                let silent = last_sound.is_some_and(|at| now - at < SOUND_INTERVAL);
                match show(&notifier, &toast, image.as_deref(), silent, &clicks) {
                    Ok(notification) => {
                        if !silent {
                            last_sound = Some(now);
                        }
                        let channel = shown.entry(toast.id.channel).or_default();
                        channel.push_back(notification);
                        if channel.len() > TOASTS_PER_CHANNEL
                            && let Some(oldest) = channel.pop_front()
                        {
                            take_down(&notifier, &oldest);
                        }
                    }
                    Err(err) => eprintln!("toast failed: {err}"),
                }
            }
            // Asking the history about channels never toasted would be a COM
            // call per read.
            Request::Clear { channel } => {
                if shown.remove(&channel).is_some() {
                    _ = ToastNotificationManager::History().and_then(|history| {
                        history.RemoveGroupWithId(
                            &HSTRING::from(channel.to_string()),
                            &HSTRING::from(APP_ID),
                        )
                    });
                }
            }
        }
    }
}

/// Off the screen if it's still up, and out of the notification center.
fn take_down(notifier: &ToastNotifier, notification: &ToastNotification) {
    _ = notifier.Hide(notification);
    _ = (|| {
        ToastNotificationManager::History()?.RemoveGroupedTagWithId(
            &notification.Tag()?,
            &notification.Group()?,
            &HSTRING::from(APP_ID),
        )
    })();
}

fn show(
    notifier: &ToastNotifier,
    toast: &Toast,
    image: Option<&Path>,
    silent: bool,
    clicks: &UnboundedSender<ToastId>,
) -> windows::core::Result<ToastNotification> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(toast, image, silent)))?;
    let notification = ToastNotification::CreateToastNotification(&document)?;
    // The group lets a channel's toasts be cleared together once it's read.
    notification.SetGroup(&HSTRING::from(toast.id.channel.to_string()))?;
    notification.SetTag(&HSTRING::from(toast.id.message.to_string()))?;

    let id = toast.id;
    let clicks = clicks.clone();
    notification.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, _| {
            _ = clicks.unbounded_send(id);
            Ok(())
        },
    ))?;
    notifier.Show(&notification)?;
    Ok(notification)
}

fn toast_xml(toast: &Toast, image: Option<&Path>, silent: bool) -> String {
    let body = if toast.body.chars().count() > BODY_LIMIT {
        let cut: String = toast.body.chars().take(BODY_LIMIT).collect();
        format!("{}…", cut.trim_end())
    } else {
        toast.body.clone()
    };
    let image = image
        .map(|path| {
            format!(
                r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
                escape(&path.display().to_string())
            )
        })
        .unwrap_or_default();
    let audio = if silent {
        r#"<audio silent="true"/>"#
    } else {
        r#"<audio src="ms-winsoundevent:Notification.IM"/>"#
    };
    format!(
        concat!(
            r#"<toast><visual><binding template="ToastGeneric">"#,
            r#"<text hint-maxLines="1">{}</text><text>{}</text>{}"#,
            r#"</binding></visual>{}</toast>"#,
        ),
        escape(&toast.title),
        escape(&body),
        image,
        audio,
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML 1.0 refuses most control characters, and one would fail the
            // whole toast.
            ch if ch.is_control() && !matches!(ch, '\n' | '\t') => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Rewritten every start, so a moved config directory still has its icon.
fn register_app_id() -> windows::core::Result<()> {
    let icon = crate::platform::prefs::config_file("notification-icon.png");
    if let Some(icon) = &icon {
        if let Some(dir) = icon.parent() {
            _ = std::fs::create_dir_all(dir);
        }
        _ = std::fs::write(icon, include_bytes!("../../../assets/logo.png"));
    }

    let mut key = HKEY::default();
    let path = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &path,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    let result = set_string(key, w!("DisplayName"), APP_ID).and_then(|()| match &icon {
        Some(icon) => set_string(key, w!("IconUri"), &icon.display().to_string()),
        None => Ok(()),
    });
    unsafe {
        _ = RegCloseKey(key);
    }
    result
}

fn set_string(key: HKEY, name: PCWSTR, value: &str) -> windows::core::Result<()> {
    let bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let status = unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(&bytes)) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(status.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_escaped_and_capped() {
        let toast = Toast {
            id: ToastId {
                guild: None,
                channel: 1,
                message: 2,
            },
            title: "a <b> & \"c\"\u{7}".into(),
            body: "x".repeat(BODY_LIMIT + 10),
            avatar_url: None,
        };
        let xml = toast_xml(&toast, Some(Path::new(r"C:\a&b.png")), true);
        assert!(xml.contains("a &lt;b&gt; &amp; &quot;c&quot;</text>"));
        assert!(xml.contains(&format!("{}…", "x".repeat(BODY_LIMIT))));
        assert!(xml.contains(r#"<audio silent="true"/>"#));
        XmlDocument::new()
            .unwrap()
            .LoadXml(&HSTRING::from(xml))
            .unwrap();
    }
}
