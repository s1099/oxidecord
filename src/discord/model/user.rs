//! The signed-in user and the profile popout's view of everyone else.

use serde::Deserialize;
use twilight_model::id::{Id, marker::UserMarker};

use super::cdn;
use super::time::format_snowflake_date;

/// Size the profile popout renders the avatar and banner at.
const PROFILE_AVATAR_SIZE: u32 = 160;
const PROFILE_BANNER_SIZE: u32 = 480;

#[derive(Clone)]
pub struct CurrentUser {
    /// Display name: the global display name when set, else the username.
    pub name: String,
    /// The `@handle` username.
    pub username: String,
    pub avatar_url: Option<String>,
}

/// Another user, as shown in the profile popout opened from their avatar.
#[derive(Clone)]
pub struct UserProfile {
    /// Display name: the global display name when set, else the username.
    pub name: String,
    /// The `@handle` username.
    pub username: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    /// The banner's solid colour, used when the user has no banner image.
    /// Packed as `0xRRGGBB`.
    pub accent_color: Option<u32>,
    /// The "About Me" text. `None` when unset or when only the fallback
    /// `GET /users/{id}` data was available, which doesn't carry a bio.
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    /// When the account was registered, derived from the snowflake and
    /// formatted as `Jan 5, 2021`.
    pub created_at: String,
    pub bot: bool,
}

/// `GET /users/{id}/profile`, the endpoint the Discord client itself uses for
/// the profile popout. It has no twilight model, so it's deserialized here.
#[derive(Deserialize)]
pub(in crate::discord) struct RawProfile {
    user: RawProfileUser,
    /// The user's global profile, whose banner/bio/accent override the ones on
    /// `user` (which are the per-guild values when a guild was requested).
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

        // The global profile wins wherever it has a value; `user` is the
        // fallback for accounts that only set the fields in one place.
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

/// Builds a [`UserProfile`] from the plain `GET /users/{id}` user, used when
/// the richer profile endpoint isn't available to this token. The bio and
/// pronouns aren't part of that response, so they're left unset.
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
        name: user
            .global_name
            .clone()
            .unwrap_or_else(|| user.name.clone()),
        avatar_url: user
            .avatar
            .map(|hash| cdn::small_avatar_url(user.id.get(), &hash.to_string())),
        username: user.name,
    }
}

/// The small avatar shown beside a user's name in a message or a DM row.
pub(super) fn small_avatar_url(user: &twilight_model::user::User) -> Option<String> {
    user.avatar
        .map(|hash| cdn::small_avatar_url(user.id.get(), &hash.to_string()))
}
