//! Editing a message in place: opening one for editing, saving it (applied
//! locally first and rolled back if the request fails), and taking edits made
//! elsewhere as they arrive over the gateway.

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::button::ButtonVariant;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::input::{InputState, Position};
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::{EditLastMessage, EditingMessage, HomeScreen};
use crate::ui::depth;

/// How many lines the edit box grows to before it scrolls.
const EDIT_MAX_ROWS: usize = 16;

impl HomeScreen {
    /// Whether the current user wrote `message`. Discord only ever lets the
    /// author edit, whatever else they may manage in the channel.
    pub(in crate::screens::home) fn is_own_message(&self, message: &discord::Message) -> bool {
        self.current_user
            .as_ref()
            .map(|user| user.id)
            .or(self.self_user_id)
            == Some(message.author_id)
    }

    /// Swaps the message's text for an edit box holding it, with the cursor at
    /// the end, ready to type.
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
        if !self.is_own_message(message) {
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
            InputState::new(window, cx)
                .auto_grow(1, EDIT_MAX_ROWS)
                .default_value(content)
        });
        input.update(cx, |input, cx| input.set_cursor_position(end, window, cx));

        self.editing = Some(EditingMessage { message_id, input });
        // Up in the composer can reach a message scrolled out of view.
        self.messages_list.scroll_to_reveal_item(ix);
        cx.notify();
    }

    /// Closes the edit box without saving, handing focus back to the composer.
    pub(in crate::screens::home) fn cancel_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.take().is_none() {
            return;
        }
        self.message_input.focus_handle(cx).focus(window);
        cx.notify();
    }

    /// Saves the edit box's text over the message.
    ///
    /// Unchanged text just closes the box. Clearing the text of a message with
    /// nothing else in it would leave it empty, which Discord treats as a
    /// request to delete it, so that asks first.
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
        let content = editing.input.read(cx).value().trim().to_string();
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
        if content.is_empty() && message.images.is_empty() && message.videos.is_empty() {
            return self.confirm_delete_from_edit(message_id, window, cx);
        }

        // Show the new text now, as Discord does; the request usually
        // succeeds, and the box closing onto the old text would read as the
        // edit being lost.
        let previous = (message.content.clone(), message.edited);
        message.content = content.clone();
        message.edited = true;
        self.editing = None;
        self.send_error = None;
        self.message_input.focus_handle(cx).focus(window);
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
                        (message.content, message.edited) = previous;
                        this.send_error = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks whether to delete a message whose text was cleared in the edit box.
    /// Declining leaves the box open to carry on editing.
    fn confirm_delete_from_edit(
        &mut self,
        message_id: Id<MessageMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let screen = screen.clone();
            dialog
                .confirm()
                .border_color(depth::ring(cx))
                .w(px(440.))
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

    /// Up in an empty composer opens the user's latest message in the
    /// conversation for editing. Anywhere else, the arrow moves the cursor as
    /// usual.
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
            .find(|message| self.is_own_message(message))
            .map(|message| message.id)
        else {
            cx.propagate();
            return;
        };
        self.start_editing(message_id, window, cx);
    }

    /// Takes an edit to a message in the open conversation, made here or
    /// anywhere else.
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
