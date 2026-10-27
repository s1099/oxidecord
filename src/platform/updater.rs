//! Self-update against the project's GitHub releases.
//!
//! An update downloads the release's single `oxidecord.exe` next to the running
//! one, then swaps. Windows locks a running image, so the app starts a second
//! copy of itself with [`APPLY_UPDATE_ARG`] and quits; that helper waits for the
//! app to exit, renames the old binary aside, moves the new one in and starts
//! it. The renamed-aside file is deleted by the next launch ([`init`]) since
//! the helper can't delete the image it runs from.
//!
//! The download must match the SHA-256 digest GitHub records for the asset; a
//! release without one isn't installed. Windows only so far.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::{App, Global, SharedString};
use sha2::{Digest as _, Sha256};

use super::{http, runtime};

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/s1099/oxidecord/releases/latest";

const ASSET_NAME: &str = "oxidecord.exe";

const STAGED_NAME: &str = "oxidecord.new.exe";

/// The replaced binary is renamed to this; the helper runs from it, so the next launch cleans it
/// up.
const RETIRED_NAME: &str = "oxidecord.old.exe";

pub const APPLY_UPDATE_ARG: &str = "--apply-update";

/// GitHub rejects API requests without one.
const USER_AGENT: &str = concat!("oxidecord/", env!("CARGO_PKG_VERSION"));

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The app's process may still be winding down when the helper wakes up.
const SWAP_ATTEMPTS: u32 = 40;
const SWAP_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        version: SharedString,
        url: SharedString,
        /// Lowercase hex, as GitHub reports it.
        sha256: SharedString,
    },
    /// `percent` stays 0 when the response declared no length.
    Downloading {
        version: SharedString,
        percent: u8,
    },
    Ready {
        version: SharedString,
    },
    Failed(SharedString),
}

/// Global because the settings page is rebuilt every frame and has nowhere to
/// keep it; the update toast observes it.
#[derive(Default)]
pub struct Updater {
    status: Status,
}

impl Global for Updater {}

pub fn init(cx: &mut App) {
    cx.default_global::<Updater>();

    if let Some(retired) = sibling_path(RETIRED_NAME) {
        _ = fs::remove_file(retired);
    }
}

pub fn status(cx: &App) -> Status {
    cx.try_global::<Updater>()
        .map(|updater| updater.status.clone())
        .unwrap_or_default()
}

pub const fn supported() -> bool {
    cfg!(target_os = "windows")
}

pub fn check(cx: &mut App) {
    if matches!(status(cx), Status::Checking | Status::Downloading { .. }) {
        return;
    }
    set_status(Status::Checking, cx);

    cx.spawn(async move |cx| {
        let result = runtime::run(fetch_latest())
            .await
            .unwrap_or_else(|_| Err("the check was cut short".into()));
        cx.update(|cx| {
            let status = match result {
                Ok(Some(release)) => Status::Available {
                    version: release.version.into(),
                    url: release.url.into(),
                    sha256: release.sha256.into(),
                },
                Ok(None) => Status::UpToDate,
                Err(error) => Status::Failed(error.to_string().into()),
            };
            set_status(status, cx);
        });
    })
    .detach();
}

pub fn download(cx: &mut App) {
    let Status::Available {
        version,
        url,
        sha256,
    } = status(cx)
    else {
        return;
    };
    let Some(staged) = sibling_path(STAGED_NAME) else {
        set_status(Status::Failed("can't locate the app's folder".into()), cx);
        return;
    };
    set_status(
        Status::Downloading {
            version: version.clone(),
            percent: 0,
        },
        cx,
    );

    cx.spawn(async move |cx| {
        // Runs on Tokio and reports over a channel; only whole-percent changes are sent.
        let (tx, mut rx) = mpsc::unbounded();
        runtime::handle().spawn(download_asset(
            url.to_string(),
            sha256.to_string(),
            staged,
            tx,
        ));

        while let Some(progress) = rx.next().await {
            let version = version.clone();
            cx.update(|cx| {
                let status = match progress {
                    Progress::Percent(percent) => Status::Downloading { version, percent },
                    Progress::Done(Ok(())) => Status::Ready { version },
                    Progress::Done(Err(error)) => Status::Failed(error.into()),
                };
                set_status(status, cx);
            });
        }
    })
    .detach();
}

/// Quits so the helper can swap the binary, then it starts the new version.
pub fn install(cx: &mut App) {
    if !matches!(status(cx), Status::Ready { .. }) {
        return;
    }

    match spawn_helper() {
        Ok(()) => cx.quit(),
        Err(error) => set_status(Status::Failed(error.into()), cx),
    }
}

fn set_status(status: Status, cx: &mut App) {
    // Read without `default_global`, which counts as a change and wakes observers.
    if self::status(cx) == status {
        return;
    }
    cx.default_global::<Updater>().status = status;
    // The settings page reads the global while rendering, so it needs a new frame.
    cx.refresh_windows();
}

/// The stdin pipe signals "the app is gone": the helper reads to EOF, which the
/// OS produces when this process exits.
fn spawn_helper() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let child = Command::new(&exe)
        .arg(APPLY_UPDATE_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("couldn't start the updater: {error}"))?;

    // Dropping the handle would close the pipe and release the helper early.
    std::mem::forget(child);
    Ok(())
}

/// Called from `main` before any UI is set up; always exits the process.
pub fn apply_update() -> ! {
    // Blocks until the app's process exits and closes the write end.
    _ = std::io::stdin().read_to_end(&mut Vec::new());

    let (Ok(exe), Some(staged), Some(retired)) = (
        std::env::current_exe(),
        sibling_path(STAGED_NAME),
        sibling_path(RETIRED_NAME),
    ) else {
        std::process::exit(1);
    };

    if !staged.exists() {
        std::process::exit(1);
    }

    // Windows allows renaming a running image but not overwriting it, so the
    // old binary moves aside first. Either step can lose a race with the app's
    // last moments, hence the retry.
    for attempt in 0..SWAP_ATTEMPTS {
        _ = fs::remove_file(&retired);
        if fs::rename(&exe, &retired).is_ok() && fs::rename(&staged, &exe).is_ok() {
            _ = Command::new(&exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            std::process::exit(0);
        }
        // Restore the old binary if only the second rename failed, so giving up leaves a working
        // app.
        if !exe.exists() {
            _ = fs::rename(&retired, &exe);
        }
        if attempt + 1 < SWAP_ATTEMPTS {
            std::thread::sleep(SWAP_RETRY_DELAY);
        }
    }

    std::process::exit(1);
}

enum Progress {
    Percent(u8),
    Done(Result<(), String>),
}

struct Release {
    version: String,
    url: String,
    sha256: String,
}

/// `None` when the newest release isn't newer than this build.
async fn fetch_latest() -> Result<Option<Release>, String> {
    let response = http::client()
        .get(LATEST_RELEASE_API)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("couldn't reach GitHub: {error}"))?;

    if !response.status().is_success() {
        return Err(format!("GitHub returned {}", response.status()));
    }

    let body = response
        .bytes()
        .await
        .map_err(|error| format!("unreadable release data: {error}"))?;
    let release: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| format!("unreadable release data: {error}"))?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or("the latest release has no tag")?;

    if !is_newer(tag, CURRENT_VERSION) {
        return Ok(None);
    }

    let asset = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"].as_str() == Some(ASSET_NAME))
        .ok_or(format!("release {tag} has no {ASSET_NAME}"))?;
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or(format!("release {tag} has no {ASSET_NAME}"))?;
    // Without a digest a corrupted or substituted download is undetectable, so refuse.
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or(format!("release {tag} has no checksum for {ASSET_NAME}"))?;

    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
        sha256: sha256.to_ascii_lowercase(),
    }))
}

/// Keeps `staged` only if it hashes to `sha256`.
async fn download_asset(
    url: String,
    sha256: String,
    staged: PathBuf,
    tx: mpsc::UnboundedSender<Progress>,
) {
    let result = write_asset(url, &sha256, &staged, &tx).await;
    if result.is_err() {
        // A partial or mismatched file would be swapped in as a build.
        _ = fs::remove_file(&staged);
    }
    _ = tx.unbounded_send(Progress::Done(result));
}

async fn write_asset(
    url: String,
    sha256: &str,
    staged: &Path,
    tx: &mpsc::UnboundedSender<Progress>,
) -> Result<(), String> {
    let mut response = http::client()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("couldn't start the download: {error}"))?
        .error_for_status()
        .map_err(|error| format!("couldn't start the download: {error}"))?;

    let total = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(staged)
        .map_err(|error| format!("couldn't write beside the app: {error}"))?;

    let mut hasher = Sha256::new();
    let mut written = 0u64;
    let mut reported = 0u8;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("the download was interrupted: {error}"))?
    {
        file.write_all(&chunk)
            .map_err(|error| format!("couldn't write the download: {error}"))?;
        hasher.update(&chunk);
        written += chunk.len() as u64;

        // No length declared means no percentage.
        if let Some(scaled) = (written * 100).checked_div(total) {
            let percent = scaled.min(100) as u8;
            if percent != reported {
                reported = percent;
                if tx.unbounded_send(Progress::Percent(percent)).is_err() {
                    return Err("the download was cancelled".into());
                }
            }
        }
    }

    file.flush()
        .map_err(|error| format!("couldn't finish writing the download: {error}"))?;

    let actual: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != sha256 {
        return Err("the download didn't match its checksum".into());
    }
    Ok(())
}

fn sibling_path(name: &str) -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(name))
}

/// Compares leading numbers, ignoring a `v` prefix or `-beta` suffix. Unparseable
/// parts count as 0 so a malformed tag isn't taken for an upgrade.
fn is_newer(candidate: &str, current: &str) -> bool {
    numbers(candidate) > numbers(current)
}

fn numbers(version: &str) -> [u64; 3] {
    let mut parts = version
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .map(|part| part.parse().unwrap_or(0));

    [
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    ]
}
