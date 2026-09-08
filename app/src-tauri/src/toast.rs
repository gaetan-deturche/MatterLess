//! Windows toasts, raised from Rust so that clicking one reaches the app.
//!
//! `tauri-plugin-notification` cannot do this. Its Windows path ends in
//! `let _ = notification.show()` -- the result is discarded and no activation
//! handler is ever registered -- so a click had nowhere to go, whatever the
//! toast was branded as. The same plugin also skips setting an AppUserModelID
//! for anything running out of `target/debug` or `target/release`, which is why
//! a dev build's toasts claim to come from PowerShell.
//!
//! Both are fixed by building the toast here: `on_activated` is an **in-process**
//! event, which is exactly the case that matters -- MatterLess is running in the
//! background when a toast fires, so no COM activator is needed. (A registered
//! activator, with CLSID keys an installer has to write, is only needed to
//! *launch* an app that is not running at all.)

use tauri::{AppHandle, Manager};
use tauri_winrt_notification::{Duration, Toast};

/// Raises a toast and focuses the window if it is clicked.
///
/// `channel_id` is what the click should open, carried through the callback so a
/// notification lands the reader in the conversation it came from rather than
/// wherever they were last.
pub fn raise(
    app: &AppHandle,
    channel_id: &str,
    title: &str,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.clone();
    let target = channel_id.to_string();

    Toast::new(&app_id(app))
        .title(title)
        .text1(body)
        // Short: a chat message is worth a glance, not a quarter of a minute of
        // screen real estate.
        .duration(Duration::Short)
        .on_activated(move |_action| {
            focus(&handle, &target);
            Ok(())
        })
        .show()?;
    Ok(())
}

/// Brings the window forward and tells the shell which channel to open.
fn focus(app: &AppHandle, channel_id: &str) {
    if let Some(window) = app.get_webview_window("main") {
        // Unminimise first: a minimised window ignores `set_focus`.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        // The click *is* the reader looking at it, so stop asking for attention.
        let _ = window.request_user_attention(None);
    }
    // The shell owns navigation, so it is asked rather than driven.
    if let Err(error) = app.emit_to_shell(channel_id) {
        tracing::warn!(%error, "could not route the toast click");
    }
    tracing::info!(channel = %channel_id, "toast clicked");
}

/// The AppUserModelID to raise the toast under.
///
/// WinRT refuses to create a notifier for an id with no Start-menu shortcut
/// behind it, and a dev build has none -- so it borrows PowerShell's, which is
/// the documented workaround and the reason dev toasts are attributed to it. An
/// installed build has its own, written by the NSIS installer.
fn app_id(app: &AppHandle) -> String {
    let installed = tauri::utils::platform::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .map(|dir| {
            let path = dir.display().to_string();
            !(path.ends_with("target\\debug") || path.ends_with("target\\release"))
        })
        .unwrap_or(false);

    if installed {
        app.config().identifier.clone()
    } else {
        Toast::POWERSHELL_APP_ID.to_string()
    }
}

/// How a toast click reaches the shell.
trait ShellRouting {
    fn emit_to_shell(&self, channel_id: &str) -> tauri::Result<()>;
}

impl ShellRouting for AppHandle {
    fn emit_to_shell(&self, channel_id: &str) -> tauri::Result<()> {
        use tauri::Emitter;
        self.emit("toast-clicked", channel_id)
    }
}
