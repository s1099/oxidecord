//! The user's server ordering, as it comes out of Discord's `settings-proto`.
//!
//! `GET /users/@me/settings-proto/1` returns `PreloadedUserSettings` as a
//! base64-encoded protobuf. Field 14 of it is `GuildFolders`, which is where
//! the client keeps the rail's order: `folders` holds every top-level entry —
//! a real folder when it has an id, or a lone guild when it doesn't — and
//! `guild_positions` is the flattened order of every guild inside them.
//!
//! Pulling in a protobuf runtime for one message isn't worth it, so the wire
//! format is walked by hand below, skipping every field this file doesn't
//! name.

use serde::Deserialize;
use twilight_model::id::{Id, marker::GuildMarker};

/// The envelope the settings-proto endpoints answer with.
#[derive(Deserialize)]
pub(in crate::discord) struct RawSettingsProto {
    /// The base64-encoded `PreloadedUserSettings` message.
    pub settings: String,
}

#[derive(Debug, Default, Clone)]
pub struct GuildFolders {
    pub folders: Vec<GuildFolder>,
    /// Every guild in rail order, folders flattened into their contents.
    pub guild_positions: Vec<Id<GuildMarker>>,
}

#[derive(Debug, Default, Clone)]
pub struct GuildFolder {
    /// `None` for a bare guild sitting at the top level rather than a folder.
    pub id: Option<i64>,
    /// Unset for unnamed folders, which the client renders from their members.
    pub name: Option<String>,
    /// The folder's colour as `0xRRGGBB`.
    pub color: Option<u32>,
    pub guild_ids: Vec<Id<GuildMarker>>,
}

/// Decodes the `settings` blob and pulls `guild_folders` out of it.
pub(in crate::discord) fn parse_guild_folders(settings: &str) -> Result<GuildFolders, String> {
    let bytes = base64_decode(settings)
        .ok_or_else(|| "settings-proto blob is not valid base64".to_owned())?;

    let mut folders = GuildFolders::default();
    for (number, value) in Fields::new(&bytes) {
        if let (14, Value::Bytes(bytes)) = (number, value) {
            folders = parse_folders_message(bytes);
        }
    }
    Ok(folders)
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

/// Reads a packed repeated field of guild ids. Discord declares these as
/// `fixed64`, but packed varints share the wire type, so anything that isn't a
/// whole number of 8-byte words is read as varints instead.
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

/// Reads field 1 of a protobuf scalar wrapper message.
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

/// Walks the fields of one protobuf message, stopping at the first malformed
/// or truncated one.
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

/// Standard-alphabet base64, tolerating the padding being left off.
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
