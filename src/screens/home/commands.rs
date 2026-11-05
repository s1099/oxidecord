//! Slash commands typed into the composer as plain text: `/name` followed by
//! `option:value` pairs, as Discord's client reads text pasted over its pills.
//! Text before the first named option fills the first option left unnamed, so
//! `/me waves` works without typing `message:`.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::Task;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, MessageMarker},
};

use crate::discord::{CommandChoice, CommandIndex, CommandOption, OptionKind, SlashCommand};

pub(super) enum IndexState {
    Loading,
    Loaded(Arc<CommandIndex>),
    /// Kept so a failing index isn't asked for on every keystroke; a new
    /// session or an index update clears it.
    Failed,
}

/// A command run from here whose reply hasn't arrived.
pub(super) struct PendingCommand {
    pub app_name: String,
    /// Discord handed it to the app, which is now "thinking".
    pub created: bool,
    pub error: Option<String>,
    /// Drops the stand-in once the app has answered without a reply landing on it.
    pub settle: Option<Task<()>>,
}

impl PendingCommand {
    pub fn status(&self) -> String {
        match (&self.error, self.created) {
            (Some(error), _) => error.clone(),
            (None, true) => format!("{} is thinking…", self.app_name),
            (None, false) => "Sending command…".to_string(),
        }
    }
}

/// Which command's option, with what typed and what else is filled, the app
/// was asked to complete. Answers are cached under it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct ChoicesKey {
    pub command: String,
    pub option: String,
    pub query: String,
    /// The other options as typed, since an app may narrow by them.
    pub context: String,
}

#[derive(Default)]
pub(super) struct CommandChoices {
    pub cache: std::collections::HashMap<ChoicesKey, Vec<CommandChoice>>,
    pub pending: Option<(u64, ChoicesKey)>,
    pub debounce: Option<Task<()>>,
}

/// The command at the start of `text`, and the byte offset its options start at.
/// The longest name wins, so `/config set` beats `/config`.
pub(super) fn match_command(text: &str, commands: &[Arc<SlashCommand>]) -> Option<(usize, usize)> {
    let typed = text.strip_prefix('/')?;
    commands
        .iter()
        .enumerate()
        .filter(|(_, command)| {
            let name = command.display_name.as_str();
            typed
                .get(..name.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(name))
                && typed[name.len()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .max_by_key(|(_, command)| command.display_name.len())
        .map(|(index, command)| (index, 1 + command.display_name.len()))
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct ParsedOption {
    /// Index into the command's options.
    pub option: usize,
    /// Where its value may run, from after the colon (or the command) to the next option.
    pub span: Range<usize>,
    /// `span` trimmed.
    pub value: Range<usize>,
}

/// Each option's value in `text`, read from `start`. Names are matched only at
/// the start or after whitespace, and each once, as in Discord's client.
pub(super) fn parse_options(
    text: &str,
    start: usize,
    options: &[CommandOption],
) -> Vec<ParsedOption> {
    // (option, name start, value start)
    let mut named: Vec<(usize, usize, usize)> = Vec::new();
    let mut previous = None;
    for (at, c) in text[start..].char_indices().map(|(i, c)| (start + i, c)) {
        let at_word = previous.is_none_or(char::is_whitespace);
        previous = Some(c);
        if !at_word || c.is_whitespace() {
            continue;
        }
        let rest = &text[at..];
        let found = options.iter().enumerate().find(|(index, option)| {
            !named.iter().any(|(other, ..)| other == index)
                && rest
                    .get(..option.display_name.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(&option.display_name))
                && rest[option.display_name.len()..].starts_with(':')
        });
        if let Some((index, option)) = found {
            named.push((index, at, at + option.display_name.len() + 1));
        }
    }

    let mut parsed = Vec::new();
    let leading_end = named.first().map_or(text.len(), |(_, name, _)| *name);
    let leading = trimmed(text, start..leading_end);
    if !leading.is_empty() {
        // The first required option left unnamed, else the first unnamed one.
        let unnamed = |index: &usize| !named.iter().any(|(other, ..)| other == index);
        let implicit = (0..options.len())
            .filter(unnamed)
            .find(|&index| options[index].required)
            .or_else(|| (0..options.len()).find(unnamed));
        if let Some(option) = implicit {
            parsed.push(ParsedOption {
                option,
                span: start..leading_end,
                value: leading,
            });
        }
    }
    for (i, &(option, _, value_start)) in named.iter().enumerate() {
        let end = named.get(i + 1).map_or(text.len(), |(_, name, _)| *name);
        parsed.push(ParsedOption {
            option,
            span: value_start..end,
            value: trimmed(text, value_start..end),
        });
    }
    parsed
}

fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let slice = &text[range.clone()];
    let start = range.start + (slice.len() - slice.trim_start().len());
    let end = range.end - (slice.len() - slice.trim_end().len());
    start..end.max(start)
}

/// What the cursor is in, which decides what the composer suggests.
#[derive(PartialEq, Eq, Debug)]
pub(super) enum CommandCursor {
    /// Typing the command after the slash.
    Name { range: Range<usize> },
    /// Typing an option's name, or about to.
    OptionName { range: Range<usize> },
    /// In an option's value; `range` is all of it, so a choice replaces it whole.
    Value {
        option: usize,
        range: Range<usize>,
        query_end: usize,
    },
}

pub(super) fn command_cursor(
    text: &str,
    cursor: usize,
    matched: Option<(&SlashCommand, usize)>,
) -> Option<CommandCursor> {
    if !text.starts_with('/') || cursor == 0 {
        return None;
    }
    let Some((command, args_start)) = matched.filter(|(_, args_start)| cursor > *args_start) else {
        return text[..cursor]
            .chars()
            .all(|c| c != '\n')
            .then_some(CommandCursor::Name { range: 0..cursor });
    };
    let word_start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8())
        .max(args_start);
    let word = &text[word_start..cursor];
    let names_option = !word.contains(':')
        && command.options.iter().any(|option| {
            option
                .display_name
                .to_lowercase()
                .starts_with(&word.to_lowercase())
        });
    let parsed = parse_options(text, args_start, &command.options);
    let within = parsed
        .iter()
        .find(|parsed| parsed.span.start <= cursor && cursor <= parsed.span.end);
    match within {
        // A word that could start an option's name: a later word of a value, or
        // the first word after the command, which would otherwise fill an option.
        Some(parsed)
            if !word.is_empty()
                && names_option
                && (word_start > parsed.value.start || parsed.span.start == args_start) =>
        {
            Some(CommandCursor::OptionName {
                range: word_start..cursor,
            })
        }
        Some(parsed) if cursor >= parsed.value.start || parsed.value.is_empty() => {
            let start = parsed.value.start.min(cursor);
            Some(CommandCursor::Value {
                option: parsed.option,
                range: start..parsed.value.end.max(cursor),
                query_end: cursor,
            })
        }
        _ => Some(CommandCursor::OptionName {
            range: word_start..cursor,
        }),
    }
}

/// The value sent for an option typed as `raw`, after mentions were turned
/// into markup. `known` are choices the app offered for it.
pub(super) fn convert_value(
    option: &CommandOption,
    raw: &str,
    known: &[CommandChoice],
    guild_id: Option<Id<GuildMarker>>,
) -> Result<serde_json::Value, String> {
    let name = &option.display_name;
    let choices = if option.choices.is_empty() {
        known
    } else {
        &option.choices
    };
    if let Some(choice) = choices
        .iter()
        .find(|choice| choice.name.eq_ignore_ascii_case(raw) || choice_text(&choice.value) == raw)
    {
        return Ok(choice.value.clone());
    }
    if !option.choices.is_empty() {
        return Err(format!("Pick one of the choices for {name}."));
    }
    let id = |prefixes: &[&str]| -> Option<String> {
        if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
            return Some(raw.to_string());
        }
        let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
        prefixes.iter().find_map(|prefix| {
            let id = inner.strip_prefix(prefix)?;
            id.chars()
                .all(|c| c.is_ascii_digit())
                .then(|| id.to_string())
        })
    };
    let everyone = || (raw == "@everyone").then(|| guild_id.map(|id| id.to_string()))?;
    let value: serde_json::Value = match option.kind {
        OptionKind::String => {
            let length = raw.chars().count();
            if option.min_length.is_some_and(|min| length < min) {
                return Err(format!(
                    "{name} must be at least {} characters.",
                    option.min_length.unwrap_or_default()
                ));
            }
            if option.max_length.is_some_and(|max| length > max) {
                return Err(format!(
                    "{name} can be at most {} characters.",
                    option.max_length.unwrap_or_default()
                ));
            }
            raw.into()
        }
        OptionKind::Integer => {
            let number: i64 = raw
                .replace(',', "")
                .parse()
                .map_err(|_| format!("{name} must be a whole number."))?;
            check_range(option, number as f64)?;
            number.into()
        }
        OptionKind::Number => {
            let number: f64 = raw
                .replace(',', "")
                .parse()
                .ok()
                .filter(|number: &f64| number.is_finite())
                .ok_or_else(|| format!("{name} must be a number."))?;
            check_range(option, number)?;
            number.into()
        }
        OptionKind::Boolean => match raw.to_lowercase().as_str() {
            "true" | "yes" => true.into(),
            "false" | "no" => false.into(),
            _ => return Err(format!("{name} must be True or False.")),
        },
        OptionKind::User => id(&["@!", "@"])
            .ok_or_else(|| format!("{name} must be a user."))?
            .into(),
        OptionKind::Role => everyone()
            .or_else(|| id(&["@&"]))
            .ok_or_else(|| format!("{name} must be a role."))?
            .into(),
        OptionKind::Mentionable => everyone()
            .or_else(|| id(&["@&", "@!", "@"]))
            .ok_or_else(|| format!("{name} must be a user or role."))?
            .into(),
        OptionKind::Channel => id(&["#"])
            .ok_or_else(|| format!("{name} must be a channel."))?
            .into(),
        OptionKind::Attachment => {
            return Err("Commands that take a file aren't supported yet.".to_string());
        }
        OptionKind::SubCommand | OptionKind::SubCommandGroup => {
            return Err(format!("{name} can't be filled in."));
        }
    };
    Ok(value)
}

fn check_range(option: &CommandOption, number: f64) -> Result<(), String> {
    let name = &option.display_name;
    if let Some(min) = option.min_value.filter(|&min| number < min) {
        return Err(format!("{name} must be at least {min}."));
    }
    if let Some(max) = option.max_value.filter(|&max| number > max) {
        return Err(format!("{name} can be at most {max}."));
    }
    Ok(())
}

fn choice_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Booleans offer their two values as choices.
pub(super) fn boolean_choices() -> Vec<CommandChoice> {
    ["True", "False"]
        .into_iter()
        .map(|name| CommandChoice {
            name: name.to_string(),
            value: (name == "True").into(),
        })
        .collect()
}

/// Ids of the messages standing in for commands whose reply hasn't come.
pub(super) type PendingCommands = std::collections::HashMap<Id<MessageMarker>, PendingCommand>;

#[cfg(test)]
mod tests {
    use super::*;

    fn option(name: &str, kind: OptionKind, required: bool) -> CommandOption {
        CommandOption {
            kind,
            name: name.to_string(),
            display_name: name.to_string(),
            description: String::new(),
            required,
            autocomplete: false,
            choices: Vec::new(),
            min_value: None,
            max_value: None,
            min_length: None,
            max_length: None,
        }
    }

    fn command(name: &str, options: Vec<CommandOption>) -> Arc<SlashCommand> {
        let builtin = SlashCommand::builtins().next().unwrap();
        Arc::new(SlashCommand {
            display_name: name.to_string(),
            options,
            ..builtin
        })
    }

    #[test]
    fn matches_the_longest_command() {
        let commands = [command("config", vec![]), command("config set", vec![])];
        assert_eq!(match_command("/config set a", &commands), Some((1, 11)));
        assert_eq!(match_command("/config", &commands), Some((0, 7)));
        assert_eq!(match_command("/configure", &commands), None);
        assert_eq!(match_command("config", &commands), None);
    }

    #[test]
    fn splits_named_options_and_leading_text() {
        let options = [
            option("text", OptionKind::String, true),
            option("count", OptionKind::Integer, false),
        ];
        let text = "/say hello there count: 3";
        let parsed = parse_options(text, 4, &options);
        let values: Vec<_> = parsed
            .iter()
            .map(|parsed| (parsed.option, &text[parsed.value.clone()]))
            .collect();
        assert_eq!(values, [(0, "hello there"), (1, "3")]);

        // A name inside a word isn't an option.
        let text = "/say text:recount:2";
        let parsed = parse_options(text, 4, &options);
        assert_eq!(parsed.len(), 1);
        assert_eq!(&text[parsed[0].value.clone()], "recount:2");
    }

    #[test]
    fn finds_what_the_cursor_is_in() {
        let options = vec![
            option("text", OptionKind::String, true),
            option("count", OptionKind::Integer, false),
        ];
        let say = command("say", options);
        assert_eq!(
            command_cursor("/sa", 3, None),
            Some(CommandCursor::Name { range: 0..3 })
        );
        assert_eq!(
            command_cursor("/say ", 5, Some((&say, 4))),
            Some(CommandCursor::OptionName { range: 5..5 })
        );
        assert_eq!(
            command_cursor("/say text:hi", 12, Some((&say, 4))),
            Some(CommandCursor::Value {
                option: 0,
                range: 10..12,
                query_end: 12
            })
        );
        assert_eq!(
            command_cursor("/say text:hi co", 15, Some((&say, 4))),
            Some(CommandCursor::OptionName { range: 13..15 })
        );
    }

    #[test]
    fn converts_values_by_kind() {
        let user = option("who", OptionKind::User, true);
        assert_eq!(convert_value(&user, "<@!42>", &[], None), Ok("42".into()));
        let int = CommandOption {
            max_value: Some(10.),
            ..option("n", OptionKind::Integer, true)
        };
        assert_eq!(convert_value(&int, "7", &[], None), Ok(7.into()));
        assert!(convert_value(&int, "11", &[], None).is_err());
        let flag = option("on", OptionKind::Boolean, true);
        assert_eq!(convert_value(&flag, "True", &[], None), Ok(true.into()));
        let picked = CommandOption {
            choices: vec![CommandChoice {
                name: "Red".into(),
                value: "r".into(),
            }],
            ..option("color", OptionKind::String, true)
        };
        assert_eq!(convert_value(&picked, "red", &[], None), Ok("r".into()));
        assert!(convert_value(&picked, "blue", &[], None).is_err());
    }
}
