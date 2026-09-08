//! Parsed message bodies, in memory, keyed by post and edit.
//!
//! This replaced a SQLite table holding the same trees as JSON. The table
//! worked -- 400 hits out of 400 -- but only cut a plan build from 98 ms to
//! 68 ms, because `serde_json::from_str` on a nested tree costs nearly as much
//! as parsing the markdown that produced it (36 ms against 60 ms). The format
//! was the cost, not the lookup.
//!
//! Holding `Arc<Vec<Node>>` removes both halves of that: no serialisation on a
//! hit, and handing a body to a row is a refcount bump rather than a recursive
//! copy of the tree.
//!
//! The trade is that this dies with the process, so the first render of a
//! channel after launch parses from scratch. That was the table's only real
//! advantage, and it was worth about 24 ms once per launch -- not a table, a
//! migration and two code paths.

use matterless_core::model::Timestamp;
use matterless_render::markdown::Node;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Entries kept before trimming. A channel's window is a few hundred posts, so
/// this holds many channels' worth of scrollback.
const CAPACITY: usize = 4000;

struct Entry {
    update_at: Timestamp,
    nodes: Arc<Vec<Node>>,
    last_used: u64,
}

#[derive(Default)]
pub struct RenderCache {
    entries: Mutex<HashMap<String, Entry>>,
    clock: AtomicU64,
}

impl RenderCache {
    /// The body for this exact edit, or `None`.
    ///
    /// An edit moves `update_at`, so a changed post always misses -- a hit can
    /// never be stale text.
    pub fn get(&self, post_id: &str, update_at: Timestamp) -> Option<Arc<Vec<Node>>> {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut entries = self.lock();
        let entry = entries.get_mut(post_id)?;
        if entry.update_at != update_at {
            return None;
        }
        entry.last_used = stamp;
        Some(Arc::clone(&entry.nodes))
    }

    pub fn put(&self, post_id: &str, update_at: Timestamp, nodes: Arc<Vec<Node>>) {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut entries = self.lock();
        entries.insert(
            post_id.to_string(),
            Entry {
                update_at,
                nodes,
                last_used: stamp,
            },
        );
        if entries.len() > CAPACITY {
            Self::trim(&mut entries);
        }
    }

    /// Drops the least recently used half.
    ///
    /// Approximate rather than a true LRU: an exact one needs an intrusive list
    /// maintained on every hit, and this runs once per few thousand inserts.
    /// Halving rather than evicting one at a time keeps it that rare.
    fn trim(entries: &mut HashMap<String, Entry>) {
        let mut stamps: Vec<u64> = entries.values().map(|entry| entry.last_used).collect();
        stamps.sort_unstable();
        let cutoff = stamps[stamps.len() / 2];
        entries.retain(|_, entry| entry.last_used >= cutoff);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().expect("render cache mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Arc<Vec<Node>> {
        Arc::new(vec![Node::Text {
            value: text.to_string(),
        }])
    }

    #[test]
    fn a_hit_needs_the_same_edit_not_just_the_same_post() {
        let cache = RenderCache::default();
        cache.put("p1", 100, body("original"));

        assert!(cache.get("p1", 100).is_some());
        assert!(
            cache.get("p1", 200).is_none(),
            "an edited post must miss rather than serve the old text"
        );
        assert!(cache.get("unknown", 100).is_none());
    }

    #[test]
    fn a_hit_shares_the_tree_rather_than_copying_it() {
        let cache = RenderCache::default();
        let original = body("shared");
        cache.put("p1", 100, Arc::clone(&original));

        let first = cache.get("p1", 100).unwrap();
        let second = cache.get("p1", 100).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "two hits must be the same allocation, not two copies"
        );
        assert!(Arc::ptr_eq(&first, &original));
    }

    #[test]
    fn re_putting_a_post_replaces_its_body() {
        let cache = RenderCache::default();
        cache.put("p1", 100, body("before"));
        cache.put("p1", 200, body("after"));

        assert!(cache.get("p1", 100).is_none(), "the old edit is gone");
        let held = cache.get("p1", 200).unwrap();
        assert_eq!(
            held[0],
            Node::Text {
                value: "after".into()
            }
        );
        assert_eq!(cache.len(), 1, "replaced, not duplicated");
    }

    #[test]
    fn a_trim_keeps_the_recently_used_half_and_drops_the_rest() {
        let cache = RenderCache::default();
        for index in 0..CAPACITY {
            cache.put(&format!("old{index}"), 1, body("old"));
        }
        assert_eq!(cache.len(), CAPACITY, "at the cap, nothing trimmed yet");

        // Touch one early entry so it counts as recently used.
        let touched = "old0";
        assert!(cache.get(touched, 1).is_some());

        // Enough inserts to cause exactly one trim.
        for index in 0..(CAPACITY / 2 - 10) {
            cache.put(&format!("new{index}"), 1, body("new"));
        }

        assert!(cache.len() <= CAPACITY, "must not grow without bound");
        assert!(
            cache.get(touched, 1).is_some(),
            "an entry used just before the trim is in the recent half"
        );
        assert!(
            cache.get("old1", 1).is_none(),
            "an untouched early entry is exactly what a trim should drop"
        );
    }

    /// The eviction is deliberately approximate, and this pins what that means
    /// so the limitation is documented rather than discovered. An entry survives
    /// the *next* trim, not every future one: keep using it and it keeps living,
    /// which is the whole point of a recency policy.
    #[test]
    fn surviving_one_trim_is_not_a_permanent_reprieve() {
        let cache = RenderCache::default();
        for index in 0..CAPACITY {
            cache.put(&format!("old{index}"), 1, body("old"));
        }
        assert!(cache.get("old0", 1).is_some());

        // Two trims' worth of churn, with old0 never touched again.
        for index in 0..(CAPACITY * 2) {
            cache.put(&format!("new{index}"), 1, body("new"));
        }
        assert!(
            cache.get("old0", 1).is_none(),
            "after long disuse it is correctly gone"
        );
        assert!(cache.len() <= CAPACITY);
    }
}
