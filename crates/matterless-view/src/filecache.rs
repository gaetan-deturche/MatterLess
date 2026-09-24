//! Pictures kept on disk between runs.
//!
//! Recovered from the shell this window replaced, where it cached attachments.
//! It caches everything now: an avatar, a team icon, a custom emoji and a
//! message's pictures all arrive through one fetch, and without this every one
//! of them is downloaded again on every start -- about fifty requests before
//! the first face appears.
//!
//! Worth caching at all because none of it changes under its key. A file is
//! never rewritten by Mattermost, only archived, and an avatar's key carries
//! the moment it was last changed -- so a new picture is a new key rather than
//! new bytes at an old one.
//!
//! One file per entry with the content type in a small header, so an entry is a
//! single atomic rename and there is no index to keep in step with the
//! directory -- except for the small ones, which are the great majority and go
//! in one packed file instead. See `pack`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// How much disk the cache may hold.
///
/// Smaller than the shell's, which held attachments alone at half a gigabyte.
/// What goes through here is mostly faces and emoji -- thousands of them fit in
/// a fraction of this -- and the pictures that do not fit are the ones nobody
/// scrolls back to.
const BUDGET_BYTES: u64 = 256 * 1024 * 1024;

pub struct FileCache {
    root: PathBuf,
    /// Where everything small lives. `None` only when it could not be opened,
    /// which costs speed rather than correctness: entries go one to a file.
    small: Option<crate::pack::Pack>,
    /// How much it may hold. A field rather than the constant so the rule can
    /// be tested without writing a quarter of a gigabyte to find out.
    budget: u64,
    entries: Mutex<HashMap<String, Entry>>,
    /// A monotonic stamp, not a clock: all that matters is the order entries
    /// were last touched, and a clock can go backwards.
    tick: AtomicU64,
}

struct Entry {
    bytes: u64,
    used: u64,
    /// In the packed file rather than one of its own.
    packed: bool,
}

/// The one cache this process has.
///
/// One rather than several because the pack belongs to whoever opened it: a
/// second cache here would get it read-only and quietly stop packing anything.
/// `None` when there is nowhere to put it, which is slow rather than broken.
pub fn shared() -> Option<std::sync::Arc<FileCache>> {
    static ONE: std::sync::OnceLock<Option<std::sync::Arc<FileCache>>> = std::sync::OnceLock::new();
    ONE.get_or_init(|| {
        crate::feed::pictures_dir()
            .map(FileCache::open)
            .map(std::sync::Arc::new)
    })
    .clone()
}

impl FileCache {
    /// Opens the cache in `root`, adopting whatever is already there.
    ///
    /// Adopting rather than clearing is the point of a disk cache: the entries
    /// from the last run are still good, because nothing changes under its key.
    pub fn open(root: PathBuf) -> Self {
        Self::holding(root, BUDGET_BYTES)
    }

    /// The same, to a given size.
    pub fn holding(root: PathBuf, budget: u64) -> Self {
        if let Err(error) = std::fs::create_dir_all(&root) {
            eprintln!("no picture cache at {}: {error}", root.display());
        }
        let small = match crate::pack::Pack::open(crate::pack::beside(&root)) {
            Ok(pack) => Some(pack),
            Err(error) => {
                eprintln!("the packed pictures could not be opened ({error}), one file each");
                None
            }
        };
        let cache = Self {
            root,
            small,
            budget,
            entries: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(1),
        };
        cache.seed();
        cache.fold();
        cache
    }

    /// The largest thing worth keeping. A 150MB video would evict most of the
    /// cache to store something that is almost certainly watched once, and the
    /// fetch path works perfectly well without an entry.
    fn largest(&self) -> u64 {
        self.budget / 8
    }

    fn seed(&self) {
        if let Err(error) = std::fs::create_dir_all(&self.root) {
            eprintln!("no picture cache at {}: {error}", self.root.display());
            return;
        }
        let listing = match std::fs::read_dir(&self.root) {
            Ok(listing) => listing,
            Err(error) => return eprintln!("the picture cache cannot be listed: {error}"),
        };

        // Ordered by modification time so the recency the last run learned
        // survives the restart, rather than every entry starting out equal.
        let mut found: Vec<(String, u64, std::time::SystemTime)> = Vec::new();
        for item in listing.flatten() {
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            let Some(name) = item.file_name().to_str().map(str::to_string) else {
                continue;
            };
            // A write that was interrupted, which the next one will replace.
            if !metadata.is_file() || name.ends_with(PARTIAL) {
                continue;
            }
            // The packed file is the cache, not an entry in it.
            if crate::pack::beside(&self.root) == item.path() || name.starts_with(PACKED_NAME) {
                continue;
            }
            let Some(key) = key_of(&name) else {
                continue;
            };
            let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            found.push((key, metadata.len(), modified));
        }
        found.sort_by_key(|(_, _, modified)| *modified);

        let mut entries = self.entries.lock().expect("the picture cache");
        let mut total = 0u64;
        for (key, bytes, _) in found {
            total += bytes;
            let used = self.tick.fetch_add(1, Ordering::Relaxed);
            entries.insert(
                key,
                Entry {
                    bytes,
                    used,
                    packed: false,
                },
            );
        }
        // The pack keeps its own recency, so these carry theirs rather than
        // being renumbered by the order they happen to come back in.
        if let Some(pack) = self.small.as_ref() {
            let mut highest = 0;
            for (key, bytes, used) in pack.listing() {
                total += bytes;
                highest = highest.max(used);
                entries.insert(
                    key,
                    Entry {
                        bytes,
                        used,
                        packed: true,
                    },
                );
            }
            self.tick.fetch_max(highest + 1, Ordering::Relaxed);
        }
        if !entries.is_empty() {
            println!(
                "{} pictures cached, {}MB",
                entries.len(),
                total / (1024 * 1024)
            );
        }
    }

    /// Moves the small loose files into the pack, once.
    ///
    /// Without this the pack only fills as pictures are fetched again -- and a
    /// face is never fetched again, because its key carries the moment it was
    /// last changed. A cache from before the pack existed would stay one file
    /// per face forever.
    fn fold(&self) {
        let Some(pack) = self.small.as_ref().filter(|pack| pack.writable()) else {
            return;
        };
        let loose: Vec<(String, u64)> = {
            let entries = self.entries.lock().expect("the picture cache");
            entries
                .iter()
                .filter(|(_, entry)| !entry.packed && entry.bytes as usize <= crate::pack::LARGEST)
                .map(|(key, entry)| (key.clone(), entry.used))
                .collect()
        };
        if loose.is_empty() {
            return;
        }
        let mut moved = 0;
        for (key, used) in loose {
            let Ok((bytes, content_type)) = decode(&self.path_for(&key)) else {
                continue;
            };
            pack.write(&key, &bytes, &content_type, used);
            // Only once the pack holds it: a crash between the two leaves the
            // file behind, which is a duplicate, not a loss.
            let _ = std::fs::remove_file(self.path_for(&key));
            if let Some(entry) = self
                .entries
                .lock()
                .expect("the picture cache")
                .get_mut(&key)
            {
                entry.packed = true;
            }
            moved += 1;
        }
        if moved > 0 {
            println!("{moved} pictures packed into one file");
        }
    }

    /// The cached bytes and their content type, if this key is held.
    ///
    /// The file is tried even for a key this instance has never heard of. Two
    /// of these are open on the same directory -- the shelf reads, the socket
    /// thread writes -- and each seeded its list when it opened, so anything
    /// written since would otherwise look like a miss and be fetched again.
    /// That is exactly what the warm-up writes.
    pub fn read(&self, key: &str) -> Option<(Vec<u8>, String)> {
        let used = self.tick.fetch_add(1, Ordering::Relaxed);
        let known = {
            let mut entries = self.entries.lock().expect("the picture cache");
            match entries.get_mut(key) {
                Some(entry) => {
                    entry.used = used;
                    Some(entry.packed)
                }
                None => None,
            }
        };
        if known != Some(false)
            && let Some(pack) = self.small.as_ref()
            && let Some(held) = pack.read(key)
        {
            pack.touch(key, used);
            if known.is_none() {
                self.adopt(key, held.0.len() as u64, true);
            }
            return Some(held);
        }
        let known = known.is_some();
        match decode(&self.path_for(key)) {
            Ok(held) => {
                if !known {
                    self.adopt(key, held.0.len() as u64, false);
                }
                Some(held)
            }
            // Never claimed to hold it, so its absence is the ordinary answer
            // and worth nothing said.
            Err(_) if !known => None,
            Err(error) => {
                // A truncated entry is a miss, not a failure: drop it and let
                // the caller fetch. Happens if a write was interrupted.
                eprintln!("{key} was cached but will not read back ({error}), fetching it again");
                self.forget(key);
                None
            }
        }
    }

    /// The newest version held of a picture whose exact version is not known.
    ///
    /// An avatar's key carries the moment it was last changed, and the window
    /// asks with a zero when the store has not heard of that person yet -- so
    /// a face already on this disk misses, goes to the server, and is fetched
    /// a second time the moment the person's record lands. This answers the
    /// first ask with the face already held.
    pub fn read_like(&self, stem: &str) -> Option<(Vec<u8>, String)> {
        let newest = {
            let entries = self.entries.lock().expect("the picture cache");
            entries
                .keys()
                .filter(|key| key.starts_with(stem))
                .max_by_key(|key| key[stem.len()..].parse::<i64>().unwrap_or(0))
                .cloned()?
        };
        self.read(&newest)
    }

    /// Takes an entry another instance wrote into this one's list.
    fn adopt(&self, key: &str, bytes: u64, packed: bool) {
        let used = self.tick.fetch_add(1, Ordering::Relaxed);
        self.entries.lock().expect("the picture cache").insert(
            key.to_string(),
            Entry {
                bytes,
                used,
                packed,
            },
        );
    }

    /// Stores bytes under `key`, evicting the least recently used to stay
    /// inside the budget.
    ///
    /// Failures are said once and ignored: a cache that cannot write is slow,
    /// not broken.
    pub fn write(&self, key: &str, bytes: &[u8], content_type: &str) {
        let size = bytes.len() as u64;
        if size > self.largest() {
            return;
        }
        self.evict_for(size);
        let used = self.tick.fetch_add(1, Ordering::Relaxed);
        let packed = bytes.len() <= crate::pack::LARGEST
            && self.small.as_ref().is_some_and(crate::pack::Pack::writable);
        if packed {
            if let Some(pack) = self.small.as_ref() {
                pack.write(key, bytes, content_type, used);
            }
        } else if let Err(error) = encode(&self.path_for(key), bytes, content_type) {
            eprintln!("{key} could not be cached: {error}");
            return;
        }
        self.entries.lock().expect("the picture cache").insert(
            key.to_string(),
            Entry {
                bytes: size,
                used,
                packed,
            },
        );
    }

    /// Whether this key is held, without reading it.
    pub fn holds(&self, key: &str) -> bool {
        self.entries
            .lock()
            .expect("the picture cache")
            .contains_key(key)
    }

    /// How many entries it holds, for whoever wants to say so.
    pub fn held(&self) -> usize {
        self.entries.lock().expect("the picture cache").len()
    }

    fn forget(&self, key: &str) {
        let gone = self.entries.lock().expect("the picture cache").remove(key);
        match gone {
            Some(entry) if entry.packed => {
                if let Some(pack) = self.small.as_ref() {
                    pack.forget(key);
                }
            }
            _ => {
                let _ = std::fs::remove_file(self.path_for(key));
            }
        }
    }

    fn evict_for(&self, incoming: u64) {
        let doomed: Vec<String> = {
            let entries = self.entries.lock().expect("the picture cache");
            let mut total: u64 = entries.values().map(|entry| entry.bytes).sum();
            if total + incoming <= self.budget {
                return;
            }
            let mut order: Vec<(&String, &Entry)> = entries.iter().collect();
            order.sort_by_key(|(_, entry)| entry.used);
            let mut chosen = Vec::new();
            for (key, entry) in order {
                if total + incoming <= self.budget {
                    break;
                }
                total = total.saturating_sub(entry.bytes);
                chosen.push(key.clone());
            }
            chosen
        };
        for key in doomed {
            self.forget(&key);
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(name_of(key))
    }
}

/// What an interrupted write is called while it is being written.
const PARTIAL: &str = ".partial";

/// The packed file, and anything left beside it mid-rewrite.
const PACKED_NAME: &str = "packed";

/// One flat filename for a key, and back again.
///
/// The keys here have slashes in them -- `avatar/<person>/<when>` -- so a key
/// cannot be a path: joining one would name a directory that does not exist and
/// every write would fail. Escaped rather than hashed, because `seed` reads the
/// directory at startup and has to recover the key from the name, and because
/// two keys that differ must not become one file.
fn name_of(key: &str) -> String {
    let mut name = String::with_capacity(key.len());
    for byte in key.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => name.push(*byte as char),
            other => name.push_str(&format!("%{other:02X}")),
        }
    }
    name
}

/// The key a filename came from, or `None` if it did not come from one.
fn key_of(name: &str) -> Option<String> {
    let mut key = Vec::with_capacity(name.len());
    let mut bytes = name.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                let pair: String = [bytes.next()? as char, bytes.next()? as char]
                    .iter()
                    .collect();
                key.push(u8::from_str_radix(&pair, 16).ok()?);
            }
            other => key.push(other),
        }
    }
    String::from_utf8(key).ok()
}

/// `[content type length: u32 LE][content type][bytes]`.
///
/// The content type travels with the bytes because it cannot be re-derived: a
/// thumbnail is a JPEG whatever the original was, and the original's type is
/// only known to the post's metadata.
fn encode(path: &Path, bytes: &[u8], content_type: &str) -> std::io::Result<()> {
    let label = content_type.as_bytes();
    let length = u32::try_from(label.len()).unwrap_or(0);
    // Written beside the target and renamed, so an interrupted write cannot be
    // mistaken for a complete entry. Appended rather than `with_extension`: a
    // key may have a dot in it, and that would replace what follows it.
    let temporary = PathBuf::from(format!("{}{PARTIAL}", path.display()));
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
        let root = std::env::temp_dir().join(format!("matterless-pictures-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_written_entry_reads_back_with_its_content_type() {
        let cache = FileCache::open(scratch("roundtrip"));
        cache.write("avatar/u1/7", b"the bytes", "image/png");
        let (bytes, content_type) = cache.read("avatar/u1/7").expect("held");
        assert_eq!(bytes, b"the bytes");
        assert_eq!(content_type, "image/png");
        assert!(cache.read("avatar/u1/8").is_none(), "a different key");
    }

    /// The whole point: what was fetched last time is still here this time.
    #[test]
    fn entries_survive_reopening() {
        let root = scratch("reopen");
        {
            let cache = FileCache::open(root.clone());
            cache.write("emoji/abc", b"a picture", "image/webp");
        }
        let again = FileCache::open(root);
        assert_eq!(again.held(), 1);
        assert_eq!(again.read("emoji/abc").expect("held").0, b"a picture");
    }

    /// A key is not a path. `avatar/u1/7` has to become one flat file, and two
    /// keys that differ must not become the same one.
    #[test]
    fn a_key_with_slashes_becomes_one_file_and_comes_back() {
        for key in [
            "avatar/u1/7",
            "emoji/abc",
            "mini/file-1/thumbnail",
            "team/t1/0",
            "a_b",
            "a/b",
            "a%b",
            "\u{e9}t\u{e9}",
        ] {
            let name = name_of(key);
            assert!(!name.contains('/'), "{name} is still a path");
            assert_eq!(key_of(&name).as_deref(), Some(key), "{key} did not survive");
        }
        // The pair that would collide under a naive replacement.
        assert_ne!(name_of("a/b"), name_of("a_b"));
    }

    /// A write that was interrupted is a miss, not a crash, and it does not
    /// stay in the way of the fetch that replaces it.
    ///
    /// Big enough to have a file of its own: that is what an interrupted write
    /// can leave behind now, since a small one goes in the pack.
    #[test]
    fn a_truncated_entry_is_a_miss_not_a_failure() {
        let root = scratch("truncated");
        let cache = FileCache::open(root.clone());
        cache.write(
            "file/one",
            &vec![3u8; crate::pack::LARGEST + 1],
            "image/png",
        );
        std::fs::write(root.join(name_of("file/one")), b"xx").expect("truncate");
        assert!(cache.read("file/one").is_none());
        // And it was dropped rather than left to fail again.
        assert_eq!(cache.held(), 0);
        assert!(!root.join(name_of("file/one")).exists());
    }

    /// Past the budget, the entry nobody has asked for in longest goes first.
    ///
    /// Eight entries' worth, and each entry exactly the largest one allowed --
    /// a budget so small that `largest` rejects the test's own entries is a
    /// test that proves nothing, which is what the first attempt at this did.
    #[test]
    fn the_least_recently_used_entry_goes_first() {
        let root = scratch("evict");
        let cache = FileCache::holding(root, 64 * 1024);
        let one = vec![0u8; 8 * 1024];
        for at in 0..8 {
            cache.write(&format!("file/{at}"), &one, "image/png");
        }
        assert_eq!(cache.held(), 8, "all of them fit");

        // Asked for again, so it is no longer the one used longest ago.
        assert!(cache.read("file/0").is_some());
        for at in 8..10 {
            cache.write(&format!("file/{at}"), &one, "image/png");
        }
        assert!(
            cache.read("file/0").is_some(),
            "the one that was used again"
        );
        assert!(cache.read("file/1").is_none(), "the two that were not");
        assert!(cache.read("file/2").is_none());
        assert!(cache.read("file/9").is_some(), "and the newest is held");
    }

    /// Something enormous is not worth the room it would cost.
    #[test]
    fn something_too_big_is_not_cached() {
        let cache = FileCache::holding(scratch("toobig"), 8 * 1024);
        cache.write("file/huge", &vec![0u8; 1024 + 1], "video/mp4");
        assert_eq!(cache.held(), 0);
        assert!(cache.read("file/huge").is_none());
    }

    /// The shelf and the socket thread each hold one of these on the same
    /// directory. Whatever one writes, the other has to be able to read --
    /// otherwise the warm-up fills a disk the reader never looks at.
    #[test]
    fn one_instance_reads_what_another_wrote_after_it_opened() {
        let root = scratch("shared");
        let reading = FileCache::open(root.clone());
        let writing = FileCache::open(root);
        assert!(reading.read("avatar/late").is_none(), "nothing there yet");
        // Big enough to be a file of its own: the pack belongs to whichever of
        // the two opened it first, and this is about the loose files.
        writing.write(
            "avatar/late",
            &vec![9u8; crate::pack::LARGEST + 1],
            "image/png",
        );
        let (bytes, kind) = reading.read("avatar/late").expect("written since");
        assert_eq!(bytes.len(), crate::pack::LARGEST + 1);
        assert_eq!(kind, "image/png");
        assert_eq!(reading.held(), 1, "and taken into its own list");
    }

    /// The window asks with a zero when it does not know which version is
    /// current. The face is on this disk under the version it does know.
    #[test]
    fn a_face_is_found_under_another_version() {
        let cache = FileCache::open(scratch("versions"));
        cache.write("avatar/u1?v=100", b"the old face", "image/png");
        cache.write("avatar/u1?v=900", b"the new face", "image/png");
        cache.write("avatar/u2?v=500", b"somebody else", "image/png");
        assert!(cache.read("avatar/u1?v=0").is_none(), "not under that key");
        let (bytes, _) = cache.read_like("avatar/u1?v=").expect("held under another");
        assert_eq!(bytes, b"the new face", "the newest of the two");
        assert!(cache.read_like("avatar/u3?v=").is_none(), "nobody");
    }
}
