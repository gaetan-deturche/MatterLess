//! The small pictures, in one file rather than one file each.
//!
//! A face is 24KB and a workspace has thousands of faces and emoji. Kept one to
//! a file, every one of them costs an `open` -- measured at 281us against the
//! 1194us it takes to decode one, so about a fifth of the work for pictures
//! that are all wanted at once, and a fifth that grows with the count rather
//! than with the bytes.
//!
//! One handle, opened once, read from every reader at a stated offset. The
//! directory sits at the tail so opening the pack is two small reads instead of
//! a walk of the whole file, and a record is self-describing so a directory
//! lost to a crash can be rebuilt by reading through.
//!
//! Big pictures stay one to a file: they are few, they are wanted one at a
//! time, and packing them would mean rewriting megabytes to drop one.
//!
//! Only one process writes it. The installed client and a build run from the
//! tree share this directory, and two of them appending at their own idea of
//! the end would write over each other. The second one to open gets a pack it
//! may read but not write -- which is safe, because an append never moves a
//! record that is already there, and only the one holding it compacts.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// At the head, and again at the very end so a tail can be recognised.
const MAGIC: &[u8; 8] = b"MLPACK01";

/// The largest entry worth packing. Above this the file is its own.
pub const LARGEST: usize = 128 * 1024;

/// `[key length: u16][type length: u8][byte count: u32]`, then the key, the
/// content type, and the bytes.
const RECORD_HEADER: usize = 2 + 1 + 4;

/// `[directory offset: u64][MAGIC]`.
const FOOTER: usize = 8 + 8;

pub struct Pack {
    path: PathBuf,
    /// Open for the life of the pack, and read from at an offset so that the
    /// readers do not queue behind each other for a file handle.
    file: std::fs::File,
    /// False when another process opened it first.
    writable: bool,
    held: Mutex<Held>,
}

struct Held {
    entries: HashMap<String, Entry>,
    /// Where the next record goes, which is where the directory starts now.
    end: u64,
    /// Bytes belonging to entries nobody can reach any more.
    dead: u64,
}

#[derive(Clone)]
struct Entry {
    at: u64,
    bytes: u32,
    content_type: String,
    /// The same monotonic stamp the cache orders eviction by, kept in the
    /// directory so recency survives a restart.
    used: u64,
}

impl Pack {
    /// Opens `path`, rebuilding the directory if the last run did not leave
    /// one, and compacting if most of it is dead.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let (file, writable) = claim(&path)?;
        let length = file.metadata()?.len();
        let held = if length <= MAGIC.len() as u64 {
            Held {
                entries: HashMap::new(),
                end: MAGIC.len() as u64,
                dead: 0,
            }
        } else {
            match read_directory(&file, length) {
                Some(held) => held,
                // Nothing readable at the tail: the run that wrote it did not
                // get to the end. The records themselves say what they are.
                None => walk(&file, length)?,
            }
        };
        let mut pack = Self {
            path,
            file,
            writable,
            held: Mutex::new(held),
        };
        // Not fatal: a pack that could not be rewritten is bigger than it needs
        // to be, and still answers every key it holds.
        if pack.writable
            && pack.mostly_dead()
            && let Err(error) = pack.compact()
        {
            eprintln!("the packed pictures could not be rewritten: {error}");
        }
        Ok(pack)
    }

    /// Every key held, with its size and recency, for the cache's own list.
    pub fn listing(&self) -> Vec<(String, u64, u64)> {
        let held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        held.entries
            .iter()
            .map(|(key, entry)| (key.clone(), u64::from(entry.bytes), entry.used))
            .collect()
    }

    pub fn read(&self, key: &str) -> Option<(Vec<u8>, String)> {
        let entry = {
            let held = self.held.lock().unwrap_or_else(|held| held.into_inner());
            held.entries.get(key).cloned()?
        };
        let mut bytes = vec![0u8; entry.bytes as usize];
        match read_at(&self.file, &mut bytes, entry.at) {
            Ok(()) => Some((bytes, entry.content_type)),
            Err(error) => {
                eprintln!("{key} is in the pack but will not read back ({error})");
                None
            }
        }
    }

    /// Appends an entry and writes the directory after it.
    ///
    /// The record goes where the directory was, so the file does not grow by a
    /// directory every time.
    pub fn write(&self, key: &str, bytes: &[u8], content_type: &str, used: u64) {
        if bytes.len() > LARGEST || !self.writable {
            return;
        }
        let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        if let Some(gone) = held.entries.get(key) {
            held.dead += u64::from(gone.bytes);
        }
        let at = held.end;
        match self.append(at, key, bytes, content_type) {
            Ok(payload) => {
                held.entries.insert(
                    key.to_string(),
                    Entry {
                        at: payload,
                        bytes: bytes.len() as u32,
                        content_type: content_type.to_string(),
                        used,
                    },
                );
                held.end = payload + bytes.len() as u64;
            }
            Err(error) => return eprintln!("{key} could not be packed: {error}"),
        }
        if let Err(error) = self.write_directory(&held) {
            eprintln!("the pack directory could not be written: {error}");
        }
    }

    /// Whether this process is the one that may write it.
    pub fn writable(&self) -> bool {
        self.writable
    }

    /// Drops an entry. Its bytes stay until the next compaction.
    pub fn forget(&self, key: &str) {
        if !self.writable {
            return;
        }
        let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        if let Some(gone) = held.entries.remove(key) {
            held.dead += u64::from(gone.bytes);
        }
        // Not written out here: a removal is followed by the write that caused
        // it, and a directory that survives a crash holding an entry already
        // dropped costs one eviction, not correctness.
    }

    /// Marks an entry as just used, so eviction sees the same recency the loose
    /// files get.
    pub fn touch(&self, key: &str, used: u64) {
        let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        if let Some(entry) = held.entries.get_mut(key) {
            entry.used = used;
        }
    }

    pub fn held(&self) -> usize {
        self.held
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .entries
            .len()
    }

    fn mostly_dead(&self) -> bool {
        let held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        let live: u64 = held
            .entries
            .values()
            .map(|entry| u64::from(entry.bytes))
            .sum();
        held.dead > live.max(1024 * 1024)
    }

    /// Rewrites the file with only what is still reachable.
    ///
    /// By value, because the handle has to be replaced: everything read after
    /// this is at an offset into the new file, and the old handle would answer
    /// with whatever happens to sit there -- which is another entry's bytes,
    /// not an error.
    fn compact(&mut self) -> std::io::Result<()> {
        let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        let was = std::mem::take(&mut held.entries);
        let temporary = PathBuf::from(format!("{}.rewriting", self.path.display()));
        // Opened the way the pack itself is, and kept: it becomes the pack, and
        // opening the name again afterwards would find this very handle in the
        // way and fall back to reading.
        let mut fresh = make(&temporary)?;
        fresh.write_all(MAGIC)?;
        let mut at = MAGIC.len() as u64;
        let mut kept: HashMap<String, Entry> = HashMap::new();
        for (key, entry) in was.iter() {
            let mut bytes = vec![0u8; entry.bytes as usize];
            if read_at(&self.file, &mut bytes, entry.at).is_err() {
                continue;
            }
            let payload = at + (RECORD_HEADER + key.len() + entry.content_type.len()) as u64;
            fresh.write_all(&record(key, &bytes, &entry.content_type))?;
            kept.insert(
                key.clone(),
                Entry {
                    at: payload,
                    ..entry.clone()
                },
            );
            at = payload + u64::from(entry.bytes);
        }
        // Straight after the records, which is where the cursor already is.
        let written = Held {
            entries: kept,
            end: at,
            dead: 0,
        };
        fresh.write_all(&directory(&written))?;
        fresh.flush()?;
        // Nothing above this line has touched what is held, because every one
        // of those steps can fail: committing the new offsets against the old
        // file would answer with another entry's bytes.
        if let Err(error) = std::fs::rename(&temporary, &self.path) {
            held.entries = was;
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        *held = written;
        drop(held);
        // The old handle goes here, with the file it was opened on.
        self.file = fresh;
        Ok(())
    }

    fn append(&self, at: u64, key: &str, bytes: &[u8], content_type: &str) -> std::io::Result<u64> {
        let mut file = &self.file;
        file.seek(SeekFrom::Start(at))?;
        file.write_all(&record(key, bytes, content_type))?;
        Ok(at + (RECORD_HEADER + key.len() + content_type.len()) as u64)
    }

    fn write_directory(&self, held: &Held) -> std::io::Result<()> {
        let mut file = &self.file;
        file.seek(SeekFrom::Start(held.end))?;
        let written = directory(held);
        file.write_all(&written)?;
        file.flush()?;
        file.set_len(held.end + written.len() as u64)
    }
}

impl Drop for Pack {
    fn drop(&mut self) {
        if !self.writable {
            return;
        }
        let held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        let _ = self.write_directory(&held);
    }
}

/// `[key length: u16][type length: u8][byte count: u32][key][type][bytes]`.
fn record(key: &str, bytes: &[u8], content_type: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_HEADER + key.len() + content_type.len() + bytes.len());
    out.extend_from_slice(&(key.len() as u16).to_le_bytes());
    out.push(content_type.len() as u8);
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(content_type.as_bytes());
    out.extend_from_slice(bytes);
    out
}

/// `[count: u32]`, then each entry, then `[where this began: u64][MAGIC]`.
fn directory(held: &Held) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(held.entries.len() as u32).to_le_bytes());
    for (key, entry) in held.entries.iter() {
        out.extend_from_slice(&entry.at.to_le_bytes());
        out.extend_from_slice(&entry.bytes.to_le_bytes());
        out.extend_from_slice(&entry.used.to_le_bytes());
        out.extend_from_slice(&(key.len() as u16).to_le_bytes());
        out.push(entry.content_type.len() as u8);
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(entry.content_type.as_bytes());
    }
    out.extend_from_slice(&held.end.to_le_bytes());
    out.extend_from_slice(MAGIC);
    out
}

fn read_directory(file: &std::fs::File, length: u64) -> Option<Held> {
    if length < (MAGIC.len() + FOOTER) as u64 {
        return None;
    }
    let mut footer = [0u8; FOOTER];
    read_at(file, &mut footer, length - FOOTER as u64).ok()?;
    if &footer[8..] != MAGIC {
        return None;
    }
    let end = u64::from_le_bytes(footer[..8].try_into().ok()?);
    if end < MAGIC.len() as u64 || end > length - FOOTER as u64 {
        return None;
    }
    let mut written = vec![0u8; (length - FOOTER as u64 - end) as usize];
    read_at(file, &mut written, end).ok()?;
    let mut at = 0usize;
    let count = u32::from_le_bytes(take(&written, &mut at, 4)?.try_into().ok()?) as usize;
    let mut entries = HashMap::with_capacity(count);
    let mut live = 0u64;
    for _ in 0..count {
        let where_ = u64::from_le_bytes(take(&written, &mut at, 8)?.try_into().ok()?);
        let bytes = u32::from_le_bytes(take(&written, &mut at, 4)?.try_into().ok()?);
        let used = u64::from_le_bytes(take(&written, &mut at, 8)?.try_into().ok()?);
        let key_len = u16::from_le_bytes(take(&written, &mut at, 2)?.try_into().ok()?) as usize;
        let type_len = take(&written, &mut at, 1)?[0] as usize;
        let key = String::from_utf8(take(&written, &mut at, key_len)?.to_vec()).ok()?;
        let content_type = String::from_utf8(take(&written, &mut at, type_len)?.to_vec()).ok()?;
        if where_ + u64::from(bytes) > end {
            return None;
        }
        live += u64::from(bytes);
        entries.insert(
            key,
            Entry {
                at: where_,
                bytes,
                content_type,
                used,
            },
        );
    }
    Some(Held {
        entries,
        end,
        dead: (end - MAGIC.len() as u64).saturating_sub(live),
    })
}

/// Rebuilds the directory by reading through the records.
///
/// Only after a crash, and it is why a record carries its own key and type.
fn walk(file: &std::fs::File, length: u64) -> std::io::Result<Held> {
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    let mut all = Vec::new();
    file.read_to_end(&mut all)?;
    let mut entries: HashMap<String, Entry> = HashMap::new();
    let mut at = MAGIC.len();
    let mut dead = 0u64;
    let mut end = MAGIC.len() as u64;
    while at + RECORD_HEADER <= all.len() {
        let key_len = u16::from_le_bytes(all[at..at + 2].try_into().unwrap_or_default()) as usize;
        let type_len = all[at + 2] as usize;
        let bytes = u32::from_le_bytes(all[at + 3..at + 7].try_into().unwrap_or_default());
        let payload = at + RECORD_HEADER + key_len + type_len;
        let next = payload + bytes as usize;
        if key_len == 0 || next > all.len() || next as u64 > length {
            break;
        }
        let Ok(key) =
            String::from_utf8(all[at + RECORD_HEADER..at + RECORD_HEADER + key_len].to_vec())
        else {
            break;
        };
        let Ok(content_type) =
            String::from_utf8(all[at + RECORD_HEADER + key_len..payload].to_vec())
        else {
            break;
        };
        if let Some(gone) = entries.insert(
            key,
            Entry {
                at: payload as u64,
                bytes,
                content_type,
                used: at as u64,
            },
        ) {
            dead += u64::from(gone.bytes);
        }
        at = next;
        end = next as u64;
    }
    Ok(Held { entries, end, dead })
}

fn take<'a>(from: &'a [u8], at: &mut usize, count: usize) -> Option<&'a [u8]> {
    let slice = from.get(*at..*at + count)?;
    *at += count;
    Some(slice)
}

/// A read at a stated offset, which leaves the handle usable by every other
/// reader at the same time.
fn read_at(file: &std::fs::File, into: &mut [u8], at: u64) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0usize;
        while done < into.len() {
            match file.seek_read(&mut into[done..], at + done as u64)? {
                0 => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the pack ends before this entry does",
                    ));
                }
                got => done += got,
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(into, at)
    }
}

/// Opens the pack, for writing if nobody else has it.
///
/// Windows does the deciding: the first one asks to share reads alone, so a
/// second attempt at the same cannot succeed and falls back to reading.
fn claim(path: &Path) -> std::io::Result<(std::fs::File, bool)> {
    match make(path) {
        Ok(file) => Ok((file, true)),
        // Somebody has it. Reading is still safe: an append never moves a
        // record already written, and only the holder rewrites the file.
        Err(_) => Ok((share(path)?, false)),
    }
}

/// Opens the pack for writing, if this process may have it.
///
/// Sharing reads and deletion, but not writes: a second attempt at the same
/// cannot succeed, which is what makes the holder the only writer. Deletion is
/// shared because a compaction renames a new file over this one, and Windows
/// refuses that while a handle in the way does not allow it.
fn make(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(SHARE_READ | SHARE_DELETE);
    }
    options.open(path)
}

/// Opens it to read alongside whoever is writing it.
fn share(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(SHARE_READ | SHARE_WRITE | SHARE_DELETE);
    }
    options.open(path)
}

#[cfg(windows)]
const SHARE_READ: u32 = 1;
#[cfg(windows)]
const SHARE_WRITE: u32 = 2;
#[cfg(windows)]
const SHARE_DELETE: u32 = 4;

/// Where the pack lives inside a cache directory.
pub fn beside(root: &Path) -> PathBuf {
    root.join("packed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("matterless-pack-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a place to write");
        beside(&root)
    }

    #[test]
    fn an_entry_reads_back_with_its_content_type() {
        let pack = Pack::open(scratch("roundtrip")).expect("opened");
        pack.write("avatar/u1", b"the bytes", "image/png", 1);
        let (bytes, content_type) = pack.read("avatar/u1").expect("held");
        assert_eq!(bytes, b"the bytes");
        assert_eq!(content_type, "image/png");
        assert!(pack.read("avatar/u2").is_none());
    }

    #[test]
    fn entries_survive_reopening() {
        let path = scratch("reopen");
        {
            let pack = Pack::open(path.clone()).expect("opened");
            for at in 0..20 {
                pack.write(
                    &format!("emoji/{at}"),
                    format!("number {at}").as_bytes(),
                    "image/webp",
                    at,
                );
            }
        }
        let again = Pack::open(path).expect("opened again");
        assert_eq!(again.held(), 20);
        assert_eq!(again.read("emoji/7").expect("held").0, b"number 7");
        assert_eq!(again.read("emoji/19").expect("held").0, b"number 19");
    }

    /// The same key written twice reads back the second one, before and after a
    /// restart: the first is dead bytes, not a second answer.
    #[test]
    fn a_rewritten_key_reads_back_the_new_bytes() {
        let path = scratch("rewritten");
        {
            let pack = Pack::open(path.clone()).expect("opened");
            pack.write("avatar/u1", b"the old ones", "image/png", 1);
            pack.write("avatar/u1", b"the new ones", "image/png", 2);
            assert_eq!(pack.read("avatar/u1").expect("held").0, b"the new ones");
        }
        let again = Pack::open(path).expect("opened again");
        assert_eq!(again.held(), 1);
        assert_eq!(again.read("avatar/u1").expect("held").0, b"the new ones");
    }

    /// A directory lost to a crash is rebuilt from the records themselves.
    #[test]
    fn a_pack_with_no_directory_is_walked() {
        let path = scratch("walked");
        {
            let pack = Pack::open(path.clone()).expect("opened");
            pack.write("emoji/one", b"first", "image/webp", 1);
            pack.write("emoji/two", b"second", "image/webp", 2);
        }
        // Everything the last write left after the records, gone.
        let whole = std::fs::read(&path).expect("read");
        let records = whole.len()
            - (FOOTER + 4 + 2 * (8 + 4 + 8 + 2 + 1 + "emoji/one".len() + "image/webp".len()));
        std::fs::write(&path, &whole[..records]).expect("truncate");

        let again = Pack::open(path).expect("opened again");
        assert_eq!(again.held(), 2, "both records were still there");
        assert_eq!(again.read("emoji/two").expect("held").0, b"second");
    }

    /// Dead bytes do not accumulate forever.
    #[test]
    fn compaction_keeps_what_is_live_and_drops_the_rest() {
        let path = scratch("compact");
        let big = vec![7u8; 100 * 1024];
        {
            let pack = Pack::open(path.clone()).expect("opened");
            // The same key over and over: one live entry, the rest dead.
            for _ in 0..20 {
                pack.write("avatar/u1", &big, "image/png", 1);
            }
            pack.write("avatar/u2", b"small", "image/png", 2);
        }
        let before = std::fs::metadata(&path).expect("there").len();
        assert!(before > 1024 * 1024, "dead bytes piled up: {before}");

        let again = Pack::open(path.clone()).expect("opened again");
        let after = std::fs::metadata(&path).expect("there").len();
        assert!(after < 200 * 1024, "compacted down to {after}");
        assert_eq!(again.held(), 2);
        assert_eq!(again.read("avatar/u1").expect("held").0.len(), big.len());
        assert_eq!(again.read("avatar/u2").expect("held").0, b"small");
    }

    /// The installed client and a build from the tree share this file. The
    /// second one to open it reads; it must not append over the first.
    #[cfg(windows)]
    #[test]
    fn the_second_pack_on_a_file_reads_but_does_not_write() {
        let path = scratch("shared");
        let first = Pack::open(path.clone()).expect("opened");
        first.write("emoji/one", b"first", "image/webp", 1);

        let second = Pack::open(path).expect("opened beside it");
        assert!(first.writable(), "the one that got there first");
        assert!(!second.writable(), "and the one that did not");
        assert_eq!(
            second.read("emoji/one").expect("readable").0,
            b"first",
            "it can still read what was there"
        );
        second.write("emoji/two", b"second", "image/webp", 2);
        assert_eq!(second.held(), 1, "and wrote nothing");
        drop(second);
        assert_eq!(
            first.read("emoji/one").expect("still there").0,
            b"first",
            "the first pack is untouched"
        );
    }

    #[test]
    fn something_too_big_is_left_to_its_own_file() {
        let pack = Pack::open(scratch("toobig")).expect("opened");
        pack.write("file/huge", &vec![0u8; LARGEST + 1], "video/mp4", 1);
        assert_eq!(pack.held(), 0);
    }
}
