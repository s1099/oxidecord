//! Keeping the known threads current from the gateway, opening one as the
//! conversation, the threads popout's lists, and starting, joining, leaving
//! and closing threads.

use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{self, Membership, Thread};
use crate::screens::home::threads::{ThreadDraft, default_thread_name};
use crate::screens::home::{HomeScreen, ThreadsPopup, View};

/// Typing in the threads popout's search waits this long for a pause before asking.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Discord's epoch, 2015-01-01, in Unix milliseconds.
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// The lowest snowflake minted at `secs`.
fn snowflake_at(secs: i64) -> Option<Id<MessageMarker>> {
    u64::try_from(secs * 1000 - DISCORD_EPOCH_MS)
        .ok()
        .and_then(|ms| Id::new_checked(ms << 22))
}

impl HomeScreen {
    fn thread_parent(&self, thread: &Thread) -> Option<&discord::Channel> {
        self.guild_data
            .channels
            .get(&thread.guild_id?)?
            .iter()
            .find(|channel| Some(channel.id) == thread.parent_id)
    }

    pub(in crate::screens::home) fn store_thread(&mut self, thread: Thread) {
        let id = thread.id;
        let parent = self.thread_parent(&thread).cloned();
        self.threads.upsert(thread, parent.as_ref());
        self.sync_thread_unread(id);
    }

    /// An active thread the user is in counts toward unread marks; any other doesn't.
    fn sync_thread_unread(&mut self, id: Id<ChannelMarker>) {
        let Some(thread) = self.threads.thread(id) else {
            self.unreads.forget_thread(id);
            return;
        };
        match (thread.guild_id, thread.parent_id, thread.membership) {
            (Some(guild_id), Some(parent_id), Membership::Joined(at)) if !thread.archived => {
                let last = thread.last_message_id;
                self.unreads
                    .set_thread(guild_id, id, parent_id, snowflake_at(at));
                if let Some(last) = last {
                    self.unreads.note_latest(id, last);
                }
            }
            _ => self.unreads.forget_thread(id),
        }
    }

    pub(in crate::screens::home) fn handle_thread_list(
        &mut self,
        guild_id: Id<GuildMarker>,
        channels: Option<Vec<Id<ChannelMarker>>>,
        threads: Vec<Thread>,
        latest: Vec<(Id<ChannelMarker>, discord::Message)>,
        cx: &mut Context<Self>,
    ) {
        let stale = self.threads.stale_after_sync(
            guild_id,
            channels.as_deref(),
            &threads,
            self.selected_channel,
        );
        for id in stale {
            self.threads.remove(id);
            self.unreads.forget_thread(id);
        }
        for thread in threads {
            self.store_thread(thread);
        }
        for (thread_id, message) in latest {
            self.threads
                .latest
                .insert(thread_id, discord::MessageReference::quoting(&message));
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn handle_thread_update(
        &mut self,
        thread: Thread,
        cx: &mut Context<Self>,
    ) {
        self.store_thread(thread);
        cx.notify();
    }

    pub(in crate::screens::home) fn handle_thread_delete(
        &mut self,
        thread_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = self.threads.remove(thread_id);
        self.unreads.forget_thread(thread_id);
        self.history_cache.take(thread_id);
        // Open on it, the conversation goes back to where it branched from.
        if self.selected_channel == Some(thread_id)
            && let Some(parent_id) = removed.and_then(|known| known.thread.parent_id)
        {
            self.select_channel(parent_id, window, cx);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn handle_thread_members(
        &mut self,
        thread_id: Id<ChannelMarker>,
        member_count: Option<u32>,
        membership: Membership,
        cx: &mut Context<Self>,
    ) {
        let Some(mut thread) = self.threads.thread(thread_id).cloned() else {
            return;
        };
        if let Some(count) = member_count {
            thread.member_count = count;
        }
        if membership != Membership::Unknown {
            thread.membership = membership;
        }
        self.store_thread(thread);
        cx.notify();
    }

    /// A message arrived; in a thread, its count, newest message and preview move on.
    /// Returns whether it was in one.
    pub(in crate::screens::home) fn note_thread_message(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message: &discord::Message,
    ) -> bool {
        let in_thread = self.threads.note_message(channel_id, message);
        if in_thread {
            // Sending to an archived thread reopens it, and it counts again.
            self.sync_thread_unread(channel_id);
        }
        in_thread
    }

    /// Opens a thread as the conversation, in its guild. One the client hasn't
    /// been sent (archived, or behind a link) is fetched first.
    pub(in crate::screens::home) fn open_thread(
        &mut self,
        thread_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_thread_or(thread_id, None, window, cx);
    }

    /// As [`Self::open_thread`], but if it can't be opened `fallback` is opened in
    /// the browser instead.
    pub(in crate::screens::home) fn open_thread_or(
        &mut self,
        thread_id: Id<ChannelMarker>,
        fallback: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(known) = self.threads.get(thread_id) else {
            cx.spawn_in(window, async move |this, cx| {
                let result = discord::fetch_thread(thread_id).await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(Some(thread)) => {
                        this.store_thread(thread);
                        this.open_thread(thread_id, window, cx);
                    }
                    Ok(None) | Err(_) => {
                        if let Some(url) = &fallback {
                            cx.open_url(url);
                        }
                    }
                });
            })
            .detach();
            return;
        };
        let guild_id = known.thread.guild_id;
        self.close_threads_popup(cx);
        self.thread_draft = None;
        match guild_id {
            Some(guild_id) if self.view != View::Guild || self.selected_guild != Some(guild_id) => {
                // Opening the guild reopens its last channel.
                self.last_guild_channel.insert(guild_id, thread_id);
                self.select_guild(guild_id, window, cx);
                if self.selected_channel != Some(thread_id)
                    && self.channel_info(thread_id).is_some()
                {
                    self.select_channel(thread_id, window, cx);
                }
            }
            _ => self.select_channel(thread_id, window, cx),
        }
        cx.notify();
    }

    /// Off a message in the open channel, as Discord allows: a sent, ordinary
    /// message with no thread yet, where the user may start public threads.
    pub(in crate::screens::home) fn can_start_thread(&self, message: &discord::Message) -> bool {
        self.view == View::Guild
            && message.system.is_none()
            && !message.ephemeral
            && message.delivery == discord::Delivery::Sent
            && message.thread.is_none()
            && self.threads.get(message.id.cast()).is_none()
            && self.selected_channel_info().is_some_and(|channel| {
                channel.kind.has_threads() && channel.can_create_public_threads
            })
    }

    /// The thread open as the conversation, if one is.
    pub(in crate::screens::home) fn selected_thread(&self) -> Option<&Thread> {
        if self.view != View::Guild {
            return None;
        }
        self.threads.thread(self.selected_channel?)
    }

    /// Joining is optimistic; Discord's member update confirms it.
    pub(in crate::screens::home) fn set_thread_joined(
        &mut self,
        thread_id: Id<ChannelMarker>,
        joined: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(previous) = self
            .threads
            .thread(thread_id)
            .map(|thread| thread.membership)
        else {
            return;
        };
        let membership = if joined {
            Membership::Joined(now_secs())
        } else {
            Membership::NotJoined
        };
        self.threads.set_membership(thread_id, membership);
        self.sync_thread_unread(thread_id);
        cx.notify();

        cx.spawn(async move |this, cx| {
            if let Err(err) = discord::set_thread_joined(thread_id, joined).await {
                eprintln!("joining or leaving a thread failed: {err}");
                let _ = this.update(cx, |this, cx| {
                    this.threads.set_membership(thread_id, previous);
                    this.sync_thread_unread(thread_id);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Closing or reopening; the thread update that follows confirms it.
    pub(in crate::screens::home) fn set_thread_archived(
        &mut self,
        thread_id: Id<ChannelMarker>,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |_, cx| {
            if let Err(err) = discord::set_thread_archived(thread_id, archived).await {
                let _ = cx.update(|window, cx| {
                    let title = if archived {
                        "Couldn't close the thread"
                    } else {
                        "Couldn't reopen the thread"
                    };
                    crate::ui::dialogs::show_error(title, [err], window, cx);
                });
            }
        })
        .detach();
    }

    /// The channel whose threads the header's button lists: the open channel, or
    /// the open thread's parent.
    pub(in crate::screens::home) fn threads_parent(&self) -> Option<&discord::Channel> {
        let channel = self.selected_channel_info()?;
        if channel.kind == discord::ChannelKind::Thread {
            self.channel_info(channel.parent_id?)
        } else {
            channel.kind.has_threads().then_some(channel)
        }
    }

    pub(in crate::screens::home) fn toggle_threads(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.threads_popup.is_some() {
            return self.close_threads_popup(cx);
        }
        let Some(channel_id) = self.threads_parent().map(|channel| channel.id) else {
            return;
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search for Thread Name"));
        let search_changed = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.load_threads_page(false, cx);
            }
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        self.threads_popup = Some(ThreadsPopup {
            channel_id,
            position,
            search,
            query: String::new(),
            results: Vec::new(),
            archived: Vec::new(),
            loading: false,
            has_more: false,
            error: None,
            scroll: ScrollHandle::new(),
            _task: None,
            _search_changed: search_changed,
        });
        self.load_threads_page(false, cx);
        self.refresh_active_threads(channel_id, cx);
        cx.notify();
    }

    pub(in crate::screens::home) fn close_threads_popup(&mut self, cx: &mut Context<Self>) {
        if self.threads_popup.take().is_some() {
            cx.notify();
        }
    }

    /// The active threads a sync didn't bring: one only comes for channels the
    /// session is subscribed to, so this asks search too.
    fn refresh_active_threads(&mut self, channel_id: Id<ChannelMarker>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let search = discord::ThreadSearch {
                archived: Some(false),
                name: None,
                offset: 0,
            };
            let Ok(page) = discord::search_threads(channel_id, search).await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                for thread in page.threads {
                    this.store_thread(thread);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// With an empty search, the archived threads; otherwise threads matching it,
    /// active or not. `more` pages on from what's listed.
    pub(in crate::screens::home) fn load_threads_page(
        &mut self,
        more: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(popup) = &mut self.threads_popup else {
            return;
        };
        let query = popup.search.read(cx).value().trim().to_string();
        if more && (popup.loading || !popup.has_more || query != popup.query) {
            return;
        }
        // Typing a query the lists already show (back to empty, say) changes nothing.
        if !more && query == popup.query && (popup.loading || popup._task.is_some()) {
            return;
        }
        let offset = match (more, query.is_empty()) {
            (false, _) => 0,
            (true, true) => popup.archived.len(),
            (true, false) => popup.results.len(),
        };
        let debounce = !more && !query.is_empty();
        popup.query = query.clone();
        popup.loading = true;
        popup.error = None;
        if !more {
            popup.results.clear();
            popup.archived.clear();
            popup.has_more = false;
        }
        let channel_id = popup.channel_id;
        popup._task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            }
            let search = discord::ThreadSearch {
                archived: query.is_empty().then_some(true),
                name: (!query.is_empty()).then_some(query.clone()),
                offset,
            };
            let result = discord::search_threads(channel_id, search).await;
            let _ = this.update(cx, |this, cx| {
                let Some(popup) = this
                    .threads_popup
                    .as_mut()
                    .filter(|popup| popup.channel_id == channel_id && popup.query == query)
                else {
                    return;
                };
                popup.loading = false;
                let page = match result {
                    Ok(page) => page,
                    Err(err) => {
                        popup.error = Some(err);
                        cx.notify();
                        return;
                    }
                };
                popup.has_more = page.has_more;
                let ids: Vec<_> = page.threads.iter().map(|thread| thread.id).collect();
                if query.is_empty() {
                    popup.archived.extend(ids);
                } else {
                    popup.results.extend(ids);
                }
                for thread in page.threads {
                    this.store_thread(thread);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Opens the "New Thread" panel, off `starter` or on its own. A message that
    /// already has a thread opens that instead.
    pub(in crate::screens::home) fn open_thread_draft(
        &mut self,
        parent_id: Id<ChannelMarker>,
        starter: Option<Id<MessageMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let starter = starter.and_then(|id| {
            self.messages
                .iter()
                .find(|message| message.id == id)
                .cloned()
        });
        if let Some(message) = &starter {
            let thread_id = message.id.cast();
            if message.thread.is_some() || self.threads.get(thread_id).is_some() {
                return self.open_thread(thread_id, window, cx);
            }
        }
        // Started from inside a thread, the panel goes beside its parent.
        if self.selected_channel != Some(parent_id) {
            self.select_channel(parent_id, window, cx);
        }
        let placeholder = starter
            .as_ref()
            .map_or_else(|| "New Thread".to_string(), default_thread_name);
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let message = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Enter a message to start the conversation!")
        });
        let submit = |this: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_thread_draft(window, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name, window, submit),
            cx.subscribe_in(&message, window, submit),
        ];
        name.update(cx, |input, cx| input.focus(window, cx));
        self.close_threads_popup(cx);
        self.thread_draft = Some(ThreadDraft {
            parent_id,
            starter,
            name,
            message,
            private: false,
            creating: false,
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub(in crate::screens::home) fn close_thread_draft(&mut self, cx: &mut Context<Self>) {
        if self.thread_draft.take().is_some() {
            cx.notify();
        }
    }

    /// Creates the thread, sends its opening message, and opens it.
    pub(in crate::screens::home) fn submit_thread_draft(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &mut self.thread_draft else {
            return;
        };
        if draft.creating {
            return;
        }
        let typed = draft.name.read(cx).value().trim().to_string();
        let name = match &draft.starter {
            _ if !typed.is_empty() => typed,
            Some(starter) => default_thread_name(starter),
            None => {
                draft.error = Some("Give the thread a name.".into());
                cx.notify();
                return;
            }
        };
        let text = draft.message.read(cx).value().trim().to_string();
        if draft.starter.is_none() && text.is_empty() {
            draft.error = Some("Write a message to start the thread with.".into());
            cx.notify();
            return;
        }
        draft.creating = true;
        draft.error = None;
        let parent_id = draft.parent_id;
        let starter = draft.starter.as_ref().map(|message| message.id);
        let private = draft.private;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let created = match starter {
                Some(message_id) => {
                    discord::start_thread_from_message(parent_id, message_id, name).await
                }
                None => discord::start_thread(parent_id, name, private).await,
            };
            let sent = match &created {
                Ok(thread) if !text.is_empty() => {
                    let (nonce, _) = crate::screens::home::data::messages::next_nonce();
                    discord::send_message(
                        thread.id,
                        nonce,
                        discord::MessageDraft::text(text),
                        Vec::new(),
                        discord::UploadProgress::new(0),
                    )
                    .await
                    .map(|_| ())
                }
                _ => Ok(()),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let thread = match created {
                    Ok(thread) => thread,
                    Err(err) => {
                        if let Some(draft) = &mut this.thread_draft {
                            draft.creating = false;
                            draft.error = Some(err);
                        }
                        cx.notify();
                        return;
                    }
                };
                if let Err(err) = sent {
                    eprintln!("sending a new thread's first message failed: {err}");
                }
                let thread_id = thread.id;
                this.store_thread(thread);
                this.thread_draft = None;
                this.open_thread(thread_id, window, cx);
            });
        })
        .detach();
    }
}
