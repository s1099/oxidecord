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

/// Fetches a guild's channels, keeping those the user holds `VIEW_CHANNEL` on and
/// recording `SEND_MESSAGES` and `MANAGE_MESSAGES`. `base_permissions` and `owner` come
/// from the guild list, so the only extra request is the member object, whose role ids
/// match channel overwrites.
pub async fn fetch_channels(
    guild_id: Id<GuildMarker>,
    base_permissions: Permissions,
    owner: bool,
) -> Result<Vec<Channel>, String> {
    request(move |client| async move {
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
        // The guild-wide permissions are already known, so they seed the `@everyone` baseline;
        // roles are listed with empty permissions since only their ids matter for overwrites.
        let member_roles: Vec<(Id<RoleMarker>, Permissions)> = member
            .roles
            .iter()
            .filter(|id| id.get() != guild_id.get())
            .map(|id| (*id, Permissions::empty()))
            .collect();

        Ok(channels
            .into_iter()
            .filter_map(|channel| {
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
                    .then(|| convert_channel(channel, permissions))
                    .flatten()
            })
            .collect())
    })
    .await
}
