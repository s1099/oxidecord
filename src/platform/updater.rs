//! Self-update against the project's GitHub releases.
//!
//! Releases publish a single `oxidecord.exe` asset, so an update is a download
//! of that file next to the running one, followed by a swap. Windows keeps a
//! running executable's image file locked, so the swap can't happen while the
//! app is up: the app hands the last step to a second copy of *itself*, started
//! with [`APPLY_UPDATE_ARG`], and then quits. That helper waits for the app to
//! go away, renames the old binary aside, moves the downloaded one into its
//! place, and exits. The renamed-aside file is deleted by the next launch (see
//! [`init`]), since the helper can't delete the image it is itself running from.
//!
//! Only the Windows path exists so far; elsewhere the settings page says as
//! much and the buttons are inert.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, Global, SharedString};

use super::runtime;

/// Where releases are published. The API returns the newest one, including its
/// tag and the download URL for each asset.
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/s1099/oxidecord/releases/latest";

/// The asset to download, as named in the release.
const ASSET_NAME: &str = "oxidecord.exe";

/// Filename the download is staged under, beside the running binary.
const STAGED_NAME: &str = "oxidecord.new.exe";

/// What the binary being replaced is renamed to. It outlives the swap because
/// the helper is running from it, so it's cleaned up on the next launch.
const RETIRED_NAME: &str = "oxidecord.old.exe";

/// Argument that turns a launch into the update helper rather than the app.
pub const APPLY_UPDATE_ARG: &str = "--apply-update";

/// GitHub rejects API requests without one.
const USER_AGENT: &str = concat!("oxidecord/", env!("CARGO_PKG_VERSION"));

/// The version this build reports and compares releases against.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Attempts the final rename this many times, in case the app's process is
/// still winding down when the helper wakes up.
const SWAP_ATTEMPTS: u32 = 40;
const SWAP_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Where the updater has got to. Drives everything the settings page shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    /// Nothing has been tried yet this run.
    #[default]
    Idle,
    Checking,
    /// The check found the running version to be the newest one.
    UpToDate,
    Available {
        version: SharedString,
        url: SharedString,
    },
    /// Percentage is only meaningful when the response declared a length; it
    /// stays at 0 otherwise.
    Downloading {
        version: SharedString,
        percent: u8,
    },
    /// Downloaded and staged; the swap happens on the next quit.
    Ready {
        version: SharedString,
    },
    Failed(SharedString),
}

/// The updater's state, global because the settings page is rebuilt from
/// scratch on every frame and has nowhere of its own to keep it.
#[derive(Default)]
struct Updater {
    status: Status,
}

impl Global for Updater {}

/// Registers the global and clears the leftovers of a completed update.
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

/// Whether self-updating is implemented for this platform at all.
pub const fn supported() -> bool {
    cfg!(target_os = "windows")
}

/// Asks GitHub for the newest release and compares it with this build.
pub fn check(cx: &mut App) {
    if matches!(status(cx), Status::Checking | Status::Downloading { .. }) {
        return;
    }
    set_status(Status::Checking, cx);

    cx.spawn(async move |cx| {
        let result = on_runtime(fetch_latest()).await;
        _ = cx.update(|cx| {
            let status = match result {
                Ok(Some(release)) => Status::Available {
                    version: release.version.into(),
                    url: release.url.into(),
                },
                Ok(None) => Status::UpToDate,
                Err(error) => Status::Failed(error.to_string().into()),
            };
            set_status(status, cx);
        });
    })
    .detach();
}

/// Downloads the available release and stages it beside the running binary.
pub fn download(cx: &mut App) {
    let Status::Available { version, url } = status(cx) else {
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
        // The download runs on the Tokio runtime and reports back over a
        // channel, so progress reaches the UI without the two sides polling
        // each other; only whole-percent changes are sent.
        let (tx, mut rx) = mpsc::unbounded();
        runtime::handle().spawn(download_asset(url.to_string(), staged, tx));

        while let Some(progress) = rx.next().await {
            let version = version.clone();
            let updated = cx.update(|cx| {
                let status = match progress {
                    Progress::Percent(percent) => Status::Downloading { version, percent },
                    Progress::Done(Ok(())) => Status::Ready { version },
                    Progress::Done(Err(error)) => Status::Failed(error.into()),
                };
                set_status(status, cx);
            });
            if updated.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Hands the swap to a helper process and quits so it can go ahead.
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
    if cx.default_global::<Updater>().status == status {
        return;
    }
    cx.default_global::<Updater>().status = status;
    // The settings page reads the global while rendering, so it only reflects
    // a change once something asks for a new frame.
    cx.refresh_windows();
}

/// Starts this same binary as the update helper, with a pipe as the signal for
/// "the app is gone": the helper reads until end-of-file, which the OS produces
/// when this process exits and its end of the pipe closes.
fn spawn_helper() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let child = Command::new(&exe)
        .arg(APPLY_UPDATE_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("couldn't start the updater: {error}"))?;

    // Dropping the handle would close the pipe and release the helper before
    // the app is actually gone; leaking it leaves that to process exit.
    std::mem::forget(child);
    Ok(())
}

/// The update helper. Called from `main` before any UI is set up, and always
/// exits the process rather than returning.
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

    // Windows allows renaming a running image but not overwriting one, so the
    // old binary moves aside first and the new one takes the free name. Both
    // steps can still lose a race with the app's last moments, hence the retry.
    for attempt in 0..SWAP_ATTEMPTS {
        _ = fs::remove_file(&retired);
        if fs::rename(&exe, &retired).is_ok() && fs::rename(&staged, &exe).is_ok() {
            std::process::exit(0);
        }
        // Put the old binary back if only the second rename failed, so a give-up
        // leaves a working app behind.
        if !exe.exists() {
            _ = fs::rename(&retired, &exe);
        }
        if attempt + 1 < SWAP_ATTEMPTS {
            std::thread::sleep(SWAP_RETRY_DELAY);
        }
    }

    std::process::exit(1);
}

/// What a download reports back to the UI.
enum Progress {
    Percent(u8),
    Done(Result<(), String>),
}

struct Release {
    version: String,
    url: String,
}

/// Fetches the newest release, or `None` when it isn't newer than this build.
async fn fetch_latest() -> Result<Option<Release>, String> {
    let response = reqwest::Client::new()
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

    let url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"].as_str() == Some(ASSET_NAME))
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or(format!("release {tag} has no {ASSET_NAME}"))?;

    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
    }))
}

/// Streams the asset to `staged`, reporting each whole percent along the way.
async fn download_asset(url: String, staged: PathBuf, tx: mpsc::UnboundedSender<Progress>) {
    let result = write_asset(url, &staged, &tx).await;
    if result.is_err() {
        // A partial file would be swapped in as if it were a build.
        _ = fs::remove_file(&staged);
    }
    _ = tx.unbounded_send(Progress::Done(result));
}

async fn write_asset(
    url: String,
    staged: &Path,
    tx: &mpsc::UnboundedSender<Progress>,
) -> Result<(), String> {
    let mut response = reqwest::Client::new()
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

    let mut written = 0u64;
    let mut reported = 0u8;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("the download was interrupted: {error}"))?
    {
        file.write_all(&chunk)
            .map_err(|error| format!("couldn't write the download: {error}"))?;
        written += chunk.len() as u64;

        // Nothing to report when the response didn't declare a length.
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
        .map_err(|error| format!("couldn't finish writing the download: {error}"))
}

/// Runs a future on the Tokio runtime the network crates need, and awaits the
/// result from gpui's executor.
async fn on_runtime<T>(
    future: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime::handle().spawn(async move {
        _ = tx.send(future.await);
    });
    rx.await
        .unwrap_or_else(|_| Err("the check was cut short".into()))
}

/// A path beside the running binary.
fn sibling_path(name: &str) -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(name))
}

/// Compares two versions by their leading numbers, so a `v` prefix or a
/// `-beta` suffix doesn't get in the way. Anything unparseable counts as 0,
/// which keeps a malformed tag from being taken for an upgrade.
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
