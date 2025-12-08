use std::sync::{Arc, Mutex};
use twilight_http::Client as HttpClient;
use twilight_model::id::marker::{GuildMarker, ChannelMarker};
use crate::app::{AppState, View, GuildInfo, ChannelInfo, MessageInfo, AttachmentInfo};
use crate::utils::get_runtime_handle;

pub struct DiscordService;

impl DiscordService {
    pub fn login(state: Arc<Mutex<AppState>>, token: String) {
        {
            let mut app = state.lock().unwrap();
            app.token = Some(token.clone());
            app.loading = true;
            app.error = None;
        }
        
        let state_clone = state.clone();
        let handle = get_runtime_handle();
        
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
        
        {
            let mut app = state_clone.lock().unwrap();
            app.http_client = Some(http_client.clone());
        }
        
        handle.spawn(async move {
            match http_client.current_user_guilds().await {
                Ok(response) => {
                    match response.models().await {
                        Ok(guilds) => {
                            if let Ok(mut state) = state_clone.lock() {
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
        let handle = get_runtime_handle();

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
                                let filtered_channels: Vec<ChannelInfo> = channels
                                    .into_iter()
                                    .filter_map(|ch| {
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

                                if let Ok(mut state) = state_clone.lock() {
                                    state.channels = filtered_channels;
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
        let handle = get_runtime_handle();

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
                                                    state.messages = messages
                                                        .into_iter()
                                                        .rev() // Reverse to show oldest first
                                                        .map(|msg| {
                                                            let avatar_url = msg.author.avatar.map(|hash| {
                                                                format!(
                                                                    "https://cdn.discordapp.com/avatars/{}/{}.png",
                                                                    msg.author.id,
                                                                    hash
                                                                )
                                                            });
                                                            let attachments: Vec<AttachmentInfo> = msg.attachments
                                                                .into_iter()
                                                                .map(|att| AttachmentInfo {
                                                                    url: att.url,
                                                                    filename: att.filename,
                                                                    content_type: att.content_type,
                                                                    width: att.width,
                                                                    height: att.height,
                                                                })
                                                                .collect();
                                                            MessageInfo {
                                                                id: msg.id,
                                                                content: msg.content,
                                                                author_name: if msg.author.discriminator > 0 {
                                                                    format!("{}#{:04}", msg.author.name, msg.author.discriminator)
                                                                } else {
                                                                    msg.author.name.clone()
                                                                },
                                                                author_id: msg.author.id,
                                                                author_avatar_url: avatar_url,
                                                                timestamp: format!("{}", msg.timestamp.as_secs()),
                                                                attachments,
                                                            }
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

