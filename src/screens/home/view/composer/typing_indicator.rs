//! The "<name> is typing…" strip above the composer while others are typing.

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex};

use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Floats over the bottom of the conversation rather than taking a row of
    /// its own, so the message bar doesn't grow and shrink as people start and
    /// stop typing.
    pub(super) fn render_typing_indicator(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let names = self.typing_names()?;
        let theme = cx.theme();

        let name = |name: String| {
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(name)
        };
        let mut line = h_flex()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .whitespace_nowrap();
        line = match names.as_slice() {
            [one] => line.child(name(one.clone())).child("is typing…"),
            [first, second] => line
                .child(name(first.clone()))
                .child("and")
                .child(name(second.clone()))
                .child("are typing…"),
            [first, second, third] => line
                .child(div().flex().child(name(first.clone())).child(","))
                .child(div().flex().child(name(second.clone())).child(","))
                .child("and")
                .child(name(third.clone()))
                .child("are typing…"),
            _ => line.child("Several people are typing…"),
        };

        Some(
            h_flex()
                .absolute()
                .bottom_full()
                .left_2()
                .right_2()
                .h(px(24.))
                .px_3()
                .items_center()
                .overflow_hidden()
                // Masks the messages it sits over.
                .bg(theme.background)
                .child(line)
                .into_any_element(),
        )
    }

    /// The names of whoever else is typing in the open conversation, oldest
    /// first, or `None` when nobody is.
    fn typing_names(&self) -> Option<Vec<String>> {
        let channel_id = self.selected_channel?;
        let typists = self.typists.get(&channel_id)?;
        let names: Vec<String> = typists
            .iter()
            .map(|typist| {
                // A DM's dispatch carries no name, so it's taken from what the
                // typist has said in the conversation.
                typist
                    .name
                    .clone()
                    .or_else(|| {
                        self.messages
                            .iter()
                            .rev()
                            .find(|message| message.author_id == typist.user_id)
                            .map(|message| message.author_name.clone())
                    })
                    .unwrap_or_else(|| "Someone".to_owned())
            })
            .collect();
        (!names.is_empty()).then_some(names)
    }
}
