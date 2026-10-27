//! Going live: sharing the screen into the call.
//!
//! Like joining a call, starting a stream is a gateway command answered by
//! dispatches: `STREAM_CREATE` names the server and `STREAM_SERVER_UPDATE` says
//! where it is, in either order, and the connection opens only once both land.
//! The source is picked first because creating the stream shows the user as live
//! to everyone in the call; a picker left open or cancelled shouldn't show a
//! stream that never starts.

use gpui_kit::*;

use crate::discord;
use crate::platform::capture::{self, CaptureSettings, CaptureSource};
use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{ScreenShare, VoiceStatus};
use crate::voice::stream::{GoLive, StreamConnection, StreamEvent};

/// 720p at 30 frames a second: the most Discord shows of a stream from an
/// account without Nitro, so sending more would only be scaled away.
const STREAM_SETTINGS: CaptureSettings = CaptureSettings {
    max_width: 1280,
    max_height: 720,
    fps: 30,
    bitrate: 2_500_000,
};

impl HomeScreen {
    pub(in crate::screens::home) fn can_share_screen(&self) -> bool {
        self.voice
            .as_ref()
            .is_some_and(|call| call.status == VoiceStatus::Connected)
            && capture::is_supported()
    }

    pub(in crate::screens::home) fn toggle_screen_share(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.screen_share.is_some() {
            self.stop_screen_share(cx);
            return;
        }

        let pick = match capture::pick_source(window) {
            Ok(pick) => pick,
            Err(err) => {
                self.report_stream_error(err, cx);
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let Some(source) = pick.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.start_screen_share(source, cx));
        })
        .detach();
    }

    fn start_screen_share(&mut self, source: CaptureSource, cx: &mut Context<Self>) {
        // The call may have ended, or a stream started, while the picker was
        // open.
        if self.screen_share.is_some() || !self.can_share_screen() {
            return;
        }
        let (Some(call), Some(user_id), Some(gateway)) =
            (&mut self.voice, self.self_user_id, &self.gateway)
        else {
            return;
        };

        call.share_error = None;
        let stream_key = discord::stream_key(call.guild_id, call.channel_id, user_id);
        gateway.create_stream(call.guild_id, call.channel_id);
        gateway.set_stream_paused(&stream_key, false);

        self.screen_share = Some(ScreenShare {
            stream_key,
            source_name: source.name(),
            source: Some(source),
            server_id: None,
            endpoint: None,
            token: None,
            stream: None,
            live: false,
        });
        cx.notify();
    }

    pub(in crate::screens::home) fn stop_screen_share(&mut self, cx: &mut Context<Self>) {
        if let Some(share) = self.screen_share.take()
            && let Some(gateway) = &self.gateway
        {
            gateway.delete_stream(&share.stream_key);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn handle_stream_create(
        &mut self,
        stream_key: String,
        rtc_server_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.watched_stream(&stream_key).is_some() {
            self.handle_watch_create(rtc_server_id, cx);
            return;
        }
        let Some(share) = self.own_stream(&stream_key) else {
            return;
        };
        share.server_id = Some(rtc_server_id);
        self.try_start_stream(cx);
    }

    pub(in crate::screens::home) fn handle_stream_server(
        &mut self,
        server: discord::StreamServerInfo,
        cx: &mut Context<Self>,
    ) {
        if self.watched_stream(&server.stream_key).is_some() {
            self.handle_watch_server(server, cx);
            return;
        }
        let Some(share) = self.own_stream(&server.stream_key) else {
            return;
        };
        share.token = Some(server.token);
        // No endpoint means Discord is moving the stream; the next update
        // carries the new server.
        if let Some(endpoint) = server.endpoint {
            share.endpoint = Some(endpoint);
        }
        self.try_start_stream(cx);
    }

    /// The server ended the stream (call ended, moderator) or refused the user's
    /// watch; nothing is left to tell the gateway.
    pub(in crate::screens::home) fn handle_stream_delete(
        &mut self,
        stream_key: String,
        reason: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.watched_stream(&stream_key).is_some() {
            self.handle_watch_delete(reason, cx);
            return;
        }
        if self.own_stream(&stream_key).is_some() {
            self.screen_share = None;
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn handle_stream_event(
        &mut self,
        event: StreamEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            StreamEvent::Live { stream_key } => {
                if let Some(share) = self.own_stream(&stream_key) {
                    share.live = true;
                    cx.notify();
                }
            }
            StreamEvent::Ended { stream_key, error } => {
                if self.watched_stream(&stream_key).is_some() {
                    self.handle_watch_ended(error, cx);
                    return;
                }
                if self.own_stream(&stream_key).is_none() {
                    return;
                }
                self.stop_screen_share(cx);
                if let Some(error) = error {
                    self.report_stream_error(error, cx);
                }
            }
        }
    }

    /// Dispatches arrive for everyone's streams in the call, not only the user's.
    fn own_stream(&mut self, stream_key: &str) -> Option<&mut ScreenShare> {
        self.screen_share
            .as_mut()
            .filter(|share| share.stream_key == stream_key)
    }

    fn try_start_stream(&mut self, cx: &mut Context<Self>) {
        let session_id = self.voice.as_ref().and_then(|call| call.session_id.clone());
        let (Some(share), Some(session_id), Some(user_id), Some(events)) = (
            self.screen_share.as_mut(),
            session_id,
            self.self_user_id,
            self.stream_events.clone(),
        ) else {
            return;
        };
        // A second server update while the stream is already up is Discord
        // moving it; the source went to the first connection and can't be
        // handed to another, so that connection's end is the stream's end.
        if share.stream.is_some() {
            return;
        }
        let (Some(server_id), Some(endpoint), Some(token)) = (
            share.server_id.clone(),
            share.endpoint.clone(),
            share.token.clone(),
        ) else {
            return;
        };
        let Some(dave_channel_id) = StreamConnection::dave_channel_for(&server_id) else {
            self.stop_screen_share(cx);
            self.report_stream_error("Discord sent an unreadable stream server".into(), cx);
            return;
        };
        let Some(source) = share.source.take() else {
            return;
        };

        share.stream = Some(GoLive::start(
            StreamConnection {
                stream_key: share.stream_key.clone(),
                server_id,
                dave_channel_id,
                user_id,
                session_id,
                token,
                endpoint,
            },
            source,
            STREAM_SETTINGS,
            events,
        ));
        cx.notify();
    }

    fn report_stream_error(&mut self, error: String, cx: &mut Context<Self>) {
        eprintln!("screen share: {error}");
        if let Some(call) = &mut self.voice {
            call.share_error = Some(error);
        }
        cx.notify();
    }
}
