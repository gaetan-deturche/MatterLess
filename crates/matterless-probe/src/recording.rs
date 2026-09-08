//! Record a live session once, replay it in seconds.
//!
//! The 60-minute soak was being used as a debugging loop, which is the wrong
//! shape: an hour per iteration, and once the traffic is gone the failure cannot
//! be re-triggered. A recording makes a session a fixture -- replayable, and
//! attachable to a bug report.
//!
//! **Message text is redacted at write time.** The checks a replay runs (dedup,
//! ordering, owed-vs-delivered) depend on ids and timestamps, never on content,
//! so nothing is lost by dropping it -- and the recording becomes safe to keep
//! and to read. Notification behaviour, which *does* depend on text, is covered
//! by the deterministic fixtures in `matterless-sync/tests/hazards.rs` instead.

use anyhow::{Context, Result};
use matterless_core::model::{Channel, ChannelMember, Post, PostList, Preference, Team, User};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    /// Everything needed to rebuild the store before replaying anything.
    Bootstrap {
        me: User,
        teams: Vec<Team>,
        channels: Vec<Channel>,
        members: Vec<ChannelMember>,
        preferences: Vec<Preference>,
        thread_mode: String,
    },
    Connected {
        connection_id: String,
        resumed: bool,
    },
    Disconnected {
        reason: String,
    },
    Resync,
    Posted {
        post: Post,
        channel_id: String,
    },
    Edited {
        post: Post,
    },
    Deleted {
        post: Post,
    },
    /// Name only: enough for the tally, and these carry nothing the checks read.
    OtherEvent {
        name: String,
    },
    RestSince {
        channel_id: String,
        list: PostList,
    },
    Cycle {
        strict: bool,
    },
}

fn redact(post: &mut Post) {
    if !post.message.is_empty() {
        post.message = format!("<{} chars>", post.message.chars().count());
    }
    // Webhook payloads carry text too, and none of it is read by a replay.
    if !post.props.is_null() {
        post.props = serde_json::Value::Null;
    }
}

fn redacted(record: Record) -> Record {
    match record {
        Record::Posted {
            mut post,
            channel_id,
        } => {
            redact(&mut post);
            Record::Posted { post, channel_id }
        }
        Record::Edited { mut post } => {
            redact(&mut post);
            Record::Edited { post }
        }
        Record::Deleted { mut post } => {
            redact(&mut post);
            Record::Deleted { post }
        }
        Record::RestSince {
            channel_id,
            mut list,
        } => {
            for post in list.posts.values_mut() {
                redact(post);
            }
            Record::RestSince { channel_id, list }
        }
        other => other,
    }
}

pub struct Recorder {
    file: Option<std::fs::File>,
}

impl Recorder {
    /// `None` disables recording entirely, so the live path pays nothing.
    pub fn create(path: Option<&str>) -> Result<Self> {
        let file = match path {
            Some(path) => {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                Some(std::fs::File::create(path).with_context(|| format!("create {path}"))?)
            }
            None => None,
        };
        Ok(Self { file })
    }

    pub fn is_recording(&self) -> bool {
        self.file.is_some()
    }

    pub fn write(&mut self, record: Record) -> Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        let line = serde_json::to_string(&redacted(record))?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(())
    }
}

pub fn read_all(path: &str) -> Result<Vec<Record>> {
    let file = std::fs::File::open(path).with_context(|| format!("open {path}"))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(&line).with_context(|| format!("{path}: line {}", index + 1))?,
        );
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_core::model::PostMetadata;

    fn sample() -> Post {
        Post {
            id: "p1".into(),
            channel_id: "c1".into(),
            user_id: "u1".into(),
            root_id: String::new(),
            create_at: 100,
            update_at: 100,
            edit_at: 0,
            delete_at: 0,
            message: "something confidential about the build".into(),
            post_type: String::new(),
            file_ids: Vec::new(),
            props: serde_json::json!({"attachments": [{"text": "also secret"}]}),
            metadata: PostMetadata::default(),
            pending_post_id: String::new(),
            is_pinned: false,
        }
    }

    #[test]
    fn recording_drops_message_text_but_keeps_what_the_checks_read() {
        let record = redacted(Record::Posted {
            post: sample(),
            channel_id: "c1".into(),
        });
        let Record::Posted { post, .. } = record else {
            panic!("wrong variant");
        };
        assert!(
            !post.message.contains("confidential"),
            "message text must never reach the recording"
        );
        assert!(post.props.is_null(), "webhook payload text goes too");
        // The fields the replay checks actually depend on survive.
        assert_eq!(post.id, "p1");
        assert_eq!(post.create_at, 100);
        assert_eq!(post.update_at, 100);
        assert_eq!(post.channel_id, "c1");
    }

    #[test]
    fn a_record_round_trips_through_jsonl() {
        let original = redacted(Record::Posted {
            post: sample(),
            channel_id: "c1".into(),
        });
        let line = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&line).unwrap();
        match parsed {
            Record::Posted { post, channel_id } => {
                assert_eq!(post.id, "p1");
                assert_eq!(channel_id, "c1");
            }
            other => panic!("expected Posted, got {other:?}"),
        }
    }
}
