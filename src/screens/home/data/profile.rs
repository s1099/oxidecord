//! Opening the profile popout and the full profile, loading the user behind
//! them, and messaging them from either.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::notification::Notification;
use std::collections::HashSet;

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, ProfileModal, ProfilePopup, ProfileTab, View};

const NOTE_MAX_ROWS: usize = 6;

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

    /// Swaps the popout for the full profile of the same user.
    pub(in crate::screens::home) fn expand_profile(
        &mut self,
        tab: Option<ProfileTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(popup) = self.profile_popup.as_ref().filter(|popup| !popup.webhook) else {
            return;
        };
        let (user_id, guild_id) = (popup.user_id, popup.guild_id);
        let (name, avatar_url) = (popup.name.clone(), popup.avatar_url.clone());
        self.open_profile_modal(user_id, guild_id, name, avatar_url, tab, window, cx);
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
                let modal = this
                    .profile_modal
                    .as_mut()
                    .filter(|modal| modal.user_id == user_id && modal.guild_id == guild_id);
                match result {
                    Ok(profile) => {
                        if let Some(popup) = popup {
                            popup.profile = Some(profile.clone());
                        }
                        if let Some(modal) = modal {
                            modal.profile = Some(profile.clone());
                            this.load_widget_games(&profile, cx);
                        }
                        this.profile_cache.insert(user_id, profile);
                    }
                    Err(err) => {
                        if let Some(popup) = popup {
                            popup.error = Some(err.clone());
                        }
                        if let Some(modal) = modal {
                            modal.error = Some(err);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The full profile. Opening it closes the popout it's usually opened from;
    /// `tab` picks where it opens.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::screens::home) fn open_profile_modal(
        &mut self,
        user_id: Id<UserMarker>,
        guild_id: Option<Id<GuildMarker>>,
        name: String,
        avatar_url: Option<String>,
        tab: Option<ProfileTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_profile(cx);
        self.close_profile_modal(cx);

        let cached = self
            .profile_cache
            .get(&user_id)
            .filter(|profile| profile.guild_id == guild_id)
            .cloned();

        let note = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, NOTE_MAX_ROWS)
                .placeholder("Click to add a note")
        });
        let note_task = cx.spawn_in(window, {
            let note = note.clone();
            async move |this, cx| {
                let result = discord::fetch_user_note(user_id).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    let Some(modal) = this
                        .profile_modal
                        .as_mut()
                        .filter(|modal| modal.user_id == user_id)
                    else {
                        return;
                    };
                    match result {
                        Ok(text) => {
                            note.update(cx, |note, cx| note.set_value(text.clone(), window, cx));
                            modal.saved_note = Some(text);
                        }
                        Err(err) => eprintln!("failed to load note: {err}"),
                    }
                    cx.notify();
                });
            }
        });

        self.profile_modal = Some(ProfileModal {
            user_id,
            guild_id,
            name,
            avatar_url,
            profile: cached.clone(),
            error: None,
            tab,
            expanded_widgets: HashSet::new(),
            identity_scroll: ScrollHandle::new(),
            tab_scroll: ScrollHandle::new(),
            note,
            saved_note: None,
            _note_task: note_task,
        });
        cx.notify();

        match cached {
            Some(profile) => self.load_widget_games(&profile, cx),
            None => self.load_profile(user_id, guild_id, cx),
        }
    }

    /// Saves the note if it was changed; Discord's modal does so on blur, and
    /// closing is the last chance.
    pub(in crate::screens::home) fn close_profile_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.profile_modal.take() else {
            return;
        };
        cx.notify();
        let note = modal.note.read(cx).value().trim().to_owned();
        if modal.saved_note.is_none_or(|saved| saved.trim() == note) {
            return;
        }
        let user_id = modal.user_id;
        cx.spawn(async move |_, _| {
            if let Err(err) = discord::save_user_note(user_id, note).await {
                eprintln!("failed to save note: {err}");
            }
        })
        .detach();
    }

    /// Widget games are only ids; their names and art come from a separate
    /// lookup, made once per game.
    fn load_widget_games(&mut self, profile: &discord::UserProfile, cx: &mut Context<Self>) {
        let wanted: Vec<u64> = profile
            .widgets
            .iter()
            .flat_map(|widget| widget.games.iter().map(|game| game.app_id))
            .filter(|id| self.requested_games.insert(*id))
            .collect();
        if wanted.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_games(wanted.clone()).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(games) => {
                    this.games
                        .extend(games.into_iter().map(|game| (game.id, game)));
                    cx.notify();
                }
                Err(err) => {
                    eprintln!("failed to load games: {err}");
                    // Worth asking again the next time a profile shows them.
                    for id in wanted {
                        this.requested_games.remove(&id);
                    }
                }
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn select_profile_tab(
        &mut self,
        tab: ProfileTab,
        cx: &mut Context<Self>,
    ) {
        if let Some(modal) = &mut self.profile_modal
            && modal.tab != Some(tab)
        {
            modal.tab = Some(tab);
            modal.tab_scroll.set_offset(Point::default());
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn toggle_profile_widget(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(modal) = &mut self.profile_modal {
            if !modal.expanded_widgets.remove(&index) {
                modal.expanded_widgets.insert(index);
            }
            cx.notify();
        }
    }

    /// The modal's Message button: opens their DM, creating it if needed.
    pub(in crate::screens::home) fn message_user(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_profile_modal(cx);
        self.with_dm(user_id, window, cx, |this, channel_id, window, cx| {
            this.open_dm(channel_id, window, cx);
        });
    }

    /// Runs `then` with the DM channel shared with `user_id`, creating it first
    /// if this is the first conversation between the two.
    fn with_dm(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Id<ChannelMarker>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let existing = self
            .dms
            .list
            .iter()
            .find(|dm| dm.recipient_id == Some(user_id))
            .map(|dm| dm.id);
        if let Some(channel_id) = existing {
            then(self, channel_id, window, cx);
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
                    then(this, channel_id, window, cx);
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

    /// Sends `text` to the user's DM, opening it as Discord does.
    fn message_from_profile(
        &mut self,
        user_id: Id<UserMarker>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_profile(cx);
        self.with_dm(user_id, window, cx, move |this, channel_id, window, cx| {
            this.send_to_dm(channel_id, text, window, cx);
        });
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
