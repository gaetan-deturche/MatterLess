//! Files uploaded but not yet posted.
//!
//! Mattermost splits attaching in two: the bytes go up on their own and come
//! back with an id, then a post claims that id in `file_ids`. So between
//! dropping a file into the composer and pressing send there is server-side
//! state with no post attached to it, and this is the client's half of that --
//! held in memory only, like `PendingPosts`, because an upload nobody claims is
//! simply orphaned rather than something SQLite should remember.
//!
//! The whole `FileInfo` is kept, not just the id: the composer's tray shows the
//! name and size, and the optimistic row shows the image before the server has
//! confirmed the post.

use matterless_core::model::FileInfo;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct Uploads {
    held: Mutex<HashMap<String, FileInfo>>,
    /// Uploads still on the wire, so one can be stopped.
    ///
    /// There is no resumable upload in Mattermost: a large file is one long
    /// request, and this server allows 150MB. Aborting the task drops the
    /// connection, which is the only way to stop one -- so cancellation is a
    /// feature rather than a nicety, and this is what makes it possible.
    sending: Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>,
}

impl Uploads {
    /// Remembers an upload in flight, replacing any earlier one under the same
    /// handle.
    pub fn sending(&self, attach_id: &str, task: tauri::async_runtime::JoinHandle<()>) {
        if let Some(previous) = self
            .sending
            .lock()
            .expect("uploads in flight")
            .insert(attach_id.to_string(), task)
        {
            previous.abort();
        }
    }

    /// Forgets an upload that finished on its own.
    pub fn finished(&self, attach_id: &str) {
        self.sending
            .lock()
            .expect("uploads in flight")
            .remove(attach_id);
    }

    /// Stops an upload. True if there was one to stop.
    pub fn cancel(&self, attach_id: &str) -> bool {
        match self
            .sending
            .lock()
            .expect("uploads in flight")
            .remove(attach_id)
        {
            Some(task) => {
                task.abort();
                true
            }
            None => false,
        }
    }

    pub fn hold(&self, info: FileInfo) {
        self.lock().insert(info.id.clone(), info);
    }

    /// Takes these uploads out of the holder, in the order asked for.
    ///
    /// All or nothing: a send that names an id nobody holds is a bug or a stale
    /// tray, and posting it half-attached would silently drop a file the reader
    /// believed they had sent. The caller compares the lengths.
    pub fn claim(&self, ids: &[String]) -> Vec<FileInfo> {
        let mut held = self.lock();
        ids.iter().filter_map(|id| held.remove(id)).collect()
    }

    /// The composer removed an attachment before sending.
    pub fn release(&self, id: &str) -> bool {
        self.lock().remove(id).is_some()
    }

    pub fn count(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, FileInfo>> {
        self.held.lock().expect("uploads")
    }
}

/// Decodes a `encodeURIComponent`-style header value.
///
/// A filename is UTF-8 and arbitrary -- "réunion été.png" is an ordinary name --
/// but an HTTP header value is visible ASCII, so the shell percent-encodes it.
/// Invalid escapes are left as the literal characters rather than rejected: a
/// name is a label, and mangling one is better than refusing the upload.
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str) -> FileInfo {
        FileInfo {
            id: id.into(),
            name: format!("{id}.png"),
            extension: "png".into(),
            size: 10,
            mime_type: "image/png".into(),
            width: 1,
            height: 1,
            has_preview_image: true,
            mini_preview: None,
            post_id: String::new(),
            archived: false,
        }
    }

    #[test]
    fn claiming_takes_the_uploads_in_the_order_asked_for() {
        let uploads = Uploads::default();
        uploads.hold(info("a"));
        uploads.hold(info("b"));
        let claimed = uploads.claim(&["b".to_string(), "a".to_string()]);
        assert_eq!(
            claimed
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"],
            "the order is the reader's, not the hash map's"
        );
        assert_eq!(uploads.count(), 0, "and they are no longer held");
    }

    #[test]
    fn claiming_an_unheld_id_comes_back_short_rather_than_inventing_one() {
        let uploads = Uploads::default();
        uploads.hold(info("a"));
        let claimed = uploads.claim(&["a".to_string(), "gone".to_string()]);
        assert_eq!(claimed.len(), 1, "so the caller can refuse the send");
    }

    #[test]
    fn releasing_removes_one_without_touching_the_rest() {
        let uploads = Uploads::default();
        uploads.hold(info("a"));
        uploads.hold(info("b"));
        assert!(uploads.release("a"));
        assert!(!uploads.release("a"), "already gone");
        assert_eq!(uploads.count(), 1);
    }

    #[test]
    fn filenames_survive_the_header() {
        assert_eq!(percent_decode("holiday.png"), "holiday.png");
        assert_eq!(
            percent_decode("r%C3%A9union%20%C3%A9t%C3%A9.png"),
            "réunion été.png"
        );
        // A stray percent is a character, not a failure.
        assert_eq!(percent_decode("100%.txt"), "100%.txt");
        assert_eq!(percent_decode("%zz.txt"), "%zz.txt");
    }
}
