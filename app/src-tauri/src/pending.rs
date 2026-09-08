//! Optimistic sends, held in memory and nowhere else.
//!
//! The plan's rule is that a guess must never be persisted: if the send fails,
//! or the app dies mid-flight, SQLite should look exactly as it did before. So a
//! pending post lives here until the server confirms it, and is merged into the
//! row plan by the same `plan_channel` call that renders everything else --
//! which keeps a single render path rather than a second one for local echoes.
//!
//! Reconciliation is by `pending_post_id`, which Mattermost echoes back on the
//! websocket for exactly this purpose.

use matterless_core::model::{Post, PostMetadata, Timestamp};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PendingPost {
    pub pending_post_id: String,
    pub channel_id: String,
    pub root_id: String,
    pub message: String,
    pub create_at: Timestamp,
    pub failed: bool,
    /// Attachments already uploaded and waiting for this post to claim them.
    ///
    /// The whole `FileInfo`, not just the id: the guess row shows the
    /// attachments too, and the server's copy of them is what says how big the
    /// image is. An upload nothing ever claims is orphaned server-side, which
    /// is why a failed send can be retried or discarded freely.
    pub files: Vec<matterless_core::model::FileInfo>,
}

impl PendingPost {
    /// A stand-in `Post` so the row planner can treat it like any other. Its id
    /// is the pending id, which gives the row a stable key for its whole life.
    pub fn as_post(&self, me_id: &str) -> Post {
        Post {
            id: self.pending_post_id.clone(),
            channel_id: self.channel_id.clone(),
            user_id: me_id.to_string(),
            root_id: self.root_id.clone(),
            create_at: self.create_at,
            update_at: self.create_at,
            edit_at: 0,
            delete_at: 0,
            message: self.message.clone(),
            post_type: String::new(),
            file_ids: self.files.iter().map(|file| file.id.clone()).collect(),
            props: serde_json::Value::Null,
            metadata: PostMetadata {
                files: self.files.clone(),
                ..PostMetadata::default()
            },
            pending_post_id: self.pending_post_id.clone(),
            is_pinned: false,
        }
    }
}

#[derive(Default)]
pub struct PendingPosts {
    by_id: Mutex<HashMap<String, PendingPost>>,
}

impl PendingPosts {
    /// Mattermost's own convention for the id, which keeps it unique per sender
    /// without needing a uuid dependency.
    pub fn new_id(me_id: &str, now: Timestamp) -> String {
        format!("{me_id}:{now}")
    }

    pub fn insert(&self, post: PendingPost) {
        self.lock().insert(post.pending_post_id.clone(), post);
    }

    /// Called when the server confirms the post, by echo or by reply. Returns
    /// the channel it belonged to, so the caller knows what to redraw.
    pub fn resolve(&self, pending_post_id: &str) -> Option<String> {
        self.lock()
            .remove(pending_post_id)
            .map(|post| post.channel_id)
    }

    pub fn mark_failed(&self, pending_post_id: &str) {
        if let Some(post) = self.lock().get_mut(pending_post_id) {
            post.failed = true;
        }
    }

    pub fn get(&self, pending_post_id: &str) -> Option<PendingPost> {
        self.lock().get(pending_post_id).cloned()
    }

    pub fn discard(&self, pending_post_id: &str) -> Option<String> {
        self.resolve(pending_post_id)
    }

    pub fn for_channel(&self, channel_id: &str) -> Vec<PendingPost> {
        let mut posts: Vec<PendingPost> = self
            .lock()
            .values()
            .filter(|post| post.channel_id == channel_id)
            .cloned()
            .collect();
        posts.sort_by_key(|post| post.create_at);
        posts
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PendingPost>> {
        self.by_id.lock().expect("pending posts mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, channel: &str) -> PendingPost {
        PendingPost {
            pending_post_id: id.into(),
            channel_id: channel.into(),
            root_id: String::new(),
            message: "hello".into(),
            create_at: 1_000,
            failed: false,
            files: Vec::new(),
        }
    }

    #[test]
    fn a_pending_post_is_scoped_to_its_channel() {
        let pending = PendingPosts::default();
        pending.insert(sample("a", "c1"));
        pending.insert(sample("b", "c2"));
        assert_eq!(pending.for_channel("c1").len(), 1);
        assert_eq!(pending.for_channel("c2").len(), 1);
        assert!(pending.for_channel("c3").is_empty());
    }

    #[test]
    fn confirming_a_send_removes_it_and_names_its_channel() {
        let pending = PendingPosts::default();
        pending.insert(sample("a", "c1"));
        assert_eq!(pending.resolve("a").as_deref(), Some("c1"));
        assert!(pending.for_channel("c1").is_empty());
        // A second echo of the same post must be harmless.
        assert_eq!(pending.resolve("a"), None);
    }

    #[test]
    fn a_failed_send_is_kept_so_it_can_be_retried() {
        let pending = PendingPosts::default();
        pending.insert(sample("a", "c1"));
        pending.mark_failed("a");
        let held = pending.get("a").expect("still present");
        assert!(held.failed);
        assert_eq!(held.message, "hello", "the text must survive for a retry");
    }

    #[test]
    fn the_stand_in_post_carries_its_pending_id_as_its_row_key() {
        let post = sample("me:123", "c1").as_post("me");
        assert_eq!(post.id, "me:123");
        assert_eq!(post.pending_post_id, "me:123");
        assert_eq!(post.user_id, "me");
        assert_eq!(post.update_at, post.create_at);
    }
}
