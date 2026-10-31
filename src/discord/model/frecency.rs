//! How often and how lately each guild and channel was opened, from Discord's
//! `FrecencyUserSettings` (`settings-proto/2`). The quick switcher ranks by it.
//!
//! Field 12 is `GuildAndChannelFrecency`, a `map<fixed64, FrecencyItem>` keyed
//! by guild or channel id. An item holds `total_uses` (1), `recent_uses` (2, up
//! to ten millisecond timestamps), and the `frecency` (3) and `score` (4)
//! computed from them, which are recomputed here since they decay with time.

use std::collections::HashMap;

use super::settings::{
    Fields, Value, base64_decode, base64_encode, length_delimited, read_varint, write_varint,
};

/// Discord's client keeps this many timestamps per item.
const MAX_SAMPLES: usize = 10;

/// Discord's client writes back only the most recently used this many.
const MAX_SAVED: usize = 100;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FrecencyItem {
    pub total_uses: u32,
    /// Unix milliseconds, oldest first.
    pub recent_uses: Vec<u64>,
}

impl FrecencyItem {
    pub fn last_used(&self) -> u64 {
        self.recent_uses.last().copied().unwrap_or(0)
    }

    pub fn record(&mut self, now_ms: u64) {
        self.total_uses = self.total_uses.saturating_add(1);
        self.recent_uses.push(now_ms);
        let excess = self.recent_uses.len().saturating_sub(MAX_SAMPLES);
        self.recent_uses.drain(..excess);
    }

    /// The summed recency weights of the samples, as Discord computes it.
    pub fn score(&self, now_ms: u64) -> u32 {
        self.recent_uses
            .iter()
            .map(|&used| match now_ms.saturating_sub(used) / DAY_MS {
                0 => 100,
                1 => 70,
                2..=3 => 50,
                4..=6 => 30,
                _ => 10,
            })
            .sum()
    }

    /// Total uses scaled by how recent the sampled ones were.
    pub fn frecency(&self, now_ms: u64) -> u32 {
        if self.recent_uses.is_empty() {
            return 0;
        }
        let samples = self.recent_uses.len() as u64;
        (u64::from(self.total_uses) * u64::from(self.score(now_ms))).div_ceil(samples) as u32
    }
}

pub(in crate::discord) fn parse_frecency_settings(
    settings: &str,
) -> Result<HashMap<u64, FrecencyItem>, String> {
    let bytes = base64_decode(settings)
        .ok_or_else(|| "settings-proto blob is not valid base64".to_owned())?;
    let mut items = HashMap::new();
    for (number, value) in Fields::new(&bytes) {
        if let (12, Value::Bytes(bytes)) = (number, value) {
            for (number, value) in Fields::new(bytes) {
                if let (1, Value::Bytes(entry)) = (number, value)
                    && let Some((id, item)) = parse_entry(entry)
                {
                    items.insert(id, item);
                }
            }
        }
    }
    Ok(items)
}

fn parse_entry(bytes: &[u8]) -> Option<(u64, FrecencyItem)> {
    let mut id = None;
    let mut item = FrecencyItem::default();
    for (number, value) in Fields::new(bytes) {
        match (number, value) {
            (1, Value::Fixed64(key) | Value::Varint(key)) => id = Some(key),
            (2, Value::Bytes(bytes)) => item = parse_item(bytes),
            _ => {}
        }
    }
    Some((id.filter(|&id| id != 0)?, item))
}

fn parse_item(bytes: &[u8]) -> FrecencyItem {
    let mut item = FrecencyItem::default();
    for (number, value) in Fields::new(bytes) {
        match (number, value) {
            (1, Value::Varint(uses)) => item.total_uses = uses as u32,
            (2, Value::Varint(used)) => item.recent_uses.push(used),
            // Repeated scalars are packed in proto3.
            (2, Value::Bytes(packed)) => {
                let mut pos = 0;
                while let Some(used) = read_varint(packed, &mut pos) {
                    item.recent_uses.push(used);
                }
            }
            _ => {}
        }
    }
    item.recent_uses.sort_unstable();
    item
}

/// Encodes a `FrecencyUserSettings` holding only the guild and channel map,
/// for `PATCH /users/@me/settings-proto/2`. The whole map is sent, as
/// Discord's client does, so it must be the one loaded plus local uses.
pub(in crate::discord) fn encode_frecency_settings(
    items: &HashMap<u64, FrecencyItem>,
    now_ms: u64,
) -> String {
    let mut newest: Vec<(&u64, &FrecencyItem)> = items.iter().collect();
    newest.sort_by_key(|(_, item)| std::cmp::Reverse(item.last_used()));

    let mut map = Vec::new();
    for (&id, item) in newest.into_iter().take(MAX_SAVED) {
        let mut value = Vec::new();
        write_varint(&mut value, 1 << 3);
        write_varint(&mut value, u64::from(item.total_uses));
        let mut packed = Vec::new();
        for &used in &item.recent_uses {
            write_varint(&mut packed, used);
        }
        value.extend(length_delimited(2, &packed));
        write_varint(&mut value, 3 << 3);
        write_varint(&mut value, u64::from(item.frecency(now_ms)));
        write_varint(&mut value, 4 << 3);
        write_varint(&mut value, u64::from(item.score(now_ms)));

        let mut entry = Vec::new();
        write_varint(&mut entry, 1 << 3 | 1);
        entry.extend_from_slice(&id.to_le_bytes());
        entry.extend(length_delimited(2, &value));
        map.extend(length_delimited(1, &entry));
    }
    base64_encode(&length_delimited(12, &map))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn frecency_decays_with_age() {
        let mut item = FrecencyItem::default();
        item.record(NOW);
        item.record(NOW - 3 * DAY_MS);
        assert_eq!(item.score(NOW), 150);
        // Two uses averaging 75 points each.
        assert_eq!(item.frecency(NOW), 150);
        assert_eq!(item.frecency(NOW + 10 * DAY_MS), 20);
    }

    #[test]
    fn keeps_only_the_latest_samples() {
        let mut item = FrecencyItem::default();
        for i in 0..15 {
            item.record(NOW + i);
        }
        assert_eq!(item.total_uses, 15);
        assert_eq!(item.recent_uses.len(), MAX_SAMPLES);
        assert_eq!(item.recent_uses[0], NOW + 5);
    }

    #[test]
    fn round_trips_through_a_patch() {
        let mut item = FrecencyItem::default();
        item.record(NOW - DAY_MS);
        item.record(NOW);
        let items = HashMap::from([(123_456_789_012_345_678, item)]);
        let parsed = parse_frecency_settings(&encode_frecency_settings(&items, NOW)).unwrap();
        assert_eq!(parsed, items);
    }
}
