//! The tray of removable previews for the files staged to be sent, like
//! Discord's attachment tray.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

use crate::screens::home::HomeScreen;
use crate::screens::home::data::attachments::{PendingAttachment, format_size};
use crate::ui::button::Button;

impl HomeScreen {
    pub(super) fn render_attachment_previews(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .px_3()
            .pt_3()
            .children(self.pending_attachments.iter().map(|attachment| {
                let id = attachment.id;
                let spoiler = attachment.spoiler;
                div()
                    .relative()
                    .flex_shrink_0()
                    .child(render_preview(attachment, cx))
                    .when(spoiler, |this| this.child(spoiler_badge()))
                    .child(
                        h_flex()
                            .absolute()
                            .top(px(4.))
                            .right(px(4.))
                            .gap_1()
                            .child(
                                Button::new(("spoiler-attachment", id))
                                    .icon(if spoiler {
                                        IconName::Eye
                                    } else {
                                        IconName::EyeOff
                                    })
                                    .outline()
                                    .xsmall()
                                    .tooltip(if spoiler {
                                        "Unmark as spoiler"
                                    } else {
                                        "Mark as spoiler"
                                    })
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.toggle_attachment_spoiler(id, cx);
                                    })),
                            )
                            .child(
                                Button::new(("remove-attachment", id))
                                    .icon(IconName::Close)
                                    // Outline rather than danger: the translucent
                                    // danger fill would vanish against the image.
                                    .outline()
                                    .xsmall()
                                    .text_color(cx.theme().danger)
                                    .tooltip("Remove attachment")
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.remove_attachment(id, cx);
                                    })),
                            ),
                    )
            }))
    }
}

/// Lays a "SPOILER" label over a staged attachment that will go out marked
/// as one. The preview itself stays visible — it's the sender's own file.
fn spoiler_badge() -> impl IntoElement {
    div()
        .absolute()
        .bottom(px(6.))
        .left(px(6.))
        .px_2()
        .py(px(2.))
        .rounded_full()
        // A scrim like the one over a received spoiler, for the same reason:
        // it sits on top of arbitrary picture colours.
        .bg(black().opacity(0.7))
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child("SPOILER")
}

/// One staged attachment: the image itself when it is one, otherwise a card
/// with the file's name and size.
fn render_preview(attachment: &PendingAttachment, cx: &App) -> AnyElement {
    let theme = cx.theme();

    let Some(image) = attachment.data.image() else {
        return v_flex()
            .h(px(120.))
            .w(px(160.))
            .p_3()
            .gap_2()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(theme.muted.opacity(0.5))
            .border_1()
            .border_color(theme.border)
            .child(
                Icon::new(IconName::File)
                    .size_8()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_xs()
                    .text_center()
                    .child(attachment.filename.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format_size(attachment.data.bytes().len() as u64)),
            )
            .into_any_element();
    };

    img(image)
        .h(px(120.))
        .max_w(px(200.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.border)
        .into_any_element()
}
