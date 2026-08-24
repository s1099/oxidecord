//! The Discord login window: a webview that reports the token back once
//! Discord's own page stores it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::webview::WebView;
use gpui_component::wry::WebViewBuilder;

use crate::discord;
use crate::screens::app::AppScreen;

/// Injected into Discord's login page so it reports the token the moment the
/// page writes it to `localStorage`, which is the only place it appears.
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
            // Give GPUI a moment to finish creating the OS window the webview
            // gets parented to.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;

            // The webview has to be built on the main thread, inside the
            // window's context.
            let result = gpui::AsyncApp::update(cx, |cx| {
                cx.update_window(window_handle, |_, window, cx| {
                    let window_handle_capture = window.window_handle();
                    let async_cx_capture = cx.to_async();
                    let app_capture = app.clone();

                    let builder = WebViewBuilder::new()
                        .with_url("https://discord.com/login")
                        .with_initialization_script(TOKEN_HOOK_SCRIPT)
                        .with_ipc_handler(move |msg| {
                            let token = msg.body().trim_matches('"');
                            if let Err(err) = discord::save_token(token) {
                                eprintln!("failed to save token to the credential store: {err}");
                            }

                            let window_handle_inner = window_handle_capture;
                            let async_cx_inner = async_cx_capture.clone();
                            let app_inner = app_capture.clone();

                            let _ = async_cx_inner.update(move |cx| {
                                let _ = cx.update_window(main_window, |_, window, cx| {
                                    let _ =
                                        app_inner.update(cx, |app, cx| app.show_home(window, cx));
                                });
                                let _ = cx.update_window(window_handle_inner, |_, window, _| {
                                    window.remove_window();
                                });
                            });
                        });

                    let wry_webview = builder.build(window).expect("Failed to build webview");

                    cx.new(|cx| {
                        let mut webview = WebView::new(wry_webview, window, cx);
                        webview.show();
                        webview
                    })
                })
            });

            if let Ok(Ok(webview)) = result {
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
