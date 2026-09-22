//! Fetching the user's guilds and the channels they can see in one.

use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker},
};
use twilight_util::permission_calculator::PermissionCalculator;

use super::request;
use crate::discord::Permissions;
use crate::discord::model::{Channel, Guild, convert_channel, convert_guild};

pub async fn fetch_guilds() -> Result<Vec<Guild>, String> {
    request(|client| async move {
        let guilds = client.current_user_guilds().await?.models().await?;
        Ok(guilds.into_iter().map(convert_guild).collect())
    })
    .await
}

/// Fetches a guild's channels, keeping only the ones the current user holds
/// `VIEW_CHANNEL` on — the rest are what Discord itself hides from the sidebar.
/// Each surviving channel also records whether the user holds `SEND_MESSAGES`
/// and `MANAGE_MESSAGES`.
///
/// `base_permissions` and `owner` come from the guild list (see [`Guild`]), so
/// the only extra request here is the member object — which of their roles
/// apply — used to resolve each channel's overwrites locally.
pub async fn fetch_channels(
    guild_id: Id<GuildMarker>,
    base_permissions: Permissions,
    owner: bool,
) -> Result<Vec<Channel>, String> {
    request(move |client| async move {
        // The member object tells us which roles the user has, so
        // role-specific channel overwrites can be matched.
        let (member, channels) = tokio::try_join!(
            async {
                anyhow::Ok(
                    client
                        .current_user_guild_member(guild_id)
                        .await?
                        .model()
                        .await?,
                )
            },
            async { anyhow::Ok(client.guild_channels(guild_id).await?.models().await?) },
        )?;

        let user_id = member.user.id;
        // We already know the user's aggregate guild-wide permissions, so
        // seed the calculator with those as the `@everyone` baseline and
        // list the member's roles with empty permissions — the roles are
        // only needed by id, to match channel overwrites, not to recompute
        // the baseline.
        let member_roles: Vec<(Id<RoleMarker>, Permissions)> = member
            .roles
            .iter()
            .filter(|id| id.get() != guild_id.get())
            .map(|id| (*id, Permissions::empty()))
            .collect();

        Ok(channels
            .into_iter()
            .filter_map(|channel| {
                // Apply the channel's own overwrites to the baseline: drop
                // the channel if the user can't even view it, and carry
                // whether they may post so the composer can say otherwise.
                let permissions = {
                    let overwrites = channel.permission_overwrites.as_deref().unwrap_or(&[]);
                    let mut calculator = PermissionCalculator::new(
                        guild_id,
                        user_id,
                        base_permissions,
                        &member_roles,
                    );
                    if owner {
                        calculator = calculator.owner_id(user_id);
                    }
                    calculator.in_channel(channel.kind, overwrites)
                };

                permissions
                    .contains(Permissions::VIEW_CHANNEL)
                    .then(|| {
                        convert_channel(
                            channel,
                            permissions.contains(Permissions::SEND_MESSAGES),
                            permissions.contains(Permissions::MANAGE_MESSAGES),
                        )
                    })
                    .flatten()
            })
            .collect())
    })
    .await
}
