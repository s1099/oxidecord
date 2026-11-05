//! An activity's window: its page in a webview, with the app standing in for
//! the Discord client the page expects to be framed by.
//!
//! Discord serves an activity from `https://{app}.discordsays.com` and runs it
//! in an iframe, talking to it over the Embedded App SDK's postMessage RPC
//! ([`rpc`]). Here the page loads top-level and `shim.js` gives it a
//! `window.parent` that forwards to this window; answers go back by script.

mod rpc;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_wry::WebView;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use twilight_model::guild::Permissions;
use twilight_model::id::{Id, marker::UserMarker};
use wry::{NewWindowResponse, WebViewBuilder};

use crate::discord::{self, ActivityApp, ActivityInstance, OAuthRequest};
use crate::platform::webview;
use crate::ui::button::Button;
use crate::ui::depth::{self, radius};

use rpc::error;

const SHIM: &str = include_str!("shim.js");

/// Everything the window needs to host one activity instance.
pub struct ActivityContext {
    pub app: ActivityApp,
    pub instance: ActivityInstance,
    pub channel_name: String,
    /// Discord's channel type: 2 for a guild voice channel, 1 or 3 for a call.
    pub channel_kind: u8,
    pub permissions: Permissions,
    pub proxy_ticket: String,
    pub self_id: Id<UserMarker>,
}

impl ActivityContext {
    fn origin(&self) -> String {
        format!("https://{}.discordsays.com", self.app.id)
    }

    /// The query Discord's client loads the activity with; the SDK refuses to
    /// start without `frame_id`, `instance_id` and `platform`.
    fn url(&self, frame_id: &str) -> String {
        let instance = &self.instance;
        let channel_id = instance.channel_id.to_string();
        let guild_id = instance.guild_id.map(|id| id.to_string());
        let mut params = vec![
            ("instance_id", instance.composite_id.as_str()),
            ("location_id", instance.location_id.as_str()),
            ("launch_id", instance.launch_id.as_str()),
            ("channel_id", channel_id.as_str()),
        ];
        if let Some(guild_id) = &guild_id {
            params.push(("guild_id", guild_id));
        }
        params.extend([
            ("discord_proxy_ticket", self.proxy_ticket.as_str()),
            ("frame_id", frame_id),
            ("platform", "desktop"),
        ]);
        let query = params
            .iter()
            .map(|(key, value)| format!("{key}={}", utf8_percent_encode(value, NON_ALPHANUMERIC)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}/?{query}", self.origin())
    }
}

/// Opens the activity in a window of its own. `on_closed` runs once the window
/// is gone, however it closed.
pub fn open_activity_window(
    context: ActivityContext,
    on_closed: impl FnOnce(&mut App) + 'static,
    cx: &mut App,
) -> Option<(AnyWindowHandle, WeakEntity<ActivityWindow>)> {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(context.app.name.clone().into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(760.)), cx)),
        window_min_size: Some(size(px(480.), px(360.))),
        ..Default::default()
    };
    let mut activity = None;
    let handle = cx
        .open_window(options, |window, cx| {
            let view = cx.new(|cx| ActivityWindow::new(context, Box::new(on_closed), window, cx));
            activity = Some(view.downgrade());
            cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
        })
        .inspect_err(|err| eprintln!("failed to open the activity window: {err}"))
        .ok()?;
    Some((handle.into(), activity?))
}

pub fn close_activity_window(handle: AnyWindowHandle, cx: &mut App) {
    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
}

type OnClosed = Box<dyn FnOnce(&mut App)>;

/// From the page, through the webview's callbacks.
enum PageEvent {
    Rpc(String),
    /// A link or popup leaving the activity, opened in the browser instead.
    External(String),
}

/// Who a command's answer goes back to.
#[derive(Clone)]
struct Reply {
    cmd: String,
    nonce: Option<String>,
}

impl Reply {
    fn ok(&self, data: Value) -> Value {
        rpc::reply(&self.cmd, self.nonce.as_deref(), data)
    }

    fn err(&self, code: u32, message: &str) -> Value {
        rpc::error(&self.cmd, self.nonce.as_deref(), code, message)
    }
}

/// An `AUTHORIZE` waiting on the user, shown over the activity.
struct Consent {
    reply: Reply,
    oauth: OAuthRequest,
    app_name: String,
    icon_url: Option<String>,
}

pub struct ActivityWindow {
    context: ActivityContext,
    /// Names this mount; a handshake carrying any other is refused.
    frame_id: String,
    window_handle: AnyWindowHandle,
    webview: Option<Entity<WebView>>,
    /// The signed-in user as the API serves it; READY waits for it.
    self_user: Option<Value>,
    handshake_pending: bool,
    /// Users the activity may be told about, by id; `Value::Null` for one that
    /// couldn't be fetched.
    users: HashMap<Id<UserMarker>, Value>,
    requested_users: HashSet<Id<UserMarker>>,
    /// Participant lists asked for before every participant's user was known.
    participant_replies: Vec<Reply>,
    participants_changed: bool,
    subscriptions: HashSet<String>,
    /// Granted by `AUTHENTICATE`; until then only the unauthenticated commands answer.
    scopes: Option<Vec<String>>,
    consent: Option<Consent>,
    on_closed: Option<OnClosed>,
}

impl ActivityWindow {
    fn new(
        context: ActivityContext,
        on_closed: OnClosed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let frame_id = uuid::Uuid::new_v4().to_string();
        let (tx, mut rx) = unbounded::<PageEvent>();

        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |this, cx| this.receive(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let url = context.url(&frame_id);
        let origin = context.origin();
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            // Lets gpui finish creating the OS window the webview parents to.
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let result = gpui_kit::AsyncApp::update(cx, |cx| {
                cx.update_window(window_handle, |_, window, cx| {
                    build_webview(&url, origin, tx, window, cx)
                })
            });
            match result {
                Ok(Ok(webview)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.webview = Some(webview);
                        cx.notify();
                    });
                }
                Ok(Err(err)) => eprintln!("failed to build the activity webview: {err}"),
                Err(_) => {}
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            let user = discord::fetch_user_object(None).await;
            let _ = this.update(cx, |this, cx| this.self_user_loaded(user, cx));
        })
        .detach();

        cx.on_release(|this, cx| {
            if let Some(on_closed) = this.on_closed.take() {
                on_closed(cx);
            }
        })
        .detach();

        let mut this = Self {
            context,
            frame_id,
            window_handle,
            webview: None,
            self_user: None,
            handshake_pending: false,
            users: HashMap::new(),
            requested_users: HashSet::new(),
            participant_replies: Vec::new(),
            participants_changed: false,
            subscriptions: HashSet::new(),
            scopes: None,
            consent: None,
            on_closed: Some(on_closed),
        };
        this.remember_members();
        this
    }

    /// The instance changed: someone joined or left.
    pub fn update_instance(&mut self, instance: ActivityInstance, cx: &mut Context<Self>) {
        self.context.instance = instance;
        self.remember_members();
        self.participants_changed = true;
        self.flush_participants(cx);
    }

    fn remember_members(&mut self) {
        for participant in &self.context.instance.participants {
            if let Some(user) = &participant.user {
                self.users.insert(participant.user_id, user.clone());
            }
        }
    }

    fn send(&self, message: Value, cx: &App) {
        let Some(webview) = &self.webview else {
            return;
        };
        if let Err(err) = webview
            .read(cx)
            .evaluate_script(&format!("window.__oxidecordReceive({message})"))
        {
            eprintln!("failed to answer the activity: {err}");
        }
    }

    fn receive(&mut self, event: PageEvent, cx: &mut Context<Self>) {
        match event {
            PageEvent::External(url) => open_external(&url, cx),
            PageEvent::Rpc(raw) => match rpc::parse(&raw) {
                Some(rpc::Inbound::Handshake(handshake)) => self.handshake(handshake, cx),
                Some(rpc::Inbound::Command(command)) => self.command(command, cx),
                Some(rpc::Inbound::Close) => self.close(cx),
                None => {}
            },
        }
    }

    fn close(&self, cx: &mut Context<Self>) {
        let handle = self.window_handle;
        cx.defer(move |cx| close_activity_window(handle, cx));
    }

    fn handshake(&mut self, handshake: rpc::Handshake, cx: &mut Context<Self>) {
        if handshake.client_id != self.context.app.id.to_string() {
            self.send(
                rpc::close(rpc::close::INVALID_CLIENT_ID, "Invalid client ID"),
                cx,
            );
            return;
        }
        if handshake.frame_id != self.frame_id || handshake.v.is_some_and(|v| v != 1) {
            self.send(rpc::close(rpc::close::INVALID_ORIGIN, "Invalid frame"), cx);
            return;
        }
        // A reload starts the SDK over.
        self.subscriptions.clear();
        self.scopes = None;
        self.handshake_pending = true;
        self.send_ready(cx);
    }

    fn self_user_loaded(&mut self, user: Result<Value, String>, cx: &mut Context<Self>) {
        let user = user.unwrap_or_else(|err| {
            eprintln!("failed to fetch the user for an activity: {err}");
            json!({ "id": self.context.self_id.to_string() })
        });
        self.users.insert(self.context.self_id, user.clone());
        self.self_user = Some(user);
        self.send_ready(cx);
    }

    fn send_ready(&mut self, cx: &mut Context<Self>) {
        let Some(user) = &self.self_user else {
            return;
        };
        if std::mem::take(&mut self.handshake_pending) {
            self.send(rpc::ready(rpc::public_user(user)), cx);
        }
    }

    fn command(&mut self, command: rpc::Command, cx: &mut Context<Self>) {
        let reply = Reply {
            cmd: command.cmd.clone(),
            nonce: command.nonce.clone(),
        };
        let args = &command.args;
        let authenticated = self.scopes.is_some();
        let answer = match command.cmd.as_str() {
            // The SDK sends every console line; it must be answered or its promises pile up.
            "CAPTURE_LOG" => reply.ok(Value::Null),
            // Discord's own telemetry for the activity; nothing here collects it.
            "SEND_ANALYTICS_EVENT" => reply.ok(Value::Null),
            "SUBSCRIBE" | "UNSUBSCRIBE" => {
                let Some(event) = command.evt else {
                    return self.send(reply.err(error::INVALID_PAYLOAD, "Missing event"), cx);
                };
                if command.cmd == "SUBSCRIBE" {
                    self.subscriptions.insert(event.clone());
                } else {
                    self.subscriptions.remove(&event);
                }
                reply.ok(json!({ "evt": event }))
            }
            "AUTHORIZE" => return self.authorize(reply, args, cx),
            "AUTHENTICATE" => return self.authenticate(reply, args, cx),
            "GET_CONTEXT" => reply.ok(json!({
                "surface": {
                    "type": 0,
                    "channel_id": self.context.instance.channel_id.to_string(),
                    "guild_id": self.context.instance.guild_id.map(|id| id.to_string()),
                },
                "launch": {},
                "platform": "desktop",
            })),
            "GET_PLATFORM_BEHAVIORS" => reply.ok(json!({})),
            "ENCOURAGE_HW_ACCELERATION" => reply.ok(json!({ "enabled": true })),
            "USER_SETTINGS_GET_LOCALE" => reply.ok(json!({ "locale": discord::system_locale() })),
            "SET_CONFIG" => reply.ok(json!({
                "use_interactive_pip": args
                    .get("use_interactive_pip")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })),
            "SET_ORIENTATION_LOCK_STATE" => reply.ok(Value::Null),
            // Rich presence isn't sent anywhere yet; the activity is told it was.
            "SET_ACTIVITY" => reply.ok(args.get("activity").cloned().unwrap_or(Value::Null)),
            "GET_SKUS_EMBEDDED" => reply.ok(json!({ "skus": [] })),
            "GET_ENTITLEMENTS_EMBEDDED" => reply.ok(json!({ "entitlements": [] })),
            "OPEN_EXTERNAL_LINK" => {
                let url = args.get("url").and_then(Value::as_str).unwrap_or_default();
                let opened = is_web_url(url);
                if opened {
                    cx.open_url(url);
                }
                reply.ok(json!({ "opened": opened }))
            }
            "REQUEST_PROXY_TICKET_REFRESH" if authenticated => {
                return self.refresh_proxy_ticket(reply, cx);
            }
            "GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS"
            | "GET_INSTANCE_CONNECTED_PARTICIPANTS"
                if authenticated =>
            {
                self.participant_replies.push(reply);
                return self.flush_participants(cx);
            }
            "GET_CHANNEL" if authenticated => {
                let asked = args.get("channel_id").and_then(Value::as_str);
                let channel_id = self.context.instance.channel_id.to_string();
                if asked.is_some_and(|asked| asked != channel_id) {
                    reply.err(error::INVALID_CHANNEL, "Invalid channel ID")
                } else {
                    reply.ok(json!({
                        "id": channel_id,
                        "type": self.context.channel_kind,
                        "guild_id": self.context.instance.guild_id.map(|id| id.to_string()),
                        "name": self.context.channel_name,
                        "voice_states": [],
                        "messages": [],
                    }))
                }
            }
            "GET_CHANNEL_PERMISSIONS" if authenticated => reply.ok(json!({
                "permissions": self.context.permissions.bits().to_string(),
            })),
            "GET_CURRENT_USER" if self.has_scope("identify") => {
                let user = self.self_user.as_ref().map(rpc::public_user);
                reply.ok(user.unwrap_or(Value::Null))
            }
            "GET_USER" if authenticated => {
                return self.get_user(reply, args, cx);
            }
            "REQUEST_PROXY_TICKET_REFRESH"
            | "GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS"
            | "GET_INSTANCE_CONNECTED_PARTICIPANTS"
            | "GET_CHANNEL"
            | "GET_CHANNEL_PERMISSIONS"
            | "GET_CURRENT_USER"
            | "GET_USER" => reply.err(
                error::INVALID_PERMISSIONS,
                "Not authenticated or invalid scope",
            ),
            other => reply.err(
                error::INVALID_COMMAND,
                &format!("{other} isn't supported by this client"),
            ),
        };
        self.send(answer, cx);
    }

    fn has_scope(&self, scope: &str) -> bool {
        self.scopes
            .as_ref()
            .is_some_and(|scopes| scopes.iter().any(|s| s == scope))
    }

    fn authorize(&mut self, reply: Reply, args: &Value, cx: &mut Context<Self>) {
        let app_id = self.context.app.id;
        if args.get("client_id").and_then(Value::as_str) != Some(&app_id.to_string()) {
            return self.send(reply.err(error::INVALID_CLIENT_ID, "Invalid client ID"), cx);
        }
        if args
            .get("response_type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "code")
        {
            return self.send(
                reply.err(error::INVALID_PAYLOAD, "response_type must be \"code\""),
                cx,
            );
        }
        if self.consent.is_some() {
            return self.send(
                reply.err(error::OAUTH2, "Another authorization is in progress"),
                cx,
            );
        }
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);
        let asked_install = match args.get("integration_type") {
            Some(Value::Number(kind)) => kind.as_u64().and_then(|kind| u8::try_from(kind).ok()),
            Some(Value::String(kind)) => kind.parse().ok(),
            _ => None,
        };
        let mut oauth = OAuthRequest {
            client_id: app_id,
            scopes: rpc::scopes(args),
            state: text("state"),
            code_challenge: text("code_challenge"),
            code_challenge_method: text("code_challenge_method"),
            integration_type: 0,
        };
        cx.spawn(async move |this, cx| {
            oauth.integration_type = match asked_install {
                Some(kind) => kind,
                None => match discord::fetch_install_type(app_id).await {
                    Ok(kind) => kind,
                    Err(err) => {
                        let _ = this
                            .update(cx, |this, cx| this.send(reply.err(error::OAUTH2, &err), cx));
                        return;
                    }
                },
            };
            let preview = discord::preview_authorization(oauth.clone()).await;
            let _ = this.update(cx, |this, cx| match preview {
                Err(err) => this.send(reply.err(error::OAUTH2, &err), cx),
                // Granted before, so Discord's client doesn't ask again.
                Ok(preview) if preview.authorized => this.grant(reply, oauth, cx),
                Ok(preview) => {
                    this.consent = Some(Consent {
                        reply,
                        oauth,
                        app_name: preview.app_name,
                        icon_url: preview.app_icon_url,
                    });
                    // The webview is a native child window drawn over everything.
                    if let Some(webview) = &this.webview {
                        webview.update(cx, |webview, _| webview.hide());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn answer_consent(&mut self, approved: bool, cx: &mut Context<Self>) {
        let Some(consent) = self.consent.take() else {
            return;
        };
        if let Some(webview) = &self.webview {
            webview.update(cx, |webview, _| webview.show());
        }
        if approved {
            self.grant(consent.reply, consent.oauth, cx);
        } else {
            self.send(
                consent
                    .reply
                    .err(error::OAUTH2, "The user cancelled the authorization"),
                cx,
            );
        }
        cx.notify();
    }

    fn grant(&mut self, reply: Reply, oauth: OAuthRequest, cx: &mut Context<Self>) {
        let channel_id = self.context.instance.channel_id;
        let guild_id = self.context.instance.guild_id;
        cx.spawn(async move |this, cx| {
            let code = discord::authorize(oauth, channel_id, guild_id).await;
            let _ = this.update(cx, |this, cx| {
                let answer = match code {
                    Ok(code) => reply.ok(json!({ "code": code })),
                    Err(err) => reply.err(error::OAUTH2, &err),
                };
                this.send(answer, cx);
            });
        })
        .detach();
    }

    fn authenticate(&mut self, reply: Reply, args: &Value, cx: &mut Context<Self>) {
        let Some(token) = args
            .get("access_token")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return self.send(
                reply.err(error::INVALID_PAYLOAD, "Missing access token"),
                cx,
            );
        };
        let app_id = self.context.app.id.to_string();
        let self_id = self.context.self_id.to_string();
        cx.spawn(async move |this, cx| {
            let identity = discord::fetch_oauth_identity(token.clone()).await;
            let _ = this.update(cx, |this, cx| {
                let answer = match identity {
                    Err(err) => reply.err(error::INVALID_TOKEN, &err),
                    Ok(identity) => {
                        let application = &identity["application"];
                        if application["id"].as_str() != Some(&app_id) {
                            reply.err(error::INVALID_CLIENT_ID, "Token is for another application")
                        } else if identity["user"]["id"].as_str() != Some(&self_id) {
                            reply.err(error::INVALID_TOKEN, "Token is for another user")
                        } else {
                            let scopes: Vec<String> = identity["scopes"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|scope| scope.as_str().map(str::to_owned))
                                .collect();
                            this.scopes = Some(scopes.clone());
                            reply.ok(json!({
                                "access_token": token,
                                "user": rpc::public_user(&identity["user"]),
                                "scopes": scopes,
                                "expires": identity["expires"],
                                "application": {
                                    "id": app_id,
                                    "name": application["name"],
                                    "description": application["description"].as_str().unwrap_or_default(),
                                    "icon": application["icon"],
                                    "rpc_origins": application.get("rpc_origins").cloned().unwrap_or_else(|| json!([])),
                                },
                            }))
                        }
                    }
                };
                this.send(answer, cx);
            });
        })
        .detach();
    }

    fn refresh_proxy_ticket(&mut self, reply: Reply, cx: &mut Context<Self>) {
        let app_id = self.context.app.id;
        let channel_id = self.context.instance.channel_id;
        cx.spawn(async move |this, cx| {
            let ticket = discord::fetch_proxy_ticket(app_id, channel_id).await;
            let _ = this.update(cx, |this, cx| {
                let answer = match ticket {
                    Ok(ticket) => reply.ok(json!({ "ticket": ticket })),
                    Err(err) => reply.err(error::INVALID_PAYLOAD, &err),
                };
                this.send(answer, cx);
            });
        })
        .detach();
    }

    fn get_user(&mut self, reply: Reply, args: &Value, cx: &mut Context<Self>) {
        let Some(user_id) = args
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse().ok())
            .and_then(Id::<UserMarker>::new_checked)
        else {
            return self.send(reply.err(error::INVALID_PAYLOAD, "Invalid user ID"), cx);
        };
        if let Some(user) = self.users.get(&user_id) {
            let user = public_or_null(user);
            return self.send(reply.ok(user), cx);
        }
        cx.spawn(async move |this, cx| {
            let user = discord::fetch_user_object(Some(user_id)).await;
            let _ = this.update(cx, |this, cx| {
                let user = user.unwrap_or(Value::Null);
                this.users.insert(user_id, user.clone());
                this.send(reply.ok(public_or_null(&user)), cx);
            });
        })
        .detach();
    }

    /// Answers waiting participant requests and announces a change, once every
    /// participant's user is known; fetches the ones that aren't first.
    fn flush_participants(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<_> = self
            .context
            .instance
            .participants
            .iter()
            .map(|p| p.user_id)
            .filter(|id| !self.users.contains_key(id))
            .collect();
        if !missing.is_empty() {
            let unrequested: Vec<_> = missing
                .into_iter()
                .filter(|id| self.requested_users.insert(*id))
                .collect();
            for user_id in unrequested {
                cx.spawn(async move |this, cx| {
                    let user = discord::fetch_user_object(Some(user_id)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.users.insert(user_id, user.unwrap_or(Value::Null));
                        this.flush_participants(cx);
                    });
                })
                .detach();
            }
            return;
        }

        let participants = self.participants();
        for reply in std::mem::take(&mut self.participant_replies) {
            self.send(reply.ok(participants.clone()), cx);
        }
        if std::mem::take(&mut self.participants_changed)
            && self.subscriptions.contains(rpc::PARTICIPANTS_UPDATE)
        {
            self.send(rpc::dispatch(rpc::PARTICIPANTS_UPDATE, participants), cx);
        }
    }

    fn participants(&self) -> Value {
        let mut seen = HashSet::new();
        let participants: Vec<_> = self
            .context
            .instance
            .participants
            .iter()
            // Someone in on two clients is one participant.
            .filter(|p| seen.insert(p.user_id))
            .filter_map(|p| {
                let user = self.users.get(&p.user_id).filter(|user| !user.is_null())?;
                let mut user = rpc::public_user(user);
                if let Some(nick) = &p.nick {
                    user["nickname"] = nick.clone().into();
                }
                Some(user)
            })
            .collect();
        json!({ "participants": participants })
    }

    fn render_consent(&self, consent: &Consent, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let scopes = consent
            .oauth
            .scopes
            .iter()
            .filter(|scope| !scope.starts_with("rpc"))
            .map(|scope| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Check)
                            .size_4()
                            .text_color(theme.success),
                    )
                    .child(div().text_sm().child(rpc::describe_scope(scope)))
            });

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background)
            .child(
                depth::card(radius::CARD, cx)
                    .flex()
                    .flex_col()
                    .w(px(400.))
                    .p(px(24.))
                    .gap(px(20.))
                    .child(
                        v_flex()
                            .items_center()
                            .gap_2()
                            .when_some(consent.icon_url.clone(), |this, url| {
                                this.child(img(url).size(px(64.)).rounded(radius::CARD))
                            })
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child(consent.app_name.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child("wants to access your Discord account"),
                            ),
                    )
                    .child(v_flex().gap_2().children(scopes))
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("consent-cancel")
                                    .label("Cancel")
                                    .ghost()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.answer_consent(false, cx)
                                        }),
                                    ),
                            )
                            .child(
                                Button::new("consent-authorize")
                                    .label("Authorize")
                                    .primary()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.answer_consent(true, cx)),
                                    ),
                            ),
                    ),
            )
    }
}

impl Render for ActivityWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .flex()
            .bg(cx.theme().background)
            .when_some(self.webview.clone(), |this, webview| this.child(webview))
            .when_some(self.consent.as_ref(), |this, consent| {
                this.child(self.render_consent(consent, cx))
            })
    }
}

fn build_webview(
    url: &str,
    origin: String,
    tx: UnboundedSender<PageEvent>,
    window: &mut Window,
    cx: &mut App,
) -> anyhow::Result<Entity<WebView>> {
    let ipc_tx = tx.clone();
    let navigation_tx = tx.clone();
    let wry_webview = WebViewBuilder::new()
        .with_url(url)
        .with_initialization_script(SHIM)
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(move |request| {
            let _ = ipc_tx.unbounded_send(PageEvent::Rpc(request.body().clone()));
        })
        // The activity is confined to its own origin, as Discord's sandbox does;
        // anywhere else opens in the browser.
        .with_navigation_handler(move |url| {
            if is_same_origin(&url, &origin) || url.starts_with("about:") {
                return true;
            }
            let _ = navigation_tx.unbounded_send(PageEvent::External(url));
            false
        })
        .with_new_window_req_handler(move |url, _| {
            let _ = tx.unbounded_send(PageEvent::External(url));
            NewWindowResponse::Deny
        })
        .build(window)?;
    webview::deny_permissions(&wry_webview);
    Ok(cx.new(|cx| {
        let mut webview = WebView::new(wry_webview, window, cx);
        webview.show();
        webview
    }))
}

fn is_same_origin(url: &str, origin: &str) -> bool {
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn open_external(url: &str, cx: &mut App) {
    if is_web_url(url) {
        cx.open_url(url);
    }
}

fn public_or_null(user: &Value) -> Value {
    if user.is_null() {
        Value::Null
    } else {
        rpc::public_user(user)
    }
}

#[cfg(test)]
mod tests {
    use super::is_same_origin;

    #[test]
    fn same_origin_needs_a_boundary() {
        let origin = "https://1.discordsays.com";
        assert!(is_same_origin("https://1.discordsays.com/?a=b", origin));
        assert!(is_same_origin("https://1.discordsays.com", origin));
        assert!(!is_same_origin(
            "https://1.discordsays.com.evil.example/",
            origin
        ));
        assert!(!is_same_origin("https://discord.com/", origin));
    }
}
