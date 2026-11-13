//! Slash commands in the composer: fetching the indexes that list them,
//! suggesting commands, options and values while one is typed, and running it
//! as an interaction whose progress the gateway reports.
//!
//! As in Discord's client, a guild's index and the user's own apps are fetched
//! when a `/` is first typed in the conversation, and kept until the session
//! ends or Discord says they changed.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{
    self, AppCommand, CommandArgument, CommandChoice, CommandContext, CommandIndexScope,
    CommandInvocation, CommandSource, OptionKind, Permissions, SlashCommand,
};
use crate::screens::home::autocomplete::{Suggestion, Trigger, best, match_rank, resolve_mentions};
use crate::screens::home::commands::{
    ChoicesKey, CommandCursor, IndexState, PendingCommand, boolean_choices, command_cursor,
    convert_value, match_command, parse_options,
};
use crate::screens::home::{HomeScreen, View};

use super::messages::next_nonce;

/// Discord's client waits for a pause in typing before asking an app for choices.
const CHOICES_DEBOUNCE: Duration = Duration::from_millis(300);

/// How long a stand-in outlives the app answering, for a reply that arrives
/// without the nonce to match it by.
const SETTLE_AFTER_SUCCESS: Duration = Duration::from_secs(5);

static BUILTINS: LazyLock<Vec<Arc<SlashCommand>>> =
    LazyLock::new(|| SlashCommand::builtins().map(Arc::new).collect());

pub(in crate::screens::home) enum CommandSend {
    NotCommand,
    /// Run, or refused with `send_error` set.
    Handled,
    /// A built-in's text, sent as an ordinary message.
    Text(String),
}

/// What the open conversation is, as commands see it.
struct Conversation {
    channel_id: Id<ChannelMarker>,
    guild_id: Option<Id<GuildMarker>>,
    /// A one-on-one DM with a bot, which serves that bot's commands.
    bot_dm: Option<Id<twilight_model::id::marker::UserMarker>>,
}

impl HomeScreen {
    fn conversation(&self) -> Option<Conversation> {
        let channel_id = self.selected_channel?;
        if self.view == View::Guild {
            return Some(Conversation {
                channel_id,
                guild_id: self.selected_guild,
                bot_dm: None,
            });
        }
        let dm = self.selected_dm_info();
        Some(Conversation {
            channel_id,
            guild_id: None,
            bot_dm: dm
                .filter(|dm| dm.recipient_bot)
                .and_then(|dm| dm.recipient_id),
        })
    }

    fn command_scopes(conversation: &Conversation) -> Vec<CommandIndexScope> {
        let contextual = match (conversation.guild_id, conversation.bot_dm) {
            (Some(guild_id), _) => Some(CommandIndexScope::Guild(guild_id)),
            (None, Some(_)) => Some(CommandIndexScope::Channel(conversation.channel_id)),
            // Discord's client asks a group DM's index for nothing.
            (None, None) => None,
        };
        contextual
            .into_iter()
            .chain([CommandIndexScope::User])
            .collect()
    }

    /// The conversation's app commands the user may run, then the built-ins.
    pub(in crate::screens::home) fn available_commands(&self) -> Vec<Arc<SlashCommand>> {
        let Some(conversation) = self.conversation() else {
            return BUILTINS.clone();
        };
        let permissions = conversation.guild_id.and_then(|guild_id| {
            let guild = self.guilds.iter().find(|guild| guild.id == guild_id)?;
            Some(if guild.owner {
                Permissions::all()
            } else {
                guild.permissions
            })
        });
        let mut seen = HashSet::new();
        let mut commands = Vec::new();
        for scope in Self::command_scopes(&conversation) {
            let Some(IndexState::Loaded(index)) = self.command_indexes.get(&scope) else {
                continue;
            };
            for command in &index.commands {
                let context = match (conversation.guild_id, conversation.bot_dm) {
                    (Some(_), _) => CommandContext::Guild,
                    (None, Some(bot)) if command.app().and_then(|app| app.bot_id) == Some(bot) => {
                        CommandContext::BotDm
                    }
                    (None, Some(_)) if scope != CommandIndexScope::User => CommandContext::BotDm,
                    _ => CommandContext::PrivateChannel,
                };
                // NSFW channels aren't tracked, so age-restricted commands stay hidden.
                if command.usable(context, conversation.channel_id, permissions, false)
                    && seen.insert(command.key())
                {
                    commands.push(command.clone());
                }
            }
        }
        commands.extend(BUILTINS.iter().cloned());
        commands
    }

    fn command_indexes_loading(&self) -> bool {
        self.conversation().is_some_and(|conversation| {
            Self::command_scopes(&conversation)
                .iter()
                .any(|scope| matches!(self.command_indexes.get(scope), Some(IndexState::Loading)))
        })
    }

    fn ensure_command_indexes(&mut self, cx: &mut Context<Self>) {
        let Some(conversation) = self.conversation() else {
            return;
        };
        for scope in Self::command_scopes(&conversation) {
            if self.command_indexes.contains_key(&scope) {
                continue;
            }
            self.command_indexes.insert(scope, IndexState::Loading);
            cx.spawn(async move |this, cx| {
                let result = discord::fetch_command_index(scope).await;
                let _ = this.update(cx, |this, cx| {
                    // Dropped meanwhile by a new session or an index update, which
                    // makes this answer stale.
                    if !matches!(this.command_indexes.get(&scope), Some(IndexState::Loading)) {
                        return;
                    }
                    let state = match result {
                        Ok(index) => IndexState::Loaded(Arc::new(index)),
                        Err(err) => {
                            eprintln!("failed to fetch the command index {scope:?}: {err}");
                            IndexState::Failed
                        }
                    };
                    this.command_indexes.insert(scope, state);
                    this.refresh_autocomplete(cx);
                });
            })
            .detach();
        }
    }

    /// Suggestions while the composer holds a command: `None` when it doesn't.
    pub(in crate::screens::home) fn command_suggestions(
        &mut self,
        text: &str,
        cursor: usize,
        cx: &mut Context<Self>,
    ) -> Option<(Trigger, Range<usize>, String, Vec<Suggestion>)> {
        if !text.starts_with('/') {
            return None;
        }
        self.ensure_command_indexes(cx);
        let commands = self.available_commands();
        let matched = match_command(text, &commands);
        let cursor_in = command_cursor(
            text,
            cursor,
            matched.map(|(index, start)| (&*commands[index], start)),
        )?;
        match cursor_in {
            CommandCursor::Name { range } => {
                let query = &text[1..range.end];
                let lowered = query.to_lowercase();
                let items = best(commands.iter().filter_map(|command| {
                    Some((
                        match_rank(&command.display_name, &lowered)?,
                        Suggestion::Command(command.clone()),
                    ))
                }));
                Some((Trigger::Command, range, query.to_string(), items))
            }
            CommandCursor::OptionName { range } => {
                let (index, args_start) = matched?;
                let command = &commands[index];
                let filled: Vec<_> = parse_options(text, args_start, &command.options)
                    .into_iter()
                    .filter(|parsed| !parsed.value.is_empty() && parsed.span.end != range.end)
                    .map(|parsed| parsed.option)
                    .collect();
                let query = text[range.clone()].to_lowercase();
                // An empty word only prompts while something required is missing,
                // so Enter still sends a command whose options are all optional.
                let missing_required = command
                    .options
                    .iter()
                    .enumerate()
                    .any(|(i, option)| option.required && !filled.contains(&i));
                if query.is_empty() && !missing_required {
                    return None;
                }
                let items = best(
                    command
                        .options
                        .iter()
                        .enumerate()
                        .filter_map(|(i, option)| {
                            if filled.contains(&i) {
                                return None;
                            }
                            let (rank, len) = match_rank(&option.display_name, &query)?;
                            Some((
                                (rank + if option.required { 0 } else { 3 }, len),
                                Suggestion::CommandOption {
                                    name: option.display_name.clone(),
                                    description: option.description.clone(),
                                    required: option.required,
                                    kind: option.kind,
                                },
                            ))
                        }),
                );
                Some((Trigger::CommandOption, range, query, items))
            }
            CommandCursor::Value {
                option,
                range,
                query_end,
            } => {
                let (index, args_start) = matched?;
                let command = commands[index].clone();
                let query = text[range.start..query_end].trim().to_string();
                let items =
                    self.command_value_suggestions(&command, option, &query, text, args_start, cx);
                Some((Trigger::CommandValue, range, query, items))
            }
        }
    }

    fn command_value_suggestions(
        &mut self,
        command: &Arc<SlashCommand>,
        option: usize,
        query: &str,
        text: &str,
        args_start: usize,
        cx: &mut Context<Self>,
    ) -> Vec<Suggestion> {
        let spec = &command.options[option];
        let lowered = query.to_lowercase();
        let choices = |choices: &[CommandChoice]| {
            best(choices.iter().filter_map(|choice| {
                Some((
                    match_rank(&choice.name, &lowered)?,
                    Suggestion::Choice {
                        name: choice.name.clone(),
                    },
                ))
            }))
        };
        if !spec.choices.is_empty() {
            return choices(&spec.choices);
        }
        if spec.autocomplete {
            return match self.app_choices(command, option, query, text, args_start, cx) {
                Some(answer) => answer
                    .into_iter()
                    .map(|choice| Suggestion::Choice { name: choice.name })
                    .collect(),
                // Asked and waiting: keep what's showing rather than flicker.
                None => self
                    .autocomplete
                    .as_ref()
                    .filter(|autocomplete| autocomplete.trigger == Trigger::CommandValue)
                    .map(|autocomplete| autocomplete.items.clone())
                    .unwrap_or_default(),
            };
        }
        match spec.kind {
            OptionKind::Boolean => choices(&boolean_choices()),
            OptionKind::User | OptionKind::Mentionable | OptionKind::Role => {
                let name = lowered.trim_start_matches('@');
                if spec.kind != OptionKind::Role {
                    self.search_guild_members(name);
                }
                self.user_suggestions(name)
                    .into_iter()
                    .filter(|suggestion| {
                        !matches!(
                            (spec.kind, suggestion),
                            (_, Suggestion::Broadcast(_))
                                | (OptionKind::User, Suggestion::Role { .. })
                                | (OptionKind::Role, Suggestion::User { .. })
                        )
                    })
                    .collect()
            }
            OptionKind::Channel => self.channel_suggestions(lowered.trim_start_matches('#')),
            _ => Vec::new(),
        }
    }

    /// The app's choices for what's typed: cached, or asked for after a pause.
    /// `None` while the answer is awaited.
    fn app_choices(
        &mut self,
        command: &Arc<SlashCommand>,
        option: usize,
        query: &str,
        text: &str,
        args_start: usize,
        cx: &mut Context<Self>,
    ) -> Option<Vec<CommandChoice>> {
        let CommandSource::App(app_command) = &command.source else {
            return Some(Vec::new());
        };
        let spec = &command.options[option];
        let conversation = self.conversation()?;
        let mut arguments = self.lenient_arguments(command, text, args_start, option);
        let context = arguments
            .iter()
            .map(|argument| format!("{}={}\0", argument.name, argument.value))
            .collect();
        let key = ChoicesKey {
            command: command.key(),
            option: spec.name.clone(),
            query: query.to_string(),
            context,
        };
        if let Some(cached) = self.command_choices.cache.get(&key) {
            return Some(cached.clone());
        }
        if self
            .command_choices
            .pending
            .as_ref()
            .is_some_and(|(_, pending)| *pending == key)
        {
            return None;
        }
        let session_id = self.gateway_session.clone()?;
        arguments.push(CommandArgument {
            name: spec.name.clone(),
            kind: spec.kind as u8,
            value: query.into(),
            focused: true,
        });
        let app_command = app_command.clone();
        self.command_choices.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHOICES_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                let (nonce, _) = next_nonce();
                this.command_choices.pending = Some((nonce, key.clone()));
                let invocation = CommandInvocation {
                    command: app_command,
                    guild_id: conversation.guild_id,
                    channel_id: conversation.channel_id,
                    session_id,
                    nonce,
                    arguments,
                };
                cx.spawn(async move |this, cx| {
                    if discord::request_autocomplete(invocation).await.is_err() {
                        let _ = this.update(cx, |this, _| {
                            if this
                                .command_choices
                                .pending
                                .as_ref()
                                .is_some_and(|(pending, _)| *pending == nonce)
                            {
                                this.command_choices.pending = None;
                                // Not asked again for the same text.
                                this.command_choices.cache.insert(key, Vec::new());
                            }
                        });
                    }
                })
                .detach();
            });
        }));
        None
    }

    /// The other options as far as they convert, for an app to narrow its choices by.
    fn lenient_arguments(
        &self,
        command: &SlashCommand,
        text: &str,
        args_start: usize,
        except: usize,
    ) -> Vec<CommandArgument> {
        let text = resolve_mentions(text, &self.composer_mentions);
        let guild_id = self.conversation().and_then(|c| c.guild_id);
        parse_options(&text, args_start, &command.options)
            .into_iter()
            .filter(|parsed| parsed.option != except && !parsed.value.is_empty())
            .filter_map(|parsed| {
                let spec = &command.options[parsed.option];
                let known = self.known_choices(command, &spec.name);
                let value = convert_value(spec, &text[parsed.value], &known, guild_id).ok()?;
                Some(CommandArgument {
                    name: spec.name.clone(),
                    kind: spec.kind as u8,
                    value,
                    focused: false,
                })
            })
            .collect()
    }

    /// Every choice an app has offered for an option, so a picked name maps
    /// back to its value.
    fn known_choices(&self, command: &SlashCommand, option: &str) -> Vec<CommandChoice> {
        let key = command.key();
        self.command_choices
            .cache
            .iter()
            .filter(|(cached, _)| cached.command == key && cached.option == option)
            .flat_map(|(_, choices)| choices.iter().cloned())
            .collect()
    }

    pub(in crate::screens::home) fn receive_command_choices(
        &mut self,
        nonce: u64,
        choices: Vec<CommandChoice>,
        cx: &mut Context<Self>,
    ) {
        let Some((pending, key)) = self.command_choices.pending.take() else {
            return;
        };
        if pending != nonce {
            self.command_choices.pending = Some((pending, key));
            return;
        }
        self.command_choices.cache.insert(key, choices);
        self.refresh_autocomplete(cx);
    }

    /// Reads the composer as a command. Mentions are turned into markup first,
    /// so a user picked from the suggestions arrives as their id.
    pub(in crate::screens::home) fn try_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CommandSend {
        let raw = self.message_input.read(cx).value();
        let raw = raw.trim();
        if !raw.starts_with('/') {
            return CommandSend::NotCommand;
        }
        let text = self.resolve_emoji_names(&resolve_mentions(raw, &self.composer_mentions));
        let commands = self.available_commands();
        let Some((index, args_start)) = match_command(&text, &commands) else {
            // Sent as text, the command could land in the channel for everyone.
            if self.command_indexes_loading() {
                return self.refuse_command("Commands are still loading.".to_string(), cx);
            }
            return CommandSend::NotCommand;
        };
        let command = commands[index].clone();
        let parsed = parse_options(&text, args_start, &command.options);
        let missing = command.options.iter().enumerate().find(|(i, option)| {
            option.required
                && !parsed
                    .iter()
                    .any(|parsed| parsed.option == *i && !parsed.value.is_empty())
        });
        if let Some((_, option)) = missing {
            return self.refuse_command(format!("{} is required.", option.display_name), cx);
        }

        let app_command = match &command.source {
            CommandSource::Builtin(builtin) => {
                let message = parsed
                    .first()
                    .map_or("", |parsed| &text[parsed.value.clone()]);
                return CommandSend::Text(builtin.apply(message));
            }
            CommandSource::App(app_command) => app_command.clone(),
        };
        let guild_id = self.conversation().and_then(|c| c.guild_id);
        let mut arguments = Vec::new();
        for parsed in parsed.iter().filter(|parsed| !parsed.value.is_empty()) {
            let spec = &command.options[parsed.option];
            let known = self.known_choices(&command, &spec.name);
            match convert_value(spec, &text[parsed.value.clone()], &known, guild_id) {
                Ok(value) => arguments.push(CommandArgument {
                    name: spec.name.clone(),
                    kind: spec.kind as u8,
                    value,
                    focused: false,
                }),
                Err(err) => return self.refuse_command(err, cx),
            }
        }
        self.run_app_command(&command, app_command, arguments, window, cx);
        CommandSend::Handled
    }

    fn refuse_command(&mut self, error: String, cx: &mut Context<Self>) -> CommandSend {
        self.send_error = Some(error);
        cx.notify();
        CommandSend::Handled
    }

    /// Shown at once as the app's reply-to-be, as Discord's client does; the
    /// reply takes its place when it lands.
    fn run_app_command(
        &mut self,
        command: &SlashCommand,
        app_command: AppCommand,
        arguments: Vec<CommandArgument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conversation) = self.conversation() else {
            return;
        };
        let Some(session_id) = self.gateway_session.clone() else {
            self.refuse_command("Not connected to Discord yet.".to_string(), cx);
            return;
        };
        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.composer_mentions.clear();
        self.send_error = None;
        self.replying_to = None;
        self.typing_sent = None;
        self.set_unread_marker(None);
        self.marked_unread = false;
        if !self.reached_newest {
            self.jump_to_present(window, cx);
        }

        let (nonce, timestamp) = next_nonce();
        let pending_id = Id::new(nonce);
        if !self.messages_loading
            && let Some(invoker) = &self.current_user
        {
            let pending = discord::Message::pending_command(
                pending_id,
                &app_command.app,
                invoker,
                command.display_name.clone(),
                timestamp,
            );
            let ix = self.messages.len();
            self.messages.push(pending);
            self.splice_messages(ix..ix, 1);
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        self.pending_commands.insert(
            pending_id,
            PendingCommand {
                app_name: app_command.app.name.clone(),
                created: false,
                error: None,
                settle: None,
            },
        );
        cx.notify();

        let invocation = CommandInvocation {
            command: app_command,
            guild_id: conversation.guild_id,
            channel_id: conversation.channel_id,
            session_id,
            nonce,
            arguments,
        };
        cx.spawn(async move |this, cx| {
            if let Err(err) = discord::run_command(invocation).await {
                let _ = this.update(cx, |this, cx| this.fail_command(pending_id, err, cx));
            }
        })
        .detach();
    }

    fn fail_command(&mut self, id: Id<MessageMarker>, error: String, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_commands.get_mut(&id) else {
            return;
        };
        pending.error = Some(error);
        pending.settle = None;
        if let Some(message) = self.pending_message_mut(id) {
            message.delivery = discord::Delivery::Failed;
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn interaction_created(
        &mut self,
        nonce: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = Id::new_checked(nonce) else {
            return;
        };
        if let Some(pending) = self.pending_commands.get_mut(&id) {
            pending.created = true;
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn interaction_succeeded(
        &mut self,
        nonce: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = Id::new_checked(nonce) else {
            return;
        };
        let Some(pending) = self.pending_commands.get_mut(&id) else {
            return;
        };
        pending.created = true;
        pending.settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE_AFTER_SUCCESS).await;
            let _ = this.update(cx, |this, cx| {
                this.pending_commands.remove(&id);
                this.drop_pending_message(id, cx);
            });
        }));
        cx.notify();
    }

    pub(in crate::screens::home) fn interaction_failed(
        &mut self,
        nonce: u64,
        timed_out: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = Id::new_checked(nonce) else {
            return;
        };
        let Some(pending) = self.pending_commands.get(&id) else {
            return;
        };
        let error = if timed_out {
            format!("{} did not respond.", pending.app_name)
        } else {
            "Interaction failed.".to_string()
        };
        self.fail_command(id, error, cx);
    }

    /// The stand-in an app's reply replaces when it comes without the nonce:
    /// the newest still waiting on the same bot for a command the user ran.
    pub(in crate::screens::home) fn pending_command_answered_by(
        &self,
        reply: &discord::Message,
    ) -> Option<Id<MessageMarker>> {
        let Some(discord::Reply::Command(used)) = &reply.reply else {
            return None;
        };
        if Some(used.user_id) != self.self_user_id {
            return None;
        }
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.author_id == reply.author_id
                    && message.delivery != discord::Delivery::Sent
                    && self.pending_commands.contains_key(&message.id)
            })
            .map(|message| message.id)
    }

    fn drop_pending_message(&mut self, id: Id<MessageMarker>, cx: &mut Context<Self>) {
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == id && message.delivery != discord::Delivery::Sent)
        else {
            return;
        };
        self.messages.remove(ix);
        self.splice_messages(ix..ix + 1, 0);
        cx.notify();
    }

    /// A command answered with a form leaves no reply, so its stand-in goes.
    pub(in crate::screens::home) fn command_opened_modal(
        &mut self,
        nonce: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = Id::new_checked(nonce) else {
            return;
        };
        if self.pending_commands.remove(&id).is_some() {
            self.drop_pending_message(id, cx);
        }
    }

    /// Ephemeral replies and failed commands exist only here, so dismissing
    /// one just forgets it.
    pub(in crate::screens::home) fn dismiss_message(
        &mut self,
        id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        self.pending_commands.remove(&id);
        let Some(ix) = self.messages.iter().position(|message| message.id == id) else {
            return;
        };
        self.messages.remove(ix);
        self.splice_messages(ix..ix + 1, 0);
        cx.notify();
    }

    /// A new session starts with commands unknown, and its own id to run them under.
    pub(in crate::screens::home) fn reset_commands(&mut self, session_id: String) {
        self.gateway_session = Some(session_id);
        self.command_indexes.clear();
        self.command_choices = Default::default();
    }

    pub(in crate::screens::home) fn forget_command_index(
        &mut self,
        guild_id: Option<Id<GuildMarker>>,
    ) {
        match guild_id {
            Some(guild_id) => {
                self.command_indexes
                    .remove(&CommandIndexScope::Guild(guild_id));
            }
            None => {
                self.command_indexes.remove(&CommandIndexScope::User);
            }
        }
    }
}
