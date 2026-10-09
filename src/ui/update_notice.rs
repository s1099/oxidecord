//! The update toast: one notification in the corner that follows the updater
//! from "a new version is out" through the download to "restart to install".
//!
//! It only appears on news — a release found, a download finished, a download
//! or install failed — and stays until acted on or closed, since each of those
//! asks the user to do something. The quiet outcomes of a check (up to date,
//! or GitHub unreachable) never raise it; the settings page still reports them.
//! Once up, it reads the updater's status as it renders, so download progress
//! ticks over in place instead of re-pushing (and re-animating) the toast for
//! every percent. Closing it hides it only until the next piece of news.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::{Notification, NotificationType},
    progress::Progress,
    v_flex,
};

use crate::platform::updater::{self, Status, Updater};

/// Identifies the toast, so each push replaces the previous one rather than
/// stacking a second.
struct UpdateNotice;

/// Keeps the toast in step with the updater for as long as the subscription
/// lives.
pub fn watch<T: 'static>(window: &Window, cx: &mut Context<T>) -> Subscription {
    let mut last = updater::status(cx);

    cx.observe_global_in::<Updater>(window, move |_, window, cx| {
        let status = updater::status(cx);
        if status == last {
            return;
        }
        let previous = std::mem::replace(&mut last, status.clone());

        // The toast lives in the window's root, which isn't in place until the
        // window has finished opening.
        if window.root::<Root>().flatten().is_none() {
            return;
        }

        match (&previous, &status) {
            (_, Status::Available { .. }) => show(NotificationType::Info, window, cx),
            (_, Status::Ready { .. }) => show(NotificationType::Success, window, cx),
            // A failed check is usually just being offline, which isn't worth
            // interrupting for; a failed download or install is.
            (Status::Downloading { .. } | Status::Ready { .. }, Status::Failed(_)) => {
                show(NotificationType::Error, window, cx)
            }
            // Progress is drawn live by the toast already up, if it still is.
            (_, Status::Downloading { .. }) => {}
            _ => window.remove_notification::<UpdateNotice>(cx),
        }
    })
}

fn show(kind: NotificationType, window: &mut Window, cx: &mut App) {
    window.push_notification(
        Notification::new()
            .id::<UpdateNotice>()
            .with_type(kind)
            .autohide(false)
            .content(|_, _window, cx| content(&updater::status(cx), cx)),
        cx,
    );
}

/// The toast's body for the current status: a title, a line of detail, and
/// whatever moves things along from here.
fn content(status: &Status, cx: &App) -> AnyElement {
    let (title, detail): (SharedString, SharedString) = match status {
        Status::Available { version, .. } => (
            "Update available".into(),
            format!(
                "Oxidecord {version} is out. You're on {}.",
                updater::CURRENT_VERSION
            )
            .into(),
        ),
        Status::Downloading { version, percent } => (
            "Downloading update".into(),
            format!("Oxidecord {version} · {percent}%").into(),
        ),
        Status::Ready { version } => (
            "Update ready".into(),
            format!("Restart to finish installing Oxidecord {version}.").into(),
        ),
        Status::Failed(error) => ("Update failed".into(), error.clone()),
        // The watcher takes the toast down on these, so this only shows for
        // the frame it takes to close.
        Status::Idle | Status::Checking | Status::UpToDate => return Empty.into_any_element(),
    };

    let action = match status {
        Status::Available { .. } => Some(
            Button::new("update-download")
                .label("Download")
                .primary()
                .on_click(|_, _window, cx| updater::download(cx)),
        ),
        Status::Ready { .. } => Some(
            Button::new("update-install")
                .label("Restart now")
                .primary()
                .on_click(|_, _window, cx| updater::install(cx)),
        ),
        // The download that failed is gone along with its URL, so trying again
        // starts from a fresh check.
        Status::Failed(_) => Some(
            Button::new("update-retry")
                .label("Try again")
                .outline()
                .on_click(|_, _window, cx| updater::check(cx)),
        ),
        _ => None,
    };

    v_flex()
        .gap_1()
        // Clear of the close button, which appears over the top-right corner.
        .pr_6()
        .child(div().text_sm().font_semibold().child(title))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        )
        .when_some(
            match status {
                Status::Downloading { percent, .. } => Some(*percent),
                _ => None,
            },
            |this, percent| this.child(Progress::new().mt_1().value(percent.into())),
        )
        .when_some(action, |this, action| {
            this.child(h_flex().mt_2().child(action.small()))
        })
        .into_any_element()
}
