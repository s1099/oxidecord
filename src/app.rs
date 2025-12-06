use std::sync::{Arc, Mutex};
use twilight_http::Client as HttpClient;
use twilight_model::id::marker::{GuildMarker, ChannelMarker};
use tokio::runtime::Handle;
use std::sync::OnceLock;

static RUNTIME_HANDLE: OnceLock<Handle> = OnceLock::new();

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

    pub fn init_runtime() {
        RUNTIME_HANDLE.get_or_init(|| {
            // Try to get the current handle first
            Handle::try_current().unwrap_or_else(|_| {
                // If no runtime exists, create one in a background thread
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new()
                        .expect("Failed to create Tokio runtime");
                    let handle = rt.handle().clone();
                    tx.send(handle.clone()).unwrap();
                    rt.block_on(async {
                        loop {
                            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                        }
                    });
                });
                rx.recv().unwrap()
            })
        });
    }

    pub fn login(state: Arc<Mutex<AppState>>, token: String) {
        {
            let mut app = state.lock().unwrap();
            app.token = Some(token.clone());
            app.loading = true;
            app.error = None; // Clear any previous errors
        }
        
        let state_clone = state.clone();
        let handle = RUNTIME_HANDLE.get().expect("Tokio runtime not initialized. Call AppState::init_runtime() first.");
        
        // Create the HTTP client synchronously within the runtime context
        // HttpClient::new needs Handle::try_current() to work, so we use block_on
        // This ensures we're in the runtime context when creating the client
        let http_client_result = handle.block_on(async {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                HttpClient::new(token.clone())
            }))
        });
        
        let http_client = match http_client_result {
            Ok(client) => Arc::new(client),
            Err(_) => {
                if let Ok(mut app) = state_clone.lock() {
                    app.loading = false;
                    app.error = Some("Failed to create HTTP client. Tokio runtime error. Please ensure the application is properly initialized.".to_string());
                }
                return;
            }
        };
        
        // Store the client
        {
            let mut app = state_clone.lock().unwrap();
            app.http_client = Some(http_client.clone());
        }
        
        // Spawn the async task to fetch guilds
        handle.spawn(async move {
            // Fetch current user guilds
            match http_client.current_user_guilds().await {
                Ok(response) => {
                    match response.models().await {
                        Ok(guilds) => {
                            if let Ok(mut state) = state_clone.lock() {
                                // Convert to GuildInfo
                                state.guilds = guilds.into_iter().map(|g| GuildInfo {
                                    id: g.id,
                                    name: g.name,
                                }).collect();
                                state.current_view = View::Channel;
                                state.loading = false;
                                state.error = None;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error parsing guilds: {:?}", e);
                            if let Ok(mut state) = state_clone.lock() {
                                state.loading = false;
                                state.error = Some(format!("Error parsing guilds: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error fetching guilds: {:?}", e);
                    if let Ok(mut state) = state_clone.lock() {
                        state.loading = false;
                        state.error = Some(format!("Error fetching guilds: {}", e));
                    }
                }
            }
        });
    }

    pub fn fetch_channels(state: Arc<Mutex<AppState>>, guild_id: twilight_model::id::Id<GuildMarker>) {
        {
            let mut app = state.lock().unwrap();
            app.selected_guild = Some(guild_id);
            app.loading = true;
            app.error = None;
        }

        let state_clone = state.clone();
        let handle = RUNTIME_HANDLE.get().expect("Tokio runtime not initialized. Call AppState::init_runtime() first.");

        let http_client = {
            let app = state_clone.lock().unwrap();
            app.http_client.clone()
        };

        if let Some(client) = http_client {
            handle.spawn(async move {
                match client.guild_channels(guild_id).await {
                    Ok(response) => {
                        match response.models().await {
                            Ok(channels) => {
                                if let Ok(mut state) = state_clone.lock() {
                                    // Convert to ChannelInfo, filtering for text channels
                                    state.channels = channels
                                        .into_iter()
                                        .filter_map(|ch| {
                                            // Only include text channels (type 0)
                                            if ch.kind == twilight_model::channel::ChannelType::GuildText {
                                                Some(ChannelInfo {
                                                    id: ch.id,
                                                    name: ch.name.unwrap_or_else(|| "Unnamed".to_string()),
                                                })
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    state.loading = false;
                                    state.error = None;
                                }
                            }
                            Err(e) => {
                                eprintln!("Error parsing channels: {:?}", e);
                                if let Ok(mut state) = state_clone.lock() {
                                    state.loading = false;
                                    state.error = Some(format!("Error parsing channels: {}", e));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error fetching channels: {:?}", e);
                        if let Ok(mut state) = state_clone.lock() {
                            state.loading = false;
                            state.error = Some(format!("Error fetching channels: {}", e));
                        }
                    }
                }
            });
        } else {
            if let Ok(mut app) = state_clone.lock() {
                app.loading = false;
                app.error = Some("HTTP client not available".to_string());
            }
        }
    }

    pub fn fetch_messages(state: Arc<Mutex<AppState>>, channel_id: twilight_model::id::Id<ChannelMarker>) {
        {
            let mut app = state.lock().unwrap();
            app.selected_channel = Some(channel_id);
            app.messages.clear(); // Clear old messages
            app.loading = true;
            app.error = None;
        }

        let state_clone = state.clone();
        let handle = RUNTIME_HANDLE.get().expect("Tokio runtime not initialized. Call AppState::init_runtime() first.");

        let http_client = {
            let app = state_clone.lock().unwrap();
            app.http_client.clone()
        };

        if let Some(client) = http_client {
            handle.spawn(async move {
                match client.channel_messages(channel_id).limit(50).await {
                    Ok(response) => {
                        match response.models().await {
                            Ok(messages) => {
                                if let Ok(mut state) = state_clone.lock() {
                                    // Convert to MessageInfo
                                    state.messages = messages
                                        .into_iter()
                                        .map(|msg| MessageInfo {
                                            id: msg.id,
                                            content: msg.content,
                                            author_name: if msg.author.discriminator > 0 {
                                                format!("{}#{:04}", msg.author.name, msg.author.discriminator)
                                            } else {
                                                msg.author.name.clone()
                                            },
                                            author_id: msg.author.id,
                                            timestamp: format!("{}", msg.timestamp.as_secs()),
                                        })
                                        .collect();
                                    state.loading = false;
                                    state.error = None;
                                }
                            }
                            Err(e) => {
                                eprintln!("Error parsing messages: {:?}", e);
                                if let Ok(mut state) = state_clone.lock() {
                                    state.loading = false;
                                    state.error = Some(format!("Error parsing messages: {}", e));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error fetching messages: {:?}", e);
                        if let Ok(mut state) = state_clone.lock() {
                            state.loading = false;
                            state.error = Some(format!("Error fetching messages: {}", e));
                        }
                    }
                }
            });
        } else {
            if let Ok(mut app) = state_clone.lock() {
                app.loading = false;
                app.error = Some("HTTP client not available".to_string());
            }
        }
    }
}
