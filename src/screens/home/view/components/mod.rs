//! Widgets for message components shared by messages and modals: the select
//! menu's trigger and picker, and the modal form an app opens.

mod modal;
mod select;

pub(in crate::screens::home) use modal::ModalDialog;
pub(in crate::screens::home) use select::{PickerEvent, SelectPicker, SelectTrigger};

use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;

use crate::discord::ReactionEmoji;
use crate::screens::home::channels::channel_icon_path;
use crate::screens::home::components::{Choice, ChoiceIcon};
use crate::ui::avatar::avatar;
use crate::ui::twemoji;

/// What a select's trigger reads while something is picked.
pub(in crate::screens::home) fn picks_label(picks: &[Choice]) -> Option<String> {
    (!picks.is_empty()).then(|| {
        picks
            .iter()
            .map(|pick| pick.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    })
}

pub(in crate::screens::home) fn choice_emoji(
    emoji: &ReactionEmoji,
    size: Pixels,
    cx: &App,
) -> AnyElement {
    match (emoji.image_url(), emoji) {
        (Some(url), _) => img(url)
            .image_cache(&crate::ui::avatar::cache(cx))
            .size(size)
            .flex_shrink_0()
            .into_any_element(),
        (None, ReactionEmoji::Unicode(text)) => twemoji::glyph(text, size, &twemoji::cache(cx)),
        (None, _) => div().size(size).into_any_element(),
    }
}

fn choice_icon(choice: &Choice, cx: &App) -> Option<AnyElement> {
    let theme = cx.theme();
    Some(match &choice.icon {
        ChoiceIcon::None => return None,
        ChoiceIcon::Emoji(emoji) => choice_emoji(emoji, px(20.), cx),
        ChoiceIcon::Avatar(url) => {
            avatar(choice.label.clone(), url.clone(), px(20.)).into_any_element()
        }
        ChoiceIcon::Role(color) => div()
            .size(px(12.))
            .mx(px(4.))
            .flex_shrink_0()
            .rounded_full()
            .bg(color.map_or(theme.muted_foreground, |color| rgb(color).into()))
            .into_any_element(),
        ChoiceIcon::Channel(kind) => Icon::default()
            .path(channel_icon_path(*kind))
            .text_color(theme.muted_foreground)
            .into_any_element(),
    })
}
