use std::collections::HashSet;

use crate::discord::{Channel, ChannelKind};

/// A category and its channels (or the uncategorized channels when
/// `category` is `None`), in Discord's display order.
pub(in crate::screens::home) struct ChannelGroup {
    pub(in crate::screens::home) category: Option<Channel>,
    pub(in crate::screens::home) channels: Vec<Channel>,
}

/// Groups a guild's channels for the sidebar in Discord's display order:
/// uncategorized channels first, then each category (by position) with its
/// children, text-like channels before voice channels in each group.
pub(in crate::screens::home) fn build_channel_groups(channels: Vec<Channel>) -> Vec<ChannelGroup> {
    let (mut categories, mut others): (Vec<_>, Vec<_>) = channels
        .into_iter()
        .partition(|channel| channel.kind == ChannelKind::Category);

    categories.sort_by_key(|channel| (channel.position, channel.id.get()));
    others.sort_by_key(|channel| (channel.kind.is_voice(), channel.position, channel.id.get()));

    let category_ids: HashSet<_> = categories.iter().map(|category| category.id).collect();

    let mut groups = Vec::new();
    // Channels with no (known) parent category sit above all categories.
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
        // A category whose every channel was filtered out (e.g. the user can't
        // view any of them) shouldn't appear as an empty header.
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
