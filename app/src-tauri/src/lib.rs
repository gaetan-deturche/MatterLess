//! The Tauri shell.
//!
//! Two rules from the plan are enforced by the shape of this file rather than
//! by discipline:
//!
//! * **The UI only visualises the store.** A delta never carries content -- it
//!   names the channel that changed, and the frontend asks for that channel's
//!   row plan. One path in, one path out; a command result and a cached value
//!   cannot disagree because there is only ever one of them.
//! * **Nothing blocks the main thread.** Every command is `async` and every
//!   store call goes through `spawn_blocking`. A synchronous Tauri command runs
//!   on the main thread and freezes the webview until *all* invokes finish --
//!   the single trap that cost Auger the most.
//!
//! Mutable sync state (active channel, window focus) is owned by one engine
//! task rather than shared behind a lock, so a guard can never be held across
//! an await.

mod commands;
mod engine;
mod filecache;
mod media;
mod pending;
mod render_cache;
mod taskbar;
#[cfg(windows)]
mod toast;
mod uploads;

use matterless_core::RestClient;
use matterless_store::Store;
use matterless_sync::SyncEngine;
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::mpsc;

/// Where the client points until somebody says otherwise.
///
/// Deliberately not a real server. Which Mattermost this talks to is the
/// reader's business and is asked for at sign-in, so no host name is compiled
/// into the binary -- a client with one baked in is a client for one company.
const NO_SERVER: &str = "https://example.invalid";
const KEYRING_SERVICE: &str = "matterless";

/// The file holding the chosen server, beside the database and the log.
///
/// A plain file rather than a row in SQLite: it has to be readable *before* the
/// store is opened, since the server decides which account the store belongs
/// to, and it is not a secret -- the token is, and that lives in the keyring.
fn server_file(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join("server.txt")
}

/// The server this install is pointed at, if it has been told.
pub fn stored_server(directory: &std::path::Path) -> Option<String> {
    let held = std::fs::read_to_string(server_file(directory)).ok()?;
    let trimmed = held.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Remembers the server for next time.
pub fn store_server(directory: &std::path::Path, url: &str) -> std::io::Result<()> {
    std::fs::write(server_file(directory), url.trim())
}

pub struct AppState {
    /// Fetched images, keyed by URL including the version.
    pub media: Arc<media::MediaCache>,
    /// Attachments, on disk: a different size class from avatars, and worth
    /// keeping across restarts because a file never changes under its id.
    pub files: Arc<filecache::FileCache>,
    /// Whether the reader has collapsed threads on, resolved at bootstrap from
    /// the server default *and* the per-user preference.
    ///
    /// Held here because it decides which unread counters are the right ones,
    /// and every command that touches unread needs the same answer: passing it
    /// in from the shell would let two callers disagree.
    pub collapsed: Arc<std::sync::atomic::AtomicBool>,
    pub store: Arc<Store>,
    pub engine: Arc<SyncEngine>,
    pub rest: Arc<RestClient>,
    /// Optimistic sends, in memory only. Shared with the engine task, which
    /// clears an entry when the server echoes it back.
    pub pending: Arc<pending::PendingPosts>,
    /// Files uploaded but not yet claimed by a post.
    pub uploads: Arc<uploads::Uploads>,
    /// The server's `MaxFileSize`, read at bootstrap. Zero until then, which
    /// reads as "not known yet" rather than as "nothing is allowed".
    pub max_file_size: Arc<std::sync::atomic::AtomicI64>,
    /// Whether the websocket is up, shared with the engine task.
    pub connected: Arc<std::sync::atomic::AtomicBool>,
    /// The server's `EnableSVGs`, read at bootstrap. False until then, which is
    /// the safe way round: an SVG not drawn inline is a file card, while one
    /// drawn against the server's wishes is a document executing in the page.
    pub allow_svg: Arc<std::sync::atomic::AtomicBool>,
    /// When each channel's history was last reconciled with the server.
    ///
    /// Opening a channel used to fetch only when the local copy looked *thin*,
    /// which answers the wrong question: a channel can hold a full screen of
    /// rows and still be missing messages that arrived while the app was shut.
    /// One such hole was found in Town Square -- Friday 18:28 absent while
    /// 17:22 and Monday were both present -- and no timestamp could have caught
    /// it, because the missing post was older than the newest one held.
    pub reconciled: Arc<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>>,
    /// Parsed message bodies, shared by `Arc` so a cache hit costs a refcount
    /// bump rather than a decode and a tree copy.
    pub renders: Arc<render_cache::RenderCache>,
    /// Everything that mutates sync state goes through the engine task.
    pub to_engine: mpsc::Sender<engine::EngineMsg>,
}

/// The session token, from the OS keychain.
///
/// Phase 0 established there is no refresh path on this server, so this token is
/// the only thing standing between a restart and a login prompt -- and it is a
/// 30-day bearer credential, which is why it lives in the keychain rather than
/// in SQLite or a config file.
pub fn stored_token() -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, "session").ok()?;
    match entry.get_password() {
        Ok(token) if !token.is_empty() => Some(token),
        _ => migrate_minted_token(&entry),
    }
}

/// One-time import of the token `tools/mint_token.py` writes, so a dev machine
/// does not need a fresh login -- but it lands in the keychain, so the file is
/// not a permanent back door.
fn migrate_minted_token(entry: &keyring::Entry) -> Option<String> {
    for candidate in [
        "tools/.mm_token.json",
        "../../tools/.mm_token.json",
        "Claude/.mm_token.json",
        "../../Claude/.mm_token.json",
    ] {
        let Ok(raw) = std::fs::read_to_string(candidate) else {
            continue;
        };
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let token = parsed.get("token")?.as_str()?.to_string();
        if token.is_empty() {
            continue;
        }
        if let Err(error) = entry.set_password(&token) {
            tracing::warn!(%error, "could not move the token into the keychain");
        } else {
            tracing::info!("imported a minted token into the keychain");
        }
        return Some(token);
    }
    None
}

pub fn store_token(token: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, "session")?;
    entry.set_password(token)?;
    Ok(())
}

fn data_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// The log file lives beside the database, so the dev and installed builds keep
/// separate logs the same way they keep separate stores.
pub fn log_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    data_dir(app).join("matterless.log")
}

/// Sets up logging to a file as well as the console.
///
/// A parseable timeline beats a screenshot: it says what the shell asked for,
/// what came back and how long it took, which a picture of the window cannot.
/// **Message text is never logged** -- structure, ids, counts and timings only,
/// so the file stays safe to read and to attach to a report.
fn init_logging(app: &tauri::AppHandle) -> std::path::PathBuf {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let path = log_path(app);
    // The leading bare `info` is a **global default**, and it is the important
    // part: enumerating targets means any module not on the list is silently
    // dropped. That happened twice -- the whole frontend `ui` timeline, and then
    // the store's migration record -- each time looking like the code had not
    // run rather than like the log had swallowed it.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,matterless_app_lib=debug,ui=debug".into());

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);

    match file {
        Ok(file) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_target(true)
                        .with_writer(std::sync::Mutex::new(file)),
                )
                .init();
        }
        Err(error) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
            tracing::warn!(%error, path = %path.display(), "no log file; console only");
        }
    }
    path
}

/// Makes a hidden window visible without stealing focus or covering anything.
///
/// `WebviewWindow::show` maps to `ShowWindow(SW_SHOW)`, which activates the
/// window: the taskbar button flashes, the window jumps to the front and
/// whatever the reader was typing into loses focus. `SetWindowPos` with
/// `SWP_NOACTIVATE` and `HWND_BOTTOM` shows it in place, at the back.
#[cfg(windows)]
fn reveal_quietly(window: &tauri::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
    };

    let handle = match window.hwnd() {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(%error, "no window handle; showing the window normally");
            let _ = window.show();
            return;
        }
    };
    let placed = unsafe {
        SetWindowPos(
            handle,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
    };
    match placed {
        Ok(()) => tracing::info!("window revealed without taking focus"),
        Err(error) => {
            tracing::warn!(%error, "could not reveal quietly; showing normally");
            let _ = window.show();
        }
    }
}

#[cfg(not(windows))]
fn reveal_quietly(window: &tauri::WebviewWindow) {
    let _ = window.show();
}

/// Looks for a newer build, installs it, and restarts into it.
///
/// In the background and once, at start-up: an update is not urgent, and asking
/// mid-session would interrupt reading to talk about the app rather than about
/// anything the reader came for. It is deliberately silent when there is
/// nothing to do -- the common case -- and says so only in the log.
///
/// A dev build never asks. `tauri dev` runs an unsigned binary whose version is
/// whatever the config says, so the check would either fail or, worse, replace
/// the build under development with a release one.
fn check_for_update(app: &tauri::AppHandle) {
    if cfg!(debug_assertions) {
        tracing::debug!("no update check in a dev build");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_updater::UpdaterExt;
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(error) => {
                tracing::warn!(%error, "no updater");
                return;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                tracing::info!(version = %version, "an update is available");
                // Offered, never taken. Replacing the program someone is
                // running -- and restarting it under them -- is theirs to
                // agree to, so all that happens here is that the shell is told
                // there is something to accept. `install_update` is the only
                // path that writes anything, and only a reader's click reaches
                // it.
                if let Err(error) = app.emit("update.available", version) {
                    tracing::warn!(%error, "could not offer the update");
                }
            }
            Ok(None) => tracing::debug!("already the newest build"),
            // A failed check is not a failed start: the app runs perfectly well
            // on the version it has.
            Err(error) => tracing::warn!(%error, "could not check for an update"),
        }
    });
}

/// Shows the window and puts it in front, from wherever it was.
///
/// Not just `show()`: the window may be hidden, minimised, or behind
/// everything, and only doing all three makes clicking the tray reliable.
fn raise(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// The system tray: what keeps the app running once its window is closed.
///
/// Its menu carries the two things that cannot live in the window, because the
/// window may not be there: a way back to it, and a way to actually quit. Start
/// with Windows is here too rather than in the app's own settings, for the same
/// reason -- it is a question about the app existing, not about how it draws.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri_plugin_autostart::ManagerExt;

    let launches = app.autolaunch().is_enabled().unwrap_or(false);
    let open = MenuItem::with_id(app, "open", "Open MatterLess", true, None::<&str>)?;
    let startup = CheckMenuItem::with_id(
        app,
        "startup",
        "Start with Windows",
        true,
        launches,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &startup, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    // Its own image, not the window's.
    //
    // The window icon is 1024px and Windows would hand the tray a filtered
    // downscale of it to 16 -- the same mistake the taskbar badge made before
    // it was drawn at the size asked for. `icons/tray.png` is 32px and
    // sharpened by `tools/make_icon.py`, so the halving to 16 is clean.
    let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("main")
        .icon(tray_icon)
        .tooltip("MatterLess")
        .menu(&menu)
        // The menu is the right button's job; a left click should just bring the
        // window back, which is what every other tray app does.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => raise(app),
            "startup" => {
                let launcher = app.autolaunch();
                let was = launcher.is_enabled().unwrap_or(false);
                let outcome = if was {
                    launcher.disable()
                } else {
                    launcher.enable()
                };
                match outcome {
                    Ok(()) => tracing::info!(enabled = !was, "start with windows"),
                    Err(error) => tracing::warn!(%error, "could not change start with windows"),
                }
            }
            // The one way out, now that closing the window only hides it.
            "quit" => {
                tracing::info!("quitting from the tray");
                app.exit(0);
            }
            other => tracing::warn!(item = %other, "unknown tray menu item"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                raise(tray.app_handle());
            }
        })
        .build(app)?;
    tracing::info!(start_with_windows = launches, "tray ready");
    Ok(())
}

pub fn run() {
    // Before anything opens a connection. `reqwest` names its provider itself,
    // but the WebSocket goes through `tokio-tungstenite`, which asks rustls to
    // pick one -- and rustls panics rather than choose when the binary carries
    // both. `aws-lc-rs` is the one reqwest already uses.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a rustls crypto provider was already installed");
    }

    // Created before the builder because a URI scheme has to be registered on
    // it, before any managed state exists.
    let shared_media = Arc::new(media::MediaCache::default());
    let handler_media = Arc::clone(&shared_media);
    // A separate clone per closure: both outlive this function, so neither can
    // borrow the original.
    let state_media = Arc::clone(&shared_media);
    tauri::Builder::default()
        // Authenticated images: avatars, team icons and custom emoji all sit
        // behind the session token, so `<img src="https://...">` cannot reach
        // them -- the webview sends no Authorization header. Resolved here
        // instead, which also keeps the token out of the page and streams the
        // bytes without a base64 trip through IPC.
        .register_asynchronous_uri_scheme_protocol("mmedia", move |context, request, responder| {
            let cache = Arc::clone(&handler_media);
            let app = context.app_handle().clone();
            let path = request.uri().path().to_string();
            // The version is part of the key: a new avatar arrives under the
            // same id, and `last_picture_update` is the only thing that says so.
            let key = match request.uri().query() {
                Some(query) => format!("{path}?{query}"),
                None => path.clone(),
            };
            // A video element seeks with these; everything else never sends one.
            let range = request
                .headers()
                .get("Range")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);

            let Some(route) = media::route_for(&path) else {
                responder.respond(
                    tauri::http::Response::builder()
                        .status(404)
                        .body(Vec::new())
                        .expect("empty response"),
                );
                return;
            };

            // Attachments skip it: their bytes live on disk, and this cache is
            // sized for kilobyte avatars rather than megabyte files. Everything
            // else is kept in *both* -- memory so a face drawn twenty times on
            // one screen is not twenty file reads, disk so it survives a
            // restart.
            if media::fits_in_memory(&path) {
                if let Some(held) = cache.get(&key) {
                    tracing::debug!(key = %key, bytes = held.bytes.len(), "media served from cache");
                    responder.respond(media::respond(
                        held.bytes.clone(),
                        held.content_type.clone(),
                        range.as_deref(),
                    ));
                    return;
                }
            }

            tauri::async_runtime::spawn(async move {
                let (rest, files) = {
                    let state: tauri::State<'_, AppState> = app.state();
                    (Arc::clone(&state.rest), Arc::clone(&state.files))
                };

                // Read on a blocking thread: an attachment is megabytes, and
                // reading it on the async runtime would stall every other
                // request behind it.
                //
                // Keyed on the whole key, query included, because an avatar's
                // version is part of its identity -- dropping it would serve
                // last month's picture from disk for ever.
                let on_disk = media::disk_key(&key);
                if let Some(disk_key) = on_disk.clone() {
                    let from_disk = Arc::clone(&files);
                    let probe = disk_key.clone();
                    let held = tokio::task::spawn_blocking(move || from_disk.read(&probe))
                        .await
                        .ok()
                        .flatten();
                    if let Some((bytes, content_type)) = held {
                        tracing::debug!(key = %disk_key, bytes = bytes.len(), "file served from disk");
                        // Into memory too, so the next draw of the same face
                        // does not read the disk again.
                        if media::fits_in_memory(&path) {
                            cache.put(
                                &key,
                                Arc::new(media::Cached {
                                    bytes: bytes.clone(),
                                    content_type: content_type.clone(),
                                }),
                            );
                        }
                        responder.respond(media::respond(bytes, content_type, range.as_deref()));
                        return;
                    }
                }

                let response = match rest.fetch_bytes(&route).await {
                    Ok(Some((bytes, content_type))) => {
                        // Logged because "did the image load" is otherwise only
                        // answerable by looking at the window.
                        tracing::debug!(
                            route = %route,
                            bytes = bytes.len(),
                            content_type = %content_type,
                            "media fetched"
                        );
                        if let Some(disk_key) = on_disk {
                            let onto_disk = Arc::clone(&files);
                            let stored = bytes.clone();
                            let label = content_type.clone();
                            // Fire and forget: the response does not wait on the
                            // cache write, and a cache that cannot write is slow
                            // rather than broken.
                            tokio::task::spawn_blocking(move || {
                                onto_disk.write(&disk_key, &stored, &label)
                            });
                        }
                        if media::fits_in_memory(&path) {
                            cache.put(
                                &key,
                                Arc::new(media::Cached {
                                    bytes: bytes.clone(),
                                    content_type: content_type.clone(),
                                }),
                            );
                        }
                        Ok(media::respond(bytes, content_type, range.as_deref()))
                    }
                    // No avatar, no team icon: ordinary, and the page falls
                    // back to initials.
                    Ok(None) => {
                        tracing::debug!(route = %route, "media absent");
                        tauri::http::Response::builder().status(404).body(Vec::new())
                    }
                    Err(error) => {
                        tracing::debug!(%error, route = %route, "media fetch failed");
                        tauri::http::Response::builder().status(502).body(Vec::new())
                    }
                };
                responder.respond(response.expect("media response"));
            });
        })
        .plugin(tauri_plugin_notification::init())
        // Written as a registry Run entry, so it needs no installer step and
        // the reader can turn it off from the app rather than from Windows.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Closing the window HIDES it, and the tray is what says so.
        //
        // Toasts only exist while this process runs -- there is no push proxy
        // for a desktop client -- so quitting on the close button would quietly
        // turn notifications off, which is the one thing that lets the webapp
        // stay closed. Quitting is still one click away, in the tray menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                tracing::info!("window hidden to the tray");
            }
        })
        .setup(move |app| {
            let directory = data_dir(app.handle());
            std::fs::create_dir_all(&directory)?;
            let log = init_logging(app.handle());
            tracing::info!(path = %log.display(), "session started");
            // The identifier differs between the dev and installed configs, so
            // this path does too: a dev build never opens the real database.
            let database = directory.join("matterless.db");
            tracing::info!(path = %database.display(), "opening store");

            let store = Arc::new(Store::open(&database)?);
            // Logged every start, not only when a migration runs, so the schema
            // state is observable rather than inferred from its absence.
            match store.schema_version() {
                Ok(version) => tracing::info!(version, "store schema"),
                Err(error) => tracing::warn!(%error, "could not read the schema version"),
            }
            let engine = Arc::new(SyncEngine::new(Arc::clone(&store)));
            // The stored server if there is one, and a URL that goes nowhere
            // if there is not: the client exists from start-up, and the sign-in
            // screen points it somewhere real before anything is asked of it.
            let chosen = stored_server(&directory);
            let rest = Arc::new(RestClient::new(
                chosen.as_deref().unwrap_or(NO_SERVER),
            )?);
            tracing::info!(configured = chosen.is_some(), "server");
            // A token belongs to the server it was issued by, so it is only
            // restored once there is one.
            if chosen.is_some()
                && let Some(token) = stored_token()
            {
                rest.set_token(matterless_core::AuthToken::Session(token));
            }

            let media = Arc::clone(&state_media);
            // Attachments live beside the database rather than in it: SQLite is
            // the wrong shape for a 150 MB blob, and the cache is disposable.
            let files = Arc::new(filecache::FileCache::open(directory.join("files")));
            let pending = Arc::new(pending::PendingPosts::default());
            let uploads = Arc::new(uploads::Uploads::default());
            let connected = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let allow_svg = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reconciled = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let renders = Arc::new(render_cache::RenderCache::default());
            let (to_engine, from_ui) = mpsc::channel(256);
            app.manage(AppState {
                media,
                files,
                // Collapsed until bootstrap resolves it: it is this server's
                // default for this account, and assuming flat would count every
                // reply as channel unread for the first moments of a session.
                collapsed: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                store: Arc::clone(&store),
                engine: Arc::clone(&engine),
                rest: Arc::clone(&rest),
                pending: Arc::clone(&pending),
                uploads,
                max_file_size: Arc::new(std::sync::atomic::AtomicI64::new(0)),
                connected: Arc::clone(&connected),
                allow_svg,
                reconciled,
                renders,
                to_engine,
            });

            // What the window actually ended up as. Worth a line: the dev and
            // background launches override the config, and whether an override
            // *merges* with the base window or replaces it decides whether
            // `dragDropEnabled` is still false -- which is the difference
            // between the page seeing a dropped file and Tauri swallowing it.
            if let Some(window) = app.get_webview_window("main") {
                // A window configured invisible is a deliberately unobtrusive
                // launch: the dev config, so a rebuild does not interrupt
                // whatever is on screen. Revealed here rather than by `show()`,
                // which activates it and raises it above everything.
                //
                // `focus: false` alone is not enough, which is worth saying
                // because it looks like it should be: Windows activates a newly
                // created *visible* window regardless, so the window has to
                // start hidden for this path to be the one that shows it.
                if !window.is_visible().unwrap_or(true) {
                    reveal_quietly(&window);
                }
                match (window.inner_size(), window.is_focused()) {
                    (Ok(size), focused) => tracing::info!(
                        width = size.width,
                        height = size.height,
                        focused = focused.unwrap_or(false),
                        "main window"
                    ),
                    (Err(error), _) => tracing::warn!(%error, "window size unreadable"),
                }
            }

            check_for_update(app.handle());

            if let Err(error) = build_tray(app.handle()) {
                // A missing tray is a lesser app, not a broken one: the window
                // still works and the close button still hides it, which is
                // reachable again from the taskbar.
                tracing::warn!(%error, "no tray icon");
            }

            engine::spawn(engine, rest, pending, from_ui, connected);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::session,
            commands::set_server,
            commands::sign_in,
            commands::bootstrap,
            commands::channel_rows,
            commands::open_channel,
            commands::set_focus,
            commands::subscribe,
            commands::refresh,
            commands::send_post,
            commands::attach_file,
            commands::release_attachment,
            commands::cancel_attachment,
            commands::save_attachment,
            commands::send_typing,
            commands::statuses,
            commands::profile,
            commands::channel_by_name,
            commands::set_my_status,
            commands::suggest,
            commands::switcher,
            commands::search_messages,
            commands::fetch_around,
            commands::saved_messages,
            commands::pinned_messages,
            commands::browse_channels,
            commands::join_channel,
            commands::followed_threads,
            commands::install_update,
            commands::mark_channel_unread,
            commands::set_channel_muted,
            commands::move_channel,
            commands::add_channel_member,
            commands::channel_link,
            commands::leave_channel,
            commands::open_group_message,
            commands::open_direct_message,
            commands::edit_post,
            commands::delete_post_now,
            commands::post_text,
            commands::post_permalink,
            commands::set_reminder,
            commands::mark_post_unread,
            commands::set_post_saved,
            commands::set_post_pinned,
            commands::retry_post,
            commands::discard_post,
            commands::load_older,
            commands::mark_read,
            commands::refresh_membership,
            commands::sidebar,
            commands::toggle_reaction,
            commands::refresh_emoji,
            commands::emoji_categories,
            commands::refresh_threads,
            commands::thread_rows,
            commands::refresh_thread,
            commands::mark_thread_read,
            commands::set_thread_following,
            commands::update_badge,
            commands::raise_toast,
            commands::flash_taskbar,
            commands::clear_attention,
            commands::ui_log,
            commands::where_is_the_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MatterLess");
}
