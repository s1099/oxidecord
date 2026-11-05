//! Activities: embedded apps run in a call, listed on Discord's shelf and
//! tracked per channel by `EMBEDDED_ACTIVITY_UPDATE_V2`.

use std::collections::HashMap;

use serde::Deserialize;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, CommandMarker, GuildMarker, UserMarker},
};

use super::cdn;

/// An activity the shelf offers.
#[derive(Clone, Debug)]
pub struct ActivityApp {
    pub id: Id<ApplicationMarker>,
    pub name: String,
    pub icon_url: Option<String>,
}

/// A running copy of an activity in one channel, and who's in it.
#[derive(Clone, Debug)]
pub struct ActivityInstance {
    pub app_id: Id<ApplicationMarker>,
    /// `i-<instance>-<kind>-<guild>-<channel>`: what the activity is told as `instance_id`.
    pub composite_id: String,
    pub instance_id: String,
    pub launch_id: String,
    /// `<kind>-<guild>-<channel>`.
    pub location_id: String,
    pub channel_id: Id<ChannelMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    pub participants: Vec<ActivityParticipant>,
}

impl ActivityInstance {
    pub fn includes(&self, user_id: Id<UserMarker>, session_id: &str) -> bool {
        self.participants
            .iter()
            .any(|p| p.user_id == user_id && p.session_id == session_id)
    }
}

#[derive(Clone, Debug)]
pub struct ActivityParticipant {
    pub user_id: Id<UserMarker>,
    pub session_id: String,
    /// The user object as Discord sent it inside `member`, which is what the
    /// activity is handed; absent outside guilds.
    pub user: Option<serde_json::Value>,
    pub nick: Option<String>,
}

/// An app's `PRIMARY_ENTRY_POINT` command, run to start a new instance.
#[derive(Clone, Debug)]
pub struct EntryPoint {
    pub id: Id<CommandMarker>,
    pub version: String,
    pub name: String,
    /// Echoed back whole when the command runs.
    pub raw: serde_json::Value,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawShelf {
    #[serde(default)]
    activities: Vec<RawShelfActivity>,
    #[serde(default)]
    applications: Vec<RawShelfApp>,
}

#[derive(Deserialize)]
struct RawShelfActivity {
    application_id: Id<ApplicationMarker>,
    #[serde(default)]
    client_platform_config: HashMap<String, serde_json::Value>,
    #[serde(default)]
    shelf_rank: Option<i64>,
}

#[derive(Deserialize)]
struct RawShelfApp {
    id: Id<ApplicationMarker>,
    name: String,
    #[serde(default)]
    icon: Option<String>,
}

/// In shelf order, leaving out activities that don't run on desktop, which
/// Discord files under the `web` platform.
pub(in crate::discord) fn convert_shelf(raw: RawShelf) -> Vec<ActivityApp> {
    let apps: HashMap<_, _> = raw.applications.into_iter().map(|a| (a.id, a)).collect();
    let mut activities: Vec<_> = raw
        .activities
        .into_iter()
        .filter(|a| {
            a.client_platform_config.is_empty() || a.client_platform_config.contains_key("web")
        })
        .collect();
    activities.sort_by_key(|a| a.shelf_rank.unwrap_or(i64::MAX));
    activities
        .into_iter()
        .filter_map(|activity| {
            let app = apps.get(&activity.application_id)?;
            Some(ActivityApp {
                id: app.id,
                name: app.name.clone(),
                icon_url: app
                    .icon
                    .as_deref()
                    .map(|hash| cdn::app_icon_url(app.id, hash)),
            })
        })
        .collect()
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawActivityInstance {
    application_id: Id<ApplicationMarker>,
    composite_instance_id: String,
    #[serde(default)]
    instance_id: String,
    #[serde(default)]
    launch_id: String,
    location: RawLocation,
    #[serde(default)]
    participants: Vec<RawParticipant>,
}

#[derive(Deserialize)]
struct RawLocation {
    id: String,
    channel_id: Id<ChannelMarker>,
    #[serde(default)]
    guild_id: Option<Id<GuildMarker>>,
}

#[derive(Deserialize)]
struct RawParticipant {
    user_id: Id<UserMarker>,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    member: Option<RawParticipantMember>,
}

#[derive(Deserialize)]
struct RawParticipantMember {
    #[serde(default)]
    user: Option<serde_json::Value>,
    #[serde(default)]
    nick: Option<String>,
}

pub(in crate::discord) fn convert_activity_instance(raw: RawActivityInstance) -> ActivityInstance {
    ActivityInstance {
        app_id: raw.application_id,
        // Both are the same snowflake in every instance seen; either stands in.
        instance_id: if raw.instance_id.is_empty() {
            raw.launch_id.clone()
        } else {
            raw.instance_id
        },
        launch_id: raw.launch_id,
        composite_id: raw.composite_instance_id,
        location_id: raw.location.id,
        channel_id: raw.location.channel_id,
        guild_id: raw.location.guild_id,
        participants: raw
            .participants
            .into_iter()
            .map(|p| {
                let (user, nick) = p
                    .member
                    .map(|member| (member.user, member.nick))
                    .unwrap_or_default();
                ActivityParticipant {
                    user_id: p.user_id,
                    session_id: p.session_id,
                    user,
                    nick,
                }
            })
            .collect(),
    }
}

/// Picks the `PRIMARY_ENTRY_POINT` (type 4) command out of an app's index.
pub(in crate::discord) fn entry_point(commands: Vec<serde_json::Value>) -> Option<EntryPoint> {
    #[derive(Deserialize)]
    struct RawEntry {
        id: Id<CommandMarker>,
        #[serde(rename = "type")]
        kind: u8,
        #[serde(default)]
        version: String,
        name: String,
        #[serde(default)]
        name_default: Option<String>,
    }
    commands.into_iter().find_map(|mut value| {
        let entry = serde_json::from_value::<RawEntry>(value.clone()).ok()?;
        if entry.kind != 4 {
            return None;
        }
        // An index serves names localised, with the original beside; the
        // original is what's sent.
        let name = entry.name_default.unwrap_or(entry.name);
        if let Some(object) = value.as_object_mut() {
            object.insert("name".into(), name.clone().into());
        }
        Some(EntryPoint {
            id: entry.id,
            version: entry.version,
            name,
            raw: value,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_instance_and_its_members() {
        let raw: RawActivityInstance = serde_json::from_value(serde_json::json!({
            "application_id": "1211781489931452447",
            "composite_instance_id": "i-1420621242070732950-gc-1-2",
            "instance_id": "1420621242070732950",
            "launch_id": "1420621242070732950",
            "location": { "id": "gc-1-2", "kind": "gc", "channel_id": "2", "guild_id": "1" },
            "participants": [
                { "user_id": "5", "session_id": "abc", "nonce": null,
                  "member": { "nick": "Five", "user": { "id": "5", "username": "five" } } },
                { "user_id": "6", "session_id": "def" }
            ]
        }))
        .unwrap();
        let instance = convert_activity_instance(raw);
        assert_eq!(instance.channel_id, Id::new(2));
        assert_eq!(instance.guild_id, Some(Id::new(1)));
        assert!(instance.includes(Id::new(5), "abc"));
        assert!(!instance.includes(Id::new(5), "def"));
        assert_eq!(instance.participants[0].nick.as_deref(), Some("Five"));
        assert!(instance.participants[1].user.is_none());
    }

    #[test]
    fn finds_the_entry_point_under_its_original_name() {
        let entry = entry_point(vec![
            serde_json::json!({ "id": "1", "type": 1, "name": "play" }),
            serde_json::json!({ "id": "2", "type": 4, "version": "3", "name": "starten", "name_default": "launch" }),
        ])
        .unwrap();
        assert_eq!(entry.id, Id::new(2));
        assert_eq!(entry.name, "launch");
        assert_eq!(entry.raw["name"], "launch");
    }

    #[test]
    fn shelf_keeps_desktop_activities_in_rank_order() {
        let raw: RawShelf = serde_json::from_value(serde_json::json!({
            "activities": [
                { "application_id": "1", "shelf_rank": 2, "client_platform_config": { "web": {} } },
                { "application_id": "2", "shelf_rank": 1, "client_platform_config": { "web": {} } },
                { "application_id": "3", "shelf_rank": 0, "client_platform_config": { "ios": {} } }
            ],
            "applications": [
                { "id": "1", "name": "One", "icon": "abc" },
                { "id": "2", "name": "Two" },
                { "id": "3", "name": "Three" }
            ]
        }))
        .unwrap();
        let names: Vec<_> = convert_shelf(raw).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Two", "One"]);
    }
}
