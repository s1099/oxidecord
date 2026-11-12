//! Desktop notifications for incoming messages.
//!
//! Discord's gateway doesn't say which messages should notify; its client
//! decides from the user's settings, and so does this. A message notifies
//! unless it's the user's own, from someone they blocked, sent `@silent`, the
//! user is on Do Not Disturb, or it lands in the conversation they're looking
//! at. Past that, the channel's notification level decides
//! ([`Unreads::notify_level`](crate::screens::home::unread::Unreads::notify_level)),
//! with a mute letting only pings through.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, Block, Inline, Markdown, Mention, MessageLink, NotifyLevel};
use crate::platform::notify::{self, Toast, ToastId};
use crate::platform::prefs;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Clicking a toast opens its message.
    pub(in crate::screens::home) fn start_notifications(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (notifier, clicks) = notify::start();
        self.notifier = Some(notifier);
        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;

            let mut clicks = clicks;
            while let Some(id) = clicks.next().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.open_notification(id, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(in crate::screens::home) fn notify_message(
        &mut self,
        incoming: &discord::IncomingMessage,
        window: &Window,
    ) {
        if !self.should_notify(incoming, window) {
            return;
        }
        let Some(notifier) = &self.notifier else {
            return;
        };
        let message = &incoming.message;
        notifier.show(Toast {
            id: ToastId {
                guild: incoming.guild_id.map(Id::get),
                channel: incoming.channel_id.get(),
                message: message.id.get(),
            },
            title: self.notification_title(incoming),
            body: self.notification_body(message, incoming.guild_id),
            avatar_url: message.author_avatar_url.clone(),
        });
    }

    /// Once a conversation's read, its toasts have nothing left to say.
    pub(in crate::screens::home) fn clear_notifications(&self, channel_id: Id<ChannelMarker>) {
        if let Some(notifier) = &self.notifier {
            notifier.clear(channel_id.get());
        }
    }

    fn should_notify(&self, incoming: &discord::IncomingMessage, window: &Window) -> bool {
        let message = &incoming.message;
        let channel_id = incoming.channel_id;
        if incoming.silent
            || message.system.is_some()
            || Some(message.author_id) == self.self_user_id
            || self.blocked_users.contains(&message.author_id)
            || self.presence_status == Some(discord::PresenceStatus::DoNotDisturb)
            || prefs::with(|prefs| prefs.notifications_off)
        {
            return false;
        }
        let looking_at =
            self.selected_channel == Some(channel_id) && self.messages_channel == Some(channel_id);
        if looking_at && window.is_window_active() {
            return false;
        }

        let channel = incoming.guild_id.and_then(|guild_id| {
            self.guild_data
                .channels
                .get(&guild_id)?
                .iter()
                .find(|channel| channel.id == channel_id)
        });
        let mut level = self.unreads.notify_level(
            incoming.guild_id,
            channel_id,
            channel.and_then(|channel| channel.parent_id),
        );
        // Not in the guild's list, it's a thread. Discord notifies of a thread
        // the user follows only when pinged unless they set otherwise, and
        // thread settings aren't tracked here.
        if incoming.guild_id.is_some() && channel.is_none() && level == NotifyLevel::AllMessages {
            level = NotifyLevel::OnlyMentions;
        }
        match level {
            NotifyLevel::AllMessages => true,
            // A DM is only ever here muted, where `pings_me` lets through a
            // direct ping alone.
            NotifyLevel::OnlyMentions => self.pings_me(channel_id, incoming.guild_id, message),
            NotifyLevel::Nothing => false,
        }
    }

    /// As Discord titles them: the author, then where it was sent.
    fn notification_title(&self, incoming: &discord::IncomingMessage) -> String {
        let author = &incoming.message.author_name;
        let Some(guild_id) = incoming.guild_id else {
            // A group DM is named; a one-on-one DM is the author already.
            return match self.dms.list.iter().find(|dm| dm.id == incoming.channel_id) {
                Some(dm) if dm.recipient_id.is_none() => format!("{author} ({})", dm.name),
                _ => author.clone(),
            };
        };
        let guild = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| guild.name.as_str());
        let channel = self
            .guild_data
            .channels
            .get(&guild_id)
            .and_then(|channels| channels.iter().find(|c| c.id == incoming.channel_id))
            .map(|channel| channel.name.as_str());
        match (channel, guild) {
            (Some(channel), Some(guild)) => format!("{author} (#{channel}, {guild})"),
            (None, Some(guild)) => format!("{author} ({guild})"),
            _ => author.clone(),
        }
    }

    fn notification_body(
        &self,
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) -> String {
        let text = self.plain_text(&message.markdown, message, guild_id);
        if !text.trim().is_empty() {
            return text;
        }
        let attachments = message.images.len() + message.videos.len() + message.files.len();
        if attachments == 1 {
            return "Sent an attachment".into();
        }
        if attachments > 1 {
            return format!("Sent {attachments} attachments");
        }
        if let Some(sticker) = message.stickers.first() {
            return format!("Sent a sticker: {}", sticker.name);
        }
        if message.forward.is_some() {
            return "Forwarded a message".into();
        }
        message
            .embeds
            .iter()
            .find_map(|embed| {
                let mut text = String::new();
                if let Some(title) = &embed.title {
                    self.push_inlines(&mut text, title, message, guild_id);
                }
                if text.trim().is_empty()
                    && let Some(description) = &embed.description
                {
                    text = self.plain_text(description, message, guild_id);
                }
                (!text.trim().is_empty()).then_some(text)
            })
            .unwrap_or_else(|| "Sent a message".into())
    }

    /// Markdown flattened to what a toast can show, with mentions named.
    fn plain_text(
        &self,
        markdown: &Markdown,
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) -> String {
        let mut out = String::new();
        for block in &markdown.blocks {
            if !out.is_empty() {
                out.push('\n');
            }
            self.push_block(&mut out, block, message, guild_id);
        }
        out
    }

    fn push_block(
        &self,
        out: &mut String,
        block: &Block,
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) {
        match block {
            Block::Paragraph(inlines)
            | Block::Heading {
                content: inlines, ..
            }
            | Block::Subtext(inlines) => self.push_inlines(out, inlines, message, guild_id),
            Block::List(list) => self.push_list(out, list, message, guild_id),
            Block::Quote(blocks) => {
                for (ix, block) in blocks.iter().enumerate() {
                    if ix > 0 {
                        out.push('\n');
                    }
                    self.push_block(out, block, message, guild_id);
                }
            }
            Block::Code { code, .. } => out.push_str(code.trim_end()),
        }
    }

    fn push_list(
        &self,
        out: &mut String,
        list: &discord::List,
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) {
        for (ix, item) in list.items.iter().enumerate() {
            if ix > 0 {
                out.push('\n');
            }
            match list.start {
                Some(start) => out.push_str(&format!("{}. ", start + ix as u64)),
                None => out.push_str("• "),
            }
            self.push_inlines(out, &item.content, message, guild_id);
            for child in &item.children {
                out.push('\n');
                self.push_list(out, child, message, guild_id);
            }
        }
    }

    fn push_inlines(
        &self,
        out: &mut String,
        inlines: &[Inline],
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) {
        for inline in inlines {
            match inline {
                Inline::Text(text) | Inline::Code(text) => out.push_str(text),
                Inline::Bold(inner)
                | Inline::Italic(inner)
                | Inline::Underline(inner)
                | Inline::Strikethrough(inner) => self.push_inlines(out, inner, message, guild_id),
                // Kept hidden, as the message itself shows it.
                Inline::Spoiler(_) => out.push_str("▮▮▮▮"),
                Inline::Link { url, label } => match label {
                    Some(label) => self.push_inlines(out, label, message, guild_id),
                    None => out.push_str(url),
                },
                Inline::Mention(mention) => {
                    out.push_str(&self.mention_text(mention, message, guild_id))
                }
                Inline::Emoji(emoji) => {
                    out.push(':');
                    out.push_str(&emoji.name);
                    out.push(':');
                }
                Inline::UnicodeEmoji(emoji) => out.push_str(emoji.as_str()),
                Inline::Timestamp(timestamp) => {
                    out.push_str(&timestamp.display(chrono::Utc::now().timestamp()))
                }
            }
        }
    }

    fn mention_text(
        &self,
        mention: &Mention,
        message: &discord::Message,
        guild_id: Option<Id<GuildMarker>>,
    ) -> String {
        match mention {
            Mention::User(user_id) => {
                let name = guild_id
                    .and_then(|guild_id| self.guild_data.members.get(&guild_id)?.get(user_id))
                    .map(|member| member.name.as_str())
                    .or_else(|| {
                        message
                            .mentions
                            .iter()
                            .find(|user| user.id == *user_id)
                            .map(|user| user.name.as_str())
                    });
                format!("@{}", name.unwrap_or("unknown-user"))
            }
            Mention::Role(role_id) => {
                let name = guild_id
                    .and_then(|guild_id| self.guild_data.roles.get(&guild_id)?.get(role_id))
                    .map(|role| role.name.as_str());
                format!("@{}", name.unwrap_or("unknown-role"))
            }
            Mention::Channel(channel_id) => {
                let name = self
                    .guild_data
                    .channels
                    .values()
                    .flatten()
                    .find(|channel| channel.id == *channel_id)
                    .map(|channel| channel.name.as_str());
                format!("#{}", name.unwrap_or("unknown"))
            }
            Mention::Everyone => "@everyone".into(),
            Mention::Here => "@here".into(),
            Mention::Command(name) => format!("/{name}"),
            Mention::GuildNavigation(navigation) => navigation.label().into(),
        }
    }

    fn open_notification(&mut self, id: ToastId, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        crate::platform::tray::show(window);
        let (Some(channel_id), Some(message_id)) =
            (Id::new_checked(id.channel), Id::new_checked(id.message))
        else {
            return;
        };
        self.open_message_link(
            MessageLink {
                guild_id: id.guild.and_then(Id::new_checked),
                channel_id,
                message_id,
            },
            window,
            cx,
        );
    }
}
