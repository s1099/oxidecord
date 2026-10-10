//! Watching someone else's stream in the call.
//!
//! The same dance as going live, from the other side: a gateway command
//! (`STREAM_WATCH`) answered by `STREAM_CREATE` and `STREAM_SERVER_UPDATE`,
//! after which a connection of its own opens to the stream's server — or by
//! a `STREAM_DELETE` saying why it can't be watched.
//!
//! Decoded pictures cross to gpui over a channel, like a playing video's, and
//! are handled the same way: each replaces the last, and the last is handed
//! back to the sprite atlas (see [`HomeScreen::show_frame`]'s notes).

use std::sync::Arc;

use gpui::*;
use image::{Frame, ImageBuffer};
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord;
use crate::platform::h264::{DecodedFrame, DecoderEvent};
use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{StreamWatch, VoiceStatus};
use crate::voice::stream::{StreamConnection, WatchStream};

/// The size pictures are decoded at before the stage has been drawn and
/// said how big it is.
const DEFAULT_TARGET: (u32, u32) = (1280, 720);

impl HomeScreen {
    /// Whether `user_id`'s stream can be opened: they're live in the call the
    /// user is connected to, and it's not the user's own.
    pub(in crate::screens::home) fn can_watch(&self, user_id: Id<UserMarker>) -> bool {
        let Some(call) = self
            .voice
            .as_ref()
            .filter(|call| call.status == VoiceStatus::Connected)
        else {
            return false;
        };
        Some(user_id) != self.self_user_id
            && self
                .voice_states
                .get(&user_id)
                .is_some_and(|state| state.self_stream && state.channel_id == Some(call.channel_id))
    }

    /// Starts watching `user_id`'s stream, in place of any other.
    pub(in crate::screens::home) fn watch_stream(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_watch(user_id) {
            return;
        }
        let Some(call) = &self.voice else {
            return;
        };
        let stream_key = discord::stream_key(call.guild_id, call.channel_id, user_id);
        if self
            .watching
            .as_ref()
            .is_some_and(|watch| watch.stream_key == stream_key)
        {
            return;
        }
        // One stream at a time, like Discord's own stage.
        self.stop_watching(cx);
        let (Some(call), Some(gateway)) = (&mut self.voice, &self.gateway) else {
            return;
        };
        call.watch_error = None;
        gateway.watch_stream(&stream_key);

        let (frames, rx) = futures::channel::mpsc::unbounded::<DecoderEvent>();
        let key = stream_key.clone();
        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                let handled = this.update_in(cx, |this, window, cx| {
                    this.handle_watch_frame(&key, event, window, cx)
                });
                if !matches!(handled, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        self.watching = Some(StreamWatch {
            stream_key,
            streamer_id: user_id,
            server_id: None,
            endpoint: None,
            token: None,
            frames,
            stream: None,
            frame: None,
        });
        cx.notify();
    }

    /// Stops watching: tells the gateway, and closes the connection.
    pub(in crate::screens::home) fn stop_watching(&mut self, cx: &mut Context<Self>) {
        if let (Some(watch), Some(gateway)) = (&self.watching, &self.gateway) {
            gateway.delete_stream(&watch.stream_key);
        }
        self.end_watch();
        cx.notify();
    }

    /// Drops the watch without telling the gateway, for when it already
    /// knows: the stream ended, or the call did.
    pub(in crate::screens::home) fn end_watch(&mut self) {
        if let Some(frame) = self.watching.take().and_then(|watch| watch.frame) {
            self.retired_stream_frames.push(frame);
        }
    }

    /// Hands pictures from an ended watch back to gpui. Called on render.
    pub(in crate::screens::home) fn release_stream_frames(&mut self, window: &mut Window) {
        for frame in self.retired_stream_frames.drain(..) {
            let _ = window.drop_image(frame);
        }
    }

    /// The watched stream, if `stream_key` names it.
    pub(in crate::screens::home) fn watched_stream(
        &mut self,
        stream_key: &str,
    ) -> Option<&mut StreamWatch> {
        self.watching
            .as_mut()
            .filter(|watch| watch.stream_key == stream_key)
    }

    pub(in crate::screens::home) fn handle_watch_create(
        &mut self,
        rtc_server_id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(watch) = &mut self.watching {
            watch.server_id = Some(rtc_server_id);
        }
        self.try_start_watch(cx);
    }

    pub(in crate::screens::home) fn handle_watch_server(
        &mut self,
        server: discord::StreamServerInfo,
        cx: &mut Context<Self>,
    ) {
        let Some(watch) = &mut self.watching else {
            return;
        };
        watch.token = Some(server.token);
        // A different server while watching is Discord moving the stream:
        // drop the connection to the old one, and open one to the new as
        // soon as it's named. No endpoint means it hasn't been yet.
        if server.endpoint.is_none() || server.endpoint != watch.endpoint {
            watch.stream = None;
        }
        watch.endpoint = server.endpoint;
        self.try_start_watch(cx);
    }

    /// The stream ended, or Discord refused to show it.
    pub(in crate::screens::home) fn handle_watch_delete(
        &mut self,
        reason: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.end_watch();
        let error = match reason.as_deref() {
            Some("stream_full") => Some("This stream is full"),
            Some("unauthorized") => Some("You don't have permission to watch this stream"),
            Some("safety_guild_rate_limited") => Some("Too many streams are being watched here"),
            _ => None,
        };
        if let Some(error) = error {
            self.report_watch_error(error.into(), cx);
        }
        cx.notify();
    }

    /// The connection ended on its own.
    pub(in crate::screens::home) fn handle_watch_ended(
        &mut self,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.end_watch();
        if let Some(error) = error {
            self.report_watch_error(error, cx);
        }
        cx.notify();
    }

    /// Opens the connection once the gateway has delivered every part of it.
    fn try_start_watch(&mut self, cx: &mut Context<Self>) {
        let session_id = self.voice.as_ref().and_then(|call| call.session_id.clone());
        let (Some(watch), Some(session_id), Some(user_id), Some(events)) = (
            self.watching.as_mut(),
            session_id,
            self.self_user_id,
            self.stream_events.clone(),
        ) else {
            return;
        };
        if watch.stream.is_some() {
            return;
        }
        let (Some(server_id), Some(endpoint), Some(token)) = (
            watch.server_id.clone(),
            watch.endpoint.clone(),
            watch.token.clone(),
        ) else {
            return;
        };
        let Some(dave_channel_id) = StreamConnection::dave_channel_for(&server_id) else {
            self.stop_watching(cx);
            self.report_watch_error("Discord sent an unreadable stream server".into(), cx);
            return;
        };

        watch.stream = Some(WatchStream::start(
            StreamConnection {
                stream_key: watch.stream_key.clone(),
                server_id,
                dave_channel_id,
                user_id,
                session_id,
                token,
                endpoint,
            },
            watch.streamer_id,
            DEFAULT_TARGET,
            events,
            watch.frames.clone(),
        ));
        cx.notify();
    }

    /// Returns whether the watch this picture belongs to is still the current
    /// one; once it isn't, the pump feeding it stops.
    fn handle_watch_frame(
        &mut self,
        stream_key: &str,
        event: DecoderEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(watch) = self.watched_stream(stream_key) else {
            return false;
        };
        match event {
            DecoderEvent::Frame(frame) => {
                // Owed however the picture is handled, or the decoder stalls.
                if let Some(stream) = &watch.stream {
                    stream.frame_consumed();
                }
                if let Some(image) = to_image(frame)
                    && let Some(previous) = watch.frame.replace(image)
                {
                    let _ = window.drop_image(previous);
                }
            }
            DecoderEvent::Failed(error) => {
                self.stop_watching(cx);
                self.report_watch_error(error, cx);
            }
        }
        cx.notify();
        true
    }

    /// Shows why a stream couldn't be watched, on the call's panel.
    fn report_watch_error(&mut self, error: String, cx: &mut Context<Self>) {
        eprintln!("watching a stream: {error}");
        if let Some(call) = &mut self.voice {
            call.watch_error = Some(error);
        }
        cx.notify();
    }
}

/// Wraps decoded pixels as something `img()` can draw. They're BGRA already,
/// so this is a move.
fn to_image(frame: DecodedFrame) -> Option<Arc<RenderImage>> {
    let buffer = ImageBuffer::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}
