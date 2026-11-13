//! A select menu: the box that shows what's picked, and the list it opens.
//!
//! gpui-component's `Select` picks one value, and Discord's pick up to 25, so
//! the list is drawn here. Picking the one value of a single select confirms
//! it at once; a multi-select confirms as it closes, as in Discord's client.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable, Sizable as _, h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::{Select, SelectKind};
use crate::screens::home::HomeScreen;
use crate::screens::home::components::Choice;
use crate::ui::depth::{self, radius};

use super::choice_icon;

const TRIGGER_HEIGHT: f32 = 40.;
const LIST_MAX_HEIGHT: f32 = 320.;
const PICKER_WIDTH: f32 = 400.;

#[derive(IntoElement)]
pub(in crate::screens::home) struct SelectTrigger {
    id: ElementId,
    label: Option<String>,
    placeholder: String,
    loading: bool,
    disabled: bool,
    open: bool,
}

impl SelectTrigger {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            placeholder: String::new(),
            loading: false,
            disabled: false,
            open: false,
        }
    }

    pub fn label(mut self, label: Option<String>) -> Self {
        self.label = label;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Selectable for SelectTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl RenderOnce for SelectTrigger {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let shown = self.label.is_some();
        h_flex()
            .id(self.id)
            .w_full()
            .min_w_0()
            .h(px(TRIGGER_HEIGHT))
            .px_3()
            .gap_2()
            .rounded(radius::ITEM)
            .bg(theme.secondary)
            .border_1()
            .border_color(if self.open { theme.ring } else { theme.border })
            .text_sm()
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|this| this.border_color(theme.muted_foreground))
            })
            .when(self.disabled && !self.loading, |this| this.opacity(0.5))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if shown {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(self.label.unwrap_or(self.placeholder)),
            )
            .child(if self.loading {
                Spinner::new().small().into_any_element()
            } else {
                Icon::new(IconName::ChevronDown)
                    .small()
                    .text_color(theme.muted_foreground)
                    .into_any_element()
            })
    }
}

pub(in crate::screens::home) enum PickerEvent {
    /// A single select's pick, which closes it.
    Done,
}

pub(in crate::screens::home) struct SelectPicker {
    screen: Entity<HomeScreen>,
    select: Select,
    picks: Vec<Choice>,
    /// Only the auto-populated kinds search; a string select lists its few options.
    search: Option<Entity<InputState>>,
    choices: Vec<Choice>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PickerEvent> for SelectPicker {}

impl SelectPicker {
    pub fn new(
        screen: Entity<HomeScreen>,
        select: Select,
        picks: Vec<Choice>,
        // Asked of the screen by the opener, which may be the screen mid-update.
        choices: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let search = (select.kind != SelectKind::Text).then(|| {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
            subscriptions.push(cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh(cx);
                }
            }));
            search.update(cx, |search, cx| search.focus(window, cx));
            // Members found by a search arrive later, on the screen.
            subscriptions.push(cx.observe(&screen, |this, _, cx| this.refresh(cx)));
            search
        });
        let mut picker = Self {
            screen,
            select,
            picks,
            search,
            choices: Vec::new(),
            _subscriptions: subscriptions,
        };
        picker.show(choices, true);
        picker
    }

    pub fn select(&self) -> &Select {
        &self.select
    }

    pub fn picks(&self) -> &[Choice] {
        &self.picks
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self
            .search
            .as_ref()
            .map(|search| search.read(cx).value().to_string())
            .unwrap_or_default();
        let select = self.select.clone();
        let found = self
            .screen
            .update(cx, |screen, _| screen.select_choices(&select, &query));
        if self.show(found, query.is_empty()) {
            cx.notify();
        }
    }

    /// Returns whether the list changed.
    fn show(&mut self, found: Vec<Choice>, unfiltered: bool) -> bool {
        // The picks lead an unfiltered list, so they can be unpicked though
        // the client no longer knows them as members.
        let mut choices: Vec<Choice> = if unfiltered && self.select.kind != SelectKind::Text {
            self.picks.clone()
        } else {
            Vec::new()
        };
        for choice in found {
            if !choices.iter().any(|known| known.value == choice.value) {
                choices.push(choice);
            }
        }
        if choices == self.choices {
            return false;
        }
        self.choices = choices;
        true
    }

    fn toggle(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if self.select.max_values <= 1 {
            self.picks = vec![choice];
            cx.emit(PickerEvent::Done);
        } else if let Some(ix) = self
            .picks
            .iter()
            .position(|pick| pick.value == choice.value)
        {
            self.picks.remove(ix);
        } else if self.picks.len() < self.select.max_values as usize {
            self.picks.push(choice);
        }
        cx.notify();
    }
}

impl Render for SelectPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let multi = self.select.max_values > 1;
        let full = self.picks.len() >= self.select.max_values as usize;
        let rows: Vec<_> = self
            .choices
            .iter()
            .enumerate()
            .map(|(ix, choice)| {
                let picked = self.picks.iter().any(|pick| pick.value == choice.value);
                let blocked = multi && full && !picked;
                let clicked = choice.clone();
                h_flex()
                    .id(("select-choice", ix))
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(px(6.))
                    .when(picked, |this| this.bg(theme.accent))
                    .when(!blocked, |this| {
                        this.cursor_pointer()
                            .hover(|this| this.bg(theme.accent.opacity(0.7)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle(clicked.clone(), cx);
                            }))
                    })
                    .when(blocked, |this| this.opacity(0.5))
                    .children(choice_icon(choice, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(choice.label.clone()))
                            .children(choice.description.clone().map(|description| {
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(description)
                            })),
                    )
                    .when(picked, |this| {
                        this.child(Icon::new(IconName::Check).small().text_color(theme.primary))
                    })
            })
            .collect();
        let empty = rows.is_empty();

        v_flex()
            .w(px(PICKER_WIDTH))
            .max_w(px(PICKER_WIDTH))
            .p_1()
            .gap_1()
            .bg(theme.popover)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CONTROL)
            .shadow_lg()
            .when_some(self.search.as_ref(), |this, search| {
                this.child(div().p_1().child(Input::new(search).small()))
            })
            .child(
                v_flex()
                    .id("select-choices")
                    .w_full()
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows)
                    .when(empty, |this| {
                        this.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("No results found"),
                        )
                    }),
            )
            .when(multi, |this| {
                this.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} of up to {} picked",
                            self.picks.len(),
                            self.select.max_values
                        )),
                )
            })
    }
}
