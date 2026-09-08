//! Authenticated images, behind a custom URI scheme.
//!
//! Avatars, team icons and custom emoji all live behind the session token, so
//! they cannot be `<img src="https://…">` -- the webview sends no Authorization
//! header. A custom scheme resolves them in Rust instead: the bytes stream
//! straight into the webview without passing through JavaScript, which is both
//! faster than base64 through IPC and keeps the token out of the page.
//!
//! `mmedia://localhost/avatar/<user id>?v=<last picture update>`
//! `mmedia://localhost/team/<team id>`
//! `mmedia://localhost/emoji/<emoji id>`
//! `mmedia://localhost/file/<file id>`    -- an attachment, as uploaded
//! `mmedia://localhost/thumb/<file id>`   -- the server's JPEG thumbnail
//! `mmedia://localhost/preview/<file id>` -- the server's larger JPEG
//!
//! The `?v=` is deliberate: an avatar changes under the same id, and the
//! server's `last_picture_update` is the only thing that says so. It is part of
//! the cache key, so a new picture is a new URL and nothing has to be evicted
//! by hand.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Cached image bytes.
///
/// In memory rather than on disk: an avatar is a few kilobytes, the working set
/// is the people on screen, and a process lifetime is the natural span for it.
/// File attachments will need the disk -- they are megabytes and worth keeping
/// between runs -- but that belongs with the files workstream, not here.
#[derive(Default)]
pub struct MediaCache {
    entries: Mutex<HashMap<String, Arc<Cached>>>,
}

pub struct Cached {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Roughly a few hundred avatars. Trimmed wholesale rather than by recency: the
/// set that matters is whatever is on screen, and it is cheap to refetch.
const CAPACITY: usize = 400;

impl MediaCache {
    pub fn get(&self, key: &str) -> Option<Arc<Cached>> {
        self.entries.lock().expect("media cache").get(key).cloned()
    }

    pub fn put(&self, key: &str, entry: Arc<Cached>) {
        let mut entries = self.entries.lock().expect("media cache");
        if entries.len() >= CAPACITY {
            entries.clear();
        }
        entries.insert(key.to_string(), entry);
    }
}

/// Which server route a request maps to, or `None` if it is not one of ours.
///
/// Kept as a pure function so the routing is testable without a webview, a
/// network or a token.
pub fn route_for(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    let (kind, id) = trimmed.split_once('/')?;
    // Ids are server-generated 26-character tokens; anything else is a
    // malformed or hostile URL and gets no request made for it.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    match kind {
        "avatar" => Some(format!("/users/{id}/image")),
        "team" => Some(format!("/teams/{id}/image")),
        "emoji" => Some(format!("/emoji/{id}/image")),
        // Attachments. The thumbnail is what a message list asks for -- measured
        // at 20 KB against a 474 KB original on this server -- and the raw file
        // is only fetched when something wants the real thing.
        "file" => Some(format!("/files/{id}")),
        "thumb" => Some(format!("/files/{id}/thumbnail")),
        "preview" => Some(format!("/files/{id}/preview")),
        _ => None,
    }
}

/// Whether a route's bytes are small enough to also keep in memory.
///
/// Everything but attachments: an avatar or an emoji is drawn many times on one
/// screen, and reading each one off the disk again would put a blocking file
/// read in front of every face. Attachments are megabytes and belong only on
/// disk, which is what this cache was sized against.
pub fn fits_in_memory(path: &str) -> bool {
    !matches!(
        path.trim_start_matches('/').split('/').next(),
        Some("file") | Some("thumb") | Some("preview")
    )
}

/// A filesystem-safe cache name for a route, or `None` if it is not kept
/// between runs.
///
/// This is the only answer to "does this survive a restart", deliberately: a
/// separate predicate saying which routes go to disk was a second list that had
/// to agree with this one, and lists that must agree drift.
///
/// The test is immutability under the key, not size. Attachments qualify, and
/// so do custom emoji -- which were excluded on the grounds that they "change
/// under the same id, and the version in the URL handles that", true of an
/// avatar and not of these: an emoji route has no version because a Mattermost
/// custom emoji cannot be edited. Avatars qualify too, once the version they
/// already carry is part of the name.
///
/// Keeping either in memory alone meant re-fetching them all on every start.
/// Measured: 707 emoji at the client's eight-per-second budget, arriving one
/// every 120ms, which is a minute and a half before the picker fills.
///
/// Takes the whole key, query and all, because for an avatar the query *is* the
/// identity: the bytes change under the same id, and `?v=<last picture update>`
/// is the only thing that says which ones these are. `avatar-<id>-<version>` is
/// therefore as immutable as `file-<id>`, and a new picture simply writes a new
/// name rather than needing anything evicted.
///
/// Every part is validated here rather than trusted: an id is alphanumeric and
/// a version is an integer, so nothing this returns can leave the cache
/// directory.
pub fn disk_key(key: &str) -> Option<String> {
    let (path, query) = match key.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (key, None),
    };
    let trimmed = path.trim_start_matches('/');
    let (kind, id) = trimmed.split_once('/')?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    match kind {
        "file" | "thumb" | "preview" | "emoji" => Some(format!("{kind}-{id}")),
        "avatar" => {
            // `last_picture_update` is a signed epoch and is negative on this
            // server for accounts that have never changed their picture.
            let version = query.and_then(|query| query.strip_prefix("v="))?;
            let digits = version.strip_prefix('-').unwrap_or(version);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(format!("avatar-{id}-{}", version.replace('-', "n")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kinds_map_to_their_routes() {
        assert_eq!(
            route_for("/avatar/7gwdwjg1zjf7tb5xxdp6ieazgr").as_deref(),
            Some("/users/7gwdwjg1zjf7tb5xxdp6ieazgr/image")
        );
        assert_eq!(
            route_for("team/6sczm7ead3gz8eq7yofcc6ntca").as_deref(),
            Some("/teams/6sczm7ead3gz8eq7yofcc6ntca/image")
        );
        assert_eq!(
            route_for("/emoji/abc123").as_deref(),
            Some("/emoji/abc123/image")
        );
        // Attachments: three variants of the same file, and the thumbnail is
        // its own route rather than a resize of the original.
        assert_eq!(
            route_for("/file/4i64zi6t7pbyzgw8rnd3b6zd4c").as_deref(),
            Some("/files/4i64zi6t7pbyzgw8rnd3b6zd4c")
        );
        assert_eq!(
            route_for("/thumb/4i64zi6t7pbyzgw8rnd3b6zd4c").as_deref(),
            Some("/files/4i64zi6t7pbyzgw8rnd3b6zd4c/thumbnail")
        );
        assert_eq!(
            route_for("/preview/abc123").as_deref(),
            Some("/files/abc123/preview")
        );
    }

    #[test]
    fn immutable_routes_go_to_disk() {
        // Small enough to keep in memory as well, so a face drawn twenty times
        // on one screen is not twenty file reads.
        assert!(fits_in_memory("/avatar/abc123"));
        assert!(fits_in_memory("/emoji/abc123"));
        assert!(!fits_in_memory("/file/abc123"));

        assert_eq!(
            disk_key("preview/abc123").as_deref(),
            Some("preview-abc123")
        );
        assert_eq!(disk_key("/thumb/abc123").as_deref(), Some("thumb-abc123"));
        assert_eq!(disk_key("/file/abc123").as_deref(), Some("file-abc123"));
        assert_eq!(disk_key("/emoji/abc123").as_deref(), Some("emoji-abc123"));

        // An avatar is only immutable *at a version*, so the version is in the
        // name -- and a negative epoch is spelled, not dropped.
        assert_eq!(
            disk_key("/avatar/abc123?v=1616175447195").as_deref(),
            Some("avatar-abc123-1616175447195")
        );
        assert_eq!(
            disk_key("/avatar/abc123?v=-1777970270932").as_deref(),
            Some("avatar-abc123-n1777970270932")
        );
        // Without one there is nothing to say which picture these bytes are.
        assert!(disk_key("/avatar/abc123").is_none());
        assert!(disk_key("/avatar/abc123?v=").is_none());
        assert!(disk_key("/avatar/abc123?v=../../secret").is_none());

        // A key is a filename, so nothing that could leave the directory is one.
        // A team icon has no version, so nothing says which bytes these are.
        assert!(disk_key("/team/abc123").is_none());
        assert!(disk_key("/file/../../secret").is_none());
        assert!(disk_key("/file/a.b").is_none());
    }

    /// A path is a URL a page can compose, so it is checked rather than
    /// trusted: no traversal, no query smuggling, nothing but an id.
    #[test]
    fn anything_else_makes_no_request() {
        for path in [
            "/avatar/../../users/me",
            "/avatar/id/extra",
            "/avatar/",
            "/avatar",
            "/unknown/abc",
            "/avatar/abc%2F..",
            "",
        ] {
            assert!(route_for(path).is_none(), "{path} should not route");
        }
    }

    #[test]
    fn the_cache_holds_and_returns_bytes() {
        let cache = MediaCache::default();
        assert!(cache.get("a").is_none());
        cache.put(
            "a",
            Arc::new(Cached {
                bytes: vec![1, 2, 3],
                content_type: "image/png".into(),
            }),
        );
        let held = cache.get("a").expect("just stored");
        assert_eq!(held.bytes, vec![1, 2, 3]);
        assert_eq!(held.content_type, "image/png");
    }

    /// A new picture arrives under the same id, so the version is part of the
    /// key -- otherwise the old face would stay until a restart.
    #[test]
    fn the_version_is_part_of_the_key() {
        let cache = MediaCache::default();
        cache.put(
            "/avatar/u1?v=1",
            Arc::new(Cached {
                bytes: vec![1],
                content_type: "image/png".into(),
            }),
        );
        assert!(cache.get("/avatar/u1?v=2").is_none());
    }
}
