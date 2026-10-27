//! The signed-in user and the profile popout's view of everyone else.

use serde::Deserialize;
use twilight_model::id::{Id, marker::UserMarker};

use super::cdn;
use super::time::format_snowflake_date;
use super::upload::{DEFAULT_UPLOAD_LIMIT, premium_upload_limit};

const PROFILE_AVATAR_SIZE: u32 = 160;
const PROFILE_BANNER_SIZE: u32 = 480;

#[derive(Clone)]
pub struct CurrentUser {
    pub id: Id<UserMarker>,
    /// Global display name when set, else the username.
    pub name: String,
    pub username: String,
    pub avatar_url: Option<String>,
    /// Any tier of Nitro: custom emoji outside their own guild, and animated ones.
    pub premium: bool,
    /// From Nitro alone; a boosted guild may allow more.
    pub upload_limit: u64,
}

#[derive(Clone)]
pub struct UserProfile {
    /// Global display name when set, else the username.
    pub name: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    /// Packed `0xRRGGBB`, shown when there is no banner image.
    pub accent_color: Option<u32>,
    /// `None` when unset or when only the fallback `GET /users/{id}` data
    /// was available, which has no bio.
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    /// Derived from the snowflake, formatted `Jan 5, 2021`.
    pub created_at: String,
    pub bot: bool,
}

#[derive(Clone)]
pub struct Reactor {
    pub id: Id<UserMarker>,
    /// Global display name when set, else the username.
    pub name: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// `GET /users/{id}/profile`; twilight has no model for it.
#[derive(Deserialize)]
pub(in crate::discord) struct RawProfile {
    user: RawProfileUser,
    /// Global profile; overrides `user`'s banner/bio/accent, which are
    /// per-guild when a guild was requested.
    #[serde(default)]
    user_profile: Option<RawProfileDetails>,
}

#[derive(Deserialize)]
struct RawProfileUser {
    id: Id<UserMarker>,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    accent_color: Option<u32>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    bot: bool,
}

#[derive(Deserialize)]
struct RawProfileDetails {
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    accent_color: Option<u32>,
    #[serde(default)]
    pronouns: Option<String>,
}

impl RawProfile {
    pub(in crate::discord) fn into_profile(self) -> UserProfile {
        let details = self.user_profile;
        let user = self.user;

        // The global profile wins wherever it has a value.
        let banner = details
            .as_ref()
            .and_then(|details| details.banner.clone())
            .or(user.banner);
        let bio = details
            .as_ref()
            .and_then(|details| details.bio.clone())
            .or(user.bio);
        let accent_color = details
            .as_ref()
            .and_then(|details| details.accent_color)
            .or(user.accent_color);

        UserProfile {
            name: user.global_name.unwrap_or_else(|| user.username.clone()),
            username: user.username,
            avatar_url: user
                .avatar
                .as_deref()
                .map(|hash| cdn::avatar_url(user.id.get(), hash, PROFILE_AVATAR_SIZE)),
            banner_url: banner
                .as_deref()
                .map(|hash| cdn::banner_url(user.id.get(), hash, PROFILE_BANNER_SIZE)),
            accent_color,
            bio: bio.filter(|bio| !bio.trim().is_empty()),
            pronouns: details
                .and_then(|details| details.pronouns)
                .filter(|pronouns| !pronouns.trim().is_empty()),
            created_at: format_snowflake_date(user.id.get()),
            bot: user.bot,
        }
    }
}

/// Fallback for when `GET /users/{id}/profile` isn't available to this token;
/// that response has no bio or pronouns.
pub(in crate::discord) fn convert_user_profile(user: twilight_model::user::User) -> UserProfile {
    UserProfile {
        name: user
            .global_name
            .clone()
            .unwrap_or_else(|| user.name.clone()),
        username: user.name,
        avatar_url: user
            .avatar
            .map(|hash| cdn::avatar_url(user.id.get(), &hash.to_string(), PROFILE_AVATAR_SIZE)),
        banner_url: user
            .banner
            .map(|hash| cdn::banner_url(user.id.get(), &hash.to_string(), PROFILE_BANNER_SIZE)),
        accent_color: user.accent_color,
        bio: None,
        pronouns: None,
        created_at: format_snowflake_date(user.id.get()),
        bot: user.bot,
    }
}

pub(in crate::discord) fn convert_current_user(
    user: twilight_model::user::CurrentUser,
) -> CurrentUser {
    CurrentUser {
        id: user.id,
        name: user
            .global_name
            .clone()
            .unwrap_or_else(|| user.name.clone()),
        avatar_url: user
            .avatar
            .map(|hash| cdn::small_avatar_url(user.id.get(), &hash.to_string())),
        premium: user
            .premium_type
            .is_some_and(|tier| tier != twilight_model::user::PremiumType::None),
        upload_limit: user
            .premium_type
            .map_or(DEFAULT_UPLOAD_LIMIT, premium_upload_limit),
        username: user.name,
    }
}

pub(in crate::discord) fn convert_reactor(user: twilight_model::user::User) -> Reactor {
    Reactor {
        id: user.id,
        name: user
            .global_name
            .clone()
            .unwrap_or_else(|| user.name.clone()),
        avatar_url: small_avatar_url(&user),
        username: user.name,
    }
}

pub(super) fn small_avatar_url(user: &twilight_model::user::User) -> Option<String> {
    user.avatar
        .map(|hash| cdn::small_avatar_url(user.id.get(), &hash.to_string()))
}
