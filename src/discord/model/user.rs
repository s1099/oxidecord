//! The signed-in user and the profile popout's view of everyone else.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::markdown::Markdown;
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
    pub bio: Option<Markdown>,
    pub pronouns: Option<String>,
    /// Derived from the snowflake, formatted `Jan 5, 2021`.
    pub created_at: String,
    pub bot: bool,
    pub badges: Vec<ProfileBadge>,
    /// Only some come back; `mutual_friends_count` has the total.
    pub mutual_friends: Vec<Reactor>,
    pub mutual_friends_count: usize,
    pub mutual_guilds: Vec<Id<GuildMarker>>,
    /// What `member` was requested for; a profile opened elsewhere needs refetching.
    pub guild_id: Option<Id<GuildMarker>>,
    /// Their profile in `guild_id`, when they're a member there.
    pub member: Option<MemberProfile>,
}

#[derive(Clone)]
pub struct ProfileBadge {
    pub description: String,
    pub icon_url: String,
    pub link: Option<String>,
}

/// The user's server profile; anything unset falls back to the global one.
#[derive(Clone)]
pub struct MemberProfile {
    pub nick: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub bio: Option<Markdown>,
    pub pronouns: Option<String>,
    pub roles: Vec<Id<RoleMarker>>,
}

impl UserProfile {
    pub fn display_name(&self) -> &str {
        self.member
            .as_ref()
            .and_then(|member| member.nick.as_deref())
            .unwrap_or(&self.name)
    }

    pub fn display_avatar(&self) -> Option<&String> {
        self.member
            .as_ref()
            .and_then(|member| member.avatar_url.as_ref())
            .or(self.avatar_url.as_ref())
    }

    pub fn display_banner(&self) -> Option<&String> {
        self.member
            .as_ref()
            .and_then(|member| member.banner_url.as_ref())
            .or(self.banner_url.as_ref())
    }

    pub fn display_bio(&self) -> Option<&Markdown> {
        self.member
            .as_ref()
            .and_then(|member| member.bio.as_ref())
            .or(self.bio.as_ref())
    }

    pub fn display_pronouns(&self) -> Option<&str> {
        self.member
            .as_ref()
            .and_then(|member| member.pronouns.as_deref())
            .or(self.pronouns.as_deref())
    }
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
    #[serde(default)]
    guild_member: Option<RawProfileMember>,
    #[serde(default)]
    guild_member_profile: Option<RawProfileDetails>,
    /// These and the mutuals are `null` when the user has blocked us.
    #[serde(default)]
    badges: Option<Vec<RawBadge>>,
    #[serde(default)]
    guild_badges: Option<Vec<RawBadge>>,
    #[serde(default)]
    mutual_guilds: Option<Vec<RawMutualGuild>>,
    #[serde(default)]
    mutual_friends: Option<Vec<RawMutualFriend>>,
    #[serde(default)]
    mutual_friends_count: Option<usize>,
}

#[derive(Deserialize)]
struct RawProfileMember {
    #[serde(default)]
    nick: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    roles: Vec<Id<RoleMarker>>,
}

#[derive(Deserialize)]
struct RawBadge {
    description: String,
    icon: String,
    #[serde(default)]
    link: Option<String>,
}

#[derive(Deserialize)]
struct RawMutualGuild {
    id: Id<GuildMarker>,
}

#[derive(Deserialize)]
struct RawMutualFriend {
    id: Id<UserMarker>,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    discriminator: Option<String>,
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
    discriminator: Option<String>,
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
    /// `guild_id` is the guild the profile was requested for; the response doesn't name it.
    pub(in crate::discord) fn into_profile(self, guild_id: Option<Id<GuildMarker>>) -> UserProfile {
        let details = self.user_profile;
        let user = self.user;

        let guild_details = self.guild_member_profile;
        let member = guild_id
            .zip(self.guild_member)
            .map(|(guild_id, member)| MemberProfile {
                nick: member.nick.filter(|nick| !nick.trim().is_empty()),
                avatar_url: member.avatar.map(|hash| {
                    cdn::member_avatar_url(guild_id, user.id, &hash, PROFILE_AVATAR_SIZE)
                }),
                banner_url: guild_details
                    .as_ref()
                    .and_then(|details| details.banner.clone())
                    .or(member.banner)
                    .map(|hash| {
                        cdn::member_banner_url(guild_id, user.id, &hash, PROFILE_BANNER_SIZE)
                    }),
                bio: guild_details
                    .as_ref()
                    .and_then(|details| details.bio.as_deref())
                    .and_then(parse_bio),
                pronouns: guild_details
                    .and_then(|details| details.pronouns)
                    .filter(|pronouns| !pronouns.trim().is_empty()),
                roles: member.roles,
            });

        // The server's own badges lead, as in Discord.
        let badges = self
            .guild_badges
            .into_iter()
            .flatten()
            .chain(self.badges.into_iter().flatten())
            .map(|badge| ProfileBadge {
                icon_url: cdn::badge_icon_url(&badge.icon),
                description: badge.description,
                link: badge.link.filter(|link| !link.is_empty()),
            })
            .collect();

        let mutual_friends = self.mutual_friends.unwrap_or_default();
        let mutual_friends_count = self.mutual_friends_count.unwrap_or(mutual_friends.len());
        let mutual_friends = mutual_friends
            .into_iter()
            .map(|friend| Reactor {
                id: friend.id,
                name: friend
                    .global_name
                    .unwrap_or_else(|| friend.username.clone()),
                avatar_url: Some(cdn::small_user_avatar(
                    friend.id.get(),
                    friend.avatar.as_deref(),
                    parse_discriminator(friend.discriminator.as_deref()),
                )),
                username: friend.username,
            })
            .collect();

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
            avatar_url: Some(match user.avatar.as_deref() {
                Some(hash) => cdn::avatar_url(user.id.get(), hash, PROFILE_AVATAR_SIZE),
                None => cdn::default_avatar_url(
                    user.id.get(),
                    parse_discriminator(user.discriminator.as_deref()),
                ),
            }),
            banner_url: banner
                .as_deref()
                .map(|hash| cdn::banner_url(user.id.get(), hash, PROFILE_BANNER_SIZE)),
            accent_color,
            bio: bio.as_deref().and_then(parse_bio),
            pronouns: details
                .and_then(|details| details.pronouns)
                .filter(|pronouns| !pronouns.trim().is_empty()),
            created_at: format_snowflake_date(user.id.get()),
            bot: user.bot,
            badges,
            mutual_friends,
            mutual_friends_count,
            mutual_guilds: self
                .mutual_guilds
                .into_iter()
                .flatten()
                .map(|guild| guild.id)
                .collect(),
            guild_id,
            member,
        }
    }
}

fn parse_bio(bio: &str) -> Option<Markdown> {
    let bio = bio.trim();
    (!bio.is_empty()).then(|| Markdown::parse(bio))
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
        avatar_url: Some(match user.avatar {
            Some(hash) => cdn::avatar_url(user.id.get(), &hash.to_string(), PROFILE_AVATAR_SIZE),
            None => cdn::default_avatar_url(user.id.get(), user.discriminator),
        }),
        banner_url: user
            .banner
            .map(|hash| cdn::banner_url(user.id.get(), &hash.to_string(), PROFILE_BANNER_SIZE)),
        accent_color: user.accent_color,
        bio: None,
        pronouns: None,
        created_at: format_snowflake_date(user.id.get()),
        bot: user.bot,
        badges: Vec::new(),
        mutual_friends: Vec::new(),
        mutual_friends_count: 0,
        mutual_guilds: Vec::new(),
        guild_id: None,
        member: None,
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
        avatar_url: Some(cdn::small_user_avatar(
            user.id.get(),
            user.avatar.map(|hash| hash.to_string()).as_deref(),
            user.discriminator,
        )),
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
    let hash = user.avatar.map(|hash| hash.to_string());
    Some(cdn::small_user_avatar(
        user.id.get(),
        hash.as_deref(),
        user.discriminator,
    ))
}

/// Raw payloads carry the discriminator as a string; a missing one means a
/// migrated username, same as `"0"`.
pub(super) fn parse_discriminator(discriminator: Option<&str>) -> u16 {
    discriminator
        .and_then(|discriminator| discriminator.parse().ok())
        .unwrap_or(0)
}
