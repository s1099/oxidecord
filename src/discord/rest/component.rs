//! Interacting with message components and submitting the modals apps open.
//! Both are `POST /interactions`, answered 204; the outcome follows on the
//! gateway, matched by `nonce`.

use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, GuildMarker, MessageMarker},
};

use super::request;
use crate::discord::model::{InputKind, ModalInput};

/// A button pressed or a select's values chosen (type 3).
pub struct ComponentInteraction {
    pub application_id: Id<ApplicationMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Id<ChannelMarker>,
    pub message_id: Id<MessageMarker>,
    pub message_flags: u64,
    /// The gateway session's; Discord delivers ephemeral replies to it alone.
    pub session_id: String,
    pub nonce: u64,
    pub component_type: u8,
    pub custom_id: String,
    /// A select's picks; `None` for a button.
    pub values: Option<Vec<String>>,
}

pub async fn interact_component(interaction: ComponentInteraction) -> Result<(), String> {
    let mut data = serde_json::json!({
        "component_type": interaction.component_type,
        "custom_id": interaction.custom_id,
    });
    // Discord's client spreads a select's state into `data`, its type included.
    if let Some(values) = interaction.values {
        data["type"] = interaction.component_type.into();
        data["values"] = values.into();
    }
    let mut body = serde_json::json!({
        "type": 3,
        "nonce": interaction.nonce.to_string(),
        "channel_id": interaction.channel_id,
        "message_flags": interaction.message_flags,
        "message_id": interaction.message_id,
        "application_id": interaction.application_id,
        "session_id": interaction.session_id,
        "data": data,
    });
    if let Some(guild_id) = interaction.guild_id {
        body["guild_id"] = guild_id.to_string().into();
    }
    post(body).await
}

/// What was entered into one field; `None` is left untouched, sent as `null`.
pub enum ModalAnswer {
    Text(Option<String>),
    Values(Option<Vec<String>>),
    Checked(bool),
}

/// One of a modal's top-level components, in the app's order.
pub enum ModalEntry {
    /// Echoed back as its type alone.
    Display,
    Field(Box<ModalInput>, ModalAnswer),
}

/// A filled-in modal (type 5).
pub struct ModalSubmission {
    pub application_id: String,
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Id<ChannelMarker>,
    pub session_id: String,
    pub nonce: u64,
    /// The interaction that opened the modal.
    pub modal_id: String,
    pub custom_id: String,
    pub entries: Vec<ModalEntry>,
}

pub async fn submit_modal(submission: ModalSubmission) -> Result<(), String> {
    let components: Vec<_> = submission.entries.iter().map(modal_component).collect();
    let mut body = serde_json::json!({
        "type": 5,
        "nonce": submission.nonce.to_string(),
        "channel_id": submission.channel_id,
        "application_id": submission.application_id,
        "session_id": submission.session_id,
        "data": {
            "id": submission.modal_id,
            "custom_id": submission.custom_id,
            "components": components,
        },
    });
    if let Some(guild_id) = submission.guild_id {
        body["guild_id"] = guild_id.to_string().into();
    }
    post(body).await
}

/// The field as the app laid it out, its value filled in. Component ids are
/// left out, as Discord's client does.
fn modal_component(entry: &ModalEntry) -> serde_json::Value {
    let ModalEntry::Field(input, answer) = entry else {
        return serde_json::json!({ "type": 10 });
    };
    let (kind, value_key, value) = match (&input.kind, answer) {
        (InputKind::Text { .. }, ModalAnswer::Text(text)) => (4, "value", text.clone().into()),
        (InputKind::Select(select), ModalAnswer::Values(values)) => (
            select.kind.component_type(),
            "values",
            values.clone().into(),
        ),
        (InputKind::CheckboxGroup { .. }, ModalAnswer::Values(values)) => {
            (22, "values", values.clone().into())
        }
        (InputKind::FileUpload { .. }, ModalAnswer::Values(values)) => {
            (19, "values", values.clone().into())
        }
        (InputKind::Checkbox { .. }, ModalAnswer::Checked(checked)) => {
            (23, "value", (*checked).into())
        }
        _ => (4, "value", serde_json::Value::Null),
    };
    let mut inner = serde_json::json!({
        "type": kind,
        "custom_id": input.kind.custom_id(),
    });
    inner[value_key] = value;
    if input.in_label {
        serde_json::json!({ "type": 18, "component": inner })
    } else {
        serde_json::json!({ "type": 1, "components": [inner] })
    }
}

async fn post(body: serde_json::Value) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, "interactions".to_owned())
            .json(&body)
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::model::{Select, SelectKind};

    fn input(in_label: bool, kind: InputKind) -> ModalInput {
        ModalInput {
            label: "Label".into(),
            description: None,
            in_label,
            kind,
        }
    }

    #[test]
    fn modal_fields_echo_their_layout() {
        let text = input(
            false,
            InputKind::Text {
                custom_id: "name".into(),
                paragraph: false,
                placeholder: None,
                value: None,
                min_length: None,
                max_length: None,
                required: false,
            },
        );
        let select = input(
            true,
            InputKind::Select(Select {
                kind: SelectKind::User,
                custom_id: "who".into(),
                placeholder: None,
                min_values: 1,
                max_values: 1,
                disabled: false,
                required: true,
                options: Vec::new(),
                defaults: Vec::new(),
                channel_kinds: Vec::new(),
            }),
        );
        let entries = [
            ModalEntry::Field(Box::new(text), ModalAnswer::Text(None)),
            ModalEntry::Display,
            ModalEntry::Field(
                Box::new(select),
                ModalAnswer::Values(Some(vec!["7".into()])),
            ),
        ];
        let json: Vec<_> = entries.iter().map(modal_component).collect();
        assert_eq!(
            serde_json::Value::Array(json),
            serde_json::json!([
                {"type": 1, "components": [{"type": 4, "custom_id": "name", "value": null}]},
                {"type": 10},
                {"type": 18, "component": {"type": 5, "custom_id": "who", "values": ["7"]}},
            ])
        );
    }
}
