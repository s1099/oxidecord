//! The server rail's layout: the user's guild folders applied to their guilds.

use std::collections::HashSet;

use crate::discord::{Guild, GuildFolders};

pub(in crate::screens::home) enum RailEntry {
    Guild(Guild),
    Folder(RailFolder),
}

pub(in crate::screens::home) struct RailFolder {
    pub id: i64,
    /// Unset for folders the user never named; the rail labels those generically.
    pub name: Option<String>,
    /// `0xRRGGBB`, unset when the user never picked a colour.
    pub color: Option<u32>,
    pub guilds: Vec<Guild>,
}

/// Entries without a folder id are bare guilds and flatten into single rows.
/// Guilds the settings don't mention go last, as in Discord.
pub(in crate::screens::home) fn build_rail_entries(
    guilds: &[Guild],
    folders: Option<&GuildFolders>,
) -> Vec<RailEntry> {
    let Some(folders) = folders else {
        return guilds.iter().cloned().map(RailEntry::Guild).collect();
    };

    let mut placed = HashSet::new();
    let mut entries = Vec::new();

    for folder in &folders.folders {
        let members: Vec<Guild> = folder
            .guild_ids
            .iter()
            .filter_map(|id| guilds.iter().find(|guild| guild.id == *id).cloned())
            .collect();
        placed.extend(members.iter().map(|guild| guild.id));

        match folder.id {
            _ if members.is_empty() => {}
            Some(id) => entries.push(RailEntry::Folder(RailFolder {
                id,
                name: folder.name.clone(),
                color: folder.color,
                guilds: members,
            })),
            None => entries.extend(members.into_iter().map(RailEntry::Guild)),
        }
    }

    entries.extend(
        guilds
            .iter()
            .filter(|guild| !placed.contains(&guild.id))
            .cloned()
            .map(RailEntry::Guild),
    );
    entries
}
