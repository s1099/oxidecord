//! Fetching the user's guilds and the channels they can see in one.

use twilight_http::Client as HttpClient;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker},
};
use twilight_util::permission_calculator::PermissionCalculator;

use crate::platform::runtime;

use crate::discord::Permissions;
use crate::discord::model::{Channel, Guild, convert_channel, convert_guild};

pub fn fetch_guilds(
    token: String,
    on_done: impl FnOnce(Result<Vec<Guild>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .current_user_guilds()
                .await
                .map_err(|err| err.to_string())?;
            let guilds = response.models().await.map_err(|err| err.to_string())?;

            Ok(guilds.into_iter().map(convert_guild).collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// Fetches a guild's channels, keeping only the ones the current user holds
/// `VIEW_CHANNEL` on — the rest are what Discord itself hides from the sidebar.
///
/// `base_permissions` and `owner` come from the guild list (see [`Guild`]), so
/// the only extra request here is the member object — which of their roles
/// apply — used to resolve each channel's overwrites locally.
pub fn fetch_channels(
    token: String,
    guild_id: Id<GuildMarker>,
    base_permissions: Permissions,
    owner: bool,
    on_done: impl FnOnce(Result<Vec<Channel>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);

            // The member object tells us which roles the user has, so
            // role-specific channel overwrites can be matched.
            let member = client
                .current_user_guild_member(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .model()
                .await
                .map_err(|err| err.to_string())?;
            let channels = client
                .guild_channels(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .models()
                .await
                .map_err(|err| err.to_string())?;

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
                    // Apply the channel's own overwrites to the baseline and
                    // drop the channel if the user can't even view it.
                    let visible = {
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
                        calculator
                            .in_channel(channel.kind, overwrites)
                            .contains(Permissions::VIEW_CHANNEL)
                    };

                    visible.then(|| convert_channel(channel)).flatten()
                })
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}
