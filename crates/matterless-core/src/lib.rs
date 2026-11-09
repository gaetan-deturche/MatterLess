//! Headless Mattermost client core: REST subset, session auth, and the
//! WebSocket session with reconnect. No UI, no storage -- Phase 2 adds SQLite,
//! Phase 3 adds the Tauri shell.
//!
//! Field names mirror the server's Go `model` package so that source stays a
//! usable reference. Behaviour choices that look arbitrary are usually measured:
//! established by probing a live server before any of it was written.

pub mod auth;
pub mod error;
pub mod fuzzy;
pub mod model;
pub mod rate_limit;
pub mod rest;
pub mod search;
pub mod text;
pub mod ws;

pub use auth::{AuthToken, ReauthAction};
pub use error::{ApiError, Error, Result};
pub use model::{Channel, ChannelMember, Post, PostList, Preference, Team, ThreadMode, User};
pub use rest::RestClient;
pub use ws::{Event, Signal, WsHandle, WsSession};

/// Reads the resolved thread mode from a client config map plus the user's
/// preferences, which is the only correct way to get it: Phase 0 measured the
/// server saying `default_off` while the account said `on`.
pub fn resolve_thread_mode(
    client_config: &std::collections::HashMap<String, String>,
    preferences: &[Preference],
) -> ThreadMode {
    let server = client_config.get("CollapsedThreads").map(String::as_str);
    let user = preferences
        .iter()
        .find(|preference| {
            preference.category == "display_settings"
                && preference.name == "collapsed_reply_threads"
        })
        .map(|preference| preference.value.as_str());
    ThreadMode::resolve(server, user)
}

/// Which name people are shown by, as the official client works it out: the
/// server's `TeammateNameDisplay`, unless the reader chose their own in their
/// display settings and the server does not lock it. `username` when neither
/// says, which is the server's own default.
pub fn resolve_name_display(server: Option<&str>, locked: bool, chosen: Option<&str>) -> String {
    let server = server.filter(|mode| !mode.is_empty()).unwrap_or("username");
    chosen
        .filter(|mode| !mode.is_empty() && !locked)
        .unwrap_or(server)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_name_shown_is_the_readers_choice_unless_the_server_locks_it() {
        assert_eq!(resolve_name_display(None, false, None), "username");
        assert_eq!(
            resolve_name_display(Some("full_name"), false, None),
            "full_name"
        );
        assert_eq!(
            resolve_name_display(Some("full_name"), false, Some("nickname_full_name")),
            "nickname_full_name"
        );
        assert_eq!(
            resolve_name_display(Some("full_name"), true, Some("username")),
            "full_name",
            "locked"
        );
        assert_eq!(
            resolve_name_display(Some("full_name"), false, Some("")),
            "full_name"
        );
    }

    #[test]
    fn thread_mode_comes_from_both_sources() {
        let mut config = HashMap::new();
        config.insert("CollapsedThreads".to_string(), "default_off".to_string());
        let preferences = vec![Preference {
            user_id: "me".into(),
            category: "display_settings".into(),
            name: "collapsed_reply_threads".into(),
            value: "on".into(),
        }];
        assert_eq!(
            resolve_thread_mode(&config, &preferences),
            ThreadMode::Collapsed
        );
        assert_eq!(resolve_thread_mode(&config, &[]), ThreadMode::Flat);
    }
}
