use std::sync::{Arc, Mutex};
use twilight_http::Client as HttpClient;
use twilight_model::id::marker::GuildMarker;
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

pub struct AppState {
    pub current_view: View,
    pub token: Option<String>,
    pub http_client: Option<Arc<HttpClient>>,
    pub guilds: Vec<GuildInfo>,
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
}
