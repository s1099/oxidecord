//! What the quick switcher can open, where choosing one goes, and the
//! frecency that ranks it, loaded from and saved back to Discord.

use std::time::Duration;

use gpui_kit::*;
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::{self, ChannelKind};
use crate::screens::home::HomeScreen;
use crate::screens::home::quick_switcher::{
    Kind, Mode, NARROWED_LIMIT, RowIcon, Section, SwitcherRow, Target, now_ms, search,
};

/// Uses are batched before saving, as Discord's client does.
const FLUSH_DELAY: Duration = Duration::from_secs(30);

/// With nothing else to show, recents fill the list; beside other sections they're kept short.
const RECENT_ALONE: usize = 7;
const RECENT_BESIDE: usize = 3;

impl HomeScreen {
    pub(in crate::screens::home) fn load_frecency(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_frecency().await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(items) => {
                    if this.frecency.set_loaded(items) {
                        this.schedule_frecency_flush(cx);
                    }
                }
                // Ranks by this session's uses alone, and never saves them over Discord's.
                Err(err) => eprintln!("failed to load frecency: {err}"),
            });
        })
        .detach();
    }

    /// Credits an opened guild, channel or DM.
    pub(in crate::screens::home) fn record_use(&mut self, id: u64, cx: &mut Context<Self>) {
        self.frecency.record(id);
        self.schedule_frecency_flush(cx);
    }

    fn schedule_frecency_flush(&mut self, cx: &mut Context<Self>) {
        if !self.frecency.loaded || self.frecency.flush.is_some() {
            return;
        }
        self.frecency.flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLUSH_DELAY).await;
            let Ok(items) = this.update(cx, |this, _| {
                this.frecency.flush = None;
                this.frecency.items().clone()
            }) else {
                return;
            };
            if let Err(err) = discord::save_frecency(items, now_ms()).await {
                eprintln!("failed to save frecency: {err}");
            }
        }));
    }

    pub(in crate::screens::home) fn quick_switcher_sections(&self, query: &str) -> Vec<Section> {
        let (mode, query) = Mode::parse(query);
        if !query.is_empty() {
            let rows = search(self.switcher_candidates(mode), query, mode, &self.frecency);
            return vec![Section {
                heading: None,
                rows,
            }];
        }
        match mode {
            Mode::All => self.switcher_suggestions(),
            Mode::Users => {
                let rows = self
                    .switcher_candidates(mode)
                    .into_iter()
                    .filter(|row| row.kind == Kind::User)
                    .take(NARROWED_LIMIT)
                    .collect();
                section(Some("Direct Messages"), rows)
            }
            Mode::Guilds => {
                let now = now_ms();
                let mut rows = self.switcher_candidates(mode);
                rows.sort_by(|a, b| {
                    let used = |row: &SwitcherRow| self.frecency.of(row.frecency_id, now);
                    used(b)
                        .cmp(&used(a))
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                section(Some("Servers"), rows)
            }
            // The open guild's, in sidebar order.
            Mode::TextChannels | Mode::VoiceChannels => {
                let voice = mode == Mode::VoiceChannels;
                let Some(guild_id) = self.local_guild() else {
                    return Vec::new();
                };
                let rows = self
                    .channel_groups
                    .iter()
                    .flat_map(|group| &group.channels)
                    .filter(|channel| channel.kind.is_voice() == voice)
                    .filter_map(|channel| self.switcher_channel_row(guild_id, channel))
                    .collect();
                section(
                    Some(if voice {
                        "Voice Channels"
                    } else {
                        "Text Channels"
                    }),
                    rows,
                )
            }
        }
    }

    /// Discord's empty-query list: where the user has just been, then what
    /// pinged them, then what's unread in the open guild.
    fn switcher_suggestions(&self) -> Vec<Section> {
        let candidates = self.switcher_candidates(Mode::All);
        let find = |id: u64| {
            candidates
                .iter()
                .find(|row| row.kind != Kind::Guild && row.frecency_id == id)
        };
        let current = self.selected_channel.map(|id| id.get());

        let recent: Vec<SwitcherRow> = self
            .frecency
            .recent()
            .into_iter()
            .filter(|&id| Some(id) != current)
            .filter_map(|id| find(id).cloned())
            .take(RECENT_ALONE)
            .collect();
        let shown = |rows: &[SwitcherRow], id: u64| rows.iter().any(|row| row.frecency_id == id);

        let mut mentions: Vec<SwitcherRow> = candidates
            .iter()
            .filter(|row| {
                row.kind != Kind::Guild && row.mentions > 0 && Some(row.frecency_id) != current
            })
            .cloned()
            .collect();
        mentions
            .sort_by_key(|row| std::cmp::Reverse(self.unreads.latest(Id::new(row.frecency_id))));

        let unread: Vec<SwitcherRow> = match self.local_guild() {
            Some(guild_id) => self
                .channel_groups
                .iter()
                .flat_map(|group| &group.channels)
                .filter(|channel| {
                    !channel.kind.is_voice()
                        && Some(channel.id.get()) != current
                        && self.unreads.shows_unread(channel.id, channel.parent_id)
                })
                .filter_map(|channel| self.switcher_channel_row(guild_id, channel))
                .collect(),
            None => Vec::new(),
        };

        let others_shown = !mentions.is_empty() || !unread.is_empty();
        let recent: Vec<SwitcherRow> = recent
            .into_iter()
            .take(if others_shown {
                RECENT_BESIDE
            } else {
                RECENT_ALONE
            })
            .collect();
        mentions.retain(|row| !shown(&recent, row.frecency_id));
        let unread: Vec<SwitcherRow> = unread
            .into_iter()
            .filter(|row| !shown(&recent, row.frecency_id) && !shown(&mentions, row.frecency_id))
            .collect();

        [
            ("Previous Channels", recent),
            ("Mentions", mentions),
            ("Unread Channels", unread),
        ]
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(heading, rows)| Section {
            heading: Some(heading),
            rows,
        })
        .collect()
    }

    /// Everything of the kinds `mode` searches. Users are the DMs the user
    /// already has, since opening a new one would need a REST call Discord
    /// guards against automation.
    fn switcher_candidates(&self, mode: Mode) -> Vec<SwitcherRow> {
        let mut rows = Vec::new();
        if matches!(mode, Mode::All | Mode::Users) {
            for dm in &self.dms.list {
                let group = dm.recipient_id.is_none();
                if group && mode == Mode::Users {
                    continue;
                }
                rows.push(SwitcherRow {
                    target: Target::Dm(dm.id),
                    kind: if group { Kind::GroupDm } else { Kind::User },
                    icon: RowIcon::Image {
                        name: dm.name.clone(),
                        url: dm.avatar_url.clone(),
                    },
                    name: dm.name.clone(),
                    detail: None,
                    context: None,
                    unread: !self.unreads.is_muted(dm.id, None) && self.unreads.is_unread(dm.id),
                    mentions: self.unreads.dm_mentions(dm.id),
                    frecency_id: dm.id.get(),
                });
            }
        }
        if matches!(mode, Mode::All | Mode::TextChannels | Mode::VoiceChannels) {
            for guild in &self.guilds {
                let Some(channels) = self.guild_data.channels.get(&guild.id) else {
                    continue;
                };
                for channel in channels {
                    let wanted = match mode {
                        Mode::TextChannels => !channel.kind.is_voice(),
                        Mode::VoiceChannels => channel.kind.is_voice(),
                        _ => true,
                    };
                    if wanted && let Some(row) = self.switcher_channel_row(guild.id, channel) {
                        rows.push(row);
                    }
                }
            }
        }
        if matches!(mode, Mode::All | Mode::Guilds) {
            // Discord leaves out the guild already open.
            let open = self.local_guild();
            for guild in self.guilds.iter().filter(|guild| Some(guild.id) != open) {
                rows.push(SwitcherRow {
                    target: Target::Guild(guild.id),
                    kind: Kind::Guild,
                    icon: RowIcon::Image {
                        name: guild.name.clone(),
                        url: guild.icon_url.clone(),
                    },
                    name: guild.name.clone(),
                    detail: None,
                    context: None,
                    unread: self.unreads.is_guild_unread(guild.id),
                    mentions: self.unreads.guild_mentions(guild.id),
                    frecency_id: guild.id.get(),
                });
            }
        }
        rows
    }

    fn switcher_channel_row(
        &self,
        guild_id: Id<GuildMarker>,
        channel: &discord::Channel,
    ) -> Option<SwitcherRow> {
        if channel.kind == ChannelKind::Category {
            return None;
        }
        let channels = self.guild_data.channels.get(&guild_id);
        let category = channel.parent_id.and_then(|parent| {
            channels?
                .iter()
                .find(|other| other.id == parent)
                .map(|other| other.name.clone())
        });
        let guild = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| guild.name.clone());
        let voice = channel.kind.is_voice();
        Some(SwitcherRow {
            target: Target::Channel {
                guild_id,
                channel_id: channel.id,
                voice,
            },
            kind: Kind::Channel,
            icon: RowIcon::Channel(channel.kind),
            name: channel.name.clone(),
            detail: category,
            context: guild,
            unread: !voice && self.unreads.shows_unread(channel.id, channel.parent_id),
            mentions: self.unreads.mentions(channel.id),
            frecency_id: channel.id.get(),
        })
    }

    /// Goes where Discord's switcher would: a voice channel is joined, not
    /// just looked at.
    pub(in crate::screens::home) fn switch_to(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Guild(guild_id) => self.select_guild(guild_id, window, cx),
            Target::Dm(channel_id) => self.open_dm(channel_id, window, cx),
            Target::Channel {
                guild_id,
                channel_id,
                voice: false,
            } => {
                // Opening the guild reopens its last channel, so this one opens
                // even if its list has yet to arrive.
                self.last_guild_channel.insert(guild_id, channel_id);
                self.select_guild(guild_id, window, cx);
                if self.channel_info(channel_id).is_some() {
                    self.select_channel(channel_id, window, cx);
                }
            }
            Target::Channel {
                guild_id,
                channel_id,
                voice: true,
            } => {
                self.select_guild(guild_id, window, cx);
                if self.in_voice_channel(channel_id) {
                    self.show_voice_channel(channel_id, window, cx);
                } else {
                    self.join_voice_channel(channel_id, window, cx);
                }
            }
        }
        cx.notify();
    }
}

fn section(heading: Option<&'static str>, rows: Vec<SwitcherRow>) -> Vec<Section> {
    if rows.is_empty() {
        return Vec::new();
    }
    vec![Section { heading, rows }]
}
