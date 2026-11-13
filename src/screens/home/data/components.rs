//! Message components in use: pressing buttons and picking from selects as
//! interactions, the guild's users, roles and channels an auto-populated select
//! offers, and routing an app's modal and its outcome.

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord::{self, ChannelKind, Select, SelectKind, SelectValue};
use crate::screens::home::HomeScreen;
use crate::screens::home::autocomplete::{Suggestion, best, match_rank};
use crate::screens::home::components::{
    Choice, ChoiceIcon, OpenModal, OpenSelect, PendingInteraction,
};
use crate::screens::home::view::components::{ModalDialog, PickerEvent, SelectPicker};

use super::messages::next_nonce;

/// How long a press spins before it's given up on. Discord fails an
/// interaction the app doesn't answer within three seconds, so this only
/// catches the gateway's answer going missing.
const INTERACTION_TIMEOUT: Duration = Duration::from_secs(15);

impl HomeScreen {
    pub(in crate::screens::home) fn press_component_button(
        &mut self,
        message_id: Id<MessageMarker>,
        custom_id: String,
        cx: &mut Context<Self>,
    ) {
        self.interact_component(message_id, 2, custom_id, None, cx);
    }

    /// Whether an interaction on `custom_id` is waiting on its app.
    pub(in crate::screens::home) fn component_pending(
        &self,
        message_id: Id<MessageMarker>,
        custom_id: &str,
    ) -> bool {
        self.components
            .pending
            .values()
            .any(|pending| pending.message_id == message_id && pending.custom_id == custom_id)
    }

    /// Discord's client lets a message have one interaction in flight; its
    /// other buttons and selects wait.
    pub(in crate::screens::home) fn components_busy(&self, message_id: Id<MessageMarker>) -> bool {
        self.components
            .pending
            .values()
            .any(|pending| pending.message_id == message_id)
    }

    fn interact_component(
        &mut self,
        message_id: Id<MessageMarker>,
        component_type: u8,
        custom_id: String,
        values: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if self.components_busy(message_id) {
            return;
        }
        let Some(message) = self
            .messages
            .iter()
            .find(|message| message.id == message_id)
        else {
            return;
        };
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(session_id) = self.gateway_session.clone() else {
            self.components
                .failed
                .insert(message_id, "Not connected to Discord yet.".into());
            cx.notify();
            return;
        };
        let (nonce, _) = next_nonce();
        let interaction = discord::ComponentInteraction {
            // A bot's own messages may lack it; the bot's id is its app's.
            application_id: message
                .application_id
                .unwrap_or_else(|| message.author_id.cast()),
            guild_id: self.local_guild(),
            channel_id,
            message_id,
            message_flags: message.flags,
            session_id,
            nonce,
            component_type,
            custom_id: custom_id.clone(),
            values,
        };
        self.components.failed.remove(&message_id);
        let timeout = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTERACTION_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                this.component_interaction_failed(nonce, true, cx);
            });
        });
        self.components.pending.insert(
            nonce,
            PendingInteraction {
                message_id,
                custom_id,
                _timeout: timeout,
            },
        );
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Err(err) = discord::interact_component(interaction).await {
                eprintln!("component interaction failed: {err}");
                let _ = this.update(cx, |this, cx| {
                    this.component_interaction_failed(nonce, false, cx)
                });
            }
        })
        .detach();
    }

    /// Returns whether the nonce was a component's.
    pub(in crate::screens::home) fn component_interaction_succeeded(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.modal_outcome(nonce, None, window, cx) {
            return true;
        }
        if self.components.pending.remove(&nonce).is_none() {
            return false;
        }
        cx.notify();
        true
    }

    pub(in crate::screens::home) fn component_interaction_failed(
        &mut self,
        nonce: u64,
        timed_out: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pending) = self.components.pending.remove(&nonce) else {
            return false;
        };
        let app = self
            .messages
            .iter()
            .find(|message| message.id == pending.message_id)
            .map(|message| self.author_name(message.author_id, &message.author_name));
        let error = match (timed_out, app) {
            (true, Some(app)) => format!("{app} did not respond in time"),
            (true, None) => "The application did not respond in time".into(),
            (false, _) => "This interaction failed.".into(),
        };
        self.components.failed.insert(pending.message_id, error);
        cx.notify();
        true
    }

    /// An edit is the app's answer: what it now shows replaces the picks, and
    /// any spinner or failure on it.
    pub(in crate::screens::home) fn component_message_changed(
        &mut self,
        message_id: Id<MessageMarker>,
    ) {
        self.components.failed.remove(&message_id);
        self.components
            .pending
            .retain(|_, pending| pending.message_id != message_id);
        self.components
            .picks
            .retain(|(picked_on, _), _| *picked_on != message_id);
    }

    /// The choices a select shows as picked: the latest picks, else its defaults.
    pub(in crate::screens::home) fn select_picks(
        &self,
        message_id: Id<MessageMarker>,
        select: &Select,
    ) -> Vec<Choice> {
        if let Some(picks) = self
            .components
            .picks
            .get(&(message_id, select.custom_id.clone()))
        {
            return picks.clone();
        }
        self.default_choices(select)
    }

    pub(in crate::screens::home) fn default_choices(&self, select: &Select) -> Vec<Choice> {
        match select.kind {
            SelectKind::Text => select
                .options
                .iter()
                .filter(|option| option.default)
                .map(option_choice)
                .collect(),
            _ => select
                .defaults
                .iter()
                .map(|value| self.value_choice(*value))
                .collect(),
        }
    }

    fn value_choice(&self, value: SelectValue) -> Choice {
        let (label, icon) = match value {
            SelectValue::User(id) => {
                let known = self
                    .local_guild()
                    .and_then(|guild_id| self.guild_data.members.get(&guild_id)?.get(&id));
                let author = self.messages.iter().find(|message| message.author_id == id);
                let label = known
                    .map(|member| member.name.clone())
                    .or_else(|| author.map(|message| message.author_name.clone()))
                    .unwrap_or_else(|| format!("User {id}"));
                let avatar = known
                    .and_then(|member| member.avatar_url.clone())
                    .or_else(|| author.and_then(|message| message.author_avatar_url.clone()));
                (label, ChoiceIcon::Avatar(avatar))
            }
            SelectValue::Role(id) => {
                let role = self
                    .local_guild()
                    .and_then(|guild_id| self.guild_data.roles.get(&guild_id)?.get(&id));
                (
                    role.map_or_else(|| "Unknown role".into(), |role| role.name.clone()),
                    ChoiceIcon::Role(role.and_then(|role| role.color)),
                )
            }
            SelectValue::Channel(id) => {
                let channel = self
                    .channel_groups
                    .iter()
                    .flat_map(|group| group.category.iter().chain(&group.channels))
                    .find(|channel| channel.id == id);
                (
                    channel.map_or_else(|| "unknown-channel".into(), |c| c.name.clone()),
                    ChoiceIcon::Channel(channel.map_or(ChannelKind::Text, |c| c.kind)),
                )
            }
        };
        Choice {
            value: value.id(),
            label,
            description: None,
            icon,
        }
    }

    /// What a select offers for `query`: its own options, or the guild's
    /// users, roles or channels the client knows of.
    pub(in crate::screens::home) fn select_choices(
        &mut self,
        select: &Select,
        query: &str,
    ) -> Vec<Choice> {
        let query = query.trim().to_lowercase();
        let (users, roles) = match select.kind {
            SelectKind::Text => {
                return select
                    .options
                    .iter()
                    .filter(|option| match_rank(&option.label, &query).is_some())
                    .map(option_choice)
                    .collect();
            }
            SelectKind::Channel => return self.channel_choices(select, &query),
            SelectKind::User => (true, false),
            SelectKind::Role => (false, true),
            SelectKind::Mentionable => (true, true),
        };
        let mut choices = Vec::new();
        if users {
            self.search_guild_members(&query);
            choices.extend(
                self.user_suggestions(&query).into_iter().filter_map(
                    |suggestion| match suggestion {
                        Suggestion::User {
                            id,
                            name,
                            username,
                            avatar_url,
                        } => Some(Choice {
                            value: id.to_string(),
                            label: name,
                            description: username,
                            icon: ChoiceIcon::Avatar(avatar_url),
                        }),
                        _ => None,
                    },
                ),
            );
        }
        if roles && let Some(guild_id) = self.local_guild() {
            let mut all: Vec<_> = self
                .guild_data
                .roles
                .get(&guild_id)
                .into_iter()
                .flat_map(|roles| roles.values())
                .collect();
            all.sort_by_key(|role| std::cmp::Reverse(role.position));
            choices.extend(best(all.into_iter().filter_map(|role| {
                Some((
                    match_rank(&role.name, &query)?,
                    Choice {
                        value: role.id.to_string(),
                        label: role.name.clone(),
                        description: None,
                        icon: ChoiceIcon::Role(role.color),
                    },
                ))
            })));
        }
        choices
    }

    fn channel_choices(&self, select: &Select, query: &str) -> Vec<Choice> {
        best(
            self.channel_groups
                .iter()
                .flat_map(|group| {
                    let category = group.category.as_ref().map(|category| &category.name);
                    group
                        .category
                        .iter()
                        .chain(&group.channels)
                        .map(move |channel| (channel, category))
                })
                .filter(|(channel, _)| {
                    if select.channel_kinds.is_empty() {
                        channel.kind != ChannelKind::Category
                    } else {
                        select.channel_kinds.contains(&channel.kind)
                    }
                })
                .filter_map(|(channel, category)| {
                    Some((
                        match_rank(&channel.name, query)?,
                        Choice {
                            value: channel.id.to_string(),
                            label: channel.name.clone(),
                            description: category
                                .filter(|_| channel.kind != ChannelKind::Category)
                                .cloned(),
                            icon: ChoiceIcon::Channel(channel.kind),
                        },
                    ))
                }),
        )
    }

    pub(in crate::screens::home) fn open_component_select(
        &mut self,
        message_id: Id<MessageMarker>,
        select: Select,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picks = self.select_picks(message_id, &select);
        let choices = self.select_choices(&select, "");
        let custom_id = select.custom_id.clone();
        let screen = cx.entity();
        let picker = cx.new(|cx| SelectPicker::new(screen, select, picks, choices, window, cx));
        let picked = cx.subscribe(&picker, move |this, _, event: &PickerEvent, cx| {
            let PickerEvent::Done = event;
            this.close_component_select(cx);
        });
        self.components.open_select = Some(OpenSelect {
            message_id,
            custom_id,
            picker,
            _picked: picked,
        });
        cx.notify();
    }

    /// Sends the picks if they changed and are enough.
    pub(in crate::screens::home) fn close_component_select(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.components.open_select.take() else {
            return;
        };
        cx.notify();
        let picker = open.picker.read(cx);
        let select = picker.select().clone();
        let picks = picker.picks().to_vec();
        let key = (open.message_id, open.custom_id.clone());
        let before = self.select_picks(open.message_id, &select);
        let same = picks.len() == before.len()
            && picks
                .iter()
                .all(|pick| before.iter().any(|old| old.value == pick.value));
        if same || picks.len() < select.min_values as usize {
            return;
        }
        let values = picks.iter().map(|pick| pick.value.clone()).collect();
        self.components.picks.insert(key, picks);
        self.interact_component(
            open.message_id,
            select.kind.component_type(),
            open.custom_id,
            Some(values),
            cx,
        );
    }

    /// Opens an app's form, if it answers an interaction from here.
    pub(in crate::screens::home) fn open_modal(
        &mut self,
        modal: discord::Modal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(nonce) = modal.nonce {
            self.components.pending.remove(&nonce);
            self.command_opened_modal(nonce, cx);
        }
        if let Some(open) = self.components.modal.take()
            && let Some(dialog) = open.dialog.upgrade()
        {
            // A newer form replaces it, as Discord's client does.
            drop(dialog);
            window.close_dialog(cx);
        }
        let screen = cx.entity();
        let guild_id = self.local_guild();
        let dialog = cx.new(|cx| ModalDialog::new(screen, modal, guild_id, window, cx));
        self.components.modal = Some(OpenModal {
            dialog: dialog.downgrade(),
            nonce: None,
        });
        ModalDialog::open(dialog, window, cx);
        cx.notify();
    }

    /// Notes the nonce a modal was submitted under, for the outcome to find it.
    pub(in crate::screens::home) fn modal_submitted(&mut self, nonce: u64) {
        if let Some(open) = &mut self.components.modal {
            open.nonce = Some(nonce);
        }
    }

    pub(in crate::screens::home) fn forget_modal(&mut self) {
        self.components.modal = None;
    }

    /// Closes the modal on success, or shows why it failed. Returns whether
    /// the nonce was its submission's.
    pub(in crate::screens::home) fn modal_outcome(
        &mut self,
        nonce: u64,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = &self.components.modal else {
            return false;
        };
        if open.nonce != Some(nonce) {
            return false;
        }
        let dialog = open.dialog.upgrade();
        match (error, dialog) {
            (None, _) => {
                self.components.modal = None;
                window.close_dialog(cx);
            }
            (Some(error), Some(dialog)) => {
                dialog.update(cx, |dialog, cx| dialog.failed(error, cx));
            }
            (Some(_), None) => self.components.modal = None,
        }
        true
    }
}

fn option_choice(option: &discord::SelectOption) -> Choice {
    Choice {
        value: option.value.clone(),
        label: option.label.clone(),
        description: option.description.clone(),
        icon: option
            .emoji
            .clone()
            .map_or(ChoiceIcon::None, ChoiceIcon::Emoji),
    }
}
