//! The Embedded App SDK's postMessage protocol: `[opcode, payload]` pairs,
//! with commands and their answers in `FRAME`s shaped like Discord's RPC.

use serde::Deserialize;
use serde_json::{Value, json};

pub const HANDSHAKE: u8 = 0;
pub const FRAME: u8 = 1;
pub const CLOSE: u8 = 2;

pub const PARTICIPANTS_UPDATE: &str = "ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE";

/// RPC error codes, as the SDK names them.
pub mod error {
    pub const INVALID_PAYLOAD: u32 = 4000;
    pub const INVALID_COMMAND: u32 = 4002;
    pub const INVALID_CHANNEL: u32 = 4005;
    pub const INVALID_PERMISSIONS: u32 = 4006;
    pub const INVALID_CLIENT_ID: u32 = 4007;
    pub const INVALID_TOKEN: u32 = 4009;
    /// Discord's RPC answers a refused or failed authorization with this.
    pub const OAUTH2: u32 = 5000;
}

/// Close codes for `[2, …]`.
pub mod close {
    pub const INVALID_CLIENT_ID: u32 = 4000;
    pub const INVALID_ORIGIN: u32 = 4001;
}

#[derive(Deserialize)]
pub struct Handshake {
    #[serde(default)]
    pub v: Option<u32>,
    pub client_id: String,
    pub frame_id: String,
}

#[derive(Deserialize)]
pub struct Command {
    pub cmd: String,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub evt: Option<String>,
}

pub enum Inbound {
    Handshake(Handshake),
    Command(Command),
    Close,
}

pub fn parse(raw: &str) -> Option<Inbound> {
    let (opcode, payload): (u8, Value) = serde_json::from_str(raw).ok()?;
    match opcode {
        HANDSHAKE => serde_json::from_value(payload).ok().map(Inbound::Handshake),
        FRAME => serde_json::from_value(payload).ok().map(Inbound::Command),
        CLOSE => Some(Inbound::Close),
        _ => None,
    }
}

pub fn reply(cmd: &str, nonce: Option<&str>, data: Value) -> Value {
    json!([FRAME, { "cmd": cmd, "data": data, "evt": null, "nonce": nonce }])
}

pub fn error(cmd: &str, nonce: Option<&str>, code: u32, message: &str) -> Value {
    json!([FRAME, {
        "cmd": cmd,
        "evt": "ERROR",
        "nonce": nonce,
        "data": { "code": code, "message": message },
    }])
}

pub fn dispatch(evt: &str, data: Value) -> Value {
    json!([FRAME, { "cmd": "DISPATCH", "evt": evt, "data": data, "nonce": null }])
}

pub fn close(code: u32, message: &str) -> Value {
    json!([CLOSE, { "code": code, "message": message }])
}

pub fn ready(user: Value) -> Value {
    dispatch(
        "READY",
        json!({
            "v": 1,
            "config": {
                "cdn_host": "cdn.discordapp.com",
                "api_endpoint": "//discord.com/api",
                "environment": "production",
            },
            "user": user,
        }),
    )
}

/// Only the public fields of a user object. `users/@me` carries the email and
/// phone number, which an activity never gets without asking.
pub fn public_user(user: &Value) -> Value {
    let field = |name: &str| user.get(name).cloned().unwrap_or(Value::Null);
    let flags = user
        .get("public_flags")
        .or_else(|| user.get("flags"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "id": field("id"),
        "username": user.get("username").cloned().unwrap_or_else(|| "".into()),
        "discriminator": user.get("discriminator").cloned().unwrap_or_else(|| "0".into()),
        "global_name": field("global_name"),
        "avatar": field("avatar"),
        "avatar_decoration_data": field("avatar_decoration_data"),
        "bot": user.get("bot").and_then(Value::as_bool).unwrap_or(false),
        "flags": flags,
        "public_flags": flags,
    })
}

/// `scope` arrives as an array, or from older SDKs as one space-separated string.
pub fn scopes(args: &Value) -> Vec<String> {
    match args.get("scope") {
        Some(Value::Array(scopes)) => scopes
            .iter()
            .filter_map(|scope| scope.as_str().map(str::to_owned))
            .collect(),
        Some(Value::String(scopes)) => scopes.split_whitespace().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

/// Discord's consent screen wording for the scopes activities ask for.
pub fn describe_scope(scope: &str) -> String {
    match scope {
        "identify" => "Access your username, avatar, and banner",
        "email" => "Access your email address",
        "guilds" => "Know what servers you're in",
        "guilds.join" => "Join servers for you",
        "guilds.members.read" => {
            "Read your member info (nickname, avatar, roles) in servers you're in"
        }
        "connections" => "Access your third-party connections",
        "relationships.read" => "Know who your friends are",
        "dm_channels.read" => "See your direct messages list",
        "rpc.activities.write" => "Update your activity status",
        "rpc.voice.read" => "Know who's in your voice channel and when they speak",
        "rpc.voice.write" => "Change your voice settings",
        "applications.commands" => "Use commands in your servers and DMs",
        "applications.entitlements" => "Read what you've bought from this app",
        "openid" => "Sign you in with your Discord account",
        "activities.read" => "Read your activity status",
        "activities.write" => "Update your activity status",
        // The SDK's own; granted to every activity.
        "rpc" | "rpc.embedded_app" => "Talk to Discord while it runs",
        other => return other.to_owned(),
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frames_and_handshakes() {
        let Some(Inbound::Handshake(handshake)) =
            parse(r#"[0,{"v":1,"encoding":"json","client_id":"1","frame_id":"f"}]"#)
        else {
            panic!("handshake");
        };
        assert_eq!(handshake.frame_id, "f");
        let Some(Inbound::Command(command)) =
            parse(r#"[1,{"cmd":"SUBSCRIBE","evt":"X","args":{},"nonce":"n"}]"#)
        else {
            panic!("command");
        };
        assert_eq!(command.cmd, "SUBSCRIBE");
        assert_eq!(command.evt.as_deref(), Some("X"));
        assert!(parse("[9,{}]").is_none());
    }

    #[test]
    fn public_user_drops_private_fields() {
        let user = public_user(&json!({
            "id": "1", "username": "a", "email": "a@b.c", "phone": "1", "public_flags": 64
        }));
        assert!(user.get("email").is_none());
        assert!(user.get("phone").is_none());
        assert_eq!(user["flags"], 64);
        assert_eq!(user["discriminator"], "0");
    }

    #[test]
    fn reads_scopes_either_way() {
        assert_eq!(
            scopes(&json!({ "scope": ["identify", "guilds"] })),
            ["identify", "guilds"]
        );
        assert_eq!(
            scopes(&json!({ "scope": "identify guilds" })),
            ["identify", "guilds"]
        );
    }
}
