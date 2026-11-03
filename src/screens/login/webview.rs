//! The Discord login window: a webview that reports the token back once
//! Discord's own page stores it.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_wry::WebView;
use wry::WebViewBuilder;

use crate::discord;
use crate::screens::app::AppScreen;

/// Injected into Discord's login page; the token only ever appears in `localStorage`.
const TOKEN_HOOK_SCRIPT: &str = r#"
    (function() {
        const storage = window.localStorage;
        const customSetItem = function(key, value) {
            storage.setItem(key, value);
            if (key === 'token') {
                window.ipc.postMessage(value);
            }
        };
        try {
            const newStorage = new Proxy(storage, {
                get(target, prop, receiver) {
                    if (prop === 'setItem') {
                        return customSetItem;
                    }
                    const val = Reflect.get(target, prop, receiver);
                    if (typeof val === 'function') {
                        return val.bind(target);
                    }
                    return val;
                }
            });
            Object.defineProperty(window, 'localStorage', {
                value: newStorage,
                writable: true,
                configurable: true
            });
        } catch (e) {
            const originalSet = storage.setItem;
            storage.setItem = function(key, value) {
                originalSet.apply(this, arguments);
                if (key === 'token') {
                    window.ipc.postMessage(value);
                }
            };
        }
    })();
"#;

/// Discord's page asks for the camera among other things, and WebView2 prompts for each.
/// Signing in needs none of them, so they're refused before a prompt shows.
#[cfg(target_os = "windows")]
fn deny_permissions(webview: &wry::WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_STATE_DENY;
    use webview2_com::PermissionRequestedEventHandler;
    use wry::WebViewExtWindows as _;

    let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
        if let Some(args) = args {
            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)? };
        }
        Ok(())
    }));
    let mut token = Default::default();
    if let Err(err) = unsafe {
        webview
            .webview()
            .add_PermissionRequested(&handler, &mut token)
    } {
        eprintln!("failed to block the login page's permission prompts: {err}");
    }
}

pub struct LoginWebview {
    webview: Option<Entity<WebView>>,
}

impl LoginWebview {
    pub fn new(
        app: WeakEntity<AppScreen>,
        main_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let window_handle = window.window_handle();

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            // Lets GPUI finish creating the OS window the webview parents to.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;

            // Must be built on the main thread, inside the window's context.
            let result = gpui_kit::AsyncApp::update(cx, |cx| {
                cx.update_window(window_handle, |_, window, cx| {
                    let window_handle_capture = window.window_handle();
                    let async_cx_capture = cx.to_async();
                    let app_capture = app.clone();

                    // Incognito so a session left from the last login doesn't sign the
                    // same account straight back in when adding another.
                    let builder = WebViewBuilder::new()
                        .with_incognito(true)
                        .with_url("https://discord.com/login")
                        .with_initialization_script(TOKEN_HOOK_SCRIPT)
                        .with_ipc_handler(move |msg| {
                            let token = msg.body().trim_matches('"');
                            let saved = discord::save_token(token);
                            if let Err(err) = &saved {
                                eprintln!("failed to save token to the credential store: {err}");
                            }

                            let window_handle_inner = window_handle_capture;
                            let async_cx_inner = async_cx_capture.clone();
                            let app_inner = app_capture.clone();

                            async_cx_inner.update(move |cx| {
                                if saved.is_ok() {
                                    let _ = cx.update_window(main_window, |_, window, cx| {
                                        let _ = app_inner
                                            .update(cx, |app, cx| app.show_home(window, cx));
                                    });
                                }
                                let _ = cx.update_window(window_handle_inner, |_, window, _| {
                                    window.remove_window();
                                });
                            });
                        });

                    let wry_webview = builder.build(window).expect("Failed to build webview");
                    #[cfg(target_os = "windows")]
                    deny_permissions(&wry_webview);

                    cx.new(|cx| {
                        let mut webview = WebView::new(wry_webview, window, cx);
                        webview.show();
                        webview
                    })
                })
            });

            if let Ok(webview) = result {
                let _ = this.update(cx, |this, cx| {
                    this.webview = Some(webview);
                    cx.notify();
                });
            }
        })
        .detach();

        Self { webview: None }
    }
}

impl Render for LoginWebview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .when_some(self.webview.clone(), |this, webview| this.child(webview))
    }
}
