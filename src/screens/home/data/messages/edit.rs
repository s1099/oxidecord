//! Editing a message in place: opening, saving (applied locally, rolled back on
//! failure), and edits arriving over the gateway.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Position, TextareaState};
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::{EditLastMessage, EditingMessage, HomeScreen};
use crate::ui::depth;

const EDIT_MAX_ROWS: usize = 16;

impl HomeScreen {
    /// Discord lets only the author edit, whatever else they manage.
    pub(in crate::screens::home) fn is_own_message(&self, message: &discord::Message) -> bool {
        self.current_user
            .as_ref()
            .map(|user| user.id)
            .or(self.self_user_id)
            == Some(message.author_id)
    }

    pub(in crate::screens::home) fn start_editing(
        &mut self,
        message_id: Id<MessageMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editing) = &self.editing
            && editing.message_id == message_id
        {
            editing
                .input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        else {
            return;
        };
        let message = &self.messages[ix];
        // A message still sending has no id on the server to edit yet.
        if !self.is_own_message(message) || message.delivery != discord::Delivery::Sent {
            return;
        }

        let content = message.content.clone();
        // Positions count lines and characters, not bytes.
        let last_line = content.split('\n').next_back().unwrap_or_default();
        let end = Position::new(
            content.matches('\n').count() as u32,
            last_line.chars().count() as u32,
        );
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, EDIT_MAX_ROWS)
                .default_value(content)
        });
        input.update(cx, |input, cx| input.set_cursor_position(end, window, cx));

        self.editing = Some(EditingMessage { message_id, input });
        // Up in the composer can reach a message scrolled out of view.
        self.messages_list.scroll_to_reveal_item(ix);
        cx.notify();
    }

    pub(in crate::screens::home) fn cancel_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.take().is_none() {
            return;
        }
        self.message_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Unchanged text just closes the box. Clearing a message with nothing else in
    /// it would leave it empty, which Discord treats as a delete, so that asks first.
    pub(in crate::screens::home) fn save_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(editing) = &self.editing else {
            return;
        };
        let message_id = editing.message_id;
        let content = self.resolve_emoji_names(editing.input.read(cx).value().trim());
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        else {
            // The message went away while it was being edited.
            return self.cancel_edit(window, cx);
        };

        if content == message.content {
            return self.cancel_edit(window, cx);
        }
        if content.is_empty()
            && message.images.is_empty()
            && message.videos.is_empty()
            && message.files.is_empty()
        {
            return self.confirm_delete_from_edit(message_id, window, cx);
        }

        // Shown now, as Discord does; the box closing onto the old text would read as
        // the edit being lost.
        let previous = (message.content.clone(), message.edited);
        message.set_content(content.clone());
        message.edited = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_secs() as i64);
        self.editing = None;
        self.send_error = None;
        self.message_input.focus_handle(cx).focus(window, cx);
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::edit_message(channel_id, message_id, content).await;
            let _ = this.update(cx, |this, cx| {
                // Nothing to update if the user switched channels meanwhile:
                // the message isn't in the list they're looking at.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                let Some(message) = this
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                else {
                    return;
                };
                match result {
                    // The server's copy carries embeds for any links the edit
                    // added, which the local one can't know about.
                    Ok(edited) => message.apply_edit(edited),
                    Err(err) => {
                        let (content, edited) = previous;
                        message.set_content(content);
                        message.edited = edited;
                        this.send_error = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_delete_from_edit(
        &mut self,
        message_id: Id<MessageMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, cx| {
            let screen = screen.clone();
            dialog
                .confirm()
                .border_color(depth::ring(cx))
                .width(px(440.))
                .title("Delete Message")
                .child("Are you sure you want to delete this message?")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, _window, cx| {
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |this, cx| this.delete_message(message_id, cx));
                    }
                    true
                })
        });
    }

    /// Only in an empty composer; elsewhere the arrow moves the cursor as usual.
    pub(in crate::screens::home) fn on_edit_last_message(
        &mut self,
        _: &EditLastMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.message_input.read(cx).value().is_empty() {
            cx.propagate();
            return;
        }
        let Some(message_id) = self
            .messages
            .iter()
            .rev()
            .find(|message| {
                self.is_own_message(message) && message.delivery == discord::Delivery::Sent
            })
            .map(|message| message.id)
        else {
            cx.propagate();
            return;
        };
        self.start_editing(message_id, window, cx);
    }

    pub(in crate::screens::home) fn handle_message_update(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(incoming.channel_id) || self.messages_loading {
            return;
        }
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == incoming.message.id)
        else {
            return;
        };
        self.messages[ix].apply_edit(incoming.message);
        // The list only remeasures rows it draws, so one edited off screen
        // would keep its old height; re-splicing it drops that.
        self.messages_list.splice(ix..ix + 1, 1);
        cx.notify();
    }
}
