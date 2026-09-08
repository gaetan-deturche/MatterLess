//! Serde mirrors of the server's `model` package. Field names are kept exactly
//! as the wire sends them so the Go source stays a usable reference.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Timestamp = i64; // milliseconds since epoch, server clock

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub last_picture_update: Timestamp,
    #[serde(default)]
    pub notify_props: HashMap<String, String>,
    #[serde(default)]
    pub roles: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Team {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub team_id: String,
    /// "O" open, "P" private, "D" direct, "G" group
    #[serde(default, rename = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub total_msg_count: i64,
    #[serde(default)]
    pub total_msg_count_root: i64,
    #[serde(default)]
    pub last_post_at: Timestamp,
    #[serde(default)]
    pub delete_at: Timestamp,
}

/// Unread and mention counts are *derived* from this, never read from a flag.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelMember {
    pub channel_id: String,
    pub user_id: String,
    #[serde(default)]
    pub last_viewed_at: Timestamp,
    #[serde(default)]
    pub msg_count: i64,
    #[serde(default)]
    pub msg_count_root: i64,
    #[serde(default)]
    pub mention_count: i64,
    #[serde(default)]
    pub mention_count_root: i64,
    #[serde(default)]
    pub notify_props: HashMap<String, String>,
}

impl ChannelMember {
    /// A channel is muted when `mark_unread` is "mention".
    pub fn is_muted(&self) -> bool {
        self.notify_props.get("mark_unread").map(String::as_str) == Some("mention")
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PostMetadata {
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub files: Vec<FileInfo>,
    #[serde(default)]
    pub embeds: Vec<Embed>,
    #[serde(default)]
    pub emojis: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Reaction {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub post_id: String,
    #[serde(default)]
    pub emoji_name: String,
    #[serde(default)]
    pub create_at: Timestamp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub extension: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub mime_type: String,
    /// Zero for anything the server did not decode as an image: the fields are
    /// absent from the JSON for a PDF, not null (captured
    /// against a live server).
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    /// The server generated a JPEG thumbnail and preview for this one, which is
    /// what makes `/files/{id}/thumbnail` worth asking for -- 20 KB against a
    /// 474 KB original, measured.
    #[serde(default)]
    pub has_preview_image: bool,
    /// A ~1 KB base64 JPEG the server ships *inside the post's metadata*, so an
    /// image has something to show before a single byte is fetched. Free
    /// placeholder: it costs no request.
    #[serde(default)]
    pub mini_preview: Option<String>,
    /// Empty until the file is attached to a post: an upload response carries
    /// no post id yet.
    #[serde(default)]
    pub post_id: String,
    /// The file was archived out of storage; the bytes are gone.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Post {
    pub id: String,
    #[serde(default)]
    pub create_at: Timestamp,
    #[serde(default)]
    pub update_at: Timestamp,
    #[serde(default)]
    pub edit_at: Timestamp,
    #[serde(default)]
    pub delete_at: Timestamp,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub root_id: String,
    #[serde(default)]
    pub message: String,
    /// Empty for ordinary posts; "system_*" for joins, leaves, header changes.
    #[serde(default, rename = "type")]
    pub post_type: String,
    #[serde(default)]
    pub file_ids: Vec<String>,
    /// Heterogeneous by design: webhook posts put their whole payload in
    /// `attachments` here and leave `message` empty.
    #[serde(default)]
    pub props: serde_json::Value,
    #[serde(default)]
    pub metadata: PostMetadata,
    /// Echoed back on the websocket, which is how an optimistic send reconciles.
    #[serde(default)]
    pub pending_post_id: String,
    /// Pinned to its channel. Channel-wide state, not per-reader.
    #[serde(default)]
    pub is_pinned: bool,
}

impl Post {
    pub fn is_reply(&self) -> bool {
        !self.root_id.is_empty()
    }

    pub fn is_system(&self) -> bool {
        self.post_type.starts_with("system_")
    }

    pub fn is_deleted(&self) -> bool {
        self.delete_at != 0
    }

    /// True when the post carries a webhook-style attachment payload, which must
    /// be rendered instead of `message`.
    pub fn has_attachments(&self) -> bool {
        self.props
            .get("attachments")
            .and_then(|value| value.as_array())
            .is_some_and(|array| !array.is_empty())
    }

    /// Cache key for the parsed markdown row: an edit bumps `update_at`.
    pub fn render_key(&self) -> (&str, Timestamp) {
        (&self.id, self.update_at)
    }
}

/// `GET /channels/{id}/posts` returns an ordered id list plus a map that also
/// contains thread context absent from `order` -- measured at 27 of 87 posts.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostList {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub posts: HashMap<String, Post>,
    #[serde(default)]
    pub next_post_id: String,
    #[serde(default)]
    pub prev_post_id: String,
    #[serde(default)]
    pub has_next: bool,
}

impl PostList {
    /// Posts named by `order`, newest first, as the server ordered them.
    pub fn in_order(&self) -> impl Iterator<Item = &Post> {
        self.order.iter().filter_map(|id| self.posts.get(id))
    }

    /// Posts present in the map but not in `order`: thread context. They must be
    /// stored (a reply needs its root) but must not be placed in the stream.
    pub fn context_only(&self) -> impl Iterator<Item = &Post> {
        self.posts
            .values()
            .filter(|post| !self.order.contains(&post.id))
    }

    pub fn oldest_in_order(&self) -> Option<&Post> {
        self.order.last().and_then(|id| self.posts.get(id))
    }

    pub fn newest_in_order(&self) -> Option<&Post> {
        self.order.first().and_then(|id| self.posts.get(id))
    }
}

/// A custom emoji: an image, addressed by id.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomEmoji {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub creator_id: String,
}

/// One sidebar category, as the server organises the channel list.
///
/// Shape captured live, not from the docs: four types --
/// `favorites`, `custom`, `channels`, `direct_messages` -- each carrying its own
/// `sorting` (`manual`, `recent`, or empty) and its channels in order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SidebarCategory {
    pub id: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(rename = "type", default)]
    pub category_type: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub sort_order: i64,
    /// `manual` keeps `channel_ids` order, `recent` sorts by last post, and an
    /// empty value means the server left it at its default (alphabetical).
    #[serde(default)]
    pub sorting: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub channel_ids: Vec<String>,
}

/// The categories of one team, plus the order they are shown in.
///
/// The order arrives separately from `sort_order`, so it is kept rather than
/// re-derived.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SidebarCategories {
    #[serde(default)]
    pub categories: Vec<SidebarCategory>,
    #[serde(default)]
    pub order: Vec<String>,
}

/// One followed thread, as `GET /users/{me}/teams/{team}/threads` returns it.
///
/// Shape captured from a live server, not guessed:
/// `id` is the root post's id, the counts are per thread rather than per
/// channel, and `participants` come back as user objects whose fields are all
/// empty except `id` unless `extended=true` is asked for -- so names are
/// resolved from the local users table, as everywhere else.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserThread {
    /// The root post's id.
    pub id: String,
    #[serde(default)]
    pub reply_count: i64,
    #[serde(default)]
    pub last_reply_at: Timestamp,
    #[serde(default)]
    pub last_viewed_at: Timestamp,
    #[serde(default)]
    pub unread_replies: i64,
    #[serde(default)]
    pub unread_mentions: i64,
    #[serde(default)]
    pub is_urgent: bool,
    #[serde(default)]
    pub delete_at: Timestamp,
    /// The root itself, which is how a followed thread can be listed without
    /// its channel being loaded.
    pub post: Post,
    #[serde(default)]
    pub participants: Option<Vec<ThreadParticipant>>,
}

/// Only the id is populated unless `extended=true`; the rest arrive empty.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadParticipant {
    pub id: String,
}

/// A page of followed threads, with the totals the server keeps for the tab.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserThreads {
    #[serde(default)]
    pub threads: Vec<UserThread>,
    #[serde(default)]
    pub total: i64,
    #[serde(default)]
    pub total_unread_threads: i64,
    #[serde(default)]
    pub total_unread_mentions: i64,
    #[serde(default)]
    pub total_unread_urgent_mentions: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Preference {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

/// A preview the server attached to a post.
///
/// Three kinds arrive from this server, captured live rather than taken from
/// the documentation:
///
/// * `opengraph` -- a fetched page, with `data` holding title, description,
///   `site_name` and `images`.
/// * `permalink` -- a link to another message, with `data.post` holding the
///   whole quoted post. It carries **no `url`**, unlike the other two.
/// * `link` -- a bare URL the server did not (or could not) fetch metadata
///   for: `{type, url}` and nothing else, so there is nothing to draw that the
///   message text does not already say.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Embed {
    #[serde(rename = "type", default)]
    pub embed_type: String,
    #[serde(default)]
    pub url: String,
    /// Shape depends entirely on `embed_type`, so it stays as JSON here and is
    /// read by the row planner.
    #[serde(default)]
    pub data: serde_json::Value,
}

/// Someone's presence.
///
/// `manual` matters: a status the reader chose themselves must not be quietly
/// replaced by activity, which is how "away" would erase a deliberate "do not
/// disturb".
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Status {
    #[serde(default)]
    pub user_id: String,
    /// `online`, `away`, `dnd`, `offline`, or `ooo` for out of office.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    pub last_activity_at: Timestamp,
    /// When a `dnd` window ends, zero when it does not.
    #[serde(default)]
    pub dnd_end_time: Timestamp,
}

/// Server-wide settings plus the per-user override that wins over them. Phase 0
/// measured `CollapsedThreads: default_off` on the server against
/// `collapsed_reply_threads: on` for the account -- both must be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadMode {
    Disabled,
    Collapsed,
    Flat,
}

impl ThreadMode {
    pub fn resolve(server_setting: Option<&str>, user_preference: Option<&str>) -> Self {
        match server_setting {
            Some("disabled") | None => return ThreadMode::Disabled,
            _ => {}
        }
        match user_preference {
            Some("on") => ThreadMode::Collapsed,
            Some("off") => ThreadMode::Flat,
            // No explicit preference: the server default decides.
            _ => match server_setting {
                Some("default_on") => ThreadMode::Collapsed,
                _ => ThreadMode::Flat,
            },
        }
    }
}

/// What `POST /files` answers with. `client_ids` echoes back whatever the
/// request sent, and comes back empty when it sent none.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileUploadResponse {
    #[serde(default)]
    pub file_infos: Vec<FileInfo>,
    #[serde(default)]
    pub client_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewPost<'a> {
    pub channel_id: &'a str,
    pub message: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub root_id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub pending_post_id: &'a str,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    pub file_ids: &'a [String],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_mode_prefers_the_user_over_the_server() {
        // The exact pairing Phase 0 measured.
        assert_eq!(
            ThreadMode::resolve(Some("default_off"), Some("on")),
            ThreadMode::Collapsed
        );
        assert_eq!(
            ThreadMode::resolve(Some("default_on"), Some("off")),
            ThreadMode::Flat
        );
        assert_eq!(
            ThreadMode::resolve(Some("default_on"), None),
            ThreadMode::Collapsed
        );
        assert_eq!(
            ThreadMode::resolve(Some("default_off"), None),
            ThreadMode::Flat
        );
        // Server off overrides any preference.
        assert_eq!(
            ThreadMode::resolve(Some("disabled"), Some("on")),
            ThreadMode::Disabled
        );
    }

    #[test]
    fn context_posts_are_separated_from_the_stream() {
        let json = r#"{
            "order": ["p1"],
            "posts": {
                "p1": {"id":"p1","message":"reply","root_id":"root1"},
                "root1": {"id":"root1","message":"the root"}
            }
        }"#;
        let list: PostList = serde_json::from_str(json).unwrap();
        assert_eq!(list.in_order().count(), 1);
        let context: Vec<_> = list.context_only().map(|p| p.id.clone()).collect();
        assert_eq!(context, vec!["root1".to_string()]);
    }

    #[test]
    fn webhook_attachments_are_detected() {
        let json = r#"{"id":"a","message":"","props":{"attachments":[{"text":"build failed"}]}}"#;
        let post: Post = serde_json::from_str(json).unwrap();
        assert!(post.has_attachments());
        assert!(post.message.is_empty());
    }
}
