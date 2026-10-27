use std::collections::HashSet;

use crate::discord::{Channel, ChannelKind};

/// `category` is `None` for the uncategorized channels.
pub(in crate::screens::home) struct ChannelGroup {
    pub(in crate::screens::home) category: Option<Channel>,
    pub(in crate::screens::home) channels: Vec<Channel>,
}

/// Discord's display order: uncategorized first, then categories by position,
/// text-like channels before voice within each group.
pub(in crate::screens::home) fn build_channel_groups(channels: Vec<Channel>) -> Vec<ChannelGroup> {
    let (mut categories, mut others): (Vec<_>, Vec<_>) = channels
        .into_iter()
        .partition(|channel| channel.kind == ChannelKind::Category);

    categories.sort_by_key(|channel| (channel.position, channel.id.get()));
    others.sort_by_key(|channel| (channel.kind.is_voice(), channel.position, channel.id.get()));

    let category_ids: HashSet<_> = categories.iter().map(|category| category.id).collect();

    let mut groups = Vec::new();
    let uncategorized: Vec<_> = others
        .iter()
        .filter(|channel| {
            channel
                .parent_id
                .is_none_or(|id| !category_ids.contains(&id))
        })
        .cloned()
        .collect();
    if !uncategorized.is_empty() {
        groups.push(ChannelGroup {
            category: None,
            channels: uncategorized,
        });
    }
    for category in categories {
        let channels: Vec<_> = others
            .iter()
            .filter(|channel| channel.parent_id == Some(category.id))
            .cloned()
            .collect();
        // Skip categories whose channels were all filtered out (not viewable).
        if channels.is_empty() {
            continue;
        }
        groups.push(ChannelGroup {
            category: Some(category),
            channels,
        });
    }
    groups
}

pub(in crate::screens::home) fn channel_icon_path(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Text => "icons/hash.svg",
        ChannelKind::Announcement => "icons/megaphone.svg",
        ChannelKind::Voice | ChannelKind::Stage => "icons/volume-2.svg",
        ChannelKind::Forum => "icons/messages-square.svg",
        ChannelKind::Category => "icons/chevron-down.svg",
    }
}
