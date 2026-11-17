//! Composing a poll: a question, up to ten answers, how long it runs, and
//! whether more than one answer can be picked. Posting sends it as a message
//! of its own.

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Sizable as _, WindowExt as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    select::{Select, SelectState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::PollDraft;
use crate::screens::home::HomeScreen;
use crate::ui::button::Button;
use crate::ui::depth;

const DIALOG_WIDTH: f32 = 480.;

/// The lengths Discord's client offers, in hours.
const DURATIONS: [(&str, u32); 7] = [
    ("1 hour", 1),
    ("4 hours", 4),
    ("8 hours", 8),
    ("24 hours", 24),
    ("3 days", 72),
    ("1 week", 168),
    ("2 weeks", 336),
];
/// 24 hours, the client's default.
const DEFAULT_DURATION: usize = 3;

pub(in crate::screens::home) struct PollDialog {
    screen: WeakEntity<HomeScreen>,
    question: Entity<InputState>,
    answers: Vec<Entity<InputState>>,
    duration: Entity<SelectState<Vec<&'static str>>>,
    multiselect: bool,
    problem: Option<String>,
}

impl PollDialog {
    fn new(screen: WeakEntity<HomeScreen>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let question = cx
            .new(|cx| InputState::new(window, cx).placeholder("What question do you want to ask?"));
        question.update(cx, |input, cx| input.focus(window, cx));
        let answers = (0..2).map(|_| new_answer(window, cx)).collect();
        let duration = cx.new(|cx| {
            SelectState::new(
                DURATIONS
                    .iter()
                    .map(|(label, _)| *label)
                    .collect::<Vec<_>>(),
                Some(IndexPath::default().row(DEFAULT_DURATION)),
                window,
                cx,
            )
        });
        Self {
            screen,
            question,
            answers,
            duration,
            multiselect: false,
            problem: None,
        }
    }

    pub fn open(screen: WeakEntity<HomeScreen>, window: &mut Window, cx: &mut App) {
        let dialog = cx.new(|cx| Self::new(screen, window, cx));
        window.open_dialog(cx, move |frame, _, cx| {
            let post = dialog.clone();
            frame
                .border_color(depth::ring(cx))
                .w(px(DIALOG_WIDTH))
                .title("Create a Poll")
                .child(dialog.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("poll-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("poll-post").primary().label("Post").on_click(
                            move |_, window, cx| {
                                post.update(cx, |dialog, cx| dialog.post(window, cx));
                            },
                        )),
                )
        });
    }

    fn draft(&self, cx: &App) -> Result<PollDraft, String> {
        let question = self.question.read(cx).value().trim().to_owned();
        if question.is_empty() {
            return Err("Ask a question.".into());
        }
        if question.chars().count() > PollDraft::MAX_QUESTION {
            return Err(format!(
                "The question can be at most {} characters.",
                PollDraft::MAX_QUESTION
            ));
        }
        let answers: Vec<String> = self
            .answers
            .iter()
            .map(|answer| answer.read(cx).value().trim().to_owned())
            .filter(|answer| !answer.is_empty())
            .collect();
        if answers.is_empty() {
            return Err("Add at least one answer.".into());
        }
        if answers
            .iter()
            .any(|answer| answer.chars().count() > PollDraft::MAX_ANSWER)
        {
            return Err(format!(
                "Answers can be at most {} characters.",
                PollDraft::MAX_ANSWER
            ));
        }
        let duration = self
            .duration
            .read(cx)
            .selected_index(cx)
            .map_or(DEFAULT_DURATION, |ix| ix.row);
        Ok(PollDraft {
            question,
            answers,
            duration_hours: DURATIONS[duration.min(DURATIONS.len() - 1)].1,
            allow_multiselect: self.multiselect,
        })
    }

    fn post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err(problem) => {
                self.problem = Some(problem);
                cx.notify();
                return;
            }
        };
        window.close_dialog(cx);
        if let Some(screen) = self.screen.upgrade() {
            screen.update(cx, |screen, cx| screen.send_poll(draft, window, cx));
        }
    }

    fn add_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.answers.len() >= PollDraft::MAX_ANSWERS {
            return;
        }
        let answer = new_answer(window, cx);
        answer.update(cx, |input, cx| input.focus(window, cx));
        self.answers.push(answer);
        cx.notify();
    }

    fn remove_answer(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.answers.len() {
            self.answers.remove(ix);
            cx.notify();
        }
    }
}

fn new_answer(window: &mut Window, cx: &mut Context<PollDialog>) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("Type your answer"))
}

fn label(text: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

impl Render for PollDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // Two answers is the floor the form starts with; fewer would read as a broken poll.
        let removable = self.answers.len() > 2;
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Question"))
                    .child(Input::new(&self.question)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(label("Answers"))
                    .children(self.answers.iter().enumerate().map(|(ix, answer)| {
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(div().flex_1().child(Input::new(answer)))
                            .when(removable, |this| {
                                this.child(
                                    Button::new(("poll-remove-answer", ix))
                                        .icon(IconName::Close)
                                        .ghost()
                                        .small()
                                        .tooltip("Remove answer")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.remove_answer(ix, cx);
                                        })),
                                )
                            })
                    }))
                    .when(self.answers.len() < PollDraft::MAX_ANSWERS, |this| {
                        this.child(
                            h_flex().child(
                                Button::new("poll-add-answer")
                                    .icon(IconName::Plus)
                                    .ghost()
                                    .small()
                                    .label("Add Another Answer")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_answer(window, cx);
                                    })),
                            ),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_end()
                    .child(
                        v_flex()
                            .gap_1()
                            .w(px(180.))
                            .child(label("Duration"))
                            .child(Select::new(&self.duration)),
                    )
                    .child(
                        h_flex().h(px(32.)).items_center().child(
                            Checkbox::new("poll-multiselect")
                                .label("Allow multiple answers")
                                .checked(self.multiselect)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.multiselect = *checked;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .when_some(self.problem.clone(), |this, problem| {
                this.child(div().text_sm().text_color(theme.danger).child(problem))
            })
    }
}

impl HomeScreen {
    pub(in crate::screens::home) fn open_poll_creator(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        PollDialog::open(cx.entity().downgrade(), window, cx);
    }
}
