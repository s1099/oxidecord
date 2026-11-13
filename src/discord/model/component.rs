//! Message components: an app's buttons and select menus, the layout blocks of
//! a components-v2 message (containers, sections, text, media), and the modal
//! forms an app opens in answer to an interaction.

use std::sync::Arc;

use serde::Deserialize;

use twilight_model::channel::message::component::{
    self as raw, ButtonStyle as RawButtonStyle, SelectDefaultValue, SelectMenuType,
    SeparatorSpacingSize, TextInputStyle,
};
use twilight_model::channel::{Attachment, ChannelType};
use twilight_model::id::{
    Id,
    marker::{AttachmentMarker, ChannelMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::channel::ChannelKind;
use super::markdown::Markdown;
use super::message::{
    FileAttachment, ImageAttachment, ReactionEmoji, SPOILER_PREFIX, VideoAttachment,
    convert_attachments, convert_reaction_emoji,
};

#[derive(Clone)]
pub enum Component {
    /// Buttons, or one select menu, in a row.
    ActionRow(Vec<Component>),
    Button(Button),
    Select(Select),
    /// Text beside an accessory: a button or a thumbnail.
    Section {
        components: Vec<Component>,
        accessory: Option<Box<Component>>,
    },
    Text(TextDisplay),
    Thumbnail(MediaItem),
    Gallery(Vec<MediaItem>),
    File(FileItem),
    Separator {
        divider: bool,
        large: bool,
    },
    Container {
        accent: Option<u32>,
        spoiler: bool,
        components: Vec<Component>,
    },
}

#[derive(Clone)]
pub struct TextDisplay {
    pub markdown: Arc<Markdown>,
    /// Numbers the text for selection, in reading order across the message.
    pub index: usize,
}

/// A gallery item or thumbnail. `index` numbers it among the message's media,
/// as an attachment's position does, for the player and the spoiler state.
#[derive(Clone)]
pub struct MediaItem {
    pub media: Media,
    pub index: usize,
}

#[derive(Clone)]
pub enum Media {
    Image(ImageAttachment),
    Video(VideoAttachment),
}

#[derive(Clone)]
pub struct FileItem {
    pub file: FileAttachment,
    /// Numbered with the media, as [`MediaItem::index`].
    pub index: usize,
}

#[derive(Clone)]
pub struct Button {
    pub style: ButtonStyle,
    pub label: Option<String>,
    pub emoji: Option<ReactionEmoji>,
    pub disabled: bool,
    pub action: ButtonAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Success,
    Danger,
    Link,
    Premium,
}

#[derive(Clone)]
pub enum ButtonAction {
    /// Sent to the app as an interaction.
    Interaction(String),
    Link(String),
    /// A store purchase, which this client can't make.
    Premium,
}

#[derive(Clone)]
pub struct Select {
    pub kind: SelectKind,
    pub custom_id: String,
    pub placeholder: Option<String>,
    pub min_values: u8,
    pub max_values: u8,
    pub disabled: bool,
    /// In a modal: whether it must be answered. Messages' selects ignore it.
    pub required: bool,
    /// A string select's options; the others choose among the guild's users,
    /// roles or channels.
    pub options: Vec<SelectOption>,
    /// What an auto-populated select starts with.
    pub defaults: Vec<SelectValue>,
    /// Which channels a channel select offers; empty allows any.
    pub channel_kinds: Vec<ChannelKind>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
    Text,
    User,
    Role,
    Mentionable,
    Channel,
}

impl SelectKind {
    /// The component type sent with an interaction.
    pub fn component_type(self) -> u8 {
        match self {
            Self::Text => 3,
            Self::User => 5,
            Self::Role => 6,
            Self::Mentionable => 7,
            Self::Channel => 8,
        }
    }
}

#[derive(Clone)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
    pub emoji: Option<ReactionEmoji>,
    pub default: bool,
}

/// A pick in an auto-populated select.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SelectValue {
    User(Id<UserMarker>),
    Role(Id<RoleMarker>),
    Channel(Id<ChannelMarker>),
}

impl SelectValue {
    /// The id as sent in an interaction's `values`.
    pub fn id(self) -> String {
        match self {
            Self::User(id) => id.to_string(),
            Self::Role(id) => id.to_string(),
            Self::Channel(id) => id.to_string(),
        }
    }
}

impl Component {
    /// Whether anything in it is drawn; a message carrying only unknown
    /// components still reads as empty.
    pub fn is_visible(&self) -> bool {
        match self {
            Self::ActionRow(children)
            | Self::Container {
                components: children,
                ..
            } => children.iter().any(Self::is_visible),
            _ => true,
        }
    }
}

/// Numbers text and media across a message as its components are converted.
struct Counter<'a> {
    attachments: &'a [Attachment],
    texts: usize,
    media: usize,
}

pub(super) fn convert_components(
    components: Vec<raw::Component>,
    attachments: &[Attachment],
) -> Vec<Component> {
    let mut counter = Counter {
        attachments,
        texts: 0,
        media: 0,
    };
    convert_list(components, &mut counter)
}

fn convert_list(components: Vec<raw::Component>, counter: &mut Counter) -> Vec<Component> {
    components
        .into_iter()
        .filter_map(|component| convert(component, counter))
        .collect()
}

fn convert(component: raw::Component, counter: &mut Counter) -> Option<Component> {
    Some(match component {
        raw::Component::ActionRow(row) => {
            Component::ActionRow(convert_list(row.components, counter))
        }
        raw::Component::Button(button) => Component::Button(convert_button(button)),
        raw::Component::SelectMenu(select) => Component::Select(convert_select(select)),
        raw::Component::Section(section) => Component::Section {
            components: convert_list(section.components, counter),
            accessory: convert(*section.accessory, counter).map(Box::new),
        },
        raw::Component::TextDisplay(text) => {
            let index = counter.texts;
            counter.texts += 1;
            Component::Text(TextDisplay {
                markdown: Arc::new(Markdown::parse(&text.content)),
                index,
            })
        }
        raw::Component::Thumbnail(thumbnail) => Component::Thumbnail(media_item(
            &thumbnail.media,
            thumbnail.spoiler.unwrap_or(false),
            counter,
        )?),
        raw::Component::MediaGallery(gallery) => Component::Gallery(
            gallery
                .items
                .iter()
                .filter_map(|item| media_item(&item.media, item.spoiler.unwrap_or(false), counter))
                .collect(),
        ),
        raw::Component::File(file) => {
            let spoiler = file.spoiler.unwrap_or(false);
            let attachment = media_attachment(&file.file, spoiler, counter.attachments);
            let (_, _, files) = convert_attachments(&[Attachment {
                // Drawn as a file whatever it holds, as Discord does.
                content_type: Some("application/octet-stream".into()),
                ..attachment
            }]);
            let index = counter.media;
            counter.media += 1;
            Component::File(FileItem {
                file: files.into_iter().next()?,
                index,
            })
        }
        raw::Component::Separator(separator) => Component::Separator {
            divider: separator.divider.unwrap_or(true),
            large: matches!(separator.spacing, Some(SeparatorSpacingSize::Large)),
        },
        raw::Component::Container(container) => Component::Container {
            accent: container.accent_color.flatten(),
            spoiler: container.spoiler.unwrap_or(false),
            components: convert_list(container.components, counter),
        },
        _ => return None,
    })
}

fn convert_button(button: raw::Button) -> Button {
    let style = match button.style {
        RawButtonStyle::Primary => ButtonStyle::Primary,
        RawButtonStyle::Success => ButtonStyle::Success,
        RawButtonStyle::Danger => ButtonStyle::Danger,
        RawButtonStyle::Link => ButtonStyle::Link,
        RawButtonStyle::Premium => ButtonStyle::Premium,
        _ => ButtonStyle::Secondary,
    };
    let action = match (style, button.url, button.custom_id) {
        (ButtonStyle::Premium, _, _) => ButtonAction::Premium,
        (_, Some(url), _) => ButtonAction::Link(url),
        (_, None, Some(custom_id)) => ButtonAction::Interaction(custom_id),
        (_, None, None) => ButtonAction::Premium,
    };
    Button {
        style,
        label: button.label.filter(|label| !label.is_empty()),
        emoji: button.emoji.map(convert_reaction_emoji),
        disabled: button.disabled || matches!(action, ButtonAction::Premium),
        action,
    }
}

pub(super) fn convert_select(select: raw::SelectMenu) -> Select {
    let kind = match select.kind {
        SelectMenuType::User => SelectKind::User,
        SelectMenuType::Role => SelectKind::Role,
        SelectMenuType::Mentionable => SelectKind::Mentionable,
        SelectMenuType::Channel => SelectKind::Channel,
        _ => SelectKind::Text,
    };
    Select {
        kind,
        custom_id: select.custom_id,
        placeholder: select.placeholder.filter(|text| !text.is_empty()),
        min_values: select.min_values.unwrap_or(1),
        max_values: select.max_values.unwrap_or(1).max(1),
        disabled: select.disabled,
        required: select.required.unwrap_or(true),
        options: select
            .options
            .unwrap_or_default()
            .into_iter()
            .map(|option| SelectOption {
                label: option.label,
                value: option.value,
                description: option.description.filter(|text| !text.is_empty()),
                emoji: option.emoji.map(convert_reaction_emoji),
                default: option.default,
            })
            .collect(),
        defaults: select
            .default_values
            .unwrap_or_default()
            .into_iter()
            .map(|value| match value {
                SelectDefaultValue::User(id) => SelectValue::User(id),
                SelectDefaultValue::Role(id) => SelectValue::Role(id),
                SelectDefaultValue::Channel(id) => SelectValue::Channel(id),
            })
            .collect(),
        channel_kinds: select
            .channel_types
            .unwrap_or_default()
            .into_iter()
            .filter_map(|kind| match kind {
                ChannelType::GuildText => Some(ChannelKind::Text),
                ChannelType::GuildAnnouncement => Some(ChannelKind::Announcement),
                ChannelType::GuildVoice => Some(ChannelKind::Voice),
                ChannelType::GuildStageVoice => Some(ChannelKind::Stage),
                ChannelType::GuildForum => Some(ChannelKind::Forum),
                ChannelType::GuildCategory => Some(ChannelKind::Category),
                ChannelType::PublicThread
                | ChannelType::PrivateThread
                | ChannelType::AnnouncementThread => Some(ChannelKind::Thread),
                _ => None,
            })
            .collect(),
    }
}

fn media_item(
    media: &raw::UnfurledMediaItem,
    spoiler: bool,
    counter: &mut Counter,
) -> Option<MediaItem> {
    let attachment = media_attachment(media, spoiler, counter.attachments);
    let (images, videos, _) = convert_attachments(std::slice::from_ref(&attachment));
    let media = match (images.into_iter().next(), videos.into_iter().next()) {
        (Some(image), _) => Media::Image(image),
        (None, Some(video)) => Media::Video(video),
        (None, None) => return None,
    };
    let index = counter.media;
    counter.media += 1;
    Some(MediaItem { media, index })
}

/// The message's attachment a component points at, which knows its name and
/// size; an outside URL stands in as one, named after its last path segment.
fn media_attachment(
    media: &raw::UnfurledMediaItem,
    spoiler: bool,
    attachments: &[Attachment],
) -> Attachment {
    let path = |url: &str| url.split('?').next().unwrap_or(url).to_string();
    let wanted = [Some(&media.url), media.proxy_url.as_ref()]
        .into_iter()
        .flatten()
        .map(|url| path(url))
        .collect::<Vec<_>>();
    if let Some(attachment) = attachments.iter().find(|attachment| {
        wanted.contains(&path(&attachment.url)) || wanted.contains(&path(&attachment.proxy_url))
    }) {
        let mut attachment = attachment.clone();
        if spoiler && !attachment.filename.starts_with(SPOILER_PREFIX) {
            attachment.filename = format!("{SPOILER_PREFIX}{}", attachment.filename);
        }
        return attachment;
    }
    let name = path(&media.url)
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("file")
        .to_string();
    Attachment {
        content_type: media.content_type.clone(),
        ephemeral: false,
        duration_secs: None,
        filename: if spoiler {
            format!("{SPOILER_PREFIX}{name}")
        } else {
            name
        },
        flags: None,
        description: None,
        height: media.height.flatten().map(u64::from),
        id: Id::<AttachmentMarker>::new(1),
        placeholder: None,
        placeholder_version: None,
        proxy_url: media.proxy_url.clone().unwrap_or_else(|| media.url.clone()),
        size: 0,
        title: None,
        url: media.url.clone(),
        waveform: None,
        width: media.width.flatten().map(u64::from),
    }
}

/// A form an app opened in answer to an interaction (`INTERACTION_MODAL_CREATE`).
#[derive(Clone)]
pub struct Modal {
    /// The interaction that opened it, which the submission names.
    pub id: String,
    pub nonce: Option<u64>,
    pub channel_id: Id<ChannelMarker>,
    pub application_id: String,
    pub app_name: String,
    pub app_icon_url: Option<String>,
    pub title: String,
    pub custom_id: String,
    pub fields: Vec<ModalField>,
}

#[derive(Clone)]
pub enum ModalField {
    /// Shown as written; markdown isn't drawn outside the message list.
    Text(String),
    Input(ModalInput),
}

/// One answerable field and how it was wrapped, which the submission echoes.
#[derive(Clone)]
pub struct ModalInput {
    pub label: String,
    pub description: Option<String>,
    /// A legacy text input sits in an action row; newer fields in a label.
    pub in_label: bool,
    pub kind: InputKind,
}

#[derive(Clone)]
pub enum InputKind {
    Text {
        custom_id: String,
        paragraph: bool,
        placeholder: Option<String>,
        value: Option<String>,
        min_length: Option<u16>,
        max_length: Option<u16>,
        required: bool,
    },
    Select(Select),
    Checkbox {
        custom_id: String,
        default: bool,
    },
    CheckboxGroup {
        custom_id: String,
        options: Vec<SelectOption>,
        min_values: u8,
        max_values: u8,
        required: bool,
    },
    /// Uploading isn't supported; a required one blocks submitting.
    FileUpload {
        custom_id: String,
        required: bool,
    },
}

impl InputKind {
    pub fn custom_id(&self) -> &str {
        match self {
            Self::Text { custom_id, .. }
            | Self::Checkbox { custom_id, .. }
            | Self::CheckboxGroup { custom_id, .. }
            | Self::FileUpload { custom_id, .. } => custom_id,
            Self::Select(select) => &select.custom_id,
        }
    }
}

/// `INTERACTION_MODAL_CREATE`.
#[derive(Deserialize)]
pub(in crate::discord) struct RawModal {
    id: String,
    /// A string, or a number from some clients.
    #[serde(default)]
    nonce: Option<serde_json::Value>,
    channel_id: Id<ChannelMarker>,
    title: String,
    custom_id: String,
    application: RawModalApp,
    components: Vec<raw::Component>,
}

#[derive(Deserialize)]
struct RawModalApp {
    id: String,
    name: String,
    #[serde(default)]
    icon: Option<String>,
}

pub(in crate::discord) fn convert_modal(modal: RawModal) -> Modal {
    Modal {
        app_icon_url: modal
            .application
            .icon
            .as_deref()
            .map(|hash| cdn::app_icon_url(&modal.application.id, hash)),
        id: modal.id,
        nonce: match modal.nonce {
            Some(serde_json::Value::String(nonce)) => nonce.parse().ok(),
            Some(serde_json::Value::Number(nonce)) => nonce.as_u64(),
            _ => None,
        },
        channel_id: modal.channel_id,
        application_id: modal.application.id,
        app_name: modal.application.name,
        title: modal.title,
        custom_id: modal.custom_id,
        fields: convert_modal_fields(modal.components),
    }
}

fn convert_modal_fields(components: Vec<raw::Component>) -> Vec<ModalField> {
    let mut fields = Vec::new();
    for component in components {
        match component {
            raw::Component::ActionRow(row) => {
                for child in row.components {
                    if let raw::Component::TextInput(input) = child {
                        // The legacy row form names the field on the input itself.
                        #[allow(deprecated)]
                        let label = input.label.clone().unwrap_or_default();
                        fields.push(ModalField::Input(ModalInput {
                            label,
                            description: None,
                            in_label: false,
                            kind: text_input(input),
                        }));
                    }
                }
            }
            raw::Component::Label(label) => {
                let kind = match *label.component {
                    raw::Component::TextInput(input) => text_input(input),
                    raw::Component::SelectMenu(select) => InputKind::Select(convert_select(select)),
                    raw::Component::Checkbox(checkbox) => InputKind::Checkbox {
                        custom_id: checkbox.custom_id,
                        default: checkbox.default.unwrap_or(false),
                    },
                    raw::Component::CheckboxGroup(group) => InputKind::CheckboxGroup {
                        custom_id: group.custom_id,
                        options: group
                            .options
                            .into_iter()
                            .map(|option| SelectOption {
                                label: option.label,
                                value: option.value,
                                description: option.description,
                                emoji: None,
                                default: option.default.unwrap_or(false),
                            })
                            .collect(),
                        min_values: group.min_values.unwrap_or(0),
                        max_values: group.max_values.unwrap_or(u8::MAX),
                        required: group.required.unwrap_or(true),
                    },
                    raw::Component::FileUpload(upload) => InputKind::FileUpload {
                        custom_id: upload.custom_id,
                        required: upload.required.unwrap_or(true),
                    },
                    _ => continue,
                };
                fields.push(ModalField::Input(ModalInput {
                    label: label.label,
                    description: label.description.filter(|text| !text.is_empty()),
                    in_label: true,
                    kind,
                }));
            }
            raw::Component::TextDisplay(text) => {
                fields.push(ModalField::Text(text.content));
            }
            _ => {}
        }
    }
    fields
}

fn text_input(input: raw::TextInput) -> InputKind {
    InputKind::Text {
        custom_id: input.custom_id,
        paragraph: input.style == TextInputStyle::Paragraph,
        placeholder: input.placeholder.filter(|text| !text.is_empty()),
        value: input.value.filter(|text| !text.is_empty()),
        min_length: input.min_length,
        max_length: input.max_length,
        required: input.required.unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(json: &str) -> crate::discord::Message {
        let raw: twilight_model::channel::Message = serde_json::from_str(json).unwrap();
        super::super::convert_message(raw)
    }

    const MESSAGE: &str = r#"{
        "id": "1", "channel_id": "2", "type": 0, "content": "",
        "author": {"id": "3", "username": "bot", "discriminator": "0", "avatar": null, "bot": true},
        "timestamp": "2026-01-01T00:00:00+00:00", "edited_timestamp": null,
        "tts": false, "mention_everyone": false, "mentions": [], "mention_roles": [],
        "attachments": [], "embeds": [], "pinned": false,
        "application_id": "4", "flags": FLAGS,
        "components": COMPONENTS
    }"#;

    fn with(flags: u64, components: &str) -> crate::discord::Message {
        message(
            &MESSAGE
                .replace("FLAGS", &flags.to_string())
                .replace("COMPONENTS", components),
        )
    }

    #[test]
    fn buttons_keep_their_action() {
        let message = with(
            0,
            r#"[{"type": 1, "components": [
                {"type": 2, "style": 1, "label": "Go", "custom_id": "go"},
                {"type": 2, "style": 5, "label": "Site", "url": "https://example.com"},
                {"type": 2, "style": 6, "sku_id": "9"}
            ]}]"#,
        );
        assert_eq!(message.application_id, Some(Id::new(4)));
        let [Component::ActionRow(row)] = message.components.as_slice() else {
            panic!("expected one row");
        };
        let actions: Vec<_> = row
            .iter()
            .map(|component| match component {
                Component::Button(button) => &button.action,
                _ => panic!("expected buttons"),
            })
            .collect();
        assert!(matches!(actions[0], ButtonAction::Interaction(id) if id == "go"));
        assert!(matches!(actions[1], ButtonAction::Link(url) if url == "https://example.com"));
        assert!(matches!(actions[2], ButtonAction::Premium));
    }

    #[test]
    fn layout_components_number_their_text_and_media() {
        let message = with(
            1 << 15,
            r#"[{"type": 17, "accent_color": 16711680, "components": [
                {"type": 10, "content": "Hello"},
                {"type": 9, "components": [{"type": 10, "content": "Side"}],
                 "accessory": {"type": 11, "media": {"url": "https://cdn.example/a.png",
                    "content_type": "image/png", "width": 10, "height": 10}}},
                {"type": 14, "spacing": 2, "divider": false},
                {"type": 12, "items": [{"media": {"url": "https://cdn.example/b.png",
                    "content_type": "image/png"}}]}
            ]}]"#,
        );
        assert_eq!(message.flags, 1 << 15);
        let [
            Component::Container {
                accent: Some(0xff0000),
                components,
                ..
            },
        ] = message.components.as_slice()
        else {
            panic!("expected a red container");
        };
        let Component::Section {
            components: side,
            accessory: Some(accessory),
        } = &components[1]
        else {
            panic!("expected a section");
        };
        assert!(matches!(&side[0], Component::Text(text) if text.index == 1));
        assert!(matches!(&**accessory, Component::Thumbnail(item) if item.index == 0));
        assert!(matches!(
            components[2],
            Component::Separator {
                divider: false,
                large: true
            }
        ));
        assert!(
            matches!(&components[3], Component::Gallery(items) if items.len() == 1 && items[0].index == 1)
        );
    }

    #[test]
    fn selects_read_defaults_and_channel_kinds() {
        let message = with(
            0,
            r#"[{"type": 1, "components": [{"type": 8, "custom_id": "where",
                "channel_types": [0, 2], "max_values": 2,
                "default_values": [{"type": "channel", "id": "5"}]}]}]"#,
        );
        let [Component::ActionRow(row)] = message.components.as_slice() else {
            panic!("expected one row");
        };
        let Component::Select(select) = &row[0] else {
            panic!("expected a select");
        };
        assert!(select.kind == SelectKind::Channel);
        assert_eq!(select.max_values, 2);
        assert_eq!(select.defaults, [SelectValue::Channel(Id::new(5))]);
        assert!(select.channel_kinds == [ChannelKind::Text, ChannelKind::Voice]);
    }
}
