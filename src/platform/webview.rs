//! Shared setup for the app's embedded webviews: the login page and activities.

/// WebView2 prompts for each permission a page asks for (camera, microphone,
/// notifications…). Nothing the app shows needs any of them, so they're refused
/// before a prompt shows.
#[cfg(target_os = "windows")]
pub fn deny_permissions(webview: &wry::WebView) {
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
        eprintln!("failed to block a webview's permission prompts: {err}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn deny_permissions(_webview: &wry::WebView) {}
