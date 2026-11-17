//! The signed-in user and the profile popout's view of everyone else.

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::markdown::Markdown;
use super::time::{format_iso_date, format_snowflake_date};
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
    pub mutual_guilds: Vec<MutualGuild>,
    /// Linked accounts they've made public.
    pub connections: Vec<ProfileConnection>,
    /// The board's widgets, in the order they set.
    pub widgets: Vec<ProfileWidget>,
    /// `name#1234` from before the username migration.
    pub legacy_username: Option<String>,
    /// What `member` was requested for; a profile opened elsewhere needs refetching.
    pub guild_id: Option<Id<GuildMarker>>,
    /// Their profile in `guild_id`, when they're a member there.
    pub member: Option<MemberProfile>,
}

#[derive(Clone)]
pub struct MutualGuild {
    pub id: Id<GuildMarker>,
    /// Their nickname there.
    pub nick: Option<String>,
}

#[derive(Clone)]
pub struct ProfileConnection {
    /// The service, as Discord names it: `github`, `steam`, `twitch`…
    pub kind: String,
    pub name: String,
    pub verified: bool,
    /// The account's page, for services whose links can be built.
    pub url: Option<String>,
    /// The service's logo, black on transparent.
    pub icon_url: Option<String>,
    /// When the account was made on that service, formatted `Jan 5, 2021`.
    pub created_at: Option<String>,
    /// Counts the service shares, as Discord words them: "200 Games", "65 Followers".
    pub stats: Vec<String>,
}

#[derive(Clone)]
pub struct ProfileWidget {
    pub kind: WidgetKind,
    pub games: Vec<WidgetGame>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    FavoriteGames,
    PlayedGames,
    CurrentGames,
    WantToPlay,
}

impl WidgetKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::FavoriteGames => "Favorite Game",
            Self::PlayedGames => "Games I Like",
            Self::CurrentGames => "Games in Rotation",
            Self::WantToPlay => "Want to Play",
        }
    }

    /// Discord draws these as rows with the comment and tags, the rest as a cover grid.
    pub fn detailed(self) -> bool {
        matches!(self, Self::FavoriteGames | Self::CurrentGames)
    }
}

#[derive(Clone)]
pub struct WidgetGame {
    pub app_id: u64,
    pub comment: Option<String>,
    pub tags: Vec<String>,
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
    /// Formatted `Jan 5, 2021`, as "Member Since" shows it.
    pub joined_at: Option<String>,
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
    #[serde(default)]
    connected_accounts: Vec<RawConnection>,
    #[serde(default)]
    widgets: Vec<RawWidget>,
    #[serde(default)]
    legacy_username: Option<String>,
}

#[derive(Deserialize)]
struct RawConnection {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    name: String,
    #[serde(default)]
    verified: bool,
    /// Service-specific; values are mostly strings, even counts.
    #[serde(default)]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct RawWidget {
    data: RawWidgetData,
}

#[derive(Deserialize)]
struct RawWidgetData {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    games: Vec<RawWidgetGame>,
}

#[derive(Deserialize)]
struct RawWidgetGame {
    game_id: Id<twilight_model::id::marker::ApplicationMarker>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
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
    #[serde(default)]
    joined_at: Option<String>,
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
    #[serde(default)]
    nick: Option<String>,
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
                joined_at: member.joined_at.as_deref().and_then(format_iso_date),
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
                .map(|guild| MutualGuild {
                    id: guild.id,
                    nick: guild.nick.filter(|nick| !nick.trim().is_empty()),
                })
                .collect(),
            connections: self
                .connected_accounts
                .into_iter()
                .map(convert_connection)
                .collect(),
            widgets: self
                .widgets
                .into_iter()
                .filter_map(|widget| {
                    let kind = match widget.data.kind.as_str() {
                        "favorite_games" => WidgetKind::FavoriteGames,
                        "played_games" => WidgetKind::PlayedGames,
                        "current_games" => WidgetKind::CurrentGames,
                        "want_to_play_games" => WidgetKind::WantToPlay,
                        _ => return None,
                    };
                    let games: Vec<_> = widget
                        .data
                        .games
                        .into_iter()
                        .map(|game| WidgetGame {
                            app_id: game.game_id.get(),
                            comment: game.comment.filter(|comment| !comment.trim().is_empty()),
                            tags: game.tags,
                        })
                        .collect();
                    (!games.is_empty()).then_some(ProfileWidget { kind, games })
                })
                .collect(),
            legacy_username: self.legacy_username.filter(|name| !name.is_empty()),
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
        connections: Vec::new(),
        widgets: Vec::new(),
        legacy_username: None,
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

/// A game on someone's board.
#[derive(Clone)]
pub struct GameInfo {
    pub id: u64,
    pub name: String,
    /// Portrait box art.
    pub cover_url: Option<String>,
}

/// `GET /games`; twilight has no model for it.
#[derive(Deserialize)]
pub(in crate::discord) struct RawGame {
    id: Id<twilight_model::id::marker::ApplicationMarker>,
    name: String,
    #[serde(default)]
    media: Option<RawGameMedia>,
}

#[derive(Deserialize)]
struct RawGameMedia {
    #[serde(default)]
    cover: Option<RawGameImage>,
    #[serde(default)]
    icon: Option<RawGameImage>,
}

/// A CDN hash, or an outside image's URL.
#[derive(Deserialize)]
struct RawGameImage {
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

impl RawGameImage {
    fn url(&self, game_id: u64) -> Option<String> {
        if self.value.is_empty() {
            return None;
        }
        match self.kind.as_str() {
            "hash" => Some(cdn::game_asset_url(game_id, &self.value)),
            "url" => Some(self.value.clone()),
            _ => None,
        }
    }
}

impl RawGame {
    /// Without a cover, the icon stands in, as in Discord.
    pub(in crate::discord) fn into_game(self) -> GameInfo {
        let id = self.id.get();
        let cover_url = self.media.and_then(|media| {
            media
                .cover
                .and_then(|cover| cover.url(id))
                .or_else(|| media.icon.and_then(|icon| icon.url(id)))
        });
        GameInfo {
            id,
            name: self.name,
            cover_url,
        }
    }
}

/// A connected service's name as Discord shows it.
pub fn connection_label(kind: &str) -> String {
    let known = match kind {
        "amazon-music" => "Amazon Music",
        "battlenet" => "Battle.net",
        "bluesky" => "Bluesky",
        "bungie" => "Bungie.net",
        "crunchyroll" => "Crunchyroll",
        "domain" => "Domain",
        "ebay" => "eBay",
        "epicgames" => "Epic Games",
        "facebook" => "Facebook",
        "github" => "GitHub",
        "instagram" => "Instagram",
        "leagueoflegends" => "League of Legends",
        "letterboxd" => "Letterboxd",
        "mastodon" => "Mastodon",
        "paypal" => "PayPal",
        "playstation" | "playstation-stg" => "PlayStation Network",
        "reddit" => "Reddit",
        "riotgames" => "Riot Games",
        "roblox" => "Roblox",
        "samsung" => "Samsung Galaxy",
        "skype" => "Skype",
        "soundcloud" => "SoundCloud",
        "spotify" => "Spotify",
        "steam" => "Steam",
        "tiktok" => "TikTok",
        "twitch" => "Twitch",
        "twitter" => "X",
        "xbox" => "Xbox",
        "youtube" => "YouTube",
        other => return other.to_owned(),
    };
    known.to_owned()
}

/// A profile game tag's label: `looking_for_group` reads "Looking For Group".
pub fn widget_tag_label(tag: &str) -> String {
    tag.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The account's public page, for the services whose links follow from the
/// name or id alone.
fn convert_connection(account: RawConnection) -> ProfileConnection {
    let metadata = account.metadata.unwrap_or_default();
    let text = |key: &str| match metadata.get(key)? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    };
    let count = |key: &str, one: &str, many: &str| {
        let count: u64 = text(key)?.parse().ok()?;
        Some(format!("{count} {}", if count == 1 { one } else { many }))
    };
    let stats = match account.kind.as_str() {
        "steam" => vec![count("game_count", "Game", "Games")],
        "twitter" => vec![
            count("statuses_count", "Post", "Posts"),
            count("followers_count", "Follower", "Followers"),
        ],
        "reddit" => vec![count("total_karma", "Karma", "Karma")],
        _ => Vec::new(),
    };
    ProfileConnection {
        url: connection_url(&account.kind, &account.id, &account.name),
        icon_url: connection_icon_url(&account.kind),
        created_at: text("created_at").as_deref().and_then(format_iso_date),
        stats: stats.into_iter().flatten().collect(),
        kind: account.kind,
        name: account.name,
        verified: account.verified,
    }
}

/// Discord ships its own logos in the client bundle under hashed names, so
/// Simple Icons' stands in, pinned to a release.
fn connection_icon_url(kind: &str) -> Option<String> {
    let slug = match kind {
        "amazon-music" => "amazonmusic",
        "battlenet" => "battledotnet",
        "bluesky" => "bluesky",
        "bungie" => "bungie",
        "crunchyroll" => "crunchyroll",
        "ebay" => "ebay",
        "epicgames" => "epicgames",
        "facebook" => "facebook",
        "github" => "github",
        "instagram" => "instagram",
        "leagueoflegends" => "leagueoflegends",
        "letterboxd" => "letterboxd",
        "mastodon" => "mastodon",
        "paypal" => "paypal",
        "playstation" | "playstation-stg" => "playstation",
        "reddit" => "reddit",
        "riotgames" => "riotgames",
        "roblox" => "roblox",
        "samsung" => "samsung",
        "soundcloud" => "soundcloud",
        "spotify" => "spotify",
        "steam" => "steam",
        "tiktok" => "tiktok",
        "twitch" => "twitch",
        "twitter" => "x",
        "xbox" => "xbox",
        "youtube" => "youtube",
        _ => return None,
    };
    // Xbox's logo was withdrawn from later releases.
    let version = if slug == "xbox" { "11.0.0" } else { "13.21.0" };
    Some(format!(
        "https://cdn.jsdelivr.net/npm/simple-icons@{version}/icons/{slug}.svg"
    ))
}

/// Discord's client's own patterns; services it doesn't link aren't linked here.
fn connection_url(kind: &str, id: &str, name: &str) -> Option<String> {
    Some(match kind {
        "bluesky" => format!("https://bsky.app/profile/{id}"),
        "domain" => format!("https://{id}/"),
        "ebay" => format!("https://www.ebay.com/usr/{name}"),
        "github" => format!("https://github.com/{name}"),
        "instagram" => format!("https://www.instagram.com/{name}"),
        "letterboxd" => format!("https://letterboxd.com/{name}/"),
        // A federated account's id is already its URL.
        "mastodon" => id.to_owned(),
        "reddit" => format!("https://www.reddit.com/u/{name}"),
        "roblox" => format!("https://roblox.com/users/{id}/profile"),
        "spotify" => format!("https://open.spotify.com/user/{id}"),
        "steam" => format!("https://steamcommunity.com/profiles/{id}"),
        "tiktok" => format!("https://www.tiktok.com/@{name}"),
        "twitch" => format!("https://www.twitch.tv/{name}"),
        "twitter" => format!("https://x.com/{name}"),
        "youtube" => format!("https://www.youtube.com/channel/{id}"),
        _ => return None,
    })
}
