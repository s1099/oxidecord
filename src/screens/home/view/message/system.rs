//! A system message (a join, pin, boost, call, …) drawn as one line, its icon
//! in the avatar gutter.

use std::ops::Range;

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{MessageMarker, UserMarker},
};

use crate::discord::{self, SystemEvent};
use crate::screens::home::{HomeScreen, View};

use super::{AVATAR_SIZE, CONTENT_INDENT, MESSAGE_PADDING_Y};
use crate::screens::home::view::MESSAGE_PADDING_X;

/// The line box of `text_sm`, which the icon is centred on.
const LINE_HEIGHT: f32 = 22.;

enum Click {
    User(Id<UserMarker>),
    Message(Id<MessageMarker>),
}

/// The line's text and how each stretch of it is drawn.
#[derive(Default)]
struct Line {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    clicks: Vec<(Range<usize>, Click)>,
}

impl Line {
    fn push(&mut self, text: &str) -> Range<usize> {
        let start = self.text.len();
        self.text.push_str(text);
        start..self.text.len()
    }

    fn styled(&mut self, text: &str, style: HighlightStyle) -> Range<usize> {
        let range = self.push(text);
        self.highlights.push((range.clone(), style));
        range
    }
}

impl HomeScreen {
    pub(super) fn render_system_message(
        &self,
        message: &discord::Message,
        event: &SystemEvent,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let missed = self.missed_call(message, event);
        let (icon, color) = match event {
            SystemEvent::RecipientAdd { .. } | SystemEvent::Join { .. } => {
                ("icons/arrow-right.svg", theme.success)
            }
            SystemEvent::RecipientRemove { .. } => ("icons/arrow-left.svg", theme.danger),
            SystemEvent::Call { .. } if missed => ("icons/phone-off.svg", theme.danger),
            SystemEvent::Call { .. } => ("icons/phone.svg", theme.success),
            SystemEvent::ChannelNameChange { .. } | SystemEvent::StageTopic { .. } => {
                ("icons/pencil.svg", theme.muted_foreground)
            }
            SystemEvent::ChannelIconChange => ("icons/image.svg", theme.muted_foreground),
            SystemEvent::Pinned { .. } => ("icons/pin.svg", theme.muted_foreground),
            SystemEvent::Boost { .. } | SystemEvent::RoleSubscription { .. } => {
                ("icons/gem.svg", theme.primary)
            }
            SystemEvent::ChannelFollow { .. } => ("icons/megaphone.svg", theme.muted_foreground),
            SystemEvent::ThreadCreated { .. } => {
                ("icons/messages-square.svg", theme.muted_foreground)
            }
            SystemEvent::StageStart { .. }
            | SystemEvent::StageEnd { .. }
            | SystemEvent::StageSpeaker
            | SystemEvent::StageRaiseHand => ("icons/mic.svg", theme.muted_foreground),
            SystemEvent::InvitesPaused
            | SystemEvent::InvitesResumed
            | SystemEvent::RaidReported
            | SystemEvent::RaidFalseAlarm => ("icons/shield.svg", theme.warning),
            SystemEvent::Notice(_) => ("icons/flag.svg", theme.muted_foreground),
        };

        let line = self.system_line(message, event, missed, cx);
        let styled = StyledText::new(line.text).with_highlights(line.highlights);
        let (ranges, clicks): (Vec<_>, Vec<_>) = line.clicks.into_iter().unzip();
        let screen = cx.entity().downgrade();
        let mentions = message.mentions.clone();
        let (author_id, author_avatar) = (message.author_id, message.author_avatar_url.clone());
        let text = InteractiveText::new(("system-text", message.id.get()), styled).on_click(
            ranges,
            move |ix, window, cx| {
                let Some(screen) = screen.upgrade() else {
                    return;
                };
                screen.update(cx, |this, cx| match clicks[ix] {
                    Click::Message(id) => this.jump_to_message(id, window, cx),
                    Click::User(user_id) => {
                        let mention = mentions.iter().find(|user| user.id == user_id);
                        let avatar = match mention {
                            Some(user) => user.avatar_url.clone(),
                            None if user_id == author_id => author_avatar.clone(),
                            None => None,
                        };
                        let name = this
                            .author_name(user_id, mention.map_or("", |user| user.name.as_str()))
                            .to_owned();
                        this.open_profile(user_id, name, avatar, window.mouse_position(), cx);
                    }
                });
            },
        );

        h_flex()
            .w_full()
            .min_w_0()
            .items_start()
            .py(px(MESSAGE_PADDING_Y))
            .px(px(MESSAGE_PADDING_X))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .w(px(CONTENT_INDENT))
                    .h(px(LINE_HEIGHT))
                    .child(
                        h_flex()
                            .w(px(AVATAR_SIZE))
                            .justify_center()
                            .child(Icon::default().path(icon).small().text_color(color)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .text_sm()
                    .line_height(px(LINE_HEIGHT))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_color(theme.muted_foreground)
                            .child(text),
                    )
                    .when(!message.reactions.is_empty(), |this| {
                        this.child(self.render_reactions(message, cx))
                    }),
            )
            .into_any_element()
    }

    /// Whether a call notice reads as missed: it ended without the user in it.
    fn missed_call(&self, message: &discord::Message, event: &SystemEvent) -> bool {
        let SystemEvent::Call {
            participants,
            ended: Some(_),
        } = event
        else {
            return false;
        };
        self.self_user_id
            .is_some_and(|me| me != message.author_id && !participants.contains(&me))
    }

    /// The client's wording for the notice, followed by its send time.
    fn system_line(
        &self,
        message: &discord::Message,
        event: &SystemEvent,
        missed: bool,
        cx: &Context<Self>,
    ) -> Line {
        let theme = cx.theme();
        let strong = HighlightStyle {
            color: Some(theme.foreground),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let link = HighlightStyle {
            color: Some(theme.link),
            ..Default::default()
        };

        let mut line = Line::default();
        let user = |line: &mut Line, user_id: Id<UserMarker>| {
            let fallback = if user_id == message.author_id {
                message.author_name.as_str()
            } else {
                message
                    .mentions
                    .iter()
                    .find(|user| user.id == user_id)
                    .map_or("Unknown User", |user| user.name.as_str())
            };
            let style = HighlightStyle {
                color: self.role_color(user_id).map(|color| rgb(color).into()),
                ..strong
            };
            let range = line.styled(self.author_name(user_id, fallback), style);
            line.clicks.push((range, Click::User(user_id)));
        };
        let author = |line: &mut Line| user(line, message.author_id);
        let guild_name = || {
            self.selected_guild
                .filter(|_| self.view == View::Guild)
                .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
                .map_or("the server", |guild| guild.name.as_str())
        };

        match event {
            SystemEvent::RecipientAdd { target } => {
                author(&mut line);
                line.push(" added ");
                user(&mut line, *target);
                line.push(" to the group.");
            }
            SystemEvent::RecipientRemove { target } if *target == message.author_id => {
                author(&mut line);
                line.push(" left the group.");
            }
            SystemEvent::RecipientRemove { target } => {
                author(&mut line);
                line.push(" removed ");
                user(&mut line, *target);
                line.push(" from the group.");
            }
            SystemEvent::Call { ended, .. } => {
                let lasted = ended.map(|ended| discord::call_duration(ended - message.timestamp));
                if missed {
                    line.push("You missed a call from ");
                    author(&mut line);
                } else {
                    author(&mut line);
                    line.push(" started a call");
                }
                match lasted {
                    Some(lasted) => line.push(&format!(" that lasted {lasted}.")),
                    None => line.push("."),
                };
            }
            SystemEvent::ChannelNameChange { name } if name.is_empty() => {
                author(&mut line);
                line.push(" removed the channel name.");
            }
            SystemEvent::ChannelNameChange { name } => {
                author(&mut line);
                line.push(" changed the channel name: ");
                line.styled(name, strong);
            }
            SystemEvent::ChannelIconChange => {
                author(&mut line);
                line.push(" changed the channel icon.");
            }
            SystemEvent::Pinned { message: pinned } => {
                author(&mut line);
                line.push(" pinned ");
                match pinned {
                    Some(id) => {
                        let range = line.styled("a message", link);
                        line.clicks.push((range, Click::Message(*id)));
                    }
                    None => {
                        line.push("a message");
                    }
                }
                line.push(" to this channel.");
            }
            SystemEvent::Join { welcome } => {
                let (before, after) = discord::WELCOMES[*welcome];
                line.push(before);
                author(&mut line);
                line.push(after);
            }
            SystemEvent::Boost { count, level } => {
                author(&mut line);
                line.push(" just boosted the server");
                if let Some(count) = count {
                    line.push(" ");
                    line.styled(&count.to_string(), strong);
                    line.push(" times");
                }
                line.push("!");
                if let Some(level) = level {
                    line.push(" ");
                    line.styled(guild_name(), strong);
                    line.push(" has achieved ");
                    line.styled(&format!("Level {level}!"), strong);
                }
            }
            SystemEvent::ChannelFollow { channel } => {
                author(&mut line);
                line.push(" has added ");
                line.styled(channel, strong);
                line.push(" to this channel. Its most important updates will show up here.");
            }
            SystemEvent::ThreadCreated { name } => {
                author(&mut line);
                line.push(" started a thread: ");
                line.styled(name, strong);
                line.push(".");
            }
            SystemEvent::StageStart { topic } => {
                author(&mut line);
                line.push(" started ");
                line.styled(topic, strong);
            }
            SystemEvent::StageEnd { topic } => {
                author(&mut line);
                line.push(" ended ");
                line.styled(topic, strong);
            }
            SystemEvent::StageSpeaker => {
                author(&mut line);
                line.push(" is now a speaker.");
            }
            SystemEvent::StageRaiseHand => {
                author(&mut line);
                line.push(" requested to speak.");
            }
            SystemEvent::StageTopic { topic } => {
                author(&mut line);
                line.push(" changed the Stage topic: ");
                line.styled(topic, strong);
            }
            SystemEvent::RoleSubscription {
                tier,
                months,
                renewal,
            } => {
                author(&mut line);
                line.push(if *renewal { " renewed " } else { " joined " });
                line.styled(tier, strong);
                line.push(" and has been a subscriber of ");
                line.styled(guild_name(), strong);
                let unit = if *months == 1 { "month" } else { "months" };
                line.push(&format!(" for {months} {unit}!"));
            }
            SystemEvent::InvitesPaused => {
                author(&mut line);
                line.push(" turned on security actions for this server.");
            }
            SystemEvent::InvitesResumed => {
                author(&mut line);
                line.push(" turned off security actions for this server.");
            }
            SystemEvent::RaidReported => {
                author(&mut line);
                line.push(" reported a raid in ");
                line.styled(guild_name(), strong);
                line.push(".");
            }
            SystemEvent::RaidFalseAlarm => {
                author(&mut line);
                line.push(" reported a false alarm in ");
                line.styled(guild_name(), strong);
                line.push(".");
            }
            SystemEvent::Notice(text) => {
                line.push(text);
            }
        }

        line.push("  ");
        line.styled(
            &message.timestamp_label(),
            HighlightStyle {
                color: Some(theme.muted_foreground.opacity(0.8)),
                ..Default::default()
            },
        );
        line
    }
}
