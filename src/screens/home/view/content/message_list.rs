//! The scrolling message list, and the skeleton shown in its place while a
//! conversation loads.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, h_flex, skeleton::Skeleton, spinner::Spinner, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::screens::home::state::MessageListView;
use crate::ui::button::Button;
use crate::ui::depth::{self, Finish, Level, Lit as _, radius};

use crate::screens::home::view::{GROUP_GAP, MESSAGE_PADDING_X};

/// Space under the newest message, on top of the list's own padding, so it
/// doesn't sit flush against the composer.
const NEWEST_MESSAGE_GAP: f32 = 12.;

const UNREAD_BAR_HEIGHT: f32 = 28.;

/// The red rule Discord draws above the first message the user hasn't read.
fn new_messages_divider(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .w_full()
        .pt(px(GROUP_GAP))
        .px(px(MESSAGE_PADDING_X))
        .items_center()
        .child(div().flex_1().h(px(1.)).bg(theme.danger))
        .child(
            div()
                .px(px(4.))
                .rounded(px(4.))
                .bg(theme.danger)
                .text_color(theme.danger_foreground)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .child("NEW"),
        )
}

/// Placeholder message rows. The widths are fixed rather than random so the
/// skeleton doesn't reshuffle on every frame.
pub(super) fn skeleton() -> impl IntoElement {
    const WIDTHS: [f32; 12] = [
        420., 280., 360., 200., 480., 320., 260., 440., 300., 380., 220., 460.,
    ];
    v_flex()
        .flex_1()
        .w_full()
        .justify_end()
        .gap_4()
        .py_2()
        .children(WIDTHS.iter().map(|&content_width| {
            h_flex()
                .w_full()
                .px(px(MESSAGE_PADDING_X))
                .gap_3()
                .items_start()
                .child(Skeleton::new().size(px(40.)).rounded_full())
                .child(
                    v_flex()
                        .flex_1()
                        .gap_2()
                        .child(Skeleton::new().w(px(120.)).h_4().rounded_md())
                        .child(Skeleton::new().w(px(content_width)).h_4().rounded_md()),
                )
        }))
}

impl Render for MessageListView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(screen) = self.screen.upgrade() else {
            return div().into_any_element();
        };
        let (state, regions) = {
            let this = screen.read(cx);
            (this.messages_list.clone(), this.text_regions.clone())
        };
        // Each selectable text adds itself back as the list lays it out.
        regions.borrow_mut().clear();
        let weak = self.screen.clone();
        div()
            .relative()
            .size_full()
            .child(
                list(state, move |ix, _window, cx| {
                    screen.update(cx, |this, cx| this.render_message_item(ix, cx))
                })
                .size_full(),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |viewport, (), window, _| listen_for_selection(weak, viewport, window),
                )
                // Pinned to the corner: left where it falls, an absolute
                // element sits below the list, and every drag would read as
                // one past the list's top.
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

/// Feeds the mouse to the text selection for a frame. The press and release
/// listen in the capture phase, which sees every one wherever it lands: a
/// press outside the list has to drop the selection, and a release has to
/// settle it before a link under the pointer decides whether it was clicked.
fn listen_for_selection(
    screen: WeakEntity<HomeScreen>,
    viewport: Bounds<Pixels>,
    window: &mut Window,
) {
    window.on_mouse_event({
        let screen = screen.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                let _ = screen.update(cx, |this, cx| this.press_text(event, viewport, window, cx));
            }
        }
    });
    window.on_mouse_event({
        let screen = screen.clone();
        move |event: &MouseMoveEvent, phase, _, cx| {
            let dragging = phase == DispatchPhase::Bubble
                && screen
                    .read_with(cx, |this, _| this.is_dragging_text())
                    .unwrap_or(false);
            if !dragging {
                return;
            }
            let _ = screen.update(cx, |this, cx| {
                // Let go outside the window, where the release never came.
                if event.pressed_button == Some(MouseButton::Left) {
                    this.drag_text(event.position, cx);
                } else {
                    this.release_text();
                }
            });
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
            let _ = screen.update(cx, |this, _| this.release_text());
        }
    });
}

impl HomeScreen {
    pub(super) fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return skeleton().into_any_element();
        }

        if let Some(error) = &self.messages_error {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element();
        }

        let messages_list = AnyView::from(self.message_list_view.clone())
            .cached(StyleRefinement::default().flex_1());

        // The vertical padding sits on the container, not on the list: `list`
        // counts its own padding when it clamps a scroll offset but not when it
        // reports one, and smooth scrolling needs those two to agree to land on
        // the bottom and let the list repin there. See `ui::smooth_scroll`.
        let mut container = v_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .py_2()
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.messages_scroll.absorb(event, window)),
            );
        let unread_bar = self.render_unread_bar(cx);
        // Floats over the list rather than taking a row of its own: appearing
        // and disappearing in the layout would resize the viewport under a
        // scroll that is still moving, nudging the messages twice per page.
        if self.older_loading {
            container = container.child(
                h_flex()
                    .absolute()
                    // Below the unread bar, which would hide it.
                    .top(if unread_bar.is_some() {
                        px(UNREAD_BAR_HEIGHT)
                    } else {
                        px(0.)
                    })
                    .left_0()
                    .w_full()
                    .py_1()
                    .justify_center()
                    .child(Spinner::new().small().color(theme.muted_foreground)),
            );
        }

        // Note: images inside the list name `self.image_cache` on the element
        // itself rather than relying on an ancestor `image_cache(..)`. That
        // wrapper only pushes onto the cache stack during layout and paint,
        // while `list` renders its items during prepaint, so the stack would be
        // empty when the images actually resolve and they'd land in gpui's
        // global asset cache, which never evicts.
        container = container.child(messages_list);
        // Floats for the same reason: opened further back than the present,
        // the way back to it stays in reach, as Discord's bar does. Added
        // after the list so it paints over the messages rather than under.
        if !self.reached_newest {
            container = container.child(self.render_jump_to_present(cx));
        }
        container.children(unread_bar).into_any_element()
    }

    /// Discord's bar across the top of a conversation with unread messages:
    /// clicking it goes to the first of them, and its button reads them all.
    /// It goes once they're acked, which reaching the bottom does.
    fn render_unread_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (count, more, since) = self.unread_summary()?;
        let theme = cx.theme();
        let noun = if count == 1 && !more {
            "message"
        } else {
            "messages"
        };
        let more = if more { "+" } else { "" };
        Some(
            h_flex()
                .id("unread-bar")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .mx_2()
                .h(px(UNREAD_BAR_HEIGHT))
                .px_3()
                .items_center()
                .justify_between()
                .rounded_b(radius::CARD)
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .text_sm()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.jump_to_first_unread(cx)))
                .child(format!("{count}{more} new {noun} since {since}"))
                .child(
                    div()
                        .id("mark-as-read")
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|style| style.underline())
                        .on_click(cx.listener(|this, _, window, cx| {
                            // The bar's own click would scroll back up to the
                            // unread.
                            cx.stop_propagation();
                            this.mark_conversation_read(window, cx);
                        }))
                        .child("Mark As Read"),
                )
                .into_any_element(),
        )
    }

    fn render_jump_to_present(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .absolute()
            .bottom_0()
            .left_0()
            .w_full()
            .pb_3()
            .justify_center()
            .child(
                h_flex()
                    .id("jump-to-present-bar")
                    // Keeps the pointer off the messages underneath, which
                    // would otherwise light up and take its clicks.
                    .occlude()
                    .pl_4()
                    .pr_1p5()
                    .py_1p5()
                    .gap_4()
                    .items_center()
                    .rounded(radius::CARD)
                    .bg(theme.popover)
                    .border_1()
                    .border_color(depth::ring(cx))
                    .lit(Level::Overlay, Finish::Subtle, px(44.), radius::CARD, cx)
                    .text_sm()
                    .child(
                        div()
                            .text_color(theme.foreground)
                            .child("You're Viewing Older Messages"),
                    )
                    .when(self.newer_loading, |this| {
                        this.child(Spinner::new().small().color(theme.muted_foreground))
                    })
                    .child(
                        Button::new("jump-to-present")
                            .label("Jump To Present")
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.jump_to_present(window, cx);
                            })),
                    ),
            )
    }

    fn render_message_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        let unread_divider = self.unread_divider_index() == Some(ix);
        // Consecutive messages from the same author share one header, except
        // replies always show theirs so the quoted preview has room to sit
        // above the message, like Discord — and the divider breaks a group.
        let show_header = ix == 0
            || unread_divider
            || message.reply.is_some()
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        let row = self.render_message(message, show_header, cx);
        let row = if unread_divider {
            v_flex()
                .w_full()
                .child(new_messages_divider(cx))
                .child(row)
                .into_any_element()
        } else {
            row
        };
        // The space belongs to the newest row rather than the list's padding,
        // so it scrolls away with the conversation instead of shrinking the
        // viewport. Data-side splices go through `splice_messages`, which
        // re-lays out the row that loses it.
        if ix + 1 == self.messages.len() {
            div()
                .pb(px(NEWEST_MESSAGE_GAP))
                .child(row)
                .into_any_element()
        } else {
            row
        }
    }
}
