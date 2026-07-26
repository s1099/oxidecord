//! Modal dialogs shared across screens, plus the backdrop drawn behind them.
//!
//! Dialogs are opened through gpui-component's [`Root`], which collects them but
//! doesn't draw them; [`with_dialog_layer`] renders that layer over the app and
//! pushes the content behind it back, so a modal reads as being in front of the
//! app rather than floating somewhere inside it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Root, WindowExt as _, button::ButtonVariant,
    dialog::DialogButtonProps, h_flex, v_flex,
};

const BACKDROP_CONTENT_OPACITY: f32 = 0.55;

/// How dark the scrim over the faded content is.
const BACKDROP_SCRIM_ALPHA: f32 = 0.3;

/// Longest a dialog's list of problems gets before the tail is summarised, so a
/// bad multi-select can't grow the dialog past the bottom of the window.
const MAX_LISTED_PROBLEMS: usize = 6;

/// Renders `content` with the modal layer above it: while a dialog is open the
/// content is faded and covered by a scrim, then the dialog itself is drawn on
/// top. With no dialog open this is just `content`, with nothing painted over it.
pub fn with_dialog_layer(
    content: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let dialog_open = window.has_active_dialog(cx);

    div()
        .relative()
        .size_full()
        .child(
            div()
                .size_full()
                .when(dialog_open, |this| this.opacity(BACKDROP_CONTENT_OPACITY))
                .child(content),
        )
        .when(dialog_open, |this| {
            // Purely visual: the dialog's own overlay sits above this and
            // occludes the content, so the scrim doesn't need to block input.
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(BACKDROP_SCRIM_ALPHA)),
            )
        })
        .children(Root::render_dialog_layer(window, cx))
}

/// Shows a modal error dialog: a titled danger icon over one line per problem,
/// dismissed with a button, the escape key, or a click outside.
pub fn show_error(
    title: impl Into<SharedString>,
    problems: impl IntoIterator<Item = impl Into<SharedString>>,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = title.into();
    let problems: Vec<SharedString> = problems.into_iter().map(Into::into).collect();
    let hidden = problems.len().saturating_sub(MAX_LISTED_PROBLEMS);
    let listed: Vec<SharedString> = problems.into_iter().take(MAX_LISTED_PROBLEMS).collect();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let theme = cx.theme();
        // One problem reads as a sentence; several read as a list, so only the
        // list gets bullets.
        let bulleted = listed.len() > 1;

        dialog
            .alert()
            // An error is a notification, not a decision, so let a click
            // outside dismiss it as well — `alert()` turns that off.
            .overlay_closable(true)
            .w(px(440.))
            .button_props(
                DialogButtonProps::default()
                    .ok_text("Dismiss")
                    .ok_variant(ButtonVariant::Danger),
            )
            .title(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .size_8()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(theme.danger.opacity(0.12))
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .size_4()
                                    .text_color(theme.danger),
                            ),
                    )
                    .child(title.clone()),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .children(listed.iter().map(|problem| {
                        h_flex()
                            .gap_2()
                            .items_start()
                            .when(bulleted, |this| {
                                this.child(
                                    div()
                                        .flex_shrink_0()
                                        // Centres the dot on the first line oftext
                                        .mt(px(7.))
                                        .size(px(4.))
                                        .rounded_full()
                                        .bg(theme.muted_foreground),
                                )
                            })
                            .child(div().flex_1().min_w_0().child(problem.clone()))
                    }))
                    .when(hidden > 0, |this| {
                        this.child(
                            div()
                                .italic()
                                .child(format!("…and {hidden} more"))
                                .text_color(theme.muted_foreground.opacity(0.8)),
                        )
                    }),
            )
    });
}
