//! Server ordering and online status from Discord's `settings-proto`.
//!
//! `GET /users/@me/settings-proto/1` returns `PreloadedUserSettings` as a
//! base64 protobuf. Field 14 is `GuildFolders`: `folders` holds every top-level
//! entry (a real folder when it has an id, else a lone guild) and
//! `guild_positions` the flattened guild order. Field 11 is `StatusSettings`;
//! the chosen status lives here rather than in presence so it survives
//! restarts and follows the user across clients.
//!
//! The wire format is walked by hand; a protobuf runtime isn't worth it for
//! one message.

use serde::Deserialize;
use twilight_model::id::{Id, marker::GuildMarker};

#[derive(Deserialize)]
pub(in crate::discord) struct RawSettingsProto {
    pub settings: String,
}

#[derive(Debug, Default, Clone)]
pub struct UserSettings {
    pub guild_folders: GuildFolders,
    /// `None` when never picked; Discord treats it as online.
    pub status: Option<PresenceStatus>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PresenceStatus {
    #[default]
    Online,
    Idle,
    DoNotDisturb,
    /// Shown to everyone else as offline.
    Invisible,
}

impl PresenceStatus {
    pub const ALL: [Self; 4] = [
        Self::Online,
        Self::Idle,
        Self::DoNotDisturb,
        Self::Invisible,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::DoNotDisturb => "dnd",
            Self::Invisible => "invisible",
        }
    }

    /// Discord sends `offline` for both offline and invisible users.
    pub(in crate::discord) fn from_presence(status: &str) -> Option<Self> {
        match status {
            "offline" => Some(Self::Invisible),
            status => Self::parse(status),
        }
    }

    fn parse(status: &str) -> Option<Self> {
        Some(match status {
            "online" => Self::Online,
            "idle" => Self::Idle,
            "dnd" => Self::DoNotDisturb,
            "invisible" => Self::Invisible,
            _ => return None,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct GuildFolders {
    pub folders: Vec<GuildFolder>,
    /// Folders flattened into their contents.
    pub guild_positions: Vec<Id<GuildMarker>>,
}

#[derive(Debug, Default, Clone)]
pub struct GuildFolder {
    /// `None` for a bare top-level guild.
    pub id: Option<i64>,
    /// Unset for unnamed folders, which the client renders from their members.
    pub name: Option<String>,
    pub color: Option<u32>,
    pub guild_ids: Vec<Id<GuildMarker>>,
}

pub(in crate::discord) fn parse_user_settings(settings: &str) -> Result<UserSettings, String> {
    let bytes = base64_decode(settings)
        .ok_or_else(|| "settings-proto blob is not valid base64".to_owned())?;

    let mut parsed = UserSettings::default();
    for (number, value) in Fields::new(&bytes) {
        match (number, value) {
            (11, Value::Bytes(bytes)) => parsed.status = parse_status(bytes),
            (14, Value::Bytes(bytes)) => parsed.guild_folders = parse_folders_message(bytes),
            _ => {}
        }
    }
    Ok(parsed)
}

/// `StatusSettings.status`, a `StringValue` in field 1.
fn parse_status(bytes: &[u8]) -> Option<PresenceStatus> {
    Fields::new(bytes).find_map(|(number, value)| match (number, value) {
        (1, Value::Bytes(bytes)) => unwrap_bytes(bytes)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(PresenceStatus::parse),
        _ => None,
    })
}

/// Encodes a `PreloadedUserSettings` with only `status.status`, for
/// `PATCH /users/@me/settings-proto/1`. Discord merges patches, so omitted
/// fields keep their values.
pub(in crate::discord) fn encode_status_settings(status: PresenceStatus) -> String {
    let string_value = length_delimited(1, status.as_str().as_bytes());
    let status_settings = length_delimited(1, &string_value);
    base64_encode(&length_delimited(11, &status_settings))
}

fn length_delimited(number: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 4);
    write_varint(&mut out, u64::from(number) << 3 | 2);
    write_varint(&mut out, bytes.len() as u64);
    out.extend_from_slice(bytes);
    out
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn parse_folders_message(bytes: &[u8]) -> GuildFolders {
    let mut folders = GuildFolders::default();
    for (number, value) in Fields::new(bytes) {
        let Value::Bytes(bytes) = value else { continue };
        match number {
            1 => folders.folders.push(parse_folder(bytes)),
            2 => folders.guild_positions = parse_ids(bytes),
            _ => {}
        }
    }
    folders
}

fn parse_folder(bytes: &[u8]) -> GuildFolder {
    let mut folder = GuildFolder::default();
    for (number, value) in Fields::new(bytes) {
        let Value::Bytes(bytes) = value else { continue };
        // Fields 2-4 are protobuf wrappers (`Int64Value` and friends).
        match number {
            1 => folder.guild_ids = parse_ids(bytes),
            2 => folder.id = unwrap_scalar(bytes).map(|value| value as i64),
            3 => {
                folder.name = unwrap_bytes(bytes)
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .map(str::to_owned)
            }
            4 => folder.color = unwrap_scalar(bytes).map(|value| value as u32),
            _ => {}
        }
    }
    folder
}

/// Discord declares these `fixed64`, but packed varints share the wire type, so
/// anything not a whole number of 8-byte words is read as varints.
fn parse_ids(bytes: &[u8]) -> Vec<Id<GuildMarker>> {
    let raw: Vec<u64> = if bytes.len().is_multiple_of(8) {
        bytes
            .chunks_exact(8)
            .map(|word| u64::from_le_bytes(word.try_into().expect("chunk is 8 bytes")))
            .collect()
    } else {
        let mut values = Vec::new();
        let mut pos = 0;
        while let Some(value) = read_varint(bytes, &mut pos) {
            values.push(value);
        }
        values
    };

    raw.into_iter().filter_map(Id::new_checked).collect()
}

fn unwrap_scalar(bytes: &[u8]) -> Option<u64> {
    Fields::new(bytes).find_map(|(number, value)| match (number, value) {
        (1, Value::Varint(value) | Value::Fixed64(value)) => Some(value),
        (1, Value::Fixed32(value)) => Some(value as u64),
        _ => None,
    })
}

fn unwrap_bytes(bytes: &[u8]) -> Option<&[u8]> {
    Fields::new(bytes).find_map(|(number, value)| match (number, value) {
        (1, Value::Bytes(bytes)) => Some(bytes),
        _ => None,
    })
}

enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

/// Stops at the first malformed or truncated field.
struct Fields<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Fields<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let taken = self.bytes.get(self.pos..self.pos.checked_add(len)?)?;
        self.pos += len;
        Some(taken)
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let tag = read_varint(self.bytes, &mut self.pos)?;
        let number = u32::try_from(tag >> 3).ok()?;
        let value = match tag & 0b111 {
            0 => Value::Varint(read_varint(self.bytes, &mut self.pos)?),
            1 => Value::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            2 => {
                let len = usize::try_from(read_varint(self.bytes, &mut self.pos)?).ok()?;
                Value::Bytes(self.take(len)?)
            }
            5 => Value::Fixed32(u32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            // Groups (3 and 4) are deprecated and unskippable.
            _ => return None,
        };
        Some((number, value))
    }
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let buffer = chunk.iter().enumerate().fold(0u32, |buffer, (i, &byte)| {
            buffer | u32::from(byte) << (16 - 8 * i)
        });
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(buffer >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Also accepts url-safe characters and missing padding.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in input.bytes() {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' => continue,
            _ => return None,
        };

        buffer = (buffer << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_a_patch() {
        for status in PresenceStatus::ALL {
            let settings = parse_user_settings(&encode_status_settings(status)).unwrap();
            assert_eq!(settings.status, Some(status));
        }
    }

    #[test]
    fn base64_pads_to_whole_quads() {
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }
}
