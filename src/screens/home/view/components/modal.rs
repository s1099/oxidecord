//! A form an app opened in answer to an interaction: its fields, checked as
//! Discord's client checks them, then submitted as an interaction of its own.
//! The dialog stays open, its submit button spinning, until the gateway says
//! how the app took it.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::{self, InputKind, Modal, ModalAnswer, ModalEntry, ModalField, SelectKind};
use crate::screens::home::HomeScreen;
use crate::screens::home::components::Choice;
use crate::screens::home::data::messages::next_nonce;
use crate::ui::avatar::avatar;
use crate::ui::button::Button;
use crate::ui::depth;

use super::picks_label;
use super::select::{PickerEvent, SelectPicker, SelectTrigger};

const DIALOG_WIDTH: f32 = 480.;
/// Discord's client fails a submission the app hasn't answered by then.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(10);
const PARAGRAPH_ROWS: (usize, usize) = (3, 8);

enum FieldState {
    Display,
    Short(Entity<InputState>),
    Paragraph(Entity<TextareaState>),
    Select(Vec<Choice>),
    Checkbox(bool),
    Group(Vec<String>),
    Upload,
}

pub(in crate::screens::home) struct ModalDialog {
    screen: Entity<HomeScreen>,
    modal: Modal,
    guild_id: Option<Id<GuildMarker>>,
    fields: Vec<FieldState>,
    /// Why each field was refused, by index.
    problems: Vec<Option<String>>,
    submitting: bool,
    error: Option<String>,
    open_select: Option<(usize, Entity<SelectPicker>, Subscription)>,
    _watchdog: Option<Task<()>>,
}

impl ModalDialog {
    pub fn new(
        screen: Entity<HomeScreen>,
        modal: Modal,
        guild_id: Option<Id<GuildMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut first_input = true;
        let fields = modal
            .fields
            .iter()
            .map(|field| {
                let ModalField::Input(input) = field else {
                    return FieldState::Display;
                };
                match &input.kind {
                    InputKind::Text {
                        paragraph,
                        placeholder,
                        value,
                        ..
                    } => {
                        let placeholder = placeholder.clone().unwrap_or_default();
                        let value = value.clone().unwrap_or_default();
                        let focus = std::mem::take(&mut first_input);
                        if *paragraph {
                            let state = cx.new(|cx| {
                                TextareaState::new(window, cx)
                                    .auto_grow(PARAGRAPH_ROWS.0, PARAGRAPH_ROWS.1)
                                    .placeholder(placeholder)
                                    .default_value(value)
                            });
                            if focus {
                                state.update(cx, |state, cx| state.focus(window, cx));
                            }
                            FieldState::Paragraph(state)
                        } else {
                            let state = cx.new(|cx| {
                                InputState::new(window, cx)
                                    .placeholder(placeholder)
                                    .default_value(value)
                            });
                            if focus {
                                state.update(cx, |state, cx| state.focus(window, cx));
                            }
                            FieldState::Short(state)
                        }
                    }
                    // Defaults resolve against what the screen knows, which it
                    // can't be asked mid-update; the picks start empty but for
                    // a string select's own defaults.
                    InputKind::Select(select) => FieldState::Select(
                        select
                            .options
                            .iter()
                            .filter(|option| option.default)
                            .map(|option| Choice {
                                value: option.value.clone(),
                                label: option.label.clone(),
                                description: option.description.clone(),
                                icon: crate::screens::home::components::ChoiceIcon::None,
                            })
                            .collect(),
                    ),
                    InputKind::Checkbox { default, .. } => FieldState::Checkbox(*default),
                    InputKind::CheckboxGroup { options, .. } => FieldState::Group(
                        options
                            .iter()
                            .filter(|option| option.default)
                            .map(|option| option.value.clone())
                            .collect(),
                    ),
                    InputKind::FileUpload { .. } => FieldState::Upload,
                }
            })
            .collect::<Vec<_>>();
        let problems = vec![None; fields.len()];
        Self {
            screen,
            modal,
            guild_id,
            fields,
            problems,
            submitting: false,
            error: None,
            open_select: None,
            _watchdog: None,
        }
    }

    /// Fills in an auto-populated select's defaults once the screen is free.
    fn resolve_defaults(&mut self, cx: &mut Context<Self>) {
        let screen = self.screen.read(cx);
        for (field, state) in self.modal.fields.iter().zip(&mut self.fields) {
            if let (ModalField::Input(input), FieldState::Select(picks)) = (field, state)
                && let InputKind::Select(select) = &input.kind
                && select.kind != SelectKind::Text
                && picks.is_empty()
            {
                *picks = screen.default_choices(select);
            }
        }
    }

    pub fn open(dialog: Entity<Self>, window: &mut Window, cx: &mut App) {
        let screen = dialog.read(cx).screen.clone();
        let dialog_for_defaults = dialog.clone();
        window.defer(cx, move |_, cx| {
            dialog_for_defaults.update(cx, |dialog, cx| dialog.resolve_defaults(cx));
        });
        window.open_dialog(cx, move |frame, _, cx| {
            let state = dialog.read(cx);
            let title = state.modal.title.clone();
            let app_name = state.modal.app_name.clone();
            let icon = state.modal.app_icon_url.clone();
            let submitting = state.submitting;
            let blocked = state.blocked();
            let submit = dialog.clone();
            let cancel = dialog.clone();
            let screen = screen.clone();
            frame
                .overlay_closable(false)
                .border_color(depth::ring(cx))
                .w(px(DIALOG_WIDTH))
                .title(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(avatar(app_name.clone(), icon, px(24.)))
                        .child(div().min_w_0().truncate().child(title)),
                )
                .on_close(move |_, _, cx| {
                    screen.update(cx, |screen, _| screen.forget_modal());
                })
                .child(dialog.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("modal-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(move |_, window, cx| {
                                    cancel.update(cx, |dialog, cx| dialog.cancel(window, cx));
                                }),
                        )
                        .child(
                            Button::new("modal-submit")
                                .primary()
                                .label("Submit")
                                .loading(submitting)
                                .disabled(blocked)
                                .on_click(move |_, _, cx| {
                                    submit.update(cx, |dialog, cx| dialog.submit(cx));
                                }),
                        ),
                )
        });
    }

    /// A required upload can't be answered here.
    fn blocked(&self) -> bool {
        self.modal.fields.iter().any(|field| {
            matches!(
                field,
                ModalField::Input(input)
                    if matches!(input.kind, InputKind::FileUpload { required: true, .. })
            )
        })
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen.update(cx, |screen, _| screen.forget_modal());
        window.close_dialog(cx);
    }

    /// Shown when the app turns the submission down or never answers.
    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.submitting = false;
        self.error = Some(error);
        self._watchdog = None;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.submitting || self.blocked() {
            return;
        }
        let mut entries = Vec::new();
        let mut valid = true;
        for (ix, (field, state)) in self.modal.fields.iter().zip(&self.fields).enumerate() {
            let ModalField::Input(input) = field else {
                entries.push(ModalEntry::Display);
                continue;
            };
            let (answer, problem) = answer(&input.kind, state, cx);
            valid &= problem.is_none();
            self.problems[ix] = problem;
            entries.push(ModalEntry::Field(Box::new(input.clone()), answer));
        }
        if !valid {
            cx.notify();
            return;
        }
        let session_id = self.screen.read(cx).gateway_session.clone();
        let Some(session_id) = session_id else {
            self.error = Some("Not connected to Discord yet.".into());
            cx.notify();
            return;
        };
        let (nonce, _) = next_nonce();
        let submission = discord::ModalSubmission {
            application_id: self.modal.application_id.clone(),
            guild_id: self.guild_id,
            channel_id: self.modal.channel_id,
            session_id,
            nonce,
            modal_id: self.modal.id.clone(),
            custom_id: self.modal.custom_id.clone(),
            entries,
        };
        self.screen
            .update(cx, |screen, _| screen.modal_submitted(nonce));
        self.submitting = true;
        self.error = None;
        self._watchdog = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SUBMIT_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                this.failed("Something went wrong. Try again.".into(), cx);
            });
        }));
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Err(err) = discord::submit_modal(submission).await {
                eprintln!("modal submission failed: {err}");
                let _ = this.update(cx, |this, cx| {
                    this.failed("Something went wrong. Try again.".into(), cx);
                });
            }
        })
        .detach();
    }

    fn set_select_open(
        &mut self,
        ix: usize,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !open {
            self.close_select(cx);
            return;
        }
        let (Some(ModalField::Input(input)), Some(FieldState::Select(picks))) =
            (self.modal.fields.get(ix), self.fields.get(ix))
        else {
            return;
        };
        let InputKind::Select(select) = &input.kind else {
            return;
        };
        let select = select.clone();
        let picks = picks.clone();
        let choices = self
            .screen
            .update(cx, |screen, _| screen.select_choices(&select, ""));
        let screen = self.screen.clone();
        let picker = cx.new(|cx| SelectPicker::new(screen, select, picks, choices, window, cx));
        let subscription = cx.subscribe(&picker, |this, _, event: &PickerEvent, cx| {
            let PickerEvent::Done = event;
            this.close_select(cx);
        });
        self.open_select = Some((ix, picker, subscription));
        cx.notify();
    }

    fn close_select(&mut self, cx: &mut Context<Self>) {
        let Some((ix, picker, _)) = self.open_select.take() else {
            return;
        };
        let picks = picker.read(cx).picks().to_vec();
        if let Some(FieldState::Select(field)) = self.fields.get_mut(ix) {
            *field = picks;
        }
        self.problems[ix] = None;
        cx.notify();
    }

    fn render_input(
        &self,
        ix: usize,
        kind: &InputKind,
        state: &FieldState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        match (kind, state) {
            (_, FieldState::Short(input)) => Input::new(input).into_any_element(),
            (_, FieldState::Paragraph(input)) => Textarea::new(input).into_any_element(),
            (InputKind::Select(select), FieldState::Select(picks)) => {
                let id = ElementId::from(("modal-select", ix));
                let picker = self
                    .open_select
                    .as_ref()
                    .filter(|(open, _, _)| *open == ix)
                    .map(|(_, picker, _)| picker.clone());
                let trigger = SelectTrigger::new(id.clone())
                    .label(picks_label(picks))
                    .placeholder(
                        select
                            .placeholder
                            .clone()
                            .unwrap_or_else(|| "Make a selection".into()),
                    )
                    .disabled(select.disabled);
                if select.disabled {
                    return trigger.into_any_element();
                }
                Popover::new(id)
                    .appearance(false)
                    .open(picker.is_some())
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change(cx.listener(move |this, open: &bool, window, cx| {
                        this.set_select_open(ix, *open, window, cx);
                    }))
                    .trigger(trigger)
                    .content(move |_, _, _| div().children(picker.clone()))
                    .into_any_element()
            }
            (_, FieldState::Checkbox(checked)) => Checkbox::new(("modal-checkbox", ix))
                .checked(*checked)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    if let Some(FieldState::Checkbox(field)) = this.fields.get_mut(ix) {
                        *field = *checked;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            (InputKind::CheckboxGroup { options, .. }, FieldState::Group(picked)) => v_flex()
                .gap_2()
                .children(options.iter().enumerate().map(|(option_ix, option)| {
                    let value = option.value.clone();
                    Checkbox::new(("modal-group", ix * 100 + option_ix))
                        .label(option.label.clone())
                        .checked(picked.contains(&option.value))
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if let Some(FieldState::Group(field)) = this.fields.get_mut(ix) {
                                field.retain(|picked| *picked != value);
                                if *checked {
                                    field.push(value.clone());
                                }
                            }
                            this.problems[ix] = None;
                            cx.notify();
                        }))
                }))
                .into_any_element(),
            _ => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Uploading files isn't supported here.")
                .into_any_element(),
        }
    }
}

/// A field's value as submitted, and what's wrong with it, if anything.
fn answer(kind: &InputKind, state: &FieldState, cx: &App) -> (ModalAnswer, Option<String>) {
    const REQUIRED: &str = "This field is required";
    match (kind, state) {
        (
            InputKind::Text {
                required,
                min_length,
                max_length,
                ..
            },
            FieldState::Short(_) | FieldState::Paragraph(_),
        ) => {
            let text = match state {
                FieldState::Short(input) => input.read(cx).value().to_string(),
                FieldState::Paragraph(input) => input.read(cx).value().to_string(),
                _ => String::new(),
            };
            let length = text.chars().count();
            let problem = if text.trim().is_empty() {
                required.then(|| REQUIRED.to_string())
            } else if min_length.is_some_and(|min| length < min as usize) {
                Some(format!(
                    "Must be at least {} characters long",
                    min_length.unwrap_or_default()
                ))
            } else if max_length.is_some_and(|max| length > max as usize) {
                Some(format!(
                    "Must be at most {} characters long",
                    max_length.unwrap_or_default()
                ))
            } else {
                None
            };
            let value = (!text.is_empty()).then_some(text);
            (ModalAnswer::Text(value), problem)
        }
        (InputKind::Select(select), FieldState::Select(picks)) => {
            let problem = if picks.is_empty() {
                (select.required && select.min_values > 0).then(|| REQUIRED.to_string())
            } else if picks.len() < select.min_values as usize {
                Some(format!("Pick at least {}", select.min_values))
            } else {
                None
            };
            let values =
                (!picks.is_empty()).then(|| picks.iter().map(|pick| pick.value.clone()).collect());
            (ModalAnswer::Values(values), problem)
        }
        (_, FieldState::Checkbox(checked)) => (ModalAnswer::Checked(*checked), None),
        (
            InputKind::CheckboxGroup {
                min_values,
                max_values,
                required,
                ..
            },
            FieldState::Group(picked),
        ) => {
            let problem = if picked.is_empty() && *required && *min_values > 0 {
                Some(REQUIRED.to_string())
            } else if !picked.is_empty() && picked.len() < *min_values as usize {
                Some(format!("Pick at least {min_values}"))
            } else if picked.len() > *max_values as usize {
                Some(format!("Pick at most {max_values}"))
            } else {
                None
            };
            let values = (!picked.is_empty()).then(|| picked.clone());
            (ModalAnswer::Values(values), problem)
        }
        _ => (ModalAnswer::Values(None), None),
    }
}

impl Render for ModalDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut rows = Vec::new();
        for (ix, field) in self.modal.fields.iter().enumerate() {
            match field {
                ModalField::Text(text) => rows.push(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .child(text.clone())
                        .into_any_element(),
                ),
                ModalField::Input(input) => {
                    let required = match &input.kind {
                        InputKind::Text { required, .. }
                        | InputKind::CheckboxGroup { required, .. }
                        | InputKind::FileUpload { required, .. } => *required,
                        InputKind::Select(select) => select.required && select.min_values > 0,
                        InputKind::Checkbox { .. } => false,
                    };
                    let control = self.render_input(ix, &input.kind, &self.fields[ix], cx);
                    rows.push(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(input.label.clone())
                                    .when(required, |this| {
                                        this.child(div().text_color(theme.danger).child("*"))
                                    }),
                            )
                            .children(input.description.clone().map(|description| {
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(description)
                            }))
                            .child(control)
                            .children(self.problems[ix].clone().map(|problem| {
                                div().text_xs().text_color(theme.danger).child(problem)
                            }))
                            .into_any_element(),
                    );
                }
            }
        }
        v_flex()
            .w_full()
            .gap_4()
            .children(rows)
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "This form will be submitted to {}. Do not share passwords or other sensitive information.",
                        self.modal.app_name
                    )),
            )
    }
}
