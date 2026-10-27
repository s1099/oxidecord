//! The update toast, following the updater from "new version" through the
//! download to "restart to install".
//!
//! It appears only on news (release found, download finished, download or
//! install failed) and stays until acted on or closed. Quiet check outcomes
//! never raise it. Once up it reads the status as it renders, so progress
//! updates in place instead of re-pushing and re-animating the toast.

use gpui_kit::component::{
    ActiveTheme as _, Root, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::{Notification, NotificationType},
    progress::Progress,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform::updater::{self, Status, Updater};

/// Toast id, so each push replaces the previous one.
struct UpdateNotice;

pub fn watch<T: 'static>(window: &Window, cx: &mut Context<T>) -> Subscription {
    let mut last = updater::status(cx);

    cx.observe_global_in::<Updater>(window, move |_, window, cx| {
        let status = updater::status(cx);
        if status == last {
            return;
        }
        let previous = std::mem::replace(&mut last, status.clone());

        // The toast lives in the window's root, absent until the window finishes opening.
        if window.root::<Root>().flatten().is_none() {
            return;
        }

        match (&previous, &status) {
            (_, Status::Available { .. }) => show(NotificationType::Info, window, cx),
            (_, Status::Ready { .. }) => show(NotificationType::Success, window, cx),
            // A failed check is usually just being offline; a failed download or install is not.
            (Status::Downloading { .. } | Status::Ready { .. }, Status::Failed(_)) => {
                show(NotificationType::Error, window, cx)
            }
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
        // The watcher closes the toast on these; this shows for one frame.
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
        // The failed download's URL is gone, so retrying starts from a fresh check.
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
            |this, percent| {
                this.child(
                    Progress::new("update-progress")
                        .mt_1()
                        .value(percent.into()),
                )
            },
        )
        .when_some(action, |this, action| {
            this.child(h_flex().mt_2().child(action.small()))
        })
        .into_any_element()
}
