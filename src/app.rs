use std::sync::Arc;
use twilight_http::Client as HttpClient;
use twilight_model::id::marker::{GuildMarker, ChannelMarker};

#[derive(Clone, PartialEq)]
pub enum View {
    Login,
    Servers,
    Channel,
}

#[derive(Clone)]
pub struct GuildInfo {
    pub id: twilight_model::id::Id<GuildMarker>,
    pub name: String,
}

#[derive(Clone)]
pub struct ChannelInfo {
    pub id: twilight_model::id::Id<ChannelMarker>,
    pub name: String,
}

#[derive(Clone)]
pub struct MessageInfo {
    pub id: twilight_model::id::Id<twilight_model::id::marker::MessageMarker>,
    pub content: String,
    pub author_name: String,
    pub author_id: twilight_model::id::Id<twilight_model::id::marker::UserMarker>,
    pub author_avatar_url: Option<String>,
    pub timestamp: String,
}

pub struct AppState {
    pub current_view: View,
    pub token: Option<String>,
    pub http_client: Option<Arc<HttpClient>>,
    pub guilds: Vec<GuildInfo>,
    pub selected_guild: Option<twilight_model::id::Id<GuildMarker>>,
    pub channels: Vec<ChannelInfo>,
    pub selected_channel: Option<twilight_model::id::Id<ChannelMarker>>,
    pub messages: Vec<MessageInfo>,
    pub loading: bool,
    pub error: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            current_view: View::Login,
            token: None,
            http_client: None,
            guilds: Vec::new(),
            selected_guild: None,
            channels: Vec::new(),
            selected_channel: None,
            messages: Vec::new(),
            loading: false,
            error: None,
        }
    }
}
