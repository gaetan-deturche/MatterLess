//! Local SQLite store: the single source of truth the UI reads from.
//!
//! Every method here is **blocking on purpose**. Async callers wrap them in
//! `tokio::task::spawn_blocking`, which makes it impossible to hold the
//! connection guard across an `await` -- the mistake that cost Auger the most.

pub mod schema;

use matterless_core::model::{
    Channel, ChannelMember, Post, PostMetadata, Preference, Reaction, SidebarCategory, Team,
    Timestamp, User, UserThread,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// What an upsert actually did, which decides whether a delta is emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostChange {
    /// Not seen before.
    Inserted,
    /// `update_at` advanced: an edit.
    Updated,
    /// Newly carries `delete_at`.
    Tombstoned,
    /// Already held at this `update_at` or newer. **No delta.** This is where
    /// the websocket and REST delivering the same post stops being a duplicate.
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct PostOutcome {
    pub post_id: String,
    pub channel_id: String,
    pub create_at: Timestamp,
    pub change: PostChange,
}

/// Derived, never read from a flag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unread {
    pub messages: i64,
    pub messages_root: i64,
    pub mentions: i64,
    pub mentions_root: i64,
    pub muted: bool,
}

impl Unread {
    /// The counts to show, given the reader's thread mode.
    ///
    /// Under collapsed threads a reply belongs to its thread, not to the
    /// channel: the server keeps `_root` counters for exactly this, and the
    /// webapp reads them. Reading the all-posts counters instead made a channel
    /// claim unread messages for replies in threads the reader does not even
    /// follow -- the "blend of two models" that makes unread subtly wrong
    /// forever. Per-thread unread lives on the thread footer instead.
    pub fn visible(&self, collapsed: bool) -> (i64, i64) {
        if collapsed {
            (self.messages_root, self.mentions_root)
        } else {
            (self.messages, self.mentions)
        }
    }
}

/// Per-thread read state, as the server keeps it.
///
/// Not derived locally: a reply that arrives while the app is closed still has
/// to count, and only the server knows what has been read on another device.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadState {
    pub following: bool,
    pub reply_count: i64,
    pub last_reply_at: Timestamp,
    pub last_viewed_at: Timestamp,
    pub unread_replies: i64,
    pub unread_mentions: i64,
    pub is_urgent: bool,
}

/// Reply count, newest reply and distinct participants for one thread root.
pub type ThreadRollup = (i64, Timestamp, Vec<String>);

/// What the taskbar badge should show. The two parts are kept apart so the log
/// says which rule produced the number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BadgeState {
    /// Anything unread at all, in a channel that is not muted.
    pub any_unread: bool,
    /// Messages that named you, across every channel that is not muted.
    pub mentions: i64,
    /// Mentions inside followed threads.
    ///
    /// Additive rather than double-counted: with collapsed threads on, a reply
    /// does not touch `channel_members.mention_count`, so the server counts
    /// thread mentions separately and so must this.
    pub thread_mentions: i64,
    /// Unread messages in channels explicitly set to notify on every message.
    pub followed_unread: i64,
}

impl BadgeState {
    /// The number on the badge.
    pub fn attention(&self) -> i64 {
        self.mentions + self.thread_mentions + self.followed_unread
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncState {
    pub channel_id: String,
    pub synced_from: Timestamp,
    pub synced_to: Timestamp,
    pub oldest_post_id: String,
    pub newest_post_id: String,
    pub reached_beginning: bool,
    pub last_reconciled_at: Timestamp,
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        schema::prepare(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        schema::prepare(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// The applied schema version, so a caller can report it rather than
    /// guessing from whether a migration happened to log.
    pub fn schema_version(&self) -> Result<i64> {
        let connection = self.lock();
        Ok(connection.query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().expect("store mutex poisoned")
    }

    // ------------------------------------------------------------- reference

    pub fn upsert_users(&self, users: &[User]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for user in users {
            transaction.execute(
                "INSERT INTO users (id, username, first_name, last_name, nickname, email,
                                    last_picture_update, notify_props, roles)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    username = excluded.username,
                    first_name = excluded.first_name,
                    last_name = excluded.last_name,
                    nickname = excluded.nickname,
                    email = excluded.email,
                    last_picture_update = excluded.last_picture_update,
                    notify_props = excluded.notify_props,
                    roles = excluded.roles",
                params![
                    user.id,
                    user.username,
                    user.first_name,
                    user.last_name,
                    user.nickname,
                    user.email,
                    user.last_picture_update,
                    serde_json::to_string(&user.notify_props)?,
                    user.roles,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_teams(&self, teams: &[Team]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for team in teams {
            transaction.execute(
                "INSERT INTO teams (id, name, display_name) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name, display_name = excluded.display_name",
                params![team.id, team.name, team.display_name],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_channels(&self, channels: &[Channel]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for channel in channels {
            transaction.execute(
                "INSERT INTO channels (id, team_id, channel_type, name, display_name,
                                       total_msg_count, total_msg_count_root, last_post_at, delete_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    team_id = excluded.team_id,
                    channel_type = excluded.channel_type,
                    name = excluded.name,
                    display_name = excluded.display_name,
                    total_msg_count = excluded.total_msg_count,
                    total_msg_count_root = excluded.total_msg_count_root,
                    last_post_at = MAX(channels.last_post_at, excluded.last_post_at),
                    delete_at = excluded.delete_at",
                params![
                    channel.id,
                    channel.team_id,
                    channel.channel_type,
                    channel.name,
                    channel.display_name,
                    channel.total_msg_count,
                    channel.total_msg_count_root,
                    channel.last_post_at,
                    channel.delete_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_channel_members(&self, members: &[ChannelMember]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for member in members {
            transaction.execute(
                "INSERT INTO channel_members (channel_id, user_id, last_viewed_at, msg_count,
                                              msg_count_root, mention_count, mention_count_root,
                                              notify_props)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(channel_id, user_id) DO UPDATE SET
                    last_viewed_at = excluded.last_viewed_at,
                    msg_count = excluded.msg_count,
                    msg_count_root = excluded.msg_count_root,
                    mention_count = excluded.mention_count,
                    mention_count_root = excluded.mention_count_root,
                    notify_props = excluded.notify_props",
                params![
                    member.channel_id,
                    member.user_id,
                    member.last_viewed_at,
                    member.msg_count,
                    member.msg_count_root,
                    member.mention_count,
                    member.mention_count_root,
                    serde_json::to_string(&member.notify_props)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Users by id, for resolving an author to a name.
    pub fn users_by_ids(&self, ids: &[String]) -> Result<HashMap<String, User>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let connection = self.lock();
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT id, username, first_name, last_name, nickname, email,
                    last_picture_update, notify_props, roles
             FROM users WHERE id IN ({placeholders})"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(ids), |row| {
            Ok((row.get::<_, String>(0)?, row_to_user(row)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    /// One person by username, for a mention that was clicked.
    pub fn user_by_username(&self, username: &str) -> Result<Option<User>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT id, username, first_name, last_name, nickname, email,
                        last_picture_update, notify_props, roles
                 FROM users WHERE lower(username) = ?1",
                params![username.to_lowercase()],
                row_to_user,
            )
            .optional()?)
    }

    /// One channel by its slug or display name, for a `~channel` link.
    pub fn channel_by_name(&self, name: &str) -> Result<Option<Channel>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT id, team_id, channel_type, name, display_name, total_msg_count,
                        total_msg_count_root, last_post_at, delete_at
                 FROM channels
                 WHERE delete_at = 0 AND (lower(name) = ?1 OR lower(display_name) = ?1)
                 LIMIT 1",
                params![name.to_lowercase()],
                row_to_channel,
            )
            .optional()?)
    }

    /// Which of these ids we do not hold, so the caller can batch-fetch exactly
    /// those rather than issuing a request per author.
    pub fn missing_user_ids(&self, ids: &[String]) -> Result<Vec<String>> {
        let held = self.users_by_ids(ids)?;
        Ok(ids
            .iter()
            .filter(|id| !held.contains_key(*id))
            .cloned()
            .collect())
    }

    /// Live thread roots in a channel -- what the stream actually shows under
    /// collapsed threads, so it is the right thing to page against.
    pub fn root_count(&self, channel_id: &str) -> Result<i64> {
        let connection = self.lock();
        Ok(connection.query_row(
            "SELECT COUNT(*) FROM posts
             WHERE channel_id = ?1 AND root_id = '' AND delete_at = 0",
            params![channel_id],
            |row| row.get(0),
        )?)
    }

    // ----------------------------------------------------------------- posts

    /// Write-through upsert. Returns per-post outcomes so the caller emits a
    /// delta only for what genuinely changed.
    pub fn upsert_posts(&self, posts: &[Post]) -> Result<Vec<PostOutcome>> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        let mut outcomes = Vec::with_capacity(posts.len());

        for post in posts {
            let existing: Option<(Timestamp, Timestamp)> = transaction
                .query_row(
                    "SELECT update_at, delete_at FROM posts WHERE id = ?1",
                    params![post.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            let change = match existing {
                Some((held_update_at, held_delete_at)) => {
                    if post.update_at < held_update_at {
                        // Older than what we hold: a late REST page or a replayed
                        // event. Never let it overwrite.
                        PostChange::Unchanged
                    } else if post.update_at == held_update_at {
                        PostChange::Unchanged
                    } else if post.delete_at != 0 && held_delete_at == 0 {
                        PostChange::Tombstoned
                    } else {
                        PostChange::Updated
                    }
                }
                None => PostChange::Inserted,
            };

            if change != PostChange::Unchanged {
                transaction.execute(
                    "INSERT INTO posts (id, channel_id, user_id, root_id, create_at, update_at,
                                        edit_at, delete_at, message, post_type, file_ids, props,
                                        metadata, pending_post_id, is_pinned)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                     ON CONFLICT(id) DO UPDATE SET
                        channel_id = excluded.channel_id,
                        user_id = excluded.user_id,
                        root_id = excluded.root_id,
                        create_at = excluded.create_at,
                        update_at = excluded.update_at,
                        edit_at = excluded.edit_at,
                        delete_at = excluded.delete_at,
                        message = excluded.message,
                        post_type = excluded.post_type,
                        file_ids = excluded.file_ids,
                        props = excluded.props,
                        metadata = excluded.metadata,
                        pending_post_id = excluded.pending_post_id,
                        is_pinned = excluded.is_pinned",
                    params![
                        post.id,
                        post.channel_id,
                        post.user_id,
                        post.root_id,
                        post.create_at,
                        post.update_at,
                        post.edit_at,
                        post.delete_at,
                        post.message,
                        post.post_type,
                        serde_json::to_string(&post.file_ids)?,
                        serde_json::to_string(&post.props)?,
                        serde_json::to_string(&post.metadata)?,
                        post.pending_post_id,
                        post.is_pinned,
                    ],
                )?;

                // Reactions and files arrive inside post.metadata, so a post
                // fetch populates them without an extra request.
                for reaction in &post.metadata.reactions {
                    transaction.execute(
                        "INSERT INTO reactions (post_id, user_id, emoji_name, create_at)
                         VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
                        params![
                            post.id,
                            reaction.user_id,
                            reaction.emoji_name,
                            reaction.create_at
                        ],
                    )?;
                }
                for file in &post.metadata.files {
                    transaction.execute(
                        "INSERT INTO files (id, post_id, name, extension, size, mime_type,
                                            width, height)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT(id) DO UPDATE SET post_id = excluded.post_id",
                        params![
                            file.id,
                            post.id,
                            file.name,
                            file.extension,
                            file.size,
                            file.mime_type,
                            file.width,
                            file.height
                        ],
                    )?;
                }
            }

            outcomes.push(PostOutcome {
                post_id: post.id.clone(),
                channel_id: post.channel_id.clone(),
                create_at: post.create_at,
                change,
            });
        }

        transaction.commit()?;
        Ok(outcomes)
    }

    pub fn post(&self, post_id: &str) -> Result<Option<Post>> {
        let connection = self.lock();
        let post = connection
            .query_row(
                "SELECT id, channel_id, user_id, root_id, create_at, update_at, edit_at,
                        delete_at, message, post_type, file_ids, props, metadata, pending_post_id,
                        is_pinned
                 FROM posts WHERE id = ?1",
                params![post_id],
                row_to_post,
            )
            .optional()?;
        post.transpose()
    }

    /// One page of the channel stream, newest first. `before` pages backwards.
    pub fn channel_page(
        &self,
        channel_id: &str,
        before: Option<Timestamp>,
        limit: u32,
    ) -> Result<Vec<Post>> {
        self.channel_page_filtered(channel_id, before, None, limit, false)
    }

    /// One page of the channel stream, optionally roots only.
    ///
    /// `roots_only` is what collapsed threads actually need, and skipping the
    /// replies in SQL rather than after the read matters a great deal: 84% of
    /// posts here are replies that the stream discards, and reading them meant
    /// deserialising three JSON columns per post for nothing. Measured at 2023
    /// rows, this read was 50 of 78 ms of the whole plan build.
    ///
    /// `since` bounds the page from below, which is what gives the newest page a
    /// *fixed* floor. Without one it would mean "the newest N posts", and as
    /// messages arrived its window would slide forward and drop posts off its
    /// bottom -- leaving a hole between it and the page underneath.
    pub fn channel_page_filtered(
        &self,
        channel_id: &str,
        before: Option<Timestamp>,
        since: Option<Timestamp>,
        limit: u32,
        roots_only: bool,
    ) -> Result<Vec<Post>> {
        let connection = self.lock();
        let reply_clause = if roots_only { "AND root_id = ''" } else { "" };
        let mut statement = connection.prepare(&format!(
            "SELECT id, channel_id, user_id, root_id, create_at, update_at, edit_at,
                    delete_at, message, post_type, file_ids, props, metadata, pending_post_id,
                    is_pinned
             FROM posts
             WHERE channel_id = ?1 AND create_at < ?2 AND create_at >= ?3 {reply_clause}
             ORDER BY create_at DESC, id DESC
             LIMIT ?4"
        ))?;
        let cursor = before.unwrap_or(Timestamp::MAX);
        let floor = since.unwrap_or(0);
        let rows = statement.query_map(params![channel_id, cursor, floor, limit], row_to_post)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    pub fn thread_replies(&self, root_id: &str) -> Result<Vec<Post>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT id, channel_id, user_id, root_id, create_at, update_at, edit_at,
                    delete_at, message, post_type, file_ids, props, metadata, pending_post_id,
                    is_pinned
             FROM posts WHERE root_id = ?1 ORDER BY create_at ASC, id ASC",
        )?;
        let rows = statement.query_map(params![root_id], row_to_post)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// Searches locally, honouring the modifiers.
    ///
    /// The whole query is answerable here, which is the point: `from:` names a
    /// user this store already holds, `in:` names a channel it holds, and the
    /// date modifiers are a comparison on `create_at`. Only the free text needs
    /// the FTS index, and a query made of modifiers alone skips it entirely --
    /// `from:amy after:2026-09-01` is a filter, not a match.
    ///
    /// A modifier naming somebody or something unknown returns nothing rather
    /// than everything: `from:nobody` finding every message would be worse than
    /// finding none.
    pub fn search_query(
        &self,
        query: &matterless_core::search::SearchQuery,
        limit: u32,
    ) -> Result<Vec<Post>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.lock();

        // ---- modifiers to ids ------------------------------------------
        let mut authors: Vec<String> = Vec::new();
        for name in &query.from {
            let mut statement = connection.prepare(
                "SELECT id FROM users
                 WHERE lower(username) = ?1
                    OR lower(nickname) = ?1
                    OR lower(first_name || '.' || last_name) = ?1",
            )?;
            let matched = statement
                .query_map(params![name], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            if matched.is_empty() {
                return Ok(Vec::new());
            }
            authors.extend(matched);
        }

        let mut channels: Vec<String> = Vec::new();
        for name in &query.in_channels {
            let mut statement = connection.prepare(
                "SELECT id FROM channels
                 WHERE delete_at = 0 AND (lower(name) = ?1 OR lower(display_name) = ?1)",
            )?;
            let matched = statement
                .query_map(params![name], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            if matched.is_empty() {
                return Ok(Vec::new());
            }
            channels.extend(matched);
        }

        // ---- the statement ---------------------------------------------
        // Built rather than written out because which clauses apply depends on
        // what was typed; every value is still bound, never interpolated.
        let mut sql = String::from(
            "SELECT p.id, p.channel_id, p.user_id, p.root_id, p.create_at, p.update_at,
                    p.edit_at, p.delete_at, p.message, p.post_type, p.file_ids, p.props,
                    p.metadata, p.pending_post_id, p.is_pinned
             FROM posts p",
        );
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let expression = query.fts_expression();
        if let Some(expression) = &expression {
            sql.push_str(
                " JOIN posts_fts ON p.rowid = posts_fts.rowid
                 WHERE posts_fts MATCH ?",
            );
            values.push(Box::new(expression.clone()));
            sql.push_str(" AND p.delete_at = 0");
        } else {
            sql.push_str(" WHERE p.delete_at = 0");
        }

        if !authors.is_empty() {
            sql.push_str(&format!(
                " AND p.user_id IN ({})",
                vec!["?"; authors.len()].join(",")
            ));
            for id in authors {
                values.push(Box::new(id));
            }
        }
        if !channels.is_empty() {
            sql.push_str(&format!(
                " AND p.channel_id IN ({})",
                vec!["?"; channels.len()].join(",")
            ));
            for id in channels {
                values.push(Box::new(id));
            }
        }
        if let Some(since) = query.since {
            sql.push_str(" AND p.create_at >= ?");
            values.push(Box::new(since));
        }
        if let Some(until) = query.until {
            sql.push_str(" AND p.create_at < ?");
            values.push(Box::new(until));
        }
        sql.push_str(" ORDER BY p.create_at DESC LIMIT ?");
        values.push(Box::new(limit));

        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(values.iter().map(|value| value.as_ref())),
            row_to_post,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// Local full-text search over an FTS5 expression.
    ///
    /// The narrow form, kept for callers that have no modifiers to honour;
    /// `search_query` is the one a reader's query goes through.
    /// Beats the server for recent history because this deployment has no
    /// Elasticsearch.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<Post>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT p.id, p.channel_id, p.user_id, p.root_id, p.create_at, p.update_at,
                    p.edit_at, p.delete_at, p.message, p.post_type, p.file_ids, p.props,
                    p.metadata, p.pending_post_id, p.is_pinned
             FROM posts_fts
             JOIN posts p ON p.rowid = posts_fts.rowid
             WHERE posts_fts MATCH ?1 AND p.delete_at = 0
             ORDER BY p.create_at DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![query, limit], row_to_post)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    pub fn reactions(&self, post_id: &str) -> Result<Vec<Reaction>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT post_id, user_id, emoji_name, create_at FROM reactions WHERE post_id = ?1",
        )?;
        let rows = statement.query_map(params![post_id], |row| {
            Ok(Reaction {
                post_id: row.get(0)?,
                user_id: row.get(1)?,
                emoji_name: row.get(2)?,
                create_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // --------------------------------------------------------------- unreads

    /// Derived from the channel counter minus the member counter -- the only
    /// correct source. Returns `None` when membership is unknown.
    pub fn unread(&self, channel_id: &str, user_id: &str) -> Result<Option<Unread>> {
        let connection = self.lock();
        let row = connection
            .query_row(
                "SELECT c.total_msg_count, c.total_msg_count_root,
                        m.msg_count, m.msg_count_root,
                        m.mention_count, m.mention_count_root, m.notify_props
                 FROM channels c
                 JOIN channel_members m ON m.channel_id = c.id
                 WHERE c.id = ?1 AND m.user_id = ?2",
                params![channel_id, user_id],
                |row| {
                    let notify_props: String = row.get(6)?;
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        notify_props,
                    ))
                },
            )
            .optional()?;

        let Some((total, total_root, seen, seen_root, mentions, mentions_root, notify_props)) = row
        else {
            return Ok(None);
        };
        let props: HashMap<String, String> =
            serde_json::from_str(&notify_props).unwrap_or_default();
        Ok(Some(Unread {
            messages: (total - seen).max(0),
            messages_root: (total_root - seen_root).max(0),
            mentions,
            mentions_root,
            muted: props.get("mark_unread").map(String::as_str) == Some("mention"),
        }))
    }

    /// The newest post's `create_at` in a channel, in **server** time.
    ///
    /// This is what "read up to here" should record. Writing a local clock
    /// value into `last_viewed_at` and then comparing it against server
    /// `create_at` mixes two clocks: a machine running a few minutes fast would
    /// put the marker in the future and the "New messages" divider would never
    /// appear again. Zero when nothing is held, which leaves the existing marker
    /// untouched because `mark_channel_viewed` takes the later of the two.
    pub fn newest_post_at(&self, channel_id: &str) -> Result<Timestamp> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT COALESCE(MAX(create_at), 0) FROM posts
                 WHERE channel_id = ?1 AND delete_at = 0",
                params![channel_id],
                |row| row.get::<_, Timestamp>(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// When this user last viewed the channel.
    ///
    /// This is where the "New messages" divider goes. It is read once when a
    /// channel is opened and then held, because marking the channel read moves
    /// this value to now -- and recomputing from the live value would make the
    /// divider vanish while the reader is still looking at it.
    pub fn last_viewed_at(&self, channel_id: &str, user_id: &str) -> Result<Option<Timestamp>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT last_viewed_at FROM channel_members
                 WHERE channel_id = ?1 AND user_id = ?2",
                params![channel_id, user_id],
                |row| row.get::<_, Timestamp>(0),
            )
            .optional()?)
    }

    /// Records that a post arrived, so derived unread can move.
    ///
    /// Unread is `channels.total_msg_count - channel_members.msg_count`, and
    /// upserting the post alone leaves the channel counter untouched -- which is
    /// why a live message never changed a badge. The server increments the same
    /// counter, so this keeps the local derivation honest between REST refreshes.
    ///
    /// A post of one's own also advances the member counter, because sending is
    /// reading: the server does the same, and not mirroring it would show a
    /// phantom unread for every message sent.
    ///
    /// `mentions_me` advances the mention counter for the same reason unread
    /// needed this: the server increments it, and without mirroring it the
    /// badge number could only move on the next REST refresh.
    pub fn record_arrival(
        &self,
        channel_id: &str,
        is_root: bool,
        mine: bool,
        mentions_me: bool,
    ) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "UPDATE channels SET
                total_msg_count = total_msg_count + 1,
                total_msg_count_root = total_msg_count_root + ?2
             WHERE id = ?1",
            params![channel_id, i64::from(is_root)],
        )?;
        if mine {
            connection.execute(
                "UPDATE channel_members SET
                    msg_count = msg_count + 1,
                    msg_count_root = msg_count_root + ?2
                 WHERE channel_id = ?1",
                params![channel_id, i64::from(is_root)],
            )?;
        } else if mentions_me {
            connection.execute(
                "UPDATE channel_members SET
                    mention_count = mention_count + 1,
                    mention_count_root = mention_count_root + ?2
                 WHERE channel_id = ?1",
                params![channel_id, i64::from(is_root)],
            )?;
        }
        Ok(())
    }

    /// Applies what "viewed" means server-side: the member's counters catch up
    /// to the channel's, and mentions clear. This is what makes a read on
    /// another device actually clear the badge here.
    pub fn mark_channel_viewed(
        &self,
        channel_id: &str,
        user_id: &str,
        viewed_at: Timestamp,
    ) -> Result<bool> {
        let connection = self.lock();
        // COALESCE guards the case where the channel row is not known yet: the
        // columns are NOT NULL, so a missing subselect must fall back.
        let changed = connection.execute(
            "UPDATE channel_members SET
                last_viewed_at = MAX(last_viewed_at, ?3),
                msg_count = COALESCE(
                    (SELECT total_msg_count FROM channels WHERE id = ?1), msg_count),
                msg_count_root = COALESCE(
                    (SELECT total_msg_count_root FROM channels WHERE id = ?1), msg_count_root),
                mention_count = 0,
                mention_count_root = 0
             WHERE channel_id = ?1 AND user_id = ?2",
            params![channel_id, user_id, viewed_at],
        )?;
        Ok(changed > 0)
    }

    /// The badge aggregate, in one query.
    ///
    /// Muted channels contribute nothing: muting is a statement that a channel
    /// should not interrupt, so counting it would undo that.
    ///
    /// A channel contributes its whole unread count only when the channel
    /// *itself* is set to `desktop: all` -- an explicit per-channel override.
    /// A channel left on `default` contributes its mentions only, and
    /// deliberately does NOT inherit the account-wide level: Mattermost ships
    /// that as `all`, so inheriting it would make nearly every channel count as
    /// followed and turn the badge into a total-unread count (measured: 33
    /// against the official client's 1). One rule or the other per channel,
    /// never both, is what stops a mention in a followed channel counting twice.
    pub fn badge_state(&self, user_id: &str, collapsed: bool) -> Result<BadgeState> {
        let connection = self.lock();
        // Collapsed threads count roots only, for the reason `Unread::visible`
        // explains: a reply is its thread's business, and thread unread is
        // counted separately below.
        let (total, seen, mentions) = if collapsed {
            (
                "c.total_msg_count_root",
                "COALESCE(m.msg_count_root, 0)",
                "COALESCE(m.mention_count_root, 0)",
            )
        } else {
            (
                "c.total_msg_count",
                "COALESCE(m.msg_count, 0)",
                "COALESCE(m.mention_count, 0)",
            )
        };
        let mut statement = connection.prepare(&format!(
            "SELECT {total}, {seen}, {mentions}, COALESCE(m.notify_props, '{{}}'),
                    c.channel_type
             FROM channels c
             JOIN channel_members m ON m.channel_id = c.id AND m.user_id = ?1
             WHERE c.delete_at = 0"
        ))?;
        let rows = statement.query_map(params![user_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut state = BadgeState::default();
        for row in rows {
            let (total, seen, mentions, notify_props, channel_type) = row?;
            let props: HashMap<String, String> =
                serde_json::from_str(&notify_props).unwrap_or_default();
            if props.get("mark_unread").map(String::as_str) == Some("mention") {
                continue;
            }
            let unread = (total - seen).max(0);
            if unread > 0 {
                state.any_unread = true;
            }
            if props.get("desktop").map(String::as_str) == Some("all") {
                state.followed_unread += unread;
            } else if channel_type == "D" || channel_type == "G" {
                // A direct or group message is addressed to you by definition,
                // which is why the server counts every one of them as a
                // mention. Read from unread rather than the mention column so
                // it is right the instant one arrives.
                state.mentions += unread;
            } else {
                state.mentions += mentions;
            }
        }
        // Threads are their own read model, so they are their own query.
        let (unread_replies, unread_mentions) = {
            drop(statement);
            drop(connection);
            self.thread_unread_totals()?
        };
        state.thread_mentions = unread_mentions;
        if unread_replies > 0 {
            state.any_unread = true;
        }
        Ok(state)
    }

    // --------------------------------------------------------------- emoji

    /// What is known about these emoji names.
    ///
    /// Returns every name that has been asked about, mapped to its id -- empty
    /// for the ones that turned out to be standard. The caller can then tell
    /// "custom, here is the image", "standard, no image", and "never asked"
    /// apart, which is what stops it asking again.
    pub fn known_emoji(&self, names: &[String]) -> Result<HashMap<String, String>> {
        if names.is_empty() {
            return Ok(HashMap::new());
        }
        let connection = self.lock();
        let placeholders = vec!["?"; names.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT name, emoji_id FROM custom_emoji WHERE name IN ({placeholders})"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(names), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(Into::into)
    }

    /// Records an answer. An empty `emoji_id` means "standard, not custom".
    pub fn remember_emoji(&self, name: &str, emoji_id: &str) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO custom_emoji (name, emoji_id) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET emoji_id = excluded.emoji_id",
            params![name, emoji_id],
        )?;
        Ok(())
    }

    // ----------------------------------------------------------- reactions

    /// Records a reaction against a post.
    ///
    /// Reactions arrive inside `post.metadata`, which is a JSON column -- so a
    /// `reaction_added` event has to patch that, and for a long time it did
    /// not: the event emitted a delta, the plan was rebuilt from the *old*
    /// metadata, and the reaction only appeared after the next REST fetch.
    ///
    /// Returns whether anything changed, so a duplicate echo of one's own
    /// optimistic reaction costs no re-render.
    pub fn add_reaction(
        &self,
        post_id: &str,
        user_id: &str,
        emoji_name: &str,
        create_at: Timestamp,
    ) -> Result<bool> {
        self.patch_reactions(post_id, |reactions| {
            let already = reactions
                .iter()
                .any(|held| held.user_id == user_id && held.emoji_name == emoji_name);
            if already {
                return false;
            }
            reactions.push(Reaction {
                user_id: user_id.to_string(),
                post_id: post_id.to_string(),
                emoji_name: emoji_name.to_string(),
                create_at,
            });
            true
        })
    }

    pub fn remove_reaction(&self, post_id: &str, user_id: &str, emoji_name: &str) -> Result<bool> {
        self.patch_reactions(post_id, |reactions| {
            let before = reactions.len();
            reactions.retain(|held| !(held.user_id == user_id && held.emoji_name == emoji_name));
            reactions.len() != before
        })
    }

    /// Reads a post's metadata, hands its reactions to `change`, writes it back
    /// if anything moved.
    ///
    /// Deliberately does not touch `update_at`: that is the markdown cache's
    /// key, and a reaction changes nothing about the message body -- bumping it
    /// would throw away a parsed tree for no reason.
    fn patch_reactions<F>(&self, post_id: &str, change: F) -> Result<bool>
    where
        F: FnOnce(&mut Vec<Reaction>) -> bool,
    {
        let connection = self.lock();
        let held: Option<String> = connection
            .query_row(
                "SELECT metadata FROM posts WHERE id = ?1",
                params![post_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(held) = held else {
            // A reaction to a post we do not hold: nothing to patch, and the
            // post will arrive with its reactions already in it.
            return Ok(false);
        };
        let mut metadata: PostMetadata = serde_json::from_str(&held).unwrap_or_default();
        if !change(&mut metadata.reactions) {
            return Ok(false);
        }
        connection.execute(
            "UPDATE posts SET metadata = ?2 WHERE id = ?1",
            params![post_id, serde_json::to_string(&metadata)?],
        )?;
        Ok(true)
    }

    // ------------------------------------------------------------- sidebar

    /// Replaces one team's categories.
    ///
    /// Deletes that team's rows first: a channel moved out of a category has to
    /// stop being in it, and an upsert alone would leave it in both.
    pub fn upsert_sidebar(&self, team_id: &str, categories: &[SidebarCategory]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM sidebar_category_channels
             WHERE category_id IN (SELECT id FROM sidebar_categories WHERE team_id = ?1)",
            params![team_id],
        )?;
        transaction.execute(
            "DELETE FROM sidebar_categories WHERE team_id = ?1",
            params![team_id],
        )?;
        for category in categories {
            transaction.execute(
                "INSERT INTO sidebar_categories
                    (id, team_id, category_type, display_name, sort_order, sorting,
                     muted, collapsed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    category.id,
                    team_id,
                    category.category_type,
                    category.display_name,
                    category.sort_order,
                    category.sorting,
                    i64::from(category.muted),
                    i64::from(category.collapsed),
                ],
            )?;
            for (position, channel_id) in category.channel_ids.iter().enumerate() {
                transaction.execute(
                    "INSERT OR REPLACE INTO sidebar_category_channels
                        (category_id, channel_id, position)
                     VALUES (?1, ?2, ?3)",
                    params![category.id, channel_id, position as i64],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Every category, with its channels in the order the reader arranged them.
    ///
    /// Ordered by team then `sort_order`, which is the order the sidebar shows;
    /// the channels inside carry their position so a `manual` category can be
    /// honoured.
    pub fn sidebar(&self) -> Result<Vec<(SidebarCategory, Vec<String>)>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT id, team_id, category_type, display_name, sort_order, sorting,
                    muted, collapsed
             FROM sidebar_categories
             ORDER BY team_id, sort_order",
        )?;
        let categories: Vec<SidebarCategory> = statement
            .query_map([], |row| {
                Ok(SidebarCategory {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    category_type: row.get(2)?,
                    display_name: row.get(3)?,
                    sort_order: row.get(4)?,
                    sorting: row.get(5)?,
                    muted: row.get::<_, i64>(6)? != 0,
                    collapsed: row.get::<_, i64>(7)? != 0,
                    channel_ids: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut members = connection.prepare(
            "SELECT channel_id FROM sidebar_category_channels
             WHERE category_id = ?1 ORDER BY position",
        )?;
        let mut out = Vec::with_capacity(categories.len());
        for category in categories {
            let channels = members
                .query_map(params![category.id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push((category, channels));
        }
        Ok(out)
    }

    // ------------------------------------------------------------- threads

    /// Stores followed threads. The counts are the server's, verbatim.
    pub fn upsert_threads(&self, threads: &[UserThread]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for thread in threads {
            transaction.execute(
                "INSERT INTO threads (root_id, channel_id, following, reply_count,
                                      last_reply_at, last_viewed_at, unread_replies,
                                      unread_mentions, is_urgent, delete_at)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(root_id) DO UPDATE SET
                    channel_id = excluded.channel_id,
                    following = 1,
                    reply_count = excluded.reply_count,
                    last_reply_at = excluded.last_reply_at,
                    last_viewed_at = MAX(threads.last_viewed_at, excluded.last_viewed_at),
                    unread_replies = excluded.unread_replies,
                    unread_mentions = excluded.unread_mentions,
                    is_urgent = excluded.is_urgent,
                    delete_at = excluded.delete_at",
                params![
                    thread.id,
                    thread.post.channel_id,
                    thread.reply_count,
                    thread.last_reply_at,
                    thread.last_viewed_at,
                    thread.unread_replies,
                    thread.unread_mentions,
                    i64::from(thread.is_urgent),
                    thread.delete_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Everyone who has replied in these threads.
    ///
    /// Wanted before the plan is built, not after: a thread footer shows its
    /// participants' faces, and a face needs the person's row -- their name for
    /// the tooltip and their `last_picture_update` for the avatar URL. They are
    /// often not authors of anything in the window, so the page's own author
    /// list does not cover them.
    pub fn thread_participant_ids(&self, root_ids: &[String]) -> Result<Vec<String>> {
        if root_ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.lock();
        let placeholders = vec!["?"; root_ids.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT DISTINCT user_id FROM posts
             WHERE delete_at = 0 AND root_id IN ({placeholders})"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(root_ids), |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Read state for the roots in a window, for the footer rows.
    ///
    /// Scoped to the window for the same reason `thread_summaries_for` is: an
    /// unscoped read grew with stored history and was 60% of a plan build.
    pub fn thread_states_for(&self, root_ids: &[String]) -> Result<HashMap<String, ThreadState>> {
        if root_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let connection = self.lock();
        let placeholders = vec!["?"; root_ids.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT root_id, following, reply_count, last_reply_at, last_viewed_at,
                    unread_replies, unread_mentions, is_urgent
             FROM threads WHERE root_id IN ({placeholders})"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(root_ids), |row| {
            Ok((
                row.get::<_, String>(0)?,
                ThreadState {
                    following: row.get::<_, i64>(1)? != 0,
                    reply_count: row.get(2)?,
                    last_reply_at: row.get(3)?,
                    last_viewed_at: row.get(4)?,
                    unread_replies: row.get(5)?,
                    unread_mentions: row.get(6)?,
                    is_urgent: row.get::<_, i64>(7)? != 0,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(Into::into)
    }

    /// Applies "this thread has been read up to here", as the server does.
    pub fn mark_thread_viewed(&self, root_id: &str, viewed_at: Timestamp) -> Result<bool> {
        let connection = self.lock();
        let changed = connection.execute(
            "UPDATE threads SET
                last_viewed_at = MAX(last_viewed_at, ?2),
                unread_replies = 0,
                unread_mentions = 0
             WHERE root_id = ?1",
            params![root_id, viewed_at],
        )?;
        Ok(changed > 0)
    }

    /// Follows or unfollows a thread. Unfollowing keeps the row: its counts are
    /// still the truth for a thread that is merely not being watched.
    pub fn set_thread_following(&self, root_id: &str, following: bool) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO threads (root_id, following) VALUES (?1, ?2)
             ON CONFLICT(root_id) DO UPDATE SET following = excluded.following",
            params![root_id, i64::from(following)],
        )?;
        Ok(())
    }

    /// Records what a thread event said, without a round trip.
    pub fn record_thread_activity(
        &self,
        root_id: &str,
        channel_id: &str,
        reply_count: i64,
        last_reply_at: Timestamp,
        unread_replies: i64,
        unread_mentions: i64,
    ) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO threads (root_id, channel_id, reply_count, last_reply_at,
                                  unread_replies, unread_mentions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(root_id) DO UPDATE SET
                channel_id = CASE WHEN excluded.channel_id != '' THEN excluded.channel_id
                                  ELSE threads.channel_id END,
                reply_count = MAX(threads.reply_count, excluded.reply_count),
                last_reply_at = MAX(threads.last_reply_at, excluded.last_reply_at),
                unread_replies = excluded.unread_replies,
                unread_mentions = excluded.unread_mentions",
            params![
                root_id,
                channel_id,
                reply_count,
                last_reply_at,
                unread_replies,
                unread_mentions
            ],
        )?;
        Ok(())
    }

    /// Applies a `thread_read_changed` event: the counts are the server's.
    pub fn set_thread_read(
        &self,
        root_id: &str,
        viewed_at: Timestamp,
        unread_replies: i64,
        unread_mentions: i64,
    ) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO threads (root_id, last_viewed_at, unread_replies, unread_mentions)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(root_id) DO UPDATE SET
                last_viewed_at = MAX(threads.last_viewed_at, excluded.last_viewed_at),
                unread_replies = excluded.unread_replies,
                unread_mentions = excluded.unread_mentions",
            params![root_id, viewed_at, unread_replies, unread_mentions],
        )?;
        Ok(())
    }

    /// "Mark all threads read", which the server sends as a read change with an
    /// empty thread id.
    pub fn mark_all_threads_read(&self, viewed_at: Timestamp) -> Result<usize> {
        let connection = self.lock();
        Ok(connection.execute(
            "UPDATE threads SET
                last_viewed_at = MAX(last_viewed_at, ?1),
                unread_replies = 0,
                unread_mentions = 0
             WHERE following = 1",
            params![viewed_at],
        )?)
    }

    /// Which channel a thread lives in, for events that do not say.
    pub fn thread_channel(&self, root_id: &str) -> Result<Option<String>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT channel_id FROM threads WHERE root_id = ?1 AND channel_id != ''
                 UNION ALL
                 SELECT channel_id FROM posts WHERE id = ?1
                 LIMIT 1",
                params![root_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Unread replies and mentions across followed threads.
    ///
    /// Separate from channel unread by design: with collapsed threads on, a
    /// reply does not bump channel unread, so adding these to that count would
    /// be a blend of two models -- which the plan warns makes unread "subtly
    /// wrong forever".
    pub fn thread_unread_totals(&self) -> Result<(i64, i64)> {
        let connection = self.lock();
        Ok(connection.query_row(
            "SELECT COALESCE(SUM(unread_replies), 0), COALESCE(SUM(unread_mentions), 0)
             FROM threads WHERE following = 1 AND delete_at = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    /// The newest post older than `before`, which is the row above a page.
    ///
    /// A page's plan needs it to know whether its first post continues an
    /// author's run and whether its day has already been announced -- one
    /// indexed row, against rebuilding the pages above it.
    pub fn post_above(
        &self,
        channel_id: &str,
        before: Timestamp,
        roots_only: bool,
    ) -> Result<Option<Post>> {
        let page = self.channel_page_filtered(channel_id, Some(before), None, 1, roots_only)?;
        Ok(page.into_iter().next())
    }

    /// One channel by id.
    ///
    /// Notifications need a channel's name and type, and reaching for the whole
    /// sidebar to find one row cost a scan of every channel plus its unread
    /// derivation, per toast.
    /// People whose username or real name matches `query` as a subsequence.
    ///
    /// Two stages by design: SQLite filters with a wildcard-between-characters
    /// `LIKE` -- which *is* a subsequence test -- and the ranking happens in
    /// Rust, because how good a match is depends on where the letters landed
    /// and SQL cannot say. The candidate set is capped well above the wanted
    /// count so the ranking has something to choose between.
    pub fn users_matching(&self, query: &str, limit: u32) -> Result<Vec<User>> {
        let connection = self.lock();
        let pattern = matterless_core::fuzzy::like_pattern(query);
        let mut statement = connection.prepare(
            "SELECT id, username, first_name, last_name, nickname, email,
                    last_picture_update, notify_props, roles
             FROM users
             WHERE lower(username) LIKE ?1 ESCAPE '\\'
                OR lower(first_name || ' ' || last_name) LIKE ?1 ESCAPE '\\'
                OR lower(nickname) LIKE ?1 ESCAPE '\\'
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![pattern, limit * 12], row_to_user)?;
        let mut found: Vec<(i32, User)> = rows
            .collect::<rusqlite::Result<Vec<User>>>()?
            .into_iter()
            .filter_map(|user| {
                let full = format!("{} {}", user.first_name, user.last_name);
                matterless_core::fuzzy::best_score(
                    [user.username.as_str(), full.trim(), user.nickname.as_str()],
                    query,
                )
                .map(|points| (points, user))
            })
            .collect();
        // Username length as the tiebreak: with nothing typed this is "the
        // shortest names first", which is a better default than table order.
        found.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then(left.1.username.len().cmp(&right.1.username.len()))
        });
        Ok(found
            .into_iter()
            .take(limit as usize)
            .map(|(_, user)| user)
            .collect())
    }

    /// Channels whose display name or slug matches `query` as a subsequence,
    /// for the quick switcher and for `~channel` completion.
    pub fn channels_matching(&self, query: &str, limit: u32) -> Result<Vec<Channel>> {
        let connection = self.lock();
        let pattern = matterless_core::fuzzy::like_pattern(query);
        let mut statement = connection.prepare(
            "SELECT id, team_id, channel_type, name, display_name, total_msg_count,
                    total_msg_count_root, last_post_at, delete_at
             FROM channels
             WHERE delete_at = 0
               AND (lower(display_name) LIKE ?1 ESCAPE '\\'
                    OR lower(name) LIKE ?1 ESCAPE '\\')
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![pattern, limit * 12], row_to_channel)?;
        let mut found: Vec<(i32, Channel)> = rows
            .collect::<rusqlite::Result<Vec<Channel>>>()?
            .into_iter()
            .filter_map(|channel| {
                matterless_core::fuzzy::best_score(
                    [channel.display_name.as_str(), channel.name.as_str()],
                    query,
                )
                .map(|points| (points, channel))
            })
            .collect();
        // Recency breaks ties: two equally good matches are separated by which
        // one has anything happening in it.
        found.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then(right.1.last_post_at.cmp(&left.1.last_post_at))
        });
        Ok(found
            .into_iter()
            .take(limit as usize)
            .map(|(_, channel)| channel)
            .collect())
    }

    /// Custom emoji whose name matches `query` as a subsequence.
    pub fn custom_emoji_matching(&self, query: &str, limit: u32) -> Result<Vec<(String, String)>> {
        let connection = self.lock();
        let pattern = matterless_core::fuzzy::like_pattern(query);
        let mut statement = connection.prepare(
            "SELECT name, emoji_id FROM custom_emoji
             WHERE emoji_id != '' AND name LIKE ?1 ESCAPE '\\'
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![pattern, limit * 12], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut found: Vec<(i32, (String, String))> = rows
            .collect::<rusqlite::Result<Vec<(String, String)>>>()?
            .into_iter()
            .filter_map(|entry| {
                matterless_core::fuzzy::score(&entry.0, query).map(|points| (points, entry))
            })
            .collect();
        found.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then(left.1.0.len().cmp(&right.1.0.len()))
        });
        Ok(found
            .into_iter()
            .take(limit as usize)
            .map(|(_, entry)| entry)
            .collect())
    }

    /// A team's `name` -- the URL slug a permalink is built from, not its
    /// display name ("acme", not "Acme Corp").
    pub fn team_name(&self, team_id: &str) -> Result<Option<String>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT name FROM teams WHERE id = ?1",
                params![team_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Any team this reader is on. A direct message belongs to no team, but a
    /// permalink still needs one in the path -- which is what the official
    /// client does too.
    pub fn any_team_name(&self) -> Result<Option<String>> {
        let connection = self.lock();
        Ok(connection
            .query_row("SELECT name FROM teams ORDER BY name LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()?)
    }

    /// Which of these channel ids this store already holds.
    ///
    /// Asked of a browse list, whose channels come from the server: membership
    /// is the local half, and it is what tells a channel worth joining from one
    /// the reader is already in.
    pub fn known_channel_ids(&self, ids: &[String]) -> Result<std::collections::HashSet<String>> {
        if ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let connection = self.lock();
        let mut held = std::collections::HashSet::new();
        // Asked one at a time rather than through a built-up `IN` list: the
        // lists here are hundreds long, a prepared statement is reused across
        // the loop, and a hand-assembled clause is how a query gets an
        // injection in it.
        let mut statement = connection.prepare("SELECT 1 FROM channels WHERE id = ?1")?;
        for id in ids {
            if statement.exists(params![id])? {
                held.insert(id.clone());
            }
        }
        Ok(held)
    }

    /// Forgets a channel the reader has left.
    ///
    /// Its posts go with it: they are the server's, this client is no longer a
    /// member, and leaving them behind would keep the channel in search results
    /// and in the unread derivation.
    pub fn forget_channel(&self, channel_id: &str) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "DELETE FROM posts WHERE channel_id = ?1",
            params![channel_id],
        )?;
        connection.execute(
            "DELETE FROM channel_members WHERE channel_id = ?1",
            params![channel_id],
        )?;
        connection.execute("DELETE FROM channels WHERE id = ?1", params![channel_id])?;
        Ok(())
    }

    pub fn channel(&self, channel_id: &str) -> Result<Option<Channel>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT id, team_id, channel_type, name, display_name,
                        total_msg_count, total_msg_count_root, last_post_at, delete_at
                 FROM channels WHERE id = ?1",
                params![channel_id],
                row_to_channel,
            )
            .optional()?)
    }

    /// Everything the sidebar needs in one query: the channel plus its derived
    /// unread. Composing this from two calls per channel would be 114 round
    /// trips to paint a sidebar.
    pub fn channels_with_unread(&self, user_id: &str) -> Result<Vec<(Channel, Unread)>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT c.id, c.team_id, c.channel_type, c.name, c.display_name,
                    c.total_msg_count, c.total_msg_count_root, c.last_post_at, c.delete_at,
                    COALESCE(m.msg_count, 0), COALESCE(m.msg_count_root, 0),
                    COALESCE(m.mention_count, 0), COALESCE(m.mention_count_root, 0),
                    COALESCE(m.notify_props, '{}')
             FROM channels c
             LEFT JOIN channel_members m ON m.channel_id = c.id AND m.user_id = ?1
             WHERE c.delete_at = 0
             ORDER BY c.last_post_at DESC",
        )?;
        let rows = statement.query_map(params![user_id], |row| {
            let channel = Channel {
                id: row.get(0)?,
                team_id: row.get(1)?,
                channel_type: row.get(2)?,
                name: row.get(3)?,
                display_name: row.get(4)?,
                total_msg_count: row.get(5)?,
                total_msg_count_root: row.get(6)?,
                last_post_at: row.get(7)?,
                delete_at: row.get(8)?,
            };
            let seen: i64 = row.get(9)?;
            let seen_root: i64 = row.get(10)?;
            let notify_props: String = row.get(13)?;
            let props: HashMap<String, String> =
                serde_json::from_str(&notify_props).unwrap_or_default();
            let unread = Unread {
                messages: (channel.total_msg_count - seen).max(0),
                messages_root: (channel.total_msg_count_root - seen_root).max(0),
                mentions: row.get(11)?,
                mentions_root: row.get(12)?,
                muted: props.get("mark_unread").map(String::as_str) == Some("mention"),
            };
            Ok((channel, unread))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Reply counts for the roots actually on screen.
    ///
    /// Scoped to `root_ids` rather than the whole channel: the unscoped version
    /// grew with stored history instead of with the view, and measured at 60% of
    /// a plan build. Only roots in the current window can grow a footer, so only
    /// those need counting.
    pub fn thread_summaries_for(
        &self,
        channel_id: &str,
        root_ids: &[String],
    ) -> Result<HashMap<String, ThreadRollup>> {
        if root_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let connection = self.lock();
        let placeholders = vec!["?"; root_ids.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT root_id, COUNT(*), MAX(create_at), GROUP_CONCAT(DISTINCT user_id)
             FROM posts
             WHERE channel_id = ?1 AND delete_at = 0 AND root_id IN ({placeholders})
             GROUP BY root_id"
        ))?;
        let bindings = std::iter::once(channel_id.to_string())
            .chain(root_ids.iter().cloned())
            .collect::<Vec<String>>();
        let rows = statement.query_map(rusqlite::params_from_iter(bindings), |row| {
            let root_id: String = row.get(0)?;
            let participants: Option<String> = row.get(3)?;
            Ok((
                root_id,
                (
                    row.get::<_, i64>(1)?,
                    row.get::<_, Timestamp>(2)?,
                    participants
                        .unwrap_or_default()
                        .split(',')
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    // ------------------------------------------------------------ sync state

    pub fn sync_state(&self, channel_id: &str) -> Result<Option<SyncState>> {
        let connection = self.lock();
        let state = connection
            .query_row(
                "SELECT channel_id, synced_from, synced_to, oldest_post_id, newest_post_id,
                        reached_beginning, last_reconciled_at
                 FROM sync_state WHERE channel_id = ?1",
                params![channel_id],
                |row| {
                    Ok(SyncState {
                        channel_id: row.get(0)?,
                        synced_from: row.get(1)?,
                        synced_to: row.get(2)?,
                        oldest_post_id: row.get(3)?,
                        newest_post_id: row.get(4)?,
                        reached_beginning: row.get::<_, i64>(5)? != 0,
                        last_reconciled_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(state)
    }

    pub fn set_sync_state(&self, state: &SyncState) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "INSERT INTO sync_state (channel_id, synced_from, synced_to, oldest_post_id,
                                     newest_post_id, reached_beginning, last_reconciled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(channel_id) DO UPDATE SET
                synced_from = excluded.synced_from,
                synced_to = excluded.synced_to,
                oldest_post_id = excluded.oldest_post_id,
                newest_post_id = excluded.newest_post_id,
                reached_beginning = excluded.reached_beginning,
                last_reconciled_at = excluded.last_reconciled_at",
            params![
                state.channel_id,
                state.synced_from,
                state.synced_to,
                state.oldest_post_id,
                state.newest_post_id,
                i64::from(state.reached_beginning),
                state.last_reconciled_at,
            ],
        )?;
        Ok(())
    }

    pub fn set_ws_state(
        &self,
        connection_id: &str,
        sequence_number: i64,
        now: Timestamp,
    ) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "UPDATE ws_state SET connection_id = ?1, sequence_number = ?2, updated_at = ?3
             WHERE id = 1",
            params![connection_id, sequence_number, now],
        )?;
        Ok(())
    }

    pub fn ws_state(&self) -> Result<(String, i64)> {
        let connection = self.lock();
        Ok(connection.query_row(
            "SELECT connection_id, sequence_number FROM ws_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    /// Resync order for after a reconnect.
    ///
    /// Phase 0 proved this server never honours resume, so every reconnect is a
    /// full resync -- and with 114 active channels a uniform sweep would be
    /// ~14 s of rate-limited requests. Active channel first, then anything
    /// unread, then by recency; the caller trickles the tail.
    pub fn resync_order(&self, active_channel: Option<&str>, user_id: &str) -> Result<Vec<String>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT c.id
             FROM channels c
             LEFT JOIN channel_members m ON m.channel_id = c.id AND m.user_id = ?2
             WHERE c.delete_at = 0
             ORDER BY
                (c.id = ?1) DESC,
                (COALESCE(c.total_msg_count, 0) - COALESCE(m.msg_count, 0) > 0) DESC,
                COALESCE(m.mention_count, 0) DESC,
                c.last_post_at DESC",
        )?;
        let rows = statement.query_map(params![active_channel.unwrap_or(""), user_id], |row| {
            row.get::<_, String>(0)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------------------------------------------------------- render cache

    // --------------------------------------------------------- preferences

    pub fn upsert_preferences(&self, preferences: &[Preference]) -> Result<()> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        for preference in preferences {
            transaction.execute(
                "INSERT INTO preferences (user_id, category, name, value)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(user_id, category, name) DO UPDATE SET value = excluded.value",
                params![
                    preference.user_id,
                    preference.category,
                    preference.name,
                    preference.value
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Posts this reader has saved.
    ///
    /// "Saved message" is a *preference* on this server, not a field on the
    /// post -- category `flagged_post`, the post id as the name -- so it is read
    /// from where it lives rather than from the post.
    pub fn saved_post_ids(&self, user_id: &str) -> Result<std::collections::HashSet<String>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT name FROM preferences
             WHERE user_id = ?1 AND category = 'flagged_post' AND value = 'true'",
        )?;
        let rows = statement.query_map(params![user_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?)
    }

    /// Records one preference locally, so the UI reflects it before the next
    /// bootstrap re-reads them all.
    pub fn set_preference(
        &self,
        user_id: &str,
        category: &str,
        name: &str,
        value: &str,
    ) -> Result<()> {
        self.upsert_preferences(&[Preference {
            user_id: user_id.to_string(),
            category: category.to_string(),
            name: name.to_string(),
            value: value.to_string(),
        }])
    }

    pub fn delete_preference(&self, user_id: &str, category: &str, name: &str) -> Result<()> {
        let connection = self.lock();
        connection.execute(
            "DELETE FROM preferences WHERE user_id = ?1 AND category = ?2 AND name = ?3",
            params![user_id, category, name],
        )?;
        Ok(())
    }

    /// Marks a post deleted locally, after the server has accepted the delete.
    ///
    /// The websocket echo says the same thing, but a socket that is down would
    /// leave the message on screen after the reader watched it be deleted --
    /// and the two agree, because `upsert_posts` treats a tombstone as terminal.
    pub fn tombstone_post(&self, post_id: &str, at: Timestamp) -> Result<bool> {
        let connection = self.lock();
        let changed = connection.execute(
            "UPDATE posts SET delete_at = ?2 WHERE id = ?1 AND delete_at = 0",
            params![post_id, at],
        )?;
        Ok(changed > 0)
    }

    /// Records a pin locally after the server has accepted it.
    ///
    /// Deliberately leaves `update_at` alone, for the same reason patching
    /// reactions does: it is the markdown cache key, and pinning does not change
    /// a word of the message.
    pub fn set_post_pinned(&self, post_id: &str, pinned: bool) -> Result<bool> {
        let connection = self.lock();
        let changed = connection.execute(
            "UPDATE posts SET is_pinned = ?2 WHERE id = ?1 AND is_pinned != ?2",
            params![post_id, pinned],
        )?;
        Ok(changed > 0)
    }

    pub fn preference(&self, user_id: &str, category: &str, name: &str) -> Result<Option<String>> {
        let connection = self.lock();
        Ok(connection
            .query_row(
                "SELECT value FROM preferences WHERE user_id = ?1 AND category = ?2 AND name = ?3",
                params![user_id, category, name],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }
}

type PostRow = rusqlite::Result<Result<Post>>;

/// Turns what a reader typed into an FTS5 expression, or `None` when the local
/// index cannot answer it honestly.
///
/// Two jobs. It quotes every term, because FTS5 reads `-`, `*`, `:`, `^` and
/// `NEAR` as syntax -- a reader searching for `-Wall` would otherwise get a
/// parse error rather than results. And it *declines* a query carrying a
/// search modifier (`from:`, `in:`, `before:`, `after:`): the local index knows
/// nothing about authors or channels, so answering `from:amy budget` locally
/// would return every mention of a budget and quietly pretend it had honoured
/// the filter. Those go to the server alone.
pub fn fts_expression(typed: &str) -> Option<String> {
    const MODIFIERS: [&str; 6] = ["from:", "in:", "before:", "after:", "on:", "channel:"];
    let lowered = typed.to_lowercase();
    if MODIFIERS.iter().any(|modifier| lowered.contains(modifier)) {
        return None;
    }
    let terms: Vec<String> = typed
        .split_whitespace()
        .map(|term| term.replace('"', " ").trim().to_string())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let notify_props: String = row.get(7)?;
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        first_name: row.get(2)?,
        last_name: row.get(3)?,
        nickname: row.get(4)?,
        email: row.get(5)?,
        last_picture_update: row.get(6)?,
        notify_props: serde_json::from_str(&notify_props).unwrap_or_default(),
        roles: row.get(8)?,
    })
}

fn row_to_channel(row: &rusqlite::Row<'_>) -> rusqlite::Result<Channel> {
    Ok(Channel {
        id: row.get(0)?,
        team_id: row.get(1)?,
        channel_type: row.get(2)?,
        name: row.get(3)?,
        display_name: row.get(4)?,
        total_msg_count: row.get(5)?,
        total_msg_count_root: row.get(6)?,
        last_post_at: row.get(7)?,
        delete_at: row.get(8)?,
    })
}

fn row_to_post(row: &rusqlite::Row<'_>) -> PostRow {
    let file_ids: String = row.get(10)?;
    let props: String = row.get(11)?;
    let metadata: String = row.get(12)?;

    let build = || -> Result<Post> {
        Ok(Post {
            id: row.get(0)?,
            channel_id: row.get(1)?,
            user_id: row.get(2)?,
            root_id: row.get(3)?,
            create_at: row.get(4)?,
            update_at: row.get(5)?,
            edit_at: row.get(6)?,
            delete_at: row.get(7)?,
            message: row.get(8)?,
            post_type: row.get(9)?,
            file_ids: serde_json::from_str(&file_ids)?,
            props: serde_json::from_str(&props)?,
            metadata: serde_json::from_str::<PostMetadata>(&metadata)?,
            pending_post_id: row.get(13)?,
            is_pinned: row.get(14)?,
        })
    };
    Ok(build())
}

#[cfg(test)]
mod tests;
