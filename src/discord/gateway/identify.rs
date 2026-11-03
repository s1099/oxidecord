//! What the gateway is told about the client in IDENTIFY: the stable desktop app's
//! client properties, field for field, so the session shows up as the desktop client.

use std::sync::OnceLock;

use twilight_model::gateway::payload::outgoing::identify::{ClientState, IdentifyProperties};

// The stable desktop build these values were taken from (1.0.9262 on Windows) and the
// web client build it was serving. Bump them together.
const CLIENT_VERSION: &str = "1.0.9262";
const CHROME_VERSION: &str = "148.0.7778.280";
const ELECTRON_VERSION: &str = "42.11.11";
const CLIENT_BUILD_NUMBER: u64 = 660103;

/// Built once so a re-identify presents the same launch ids as the first.
pub(super) fn properties() -> IdentifyProperties {
    static PROPERTIES: OnceLock<IdentifyProperties> = OnceLock::new();
    PROPERTIES.get_or_init(build).clone()
}

/// Empty guild versions ask for every guild in full, as the app has nothing cached.
pub(super) fn client_state() -> ClientState {
    ClientState {
        guild_versions: Default::default(),
        api_code_version: Some(0),
    }
}

fn build() -> IdentifyProperties {
    let os_version = os_version();
    IdentifyProperties {
        os: os_name().to_owned(),
        browser: "Discord Client".to_owned(),
        device: None,
        release_channel: Some("stable".to_owned()),
        client_version: Some(CLIENT_VERSION.to_owned()),
        os_sdk_version: os_version
            .as_deref()
            .filter(|_| cfg!(windows))
            .and_then(|version| version.rsplit('.').next())
            .map(str::to_owned),
        os_version,
        os_arch: Some(arch().to_owned()),
        app_arch: Some(arch().to_owned()),
        system_locale: Some(system_locale()),
        has_client_mods: Some(false),
        client_launch_id: Some(uuid()),
        launch_signature: Some(uuid()),
        browser_user_agent: Some(user_agent()),
        browser_version: Some(ELECTRON_VERSION.to_owned()),
        client_build_number: Some(CLIENT_BUILD_NUMBER),
        // Comes from the desktop updater, which nothing here has; mod loaders that
        // replace it (OpenAsar) send null too.
        native_build_number: Some(None),
        client_event_source: Some(None),
        client_heartbeat_session_id: Some(uuid()),
    }
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "Mac OS X",
        _ => "Linux",
    }
}

/// Node's names for the architecture, which is what Electron reports.
fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    }
}

/// Chromium's user agent with Electron's `discord/` and `Electron/` tokens spliced in.
fn user_agent() -> String {
    let platform = match std::env::consts::OS {
        // Chromium freezes these regardless of the real OS version.
        "windows" => "Windows NT 10.0; Win64; x64",
        "macos" => "Macintosh; Intel Mac OS X 10_15_7",
        _ => "X11; Linux x86_64",
    };
    format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) \
         discord/{CLIENT_VERSION} Chrome/{CHROME_VERSION} Electron/{ELECTRON_VERSION} \
         Safari/537.36"
    )
}

/// Node's `os.release()`: `10.0.<build>` on Windows, the kernel release elsewhere.
#[cfg(windows)]
fn os_version() -> Option<String> {
    use windows::{
        Win32::System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
        },
        core::{HSTRING, PCWSTR},
    };

    // GetVersionEx reports 6.2 to an exe without a compatibility manifest, so read
    // the version from where the OS keeps it.
    let key = HSTRING::from(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let dword = |name: &str| {
        let name = HSTRING::from(name);
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: the buffer and its size describe `value`.
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut value).cast()),
                Some(&mut size),
            )
        }
        .is_ok()
        .then_some(value)
    };
    let build = {
        let name = HSTRING::from("CurrentBuildNumber");
        let mut buffer = [0u16; 32];
        let mut size = size_of_val(&buffer) as u32;
        // SAFETY: the buffer and its size in bytes describe `buffer`.
        let ok = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        }
        .is_ok();
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        ok.then(|| String::from_utf16_lossy(&buffer[..len]))?
    };
    let major = dword("CurrentMajorVersionNumber").unwrap_or(10);
    let minor = dword("CurrentMinorVersionNumber").unwrap_or(0);
    Some(format!("{major}.{minor}.{build}"))
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|release| release.trim().to_owned())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn os_version() -> Option<String> {
    None
}

#[cfg(windows)]
fn system_locale() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buffer = [0u16; 85];
    // SAFETY: the slice carries its own length.
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    // The returned length counts the terminating null.
    match usize::try_from(len) {
        Ok(len) if len > 1 => String::from_utf16_lossy(&buffer[..len - 1]),
        _ => "en-US".to_owned(),
    }
}

#[cfg(not(windows))]
fn system_locale() -> String {
    // `en_US.UTF-8` becomes `en-US`.
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .and_then(|value| value.split('.').next().map(|tag| tag.replace('_', "-")))
        .unwrap_or_else(|| "en-US".to_owned())
}
