//! A disk cache for file attachments.
//!
//! Separate from `media::MediaCache`, which holds avatars and emoji in memory,
//! because attachments are a different size class: the images already in this
//! account's channels run 474 KB and 626 KB, and the server allows 150 MB. A few
//! screenshots would evict every avatar from a memory cache, and re-downloading
//! them on every scroll past is exactly the cost this avoids.
//!
//! On disk rather than in memory for the same reason it is worth caching at all:
//! an attachment does not change under its id -- Mattermost never rewrites a
//! file, it only archives it -- so a byte fetched once is good for as long as
//! the file exists, including across restarts.
//!
//! One file per entry, with the content type in a small header, so an entry is
//! a single atomic rename and there is no index file to keep in step with the
//! directory.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// How much disk the cache may hold. Roughly a working week of screenshots, and
/// small enough not to be noticed on a developer machine.
const BUDGET_BYTES: u64 = 512 * 1024 * 1024;

/// Nothing larger than this is cached at all. A 150 MB video would evict most
/// of the cache to store something that is almost certainly watched once, and
/// the fetch path works perfectly well without a cache entry.
const LARGEST_CACHEABLE: u64 = BUDGET_BYTES / 8;

pub struct FileCache {
    root: PathBuf,
    entries: Mutex<HashMap<String, Entry>>,
    /// A monotonic stamp, not a clock: all that matters is the order entries
    /// were last touched, and a clock can go backwards.
    tick: AtomicU64,
}

struct Entry {
    bytes: u64,
    used: u64,
}

impl FileCache {
    /// Opens the cache in `root`, adopting whatever is already there.
    ///
    /// Adopting rather than clearing is the point of a disk cache: the entries
    /// from the last run are still valid, because a file's bytes never change
    /// under its id.
    pub fn open(root: PathBuf) -> Self {
        let cache = Self {
            root,
            entries: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(1),
        };
        cache.seed();
        cache
    }

    fn seed(&self) {
        if let Err(error) = std::fs::create_dir_all(&self.root) {
            tracing::warn!(%error, root = %self.root.display(), "file cache unavailable");
            return;
        }
        let listing = match std::fs::read_dir(&self.root) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::warn!(%error, "file cache cannot be listed");
                return;
            }
        };

        // Ordered by modification time so the recency the last run learned
        // survives the restart, rather than every entry starting out equal.
        let mut found: Vec<(String, u64, std::time::SystemTime)> = Vec::new();
        for item in listing.flatten() {
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Some(key) = item.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            found.push((key, metadata.len(), modified));
        }
        found.sort_by_key(|(_, _, modified)| *modified);

        let mut entries = self.entries.lock().expect("file cache");
        let mut total = 0u64;
        for (key, bytes, _) in found {
            total += bytes;
            let used = self.tick.fetch_add(1, Ordering::Relaxed);
            entries.insert(key, Entry { bytes, used });
        }
        tracing::info!(
            entries = entries.len(),
            megabytes = total / (1024 * 1024),
            "file cache opened"
        );
    }

    /// The cached bytes and their content type, if this key is held.
    pub fn read(&self, key: &str) -> Option<(Vec<u8>, String)> {
        {
            let mut entries = self.entries.lock().expect("file cache");
            let entry = entries.get_mut(key)?;
            entry.used = self.tick.fetch_add(1, Ordering::Relaxed);
        }
        match decode(&self.path_for(key)) {
            Ok(held) => Some(held),
            Err(error) => {
                // A truncated entry is a cache miss, not a failure: drop it and
                // let the caller fetch. Happens if a write was interrupted.
                tracing::warn!(key, %error, "cache entry unreadable, dropping");
                self.forget(key);
                None
            }
        }
    }

    /// Stores bytes under `key`, evicting least-recently-used entries to stay
    /// inside the budget. Failures are logged and ignored -- a cache that cannot
    /// write is slow, not broken.
    pub fn write(&self, key: &str, bytes: &[u8], content_type: &str) {
        let size = bytes.len() as u64;
        if size > LARGEST_CACHEABLE {
            tracing::debug!(key, megabytes = size / (1024 * 1024), "too large to cache");
            return;
        }
        if let Err(error) = self.evict_for(size) {
            tracing::warn!(key, %error, "cache eviction failed");
        }
        if let Err(error) = encode(&self.path_for(key), bytes, content_type) {
            tracing::warn!(key, %error, "cache write failed");
            return;
        }
        let used = self.tick.fetch_add(1, Ordering::Relaxed);
        self.entries
            .lock()
            .expect("file cache")
            .insert(key.to_string(), Entry { bytes: size, used });
    }

    fn forget(&self, key: &str) {
        self.entries.lock().expect("file cache").remove(key);
        let _ = std::fs::remove_file(self.path_for(key));
    }

    fn evict_for(&self, incoming: u64) -> std::io::Result<()> {
        let doomed: Vec<String> = {
            let entries = self.entries.lock().expect("file cache");
            let mut total: u64 = entries.values().map(|entry| entry.bytes).sum();
            if total + incoming <= BUDGET_BYTES {
                return Ok(());
            }
            let mut order: Vec<(&String, &Entry)> = entries.iter().collect();
            order.sort_by_key(|(_, entry)| entry.used);
            let mut chosen = Vec::new();
            for (key, entry) in order {
                if total + incoming <= BUDGET_BYTES {
                    break;
                }
                total = total.saturating_sub(entry.bytes);
                chosen.push(key.clone());
            }
            chosen
        };
        if !doomed.is_empty() {
            tracing::debug!(evicted = doomed.len(), "file cache made room");
        }
        for key in doomed {
            self.forget(&key);
        }
        Ok(())
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
}

/// `[content type length: u32 LE][content type][bytes]`.
///
/// The content type travels with the bytes because the handler has to answer
/// with it and cannot re-derive it: `/thumbnail` is a JPEG whatever the original
/// was, and the original's type is only known to the post's metadata.
fn encode(path: &Path, bytes: &[u8], content_type: &str) -> std::io::Result<()> {
    let label = content_type.as_bytes();
    let length = u32::try_from(label.len()).unwrap_or(0);
    // Written beside the target and renamed, so an interrupted write cannot be
    // mistaken for a complete entry.
    let temporary = path.with_extension("partial");
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&length.to_le_bytes())?;
        file.write_all(&label[..length as usize])?;
        file.write_all(bytes)?;
        file.flush()?;
    }
    std::fs::rename(&temporary, path)
}

fn decode(path: &Path) -> std::io::Result<(Vec<u8>, String)> {
    let mut file = std::fs::File::open(path)?;
    let mut length = [0u8; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 255 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "implausible content type length",
        ));
    }
    let mut label = vec![0u8; length];
    file.read_exact(&mut label)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok((bytes, String::from_utf8_lossy(&label).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("matterless-filecache-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_written_entry_reads_back_with_its_content_type() {
        let cache = FileCache::open(scratch("roundtrip"));
        cache.write("file-abc", b"the bytes", "image/png");
        let (bytes, content_type) = cache.read("file-abc").expect("held");
        assert_eq!(bytes, b"the bytes");
        assert_eq!(content_type, "image/png");
        assert!(cache.read("file-missing").is_none());
    }

    #[test]
    fn entries_survive_reopening() {
        let root = scratch("reopen");
        {
            let cache = FileCache::open(root.clone());
            cache.write("thumb-one", b"jpeg", "image/jpeg");
        }
        let reopened = FileCache::open(root);
        let (bytes, content_type) = reopened.read("thumb-one").expect("adopted");
        assert_eq!(bytes, b"jpeg");
        assert_eq!(content_type, "image/jpeg");
    }

    #[test]
    fn a_truncated_entry_is_a_miss_not_a_failure() {
        let root = scratch("truncated");
        let cache = FileCache::open(root.clone());
        cache.write("file-broken", b"payload", "image/png");
        std::fs::write(root.join("file-broken"), b"\x02").expect("truncate");
        assert!(cache.read("file-broken").is_none());
        // And it is gone, rather than failing again on the next request.
        assert!(!root.join("file-broken").exists());
    }

    #[test]
    fn the_least_recently_used_entry_goes_first() {
        let root = scratch("lru");
        let cache = FileCache::open(root.clone());
        // A budget-sized eviction would need half a gigabyte of test data, so
        // the ordering is asserted directly instead: it is the part that has a
        // choice to make.
        cache.write("file-a", b"a", "text/plain");
        cache.write("file-b", b"b", "text/plain");
        cache.write("file-c", b"c", "text/plain");
        // Touching A makes B the oldest.
        let _ = cache.read("file-a");

        let entries = cache.entries.lock().expect("entries");
        let mut order: Vec<(&String, u64)> = entries
            .iter()
            .map(|(key, entry)| (key, entry.used))
            .collect();
        order.sort_by_key(|(_, used)| *used);
        assert_eq!(order.first().expect("oldest").0, "file-b");
        assert_eq!(order.last().expect("newest").0, "file-a");
    }
}
