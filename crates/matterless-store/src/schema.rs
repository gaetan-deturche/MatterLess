//! Schema and forward-only migrations, versioned with `PRAGMA user_version`.
//!
//! Design notes that are load-bearing rather than taste:
//!
//! * Timestamps are server milliseconds, stored as INTEGER. Local clocks never
//!   touch them -- Phase 0 confirmed clock skew is real.
//! * Ordering is always `(create_at, id)`. `create_at` alone collides.
//! * `posts.update_at` is the staleness guard: an upsert with an older or equal
//!   `update_at` is ignored, which is what makes the websocket and REST paths
//!   delivering the same post harmless instead of a duplicate.
//! * A deleted post is a tombstone (`delete_at != 0`), never a row removal: a
//!   deleted thread root still has to render as a placeholder holding replies.
//! * Search is FTS5 over `posts`, because Phase 0 found this server runs the
//!   database search backend with no Elasticsearch -- for recent history a local
//!   index genuinely beats asking the server.

use rusqlite::Connection;

/// Per-thread read state, which channel-level unread cannot express.
///
/// Phase 3 proved the need with a measurement rather than an argument: with
/// collapsed threads on, a reply is never a row in the channel, so the
/// "New messages" divider -- built from `channel_members.last_viewed_at` -- had
/// nothing in the stream to mark, and `stream_has_newer` came back false on
/// every sample while one channel's newest reply sat 2.5 hours after its newest
/// root. The counts are the server's, not derived here: `unread_replies` and
/// `unread_mentions` come straight from the threads endpoint and its events.
const MIGRATION_4: &str = "
CREATE TABLE IF NOT EXISTS threads (
    root_id         TEXT PRIMARY KEY,
    channel_id      TEXT NOT NULL DEFAULT '',
    following       INTEGER NOT NULL DEFAULT 1,
    reply_count     INTEGER NOT NULL DEFAULT 0,
    last_reply_at   INTEGER NOT NULL DEFAULT 0,
    last_viewed_at  INTEGER NOT NULL DEFAULT 0,
    unread_replies  INTEGER NOT NULL DEFAULT 0,
    unread_mentions INTEGER NOT NULL DEFAULT 0,
    is_urgent       INTEGER NOT NULL DEFAULT 0,
    delete_at       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS threads_channel ON threads(channel_id, last_reply_at DESC);
";

/// The sidebar as the reader arranged it, server-side.
///
/// Two tables rather than a JSON blob of channel ids: the order inside a
/// category is a column, so a category's channels come back ordered by SQL
/// rather than by whatever the JSON decoded into.
const MIGRATION_5: &str = "
CREATE TABLE IF NOT EXISTS sidebar_categories (
    id            TEXT PRIMARY KEY,
    team_id       TEXT NOT NULL DEFAULT '',
    category_type TEXT NOT NULL DEFAULT '',
    display_name  TEXT NOT NULL DEFAULT '',
    sort_order    INTEGER NOT NULL DEFAULT 0,
    sorting       TEXT NOT NULL DEFAULT '',
    muted         INTEGER NOT NULL DEFAULT 0,
    collapsed     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sidebar_category_channels (
    category_id TEXT NOT NULL,
    channel_id  TEXT NOT NULL,
    position    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (category_id, channel_id)
);
CREATE INDEX IF NOT EXISTS sidebar_channel_lookup
    ON sidebar_category_channels(channel_id);
";

/// Which emoji names are custom, and their ids.
///
/// The negative answer is stored too -- an empty id means "asked, and it is a
/// standard emoji" -- because otherwise every `:tada:` in every channel would
/// ask the server again on every render.
const MIGRATION_6: &str = "
CREATE TABLE IF NOT EXISTS custom_emoji (
    name     TEXT PRIMARY KEY,
    emoji_id TEXT NOT NULL DEFAULT ''
);
";

// Migration 7 adds `posts.is_pinned`. A column rather than a table, so the
// repair check below looks at columns too: `ADD COLUMN` has no `IF NOT EXISTS`
// in SQLite, and it only ever runs when the column is genuinely absent.
const MIGRATION_7: &str = "
ALTER TABLE posts ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;
";

pub const TARGET_VERSION: i64 = 7;

// Migration 1 is frozen: the shell now keeps a real database with real history,
// so every change gets its own step from here.
//
// Migration 2 adds the index the thread-footer query needs. Measured before it
// existed: `thread_summaries` was 60% of a plan build (5.9 ms of 9.9 ms at 112
// rows), because grouping by `root_id` after filtering on `channel_id` had to
// build a temporary B-tree every time.
const MIGRATION_2: &str = r#"
CREATE INDEX posts_threads ON posts(channel_id, root_id) WHERE root_id != '';
"#;

const MIGRATION_1: &str = r#"
CREATE TABLE users (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL DEFAULT '',
    first_name          TEXT NOT NULL DEFAULT '',
    last_name           TEXT NOT NULL DEFAULT '',
    nickname            TEXT NOT NULL DEFAULT '',
    email               TEXT NOT NULL DEFAULT '',
    last_picture_update INTEGER NOT NULL DEFAULT 0,
    notify_props        TEXT NOT NULL DEFAULT '{}',
    roles               TEXT NOT NULL DEFAULT ''
);

CREATE TABLE teams (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL DEFAULT '',
    display_name TEXT NOT NULL DEFAULT ''
);

CREATE TABLE channels (
    id                   TEXT PRIMARY KEY,
    team_id              TEXT NOT NULL DEFAULT '',
    channel_type         TEXT NOT NULL DEFAULT '',
    name                 TEXT NOT NULL DEFAULT '',
    display_name         TEXT NOT NULL DEFAULT '',
    total_msg_count      INTEGER NOT NULL DEFAULT 0,
    total_msg_count_root INTEGER NOT NULL DEFAULT 0,
    last_post_at         INTEGER NOT NULL DEFAULT 0,
    delete_at            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX channels_by_activity ON channels(delete_at, last_post_at DESC);

-- Unread and mention counts are derived from this against channels, never
-- read from a flag.
CREATE TABLE channel_members (
    channel_id         TEXT NOT NULL,
    user_id            TEXT NOT NULL,
    last_viewed_at     INTEGER NOT NULL DEFAULT 0,
    msg_count          INTEGER NOT NULL DEFAULT 0,
    msg_count_root     INTEGER NOT NULL DEFAULT 0,
    mention_count      INTEGER NOT NULL DEFAULT 0,
    mention_count_root INTEGER NOT NULL DEFAULT 0,
    notify_props       TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (channel_id, user_id)
);

CREATE TABLE posts (
    id              TEXT PRIMARY KEY,
    channel_id      TEXT NOT NULL,
    user_id         TEXT NOT NULL DEFAULT '',
    root_id         TEXT NOT NULL DEFAULT '',
    create_at       INTEGER NOT NULL DEFAULT 0,
    update_at       INTEGER NOT NULL DEFAULT 0,
    edit_at         INTEGER NOT NULL DEFAULT 0,
    delete_at       INTEGER NOT NULL DEFAULT 0,
    message         TEXT NOT NULL DEFAULT '',
    post_type       TEXT NOT NULL DEFAULT '',
    file_ids        TEXT NOT NULL DEFAULT '[]',
    props           TEXT NOT NULL DEFAULT 'null',
    metadata        TEXT NOT NULL DEFAULT '{}',
    pending_post_id TEXT NOT NULL DEFAULT ''
);
-- The channel stream query: newest first, tie-broken by id.
CREATE INDEX posts_by_channel ON posts(channel_id, create_at DESC, id DESC);
CREATE INDEX posts_by_root    ON posts(root_id) WHERE root_id != '';
CREATE INDEX posts_by_update  ON posts(update_at);

CREATE TABLE reactions (
    post_id    TEXT NOT NULL,
    user_id    TEXT NOT NULL,
    emoji_name TEXT NOT NULL,
    create_at  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (post_id, user_id, emoji_name)
);

CREATE TABLE files (
    id        TEXT PRIMARY KEY,
    post_id   TEXT NOT NULL,
    name      TEXT NOT NULL DEFAULT '',
    extension TEXT NOT NULL DEFAULT '',
    size      INTEGER NOT NULL DEFAULT 0,
    mime_type TEXT NOT NULL DEFAULT '',
    width     INTEGER NOT NULL DEFAULT 0,
    height    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX files_by_post ON files(post_id);

-- Per-channel contiguity. `synced_from`/`synced_to` bound the one stretch of
-- history known to have no holes; a jump-to-permalink outside it creates an
-- island, and the gap has to be recorded or it becomes a silent hole later.
CREATE TABLE sync_state (
    channel_id         TEXT PRIMARY KEY,
    synced_from        INTEGER NOT NULL DEFAULT 0,
    synced_to          INTEGER NOT NULL DEFAULT 0,
    oldest_post_id     TEXT NOT NULL DEFAULT '',
    newest_post_id     TEXT NOT NULL DEFAULT '',
    reached_beginning  INTEGER NOT NULL DEFAULT 0,
    last_reconciled_at INTEGER NOT NULL DEFAULT 0
);

-- Single row. Phase 0 proved this server refuses resume even after an RST, so
-- these are kept for diagnostics and for the day a server does honour them.
CREATE TABLE ws_state (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    connection_id   TEXT NOT NULL DEFAULT '',
    sequence_number INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL DEFAULT 0
);
INSERT INTO ws_state (id) VALUES (1);

-- Server-side display and behaviour settings. `collapsed_reply_threads` here
-- overrides the server's CollapsedThreads, so this is not cosmetic.
CREATE TABLE preferences (
    user_id  TEXT NOT NULL,
    category TEXT NOT NULL,
    name     TEXT NOT NULL,
    value    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, category, name)
);

CREATE VIRTUAL TABLE posts_fts USING fts5(
    message,
    content='posts',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER posts_fts_insert AFTER INSERT ON posts BEGIN
    INSERT INTO posts_fts(rowid, message) VALUES (new.rowid, new.message);
END;
CREATE TRIGGER posts_fts_delete AFTER DELETE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, message) VALUES('delete', old.rowid, old.message);
END;
CREATE TRIGGER posts_fts_update AFTER UPDATE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, message) VALUES('delete', old.rowid, old.message);
    INSERT INTO posts_fts(rowid, message) VALUES (new.rowid, new.message);
END;
"#;

// Migration 3 removes the render cache table. It held parsed markdown as JSON
// and did get 100% hits, but only cut a plan build from 98 ms to 68 ms: decoding
// a nested tree costs nearly what parsing the markdown did. An in-process
// `Arc<Vec<Node>>` cache removes the decode and the tree copy both, and the
// table's only remaining value was ~24 ms on the first render after a launch.
const MIGRATION_3: &str = r#"
DROP TABLE IF EXISTS post_render;
"#;

/// Tables the target schema requires that the database does not have.
///
/// Cheap (one query against `sqlite_master`) and only at startup, which is a
/// small price for never again trusting a version number over the thing it
/// describes.
/// Columns added after their table, checked the same way tables are: a version
/// number is a claim, and this is the evidence.
fn missing_columns(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    const REQUIRED: [(&str, &str); 1] = [("posts", "is_pinned")];
    let mut absent = Vec::new();
    for (table, column) in REQUIRED {
        let present: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
            [column],
            |row| row.get(0),
        )?;
        if present == 0 {
            absent.push(format!("{table}.{column}"));
        }
    }
    Ok(absent)
}

fn missing_tables(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    const REQUIRED: [&str; 11] = [
        "posts",
        "channels",
        "channel_members",
        "users",
        "teams",
        "preferences",
        "sync_state",
        "threads",
        "sidebar_categories",
        "sidebar_category_channels",
        "custom_emoji",
    ];
    let mut absent = Vec::new();
    for name in REQUIRED {
        let present: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )?;
        if present == 0 {
            absent.push(name.to_string());
        }
    }
    Ok(absent)
}

/// Applies the pragmas every connection needs, then any outstanding migration.
pub fn prepare(connection: &Connection) -> rusqlite::Result<()> {
    // WAL lets readers run while a write is in flight; NORMAL is safe under WAL
    // and avoids an fsync per commit.
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "busy_timeout", 5000)?;
    migrate(connection)
}

fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    // The version is not trusted on its own. A build that carried
    // `TARGET_VERSION = 4` without yet running migration 4 stamped the database
    // as migrated and left it without a `threads` table -- after which the
    // version gate skipped the migration forever, and every thread query failed
    // with "no such table". Every migration is written to be idempotent
    // (`IF NOT EXISTS`), so the cheap fix is to also check that what a version
    // claims is actually there.
    let missing = missing_tables(connection)?;
    let absent_columns = missing_columns(connection)?;
    if version >= TARGET_VERSION && missing.is_empty() && absent_columns.is_empty() {
        return Ok(());
    }
    if !absent_columns.is_empty() {
        tracing::warn!(
            version,
            missing = absent_columns.join(","),
            "a column the schema version claims is absent; adding it"
        );
    }
    if !missing.is_empty() {
        tracing::warn!(
            version,
            missing = missing.join(","),
            "the schema version claims more than the database holds; repairing"
        );
    }
    if version < 1 {
        connection.execute_batch(MIGRATION_1)?;
    }
    if version < 2 {
        connection.execute_batch(MIGRATION_2)?;
    }
    if version < 3 {
        connection.execute_batch(MIGRATION_3)?;
    }
    if version < 4 || missing.iter().any(|name| name == "threads") {
        connection.execute_batch(MIGRATION_4)?;
    }
    if version < 5 || missing.iter().any(|name| name == "sidebar_categories") {
        connection.execute_batch(MIGRATION_5)?;
    }
    if version < 6 || missing.iter().any(|name| name == "custom_emoji") {
        connection.execute_batch(MIGRATION_6)?;
    }
    // Keyed on the column being absent rather than on the version: `ADD COLUMN`
    // is not idempotent, so asking first is the only safe form.
    if absent_columns.iter().any(|name| name == "posts.is_pinned") {
        connection.execute_batch(MIGRATION_7)?;
    }
    connection.pragma_update(None, "user_version", TARGET_VERSION)?;
    tracing::info!(from = version, to = TARGET_VERSION, "store migrated");
    Ok(())
}
