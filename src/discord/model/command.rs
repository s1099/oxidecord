//! Slash commands: the application command index Discord serves per guild,
//! bot DM and user, flattened into one entry per runnable (sub)command, plus
//! the client's own built-in text commands.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, CommandMarker, GuildMarker, UserMarker},
};

use super::cdn;

/// Where a command is being used, as Discord's `contexts` numbers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandContext {
    Guild = 0,
    BotDm = 1,
    PrivateChannel = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OptionKind {
    SubCommand = 1,
    SubCommandGroup = 2,
    String = 3,
    Integer = 4,
    Boolean = 5,
    User = 6,
    Channel = 7,
    Role = 8,
    Mentionable = 9,
    Number = 10,
    Attachment = 11,
}

impl OptionKind {
    fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::SubCommand,
            2 => Self::SubCommandGroup,
            3 => Self::String,
            4 => Self::Integer,
            5 => Self::Boolean,
            6 => Self::User,
            7 => Self::Channel,
            8 => Self::Role,
            9 => Self::Mentionable,
            10 => Self::Number,
            11 => Self::Attachment,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SubCommand | Self::SubCommandGroup => "subcommand",
            Self::String => "text",
            Self::Integer => "integer",
            Self::Boolean => "true or false",
            Self::User => "user",
            Self::Channel => "channel",
            Self::Role => "role",
            Self::Mentionable => "user or role",
            Self::Number => "number",
            Self::Attachment => "file",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CommandChoice {
    /// Localised, for display and for typing.
    pub name: String,
    /// A string or a number, sent as Discord gave it.
    pub value: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct CommandOption {
    pub kind: OptionKind,
    /// What's sent. Typed and shown as `display_name`.
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub required: bool,
    pub autocomplete: bool,
    pub choices: Vec<CommandChoice>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct CommandApp {
    pub id: Id<ApplicationMarker>,
    pub name: String,
    pub icon_url: Option<String>,
    /// The bot user that answers; Discord's client draws a pending command as it.
    pub bot_id: Option<Id<UserMarker>>,
}

/// An application's command as one leaf: a root command, or one of its
/// subcommands under its full path.
#[derive(Clone, Debug)]
pub struct AppCommand {
    pub app: Arc<CommandApp>,
    pub id: Id<CommandMarker>,
    pub version: String,
    pub guild_id: Option<Id<GuildMarker>>,
    /// The root's unlocalised name, which is what's sent.
    pub root_name: String,
    /// Subcommand group and subcommand names, unlocalised, outermost first.
    pub path: Vec<(String, OptionKind)>,
    /// The root as the index served it, echoed back when the command runs.
    pub raw: Arc<serde_json::Value>,
    contexts: Option<Vec<u8>>,
    dm_permission: Option<bool>,
    default_member_permissions: Option<Permissions>,
    nsfw: bool,
    /// Explicit `false` overwrites for channels; Discord omits commands the
    /// user's roles bar, so these are the ones left to check.
    denied_channels: Vec<Id<ChannelMarker>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuiltinCommand {
    Shrug,
    TableFlip,
    Unflip,
    Me,
    Spoiler,
}

impl BuiltinCommand {
    pub const ALL: [Self; 5] = [
        Self::Shrug,
        Self::TableFlip,
        Self::Unflip,
        Self::Me,
        Self::Spoiler,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Shrug => "shrug",
            Self::TableFlip => "tableflip",
            Self::Unflip => "unflip",
            Self::Me => "me",
            Self::Spoiler => "spoiler",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Shrug => "Appends ¯\\_(ツ)_/¯ to your message.",
            Self::TableFlip => "Appends (╯°□°)╯︵ ┻━┻ to your message.",
            Self::Unflip => "Appends ┬─┬ノ( º _ ºノ) to your message.",
            Self::Me => "Displays text with emphasis.",
            Self::Spoiler => "Marks your message as a spoiler.",
        }
    }

    /// The text sent in place of the command, as Discord's client writes it.
    pub fn apply(self, message: &str) -> String {
        match self {
            // The backslash escapes the underscore, so markdown leaves the arm whole.
            Self::Shrug => format!("{message} ¯\\_(ツ)_/¯"),
            Self::TableFlip => format!("{message} (╯°□°)╯︵ ┻━┻"),
            Self::Unflip => format!("{message} ┬─┬ノ( º _ ºノ)"),
            Self::Me => format!("_{message}_"),
            Self::Spoiler => format!("||{message}||"),
        }
        .trim()
        .to_string()
    }

    fn required(self) -> bool {
        matches!(self, Self::Me | Self::Spoiler)
    }
}

#[derive(Clone, Debug)]
pub enum CommandSource {
    Builtin(BuiltinCommand),
    App(AppCommand),
}

/// One entry in the composer's command list.
#[derive(Clone, Debug)]
pub struct SlashCommand {
    pub source: CommandSource,
    /// Localised, subcommands included (`config set`); typed after the slash.
    pub display_name: String,
    pub description: String,
    pub options: Vec<CommandOption>,
}

impl SlashCommand {
    pub fn builtins() -> impl Iterator<Item = Self> {
        BuiltinCommand::ALL.into_iter().map(|builtin| Self {
            source: CommandSource::Builtin(builtin),
            display_name: builtin.name().to_string(),
            description: builtin.description().to_string(),
            options: vec![CommandOption {
                kind: OptionKind::String,
                name: "message".to_string(),
                display_name: "message".to_string(),
                description: String::new(),
                required: builtin.required(),
                autocomplete: false,
                choices: Vec::new(),
                min_value: None,
                max_value: None,
                min_length: None,
                max_length: None,
            }],
        })
    }

    pub fn app(&self) -> Option<&CommandApp> {
        match &self.source {
            CommandSource::App(command) => Some(&command.app),
            CommandSource::Builtin(_) => None,
        }
    }

    /// Identity that survives the list being rebuilt.
    pub fn key(&self) -> String {
        match &self.source {
            CommandSource::Builtin(builtin) => format!("builtin-{}", builtin.name()),
            CommandSource::App(command) => format!("{}-{}", command.id, self.display_name),
        }
    }

    /// A simplified form of Discord's client check: the server has already
    /// left out commands the user's roles bar.
    pub fn usable(
        &self,
        context: CommandContext,
        channel_id: Id<ChannelMarker>,
        permissions: Option<Permissions>,
        nsfw_allowed: bool,
    ) -> bool {
        let CommandSource::App(command) = &self.source else {
            return true;
        };
        if command.nsfw && !nsfw_allowed {
            return false;
        }
        match &command.contexts {
            Some(contexts) => {
                if !contexts.contains(&(context as u8)) {
                    return false;
                }
            }
            None => match context {
                CommandContext::BotDm if command.dm_permission == Some(false) => return false,
                CommandContext::PrivateChannel => return false,
                _ => {}
            },
        }
        if context != CommandContext::Guild {
            return true;
        }
        if command.denied_channels.contains(&channel_id) {
            return false;
        }
        match (permissions, command.default_member_permissions) {
            (Some(permissions), _) if permissions.contains(Permissions::ADMINISTRATOR) => true,
            (Some(permissions), Some(required)) => permissions.contains(required),
            _ => true,
        }
    }
}

/// A command index as served, flattened.
#[derive(Default)]
pub struct CommandIndex {
    pub commands: Vec<Arc<SlashCommand>>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawCommandIndex {
    #[serde(default)]
    applications: Vec<RawApp>,
    #[serde(default)]
    application_commands: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawApp {
    id: Id<ApplicationMarker>,
    name: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    bot_id: Option<Id<UserMarker>>,
    #[serde(default)]
    bot: Option<RawBot>,
}

#[derive(Deserialize)]
struct RawBot {
    id: Id<UserMarker>,
}

#[derive(Deserialize)]
struct RawCommand {
    id: Id<CommandMarker>,
    application_id: Id<ApplicationMarker>,
    #[serde(rename = "type", default = "chat_input")]
    kind: u8,
    #[serde(default)]
    guild_id: Option<Id<GuildMarker>>,
    #[serde(default)]
    version: String,
    #[serde(flatten)]
    text: RawText,
    #[serde(default)]
    options: Vec<RawOption>,
    #[serde(default)]
    contexts: Option<Vec<u8>>,
    #[serde(default)]
    dm_permission: Option<bool>,
    #[serde(default)]
    default_member_permissions: Option<String>,
    #[serde(default)]
    nsfw: bool,
    #[serde(default)]
    permissions: Option<RawCommandPermissions>,
}

fn chat_input() -> u8 {
    1
}

/// In an index, `name` and `description` arrive localised and the originals
/// under `*_default`; elsewhere `name` is the original and `name_localized` the
/// translation.
#[derive(Deserialize)]
struct RawText {
    name: String,
    #[serde(default)]
    name_default: Option<String>,
    #[serde(default)]
    name_localized: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    description_localized: Option<String>,
}

impl RawText {
    /// (sent, shown, description)
    fn resolve(self) -> (String, String, String) {
        let description = self.description_localized.unwrap_or(self.description);
        match self.name_default {
            Some(default) => (default, self.name, description),
            None => {
                let shown = self.name_localized.unwrap_or_else(|| self.name.clone());
                (self.name, shown, description)
            }
        }
    }
}

#[derive(Deserialize)]
struct RawOption {
    #[serde(rename = "type")]
    kind: u8,
    #[serde(flatten)]
    text: RawText,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    autocomplete: bool,
    #[serde(default)]
    choices: Vec<RawChoice>,
    #[serde(default)]
    min_value: Option<f64>,
    #[serde(default)]
    max_value: Option<f64>,
    #[serde(default)]
    min_length: Option<usize>,
    #[serde(default)]
    max_length: Option<usize>,
    #[serde(default)]
    options: Vec<RawOption>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawChoice {
    name: String,
    #[serde(default)]
    name_localized: Option<String>,
    value: serde_json::Value,
}

impl RawChoice {
    pub(in crate::discord) fn convert(self) -> CommandChoice {
        CommandChoice {
            name: self.name_localized.unwrap_or(self.name),
            value: self.value,
        }
    }
}

#[derive(Deserialize)]
struct RawCommandPermissions {
    #[serde(default)]
    channels: HashMap<Id<ChannelMarker>, bool>,
}

pub(in crate::discord) fn convert_command_index(raw: RawCommandIndex) -> CommandIndex {
    let apps: HashMap<_, _> = raw
        .applications
        .into_iter()
        .map(|app| {
            let icon_url = app
                .icon
                .as_deref()
                .map(|hash| cdn::app_icon_url(app.id, hash));
            let app = CommandApp {
                id: app.id,
                bot_id: app.bot_id.or(app.bot.map(|bot| bot.id)),
                name: app.name,
                icon_url,
            };
            (app.id, Arc::new(app))
        })
        .collect();

    let mut commands = Vec::new();
    for value in raw.application_commands {
        let Ok(command) = serde_json::from_value::<RawCommand>(value.clone()) else {
            continue;
        };
        // User and message commands live in context menus.
        if command.kind != 1 {
            continue;
        }
        let Some(app) = apps.get(&command.application_id) else {
            continue;
        };
        let (root_name, display_name, description) = command.text.resolve();
        let base = AppCommand {
            app: app.clone(),
            id: command.id,
            version: command.version,
            guild_id: command.guild_id,
            root_name,
            path: Vec::new(),
            raw: Arc::new(normalize_raw(value)),
            contexts: command.contexts,
            dm_permission: command.dm_permission,
            default_member_permissions: command
                .default_member_permissions
                .and_then(|bits| bits.parse().ok())
                .map(Permissions::from_bits_truncate),
            nsfw: command.nsfw,
            denied_channels: command
                .permissions
                .map(|permissions| {
                    permissions
                        .channels
                        .into_iter()
                        .filter(|(_, allowed)| !allowed)
                        .map(|(channel, _)| channel)
                        .collect()
                })
                .unwrap_or_default(),
        };
        flatten(
            base,
            display_name,
            description,
            command.options,
            &mut commands,
        );
    }
    CommandIndex { commands }
}

/// One entry per leaf, as Discord's client lists them.
fn flatten(
    command: AppCommand,
    display_name: String,
    description: String,
    options: Vec<RawOption>,
    out: &mut Vec<Arc<SlashCommand>>,
) {
    let (subcommands, options): (Vec<_>, Vec<_>) = options.into_iter().partition(|option| {
        matches!(
            OptionKind::from_raw(option.kind),
            Some(OptionKind::SubCommand | OptionKind::SubCommandGroup)
        )
    });
    if subcommands.is_empty() {
        out.push(Arc::new(SlashCommand {
            source: CommandSource::App(command),
            display_name,
            description,
            options: options.into_iter().filter_map(convert_option).collect(),
        }));
        return;
    }
    for sub in subcommands {
        let Some(kind) = OptionKind::from_raw(sub.kind) else {
            continue;
        };
        let (name, shown, sub_description) = sub.text.resolve();
        let mut command = command.clone();
        command.path.push((name, kind));
        flatten(
            command,
            format!("{display_name} {shown}"),
            sub_description,
            sub.options,
            out,
        );
    }
}

fn convert_option(raw: RawOption) -> Option<CommandOption> {
    let kind = OptionKind::from_raw(raw.kind)?;
    let (name, display_name, description) = raw.text.resolve();
    Some(CommandOption {
        kind,
        name,
        display_name,
        description,
        required: raw.required,
        autocomplete: raw.autocomplete,
        choices: raw.choices.into_iter().map(RawChoice::convert).collect(),
        min_value: raw.min_value,
        max_value: raw.max_value,
        min_length: raw.min_length,
        max_length: raw.max_length,
    })
}

/// Puts the unlocalised name and description back, as Discord's client does
/// before echoing the command.
fn normalize_raw(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(object) = value.as_object_mut() {
        for field in ["name", "description"] {
            if let Some(default) = object.get(&format!("{field}_default")).cloned() {
                object.insert(field.to_string(), default);
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_subcommands_and_keeps_unlocalised_names() {
        let raw: RawCommandIndex = serde_json::from_value(serde_json::json!({
            "applications": [{ "id": "10", "name": "Bot", "icon": null, "bot_id": "11" }],
            "application_commands": [{
                "id": "20", "application_id": "10", "version": "1", "type": 1,
                "name": "konfig", "name_default": "config", "description": "d",
                "options": [
                    { "type": 2, "name": "user", "description": "", "options": [
                        { "type": 1, "name": "set", "description": "Set it", "options": [
                            { "type": 3, "name": "value", "description": "", "required": true }
                        ]}
                    ]},
                    { "type": 1, "name": "reset", "description": "Reset" }
                ]
            }, {
                "id": "21", "application_id": "10", "version": "1", "type": 2,
                "name": "User Command", "description": ""
            }]
        }))
        .unwrap();
        let index = convert_command_index(raw);
        let names: Vec<_> = index
            .commands
            .iter()
            .map(|command| command.display_name.as_str())
            .collect();
        assert_eq!(names, ["konfig user set", "konfig reset"]);
        let CommandSource::App(command) = &index.commands[0].source else {
            panic!("app command");
        };
        assert_eq!(command.root_name, "config");
        assert_eq!(command.raw["name"], "config");
        assert_eq!(command.app.bot_id, Some(Id::new(11)));
        assert_eq!(
            command
                .path
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["user", "set"]
        );
        assert!(index.commands[0].options[0].required);
    }

    #[test]
    fn builtins_write_what_discord_does() {
        assert_eq!(BuiltinCommand::Shrug.apply(""), "¯\\_(ツ)_/¯");
        assert_eq!(BuiltinCommand::Me.apply("waves"), "_waves_");
        assert_eq!(BuiltinCommand::Spoiler.apply("x"), "||x||");
    }
}
