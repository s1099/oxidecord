//! The quick switcher's search, after Discord's own: everything it finds is
//! already in memory, scored by how well the name matches and boosted by
//! frecency, how often and lately the user opened it.
//!
//! A leading `@`, `#`, `!` or `*` narrows the search to DMs, text channels,
//! voice channels or servers, and lifts the per-kind cap.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::Task;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{ChannelKind, FrecencyItem};

/// Results of each kind with no prefix; one with a prefix, or the only kind
/// that matched, shows up to [`NARROWED_LIMIT`].
const MIXED_LIMIT: usize = 5;
pub(super) const NARROWED_LIMIT: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    All,
    Users,
    TextChannels,
    VoiceChannels,
    Guilds,
}

impl Mode {
    /// Splits off the prefix. `@@` is Discord's search beyond the current
    /// guild, which with only DMs to search is the same as `@`.
    pub(super) fn parse(query: &str) -> (Self, &str) {
        let mode = match query.chars().next() {
            Some('@') => Self::Users,
            Some('#') => Self::TextChannels,
            Some('!') => Self::VoiceChannels,
            Some('*') => Self::Guilds,
            _ => return (Self::All, query.trim()),
        };
        (mode, query[1..].trim_start_matches('@').trim())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Guild(Id<GuildMarker>),
    Channel {
        guild_id: Id<GuildMarker>,
        channel_id: Id<ChannelMarker>,
        voice: bool,
    },
    Dm(Id<ChannelMarker>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Kind {
    User,
    GroupDm,
    Channel,
    Guild,
}

#[derive(Clone)]
pub(super) enum RowIcon {
    Channel(ChannelKind),
    Image { name: String, url: Option<String> },
}

#[derive(Clone)]
pub(super) struct SwitcherRow {
    pub target: Target,
    pub kind: Kind,
    pub icon: RowIcon,
    pub name: String,
    /// A channel's category.
    pub detail: Option<String>,
    /// A channel's guild.
    pub context: Option<String>,
    pub unread: bool,
    pub mentions: u32,
    /// Whose frecency ranks it: the channel, DM or guild.
    pub frecency_id: u64,
}

pub(super) struct Section {
    pub heading: Option<&'static str>,
    pub rows: Vec<SwitcherRow>,
}

/// Ranks `candidates` against an already-stripped, non-empty `query`.
pub(super) fn search(
    candidates: Vec<SwitcherRow>,
    query: &str,
    mode: Mode,
    frecency: &Frecency,
) -> Vec<SwitcherRow> {
    let query = query.to_lowercase();
    let now = now_ms();
    let max_frecency = frecency.max(now).max(1) as f32;

    let mut by_kind: HashMap<Kind, Vec<(f32, SwitcherRow)>> = HashMap::new();
    for row in candidates {
        let used = frecency.of(row.frecency_id, now);
        let score = match row.kind {
            Kind::Channel => channel_score(&row, &query, mode, used > 0),
            Kind::User => user_score(&row.name, &query),
            Kind::GroupDm | Kind::Guild => match_score(&row.name, &query),
        };
        if score <= 0. {
            continue;
        }
        let booster = 1. + used as f32 / max_frecency;
        by_kind
            .entry(row.kind)
            .or_default()
            .push((1000. * score * booster, row));
    }

    let limit = if mode == Mode::All && by_kind.len() > 1 {
        MIXED_LIMIT
    } else {
        NARROWED_LIMIT
    };
    let mut ranked: Vec<(f32, SwitcherRow)> = Vec::new();
    for mut rows in by_kind.into_values() {
        sort_ranked(&mut rows);
        rows.truncate(limit);
        ranked.extend(rows);
    }
    sort_ranked(&mut ranked);
    ranked.into_iter().map(|(_, row)| row).collect()
}

fn sort_ranked(rows: &mut [(f32, SwitcherRow)]) {
    rows.sort_by(|(a, row_a), (b, row_b)| {
        b.total_cmp(a)
            .then_with(|| row_a.name.to_lowercase().cmp(&row_b.name.to_lowercase()))
    });
}

/// Discord's tiers: exact, prefix, substring, every word somewhere, then the
/// query's letters in order.
fn match_score(name: &str, query: &str) -> f32 {
    let name = name.to_lowercase();
    if name == query {
        10.
    } else if name.starts_with(query) {
        7.
    } else if name.contains(query) {
        5.
    } else if words(query).all(|word| name.contains(word)) && words(query).next().is_some() {
        3.
    } else if is_subsequence(query, &name) {
        1.
    } else {
        0.
    }
}

fn words(query: &str) -> impl Iterator<Item = &str> {
    query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut haystack = haystack.chars();
    needle
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| haystack.any(|h| h == c))
}

/// Channel names can't hold spaces, so a space in the query also matches a
/// dash. Words the name leaves over may name the category or guild instead,
/// as in "general my-server".
fn channel_score(row: &SwitcherRow, query: &str, mode: Mode, used: bool) -> f32 {
    let name_score = |query: &str| {
        match_score(&row.name, query).max(match_score(&row.name, &query.replace(' ', "-")))
    };
    let mut score = name_score(query);
    if score == 0. {
        let words: Vec<&str> = query.split_whitespace().collect();
        for split in (1..words.len()).rev() {
            let head = name_score(&words[..split].join(" "));
            if head == 0. {
                continue;
            }
            let rest = words[split..].join(" ");
            let extra = [&row.detail, &row.context]
                .into_iter()
                .flatten()
                .map(|other| 0.5 * match_score(other, &rest))
                .sum::<f32>();
            if extra > 0. {
                score = (head + extra).min(6.);
                break;
            }
        }
    }
    if score == 0. {
        return 0.;
    }
    // Voice channels sit under text ones unless asked for with `!`.
    if mode == Mode::All && matches!(row.target, Target::Channel { voice: true, .. }) {
        score = (score - 1.).max(0.5);
    }
    if used {
        score = (score + 3.).min(if score >= 7. { 10. } else { 7. });
    }
    score
}

/// Users match on a prefix, else loosely, as in Discord.
fn user_score(name: &str, query: &str) -> f32 {
    let name = name.to_lowercase();
    if name.starts_with(query) {
        10.
    } else if is_subsequence(query, &name) {
        1.
    } else {
        0.
    }
}

/// The user's guild and channel frecency, kept in step with Discord's copy.
#[derive(Default)]
pub(super) struct Frecency {
    items: HashMap<u64, FrecencyItem>,
    /// Saving before Discord's copy has loaded would overwrite it with only
    /// this session's uses.
    pub loaded: bool,
    /// Uses before it loaded, replayed onto Discord's copy when it does.
    early_uses: Vec<(u64, u64)>,
    pub flush: Option<Task<()>>,
}

impl Frecency {
    pub(super) fn record(&mut self, id: u64) {
        let now = now_ms();
        self.items.entry(id).or_default().record(now);
        if !self.loaded {
            self.early_uses.push((id, now));
        }
    }

    /// Returns whether uses made before it loaded now need saving.
    pub(super) fn set_loaded(&mut self, items: HashMap<u64, FrecencyItem>) -> bool {
        self.items = items;
        self.loaded = true;
        let replayed = !self.early_uses.is_empty();
        for (id, at) in self.early_uses.drain(..) {
            self.items.entry(id).or_default().record(at);
        }
        replayed
    }

    pub(super) fn items(&self) -> &HashMap<u64, FrecencyItem> {
        &self.items
    }

    pub(super) fn of(&self, id: u64, now: u64) -> u32 {
        self.items.get(&id).map_or(0, |item| item.frecency(now))
    }

    fn max(&self, now: u64) -> u32 {
        self.items
            .values()
            .map(|item| item.frecency(now))
            .max()
            .unwrap_or(0)
    }

    /// Ids by when they were last used, newest first.
    pub(super) fn recent(&self) -> Vec<u64> {
        let mut ids: Vec<(&u64, &FrecencyItem)> = self.items.iter().collect();
        ids.sort_by_key(|(_, item)| std::cmp::Reverse(item.last_used()));
        ids.into_iter().map(|(&id, _)| id).collect()
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: u64, name: &str, category: &str, guild: &str, voice: bool) -> SwitcherRow {
        SwitcherRow {
            target: Target::Channel {
                guild_id: Id::new(1),
                channel_id: Id::new(id),
                voice,
            },
            kind: Kind::Channel,
            icon: RowIcon::Channel(if voice {
                ChannelKind::Voice
            } else {
                ChannelKind::Text
            }),
            name: name.to_owned(),
            detail: Some(category.to_owned()),
            context: Some(guild.to_owned()),
            unread: false,
            mentions: 0,
            frecency_id: id,
        }
    }

    fn names(rows: &[SwitcherRow]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn prefixes_pick_a_mode() {
        assert_eq!(Mode::parse("#gen"), (Mode::TextChannels, "gen"));
        assert_eq!(Mode::parse("@@ana"), (Mode::Users, "ana"));
        assert_eq!(Mode::parse("*"), (Mode::Guilds, ""));
        assert_eq!(Mode::parse(" hi "), (Mode::All, "hi"));
    }

    #[test]
    fn tiers_rank_exact_over_prefix_over_contains() {
        assert_eq!(match_score("general", "general"), 10.);
        assert_eq!(match_score("general", "gen"), 7.);
        assert_eq!(match_score("off-topic", "topic"), 5.);
        assert_eq!(match_score("dev-help-rust", "rust help"), 3.);
        assert_eq!(match_score("general", "gnrl"), 1.);
        assert_eq!(match_score("general", "xyz"), 0.);
    }

    #[test]
    fn spaces_match_dashes_and_spill_into_the_guild() {
        let rows = vec![
            channel(2, "off-topic", "Chat", "Rustaceans", false),
            channel(3, "general", "Chat", "Rustaceans", false),
            channel(4, "general", "Chat", "Gophers", false),
        ];
        let frecency = Frecency::default();
        let found = search(rows.clone(), "off topic", Mode::All, &frecency);
        assert_eq!(names(&found), ["off-topic"]);
        let found = search(rows, "general rust", Mode::All, &frecency);
        assert_eq!(found[0].frecency_id, 3);
    }

    #[test]
    fn frecency_and_voice_reorder_equal_matches() {
        let rows = vec![
            channel(2, "lounge-c", "", "", true),
            channel(3, "lounge-a", "", "", false),
            channel(4, "lounge-b", "", "", false),
        ];
        let mut frecency = Frecency::default();
        frecency.record(4);
        let found = search(rows, "lounge", Mode::All, &frecency);
        assert_eq!(names(&found), ["lounge-b", "lounge-a", "lounge-c"]);
    }
}
