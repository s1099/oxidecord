//! Keeps the taskbar thumbnail's call buttons in step with the call and acts
//! on their clicks.

use futures::StreamExt as _;
use gpui_kit::*;

use crate::platform::taskbar::{CallState, Click, ThumbBar};
use crate::screens::home::HomeScreen;

impl HomeScreen {
    pub(in crate::screens::home) fn install_thumb_bar(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some((bar, mut clicks)) = ThumbBar::install(window) else {
            return;
        };
        // Any change to the call or its mute state notifies the screen.
        let sync = cx.observe_self(|this, _| this.sync_thumb_bar());
        self.thumb_bar = Some((bar, sync));
        self.sync_thumb_bar();
        cx.spawn(async move |this, cx| {
            while let Some(click) = clicks.next().await {
                let handled = this.update(cx, |this, cx| {
                    if this.voice.is_none() {
                        return;
                    }
                    match click {
                        Click::Mute => this.toggle_voice_mute(cx),
                        Click::Deafen => this.toggle_voice_deafen(cx),
                        Click::Disconnect => this.leave_voice(cx),
                    }
                });
                if handled.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn sync_thumb_bar(&self) {
        if let Some((bar, _)) = &self.thumb_bar {
            bar.set(self.voice.as_ref().map(|_| CallState {
                muted: self.voice_settings.muted,
                deafened: self.voice_settings.deafened,
            }));
        }
    }
}
