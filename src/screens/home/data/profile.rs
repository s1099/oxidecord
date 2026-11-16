//! Opening the profile popout, loading the user behind it, and messaging them
//! from it.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, ProfilePopup, View};

impl HomeScreen {
    /// `name` and `avatar_url` come from the clicked message so the card never opens
    /// blank while fetching.
    ///
    /// Closing is the card's own business: its dismiss layer swallows presses outside
    /// it, so a second click on the same avatar never reaches this.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::screens::home) fn open_profile(
        &mut self,
        user_id: Id<UserMarker>,
        name: String,
        avatar_url: Option<String>,
        webhook: bool,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opened from a server, the card shows their profile and roles there.
        let guild_id = self
            .selected_guild
            .filter(|_| self.view == View::Guild && !webhook);
        let cached = self
            .profile_cache
            .get(&user_id)
            .filter(|profile| profile.guild_id == guild_id)
            .cloned();

        let message = cx.new(|cx| {
            InputState::new(window, cx).placeholder(format!(
                "Message @{}",
                cached
                    .as_ref()
                    .map_or(name.as_str(), |profile| profile.display_name())
            ))
        });
        let message_sent = cx.subscribe_in(
            &message,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let text = input.read(cx).value().trim().to_owned();
                    if !text.is_empty() {
                        this.message_from_profile(user_id, text, window, cx);
                    }
                }
            },
        );

        self.profile_popup = Some(ProfilePopup {
            user_id,
            guild_id,
            position,
            name,
            avatar_url,
            webhook,
            profile: cached.clone(),
            error: None,
            message,
            _message_sent: message_sent,
        });
        cx.notify();

        if cached.is_none() && !webhook {
            self.load_profile(user_id, guild_id, cx);
        }
    }

    pub(in crate::screens::home) fn close_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile_popup.take().is_some() {
            cx.notify();
        }
    }

    fn load_profile(
        &mut self,
        user_id: Id<UserMarker>,
        guild_id: Option<Id<GuildMarker>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_user_profile(user_id, guild_id).await;
            let _ = this.update(cx, |this, cx| {
                // The card may have been closed, or reopened for somebody
                // else, while the request was in flight.
                let popup = this
                    .profile_popup
                    .as_mut()
                    .filter(|popup| popup.user_id == user_id && popup.guild_id == guild_id);
                match result {
                    Ok(profile) => {
                        if let Some(popup) = popup {
                            popup.profile = Some(profile.clone());
                        }
                        this.profile_cache.insert(user_id, profile);
                    }
                    Err(err) => {
                        if let Some(popup) = popup {
                            popup.error = Some(err);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sends `text` to the user's DM, opening it as Discord does. The DM is
    /// created first if this is the first message between the two.
    fn message_from_profile(
        &mut self,
        user_id: Id<UserMarker>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_profile(cx);
        let existing = self
            .dms
            .list
            .iter()
            .find(|dm| dm.recipient_id == Some(user_id))
            .map(|dm| dm.id);
        if let Some(channel_id) = existing {
            self.send_to_dm(channel_id, text, window, cx);
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let result = discord::open_dm_with(user_id).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(dm) => {
                    let channel_id = dm.id;
                    if !this.dms.list.iter().any(|known| known.id == channel_id) {
                        this.dms.list.insert(0, dm);
                    }
                    this.send_to_dm(channel_id, text, window, cx);
                }
                Err(err) => {
                    window.push_notification(
                        Notification::error(format!("Couldn't open the conversation: {err}")),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Through the composer, so the message gets the same pending copy,
    /// mention and emoji handling as one typed there.
    fn send_to_dm(
        &mut self,
        channel_id: Id<ChannelMarker>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dm(channel_id, window, cx);
        self.message_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
        self.send_current_message(window, cx);
    }
}
