//! The memory database: one SQLite file at
//! `~/Library/Application Support/Freebyrd/memory.db`. Never under
//! Documents, which syncs to iCloud: everything in it stays on this Mac.
//!
//! `activity` holds one row per thing that happened (a prompt, a reply, an
//! edit, a page visit), with full-text search over its text. Each row has a
//! source-specific ID, so reading a source twice adds nothing twice, and each
//! source keeps a `cursor` so a run reads only what is new.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};

/// Schema versions, applied in order; `PRAGMA user_version` records how many
/// have run. Add new versions at the end; never edit one that has shipped.
const MIGRATIONS: &[&str] = &[
    r"
CREATE TABLE cursor (
    source TEXT NOT NULL,
    key TEXT NOT NULL,
    position INTEGER NOT NULL,
    identity TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (source, key)
) WITHOUT ROWID;

CREATE TABLE project (
    id INTEGER PRIMARY KEY,
    root TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);

CREATE TABLE session (
    id TEXT PRIMARY KEY,
    project_id INTEGER REFERENCES project (id),
    title TEXT,
    started_at INTEGER,
    last_at INTEGER
) WITHOUT ROWID;

CREATE TABLE page_text (
    id INTEGER PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE,
    text TEXT NOT NULL
);

CREATE TABLE activity (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    ts INTEGER NOT NULL,
    duration_ms INTEGER,
    project_id INTEGER REFERENCES project (id),
    session_id TEXT,
    app TEXT,
    title TEXT,
    url TEXT,
    detail TEXT,
    text_id INTEGER REFERENCES page_text (id),
    ext_id TEXT NOT NULL,
    UNIQUE (source, ext_id)
);
CREATE INDEX activity_ts ON activity (ts);
CREATE INDEX activity_project_ts ON activity (project_id, ts);
CREATE INDEX activity_text ON activity (text_id);

CREATE VIRTUAL TABLE activity_fts USING fts5 (
    title, detail, url,
    content = 'activity', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER activity_fts_insert AFTER INSERT ON activity BEGIN
    INSERT INTO activity_fts (rowid, title, detail, url)
    VALUES (new.id, new.title, new.detail, new.url);
END;
CREATE TRIGGER activity_fts_delete AFTER DELETE ON activity BEGIN
    INSERT INTO activity_fts (activity_fts, rowid, title, detail, url)
    VALUES ('delete', old.id, old.title, old.detail, old.url);
END;

CREATE VIRTUAL TABLE page_fts USING fts5 (
    text,
    content = 'page_text', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER page_fts_insert AFTER INSERT ON page_text BEGIN
    INSERT INTO page_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER page_fts_delete AFTER DELETE ON page_text BEGIN
    INSERT INTO page_fts (page_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TABLE recap (
    day TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    body TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE card (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    seen_at INTEGER,
    dismissed_at INTEGER
);
",
    r"
-- Version 2: conversations (WhatsApp chats, email threads) and their
-- messages; plans, the calendar-able things Freebyrd found (promises in
-- conversations, events seen on screen); buttons and data on cards.

CREATE TABLE conversation (
    id INTEGER PRIMARY KEY,
    app TEXT NOT NULL,
    key TEXT NOT NULL,
    title TEXT NOT NULL,
    work INTEGER,
    last_seen_at INTEGER NOT NULL,
    UNIQUE (app, key)
);

CREATE TABLE message (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversation (id),
    sender TEXT NOT NULL,
    text TEXT NOT NULL,
    sent_at INTEGER,
    sent_label TEXT,
    first_seen_at INTEGER NOT NULL,
    hash TEXT NOT NULL,
    checked_at INTEGER,
    UNIQUE (conversation_id, hash)
);
CREATE INDEX message_conversation ON message (conversation_id, id);
CREATE INDEX message_unchecked ON message (conversation_id) WHERE checked_at IS NULL;

CREATE VIRTUAL TABLE message_fts USING fts5 (
    sender, text,
    content = 'message', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER message_fts_insert AFTER INSERT ON message BEGIN
    INSERT INTO message_fts (rowid, sender, text) VALUES (new.id, new.sender, new.text);
END;
CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
    INSERT INTO message_fts (message_fts, rowid, sender, text)
    VALUES ('delete', old.id, old.sender, old.text);
END;

CREATE TABLE plan (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    person TEXT,
    start_at INTEGER NOT NULL,
    all_day INTEGER NOT NULL,
    minutes INTEGER,
    location TEXT,
    quote TEXT NOT NULL,
    source TEXT NOT NULL,
    conversation_id INTEGER REFERENCES conversation (id),
    message_id INTEGER REFERENCES message (id),
    state TEXT NOT NULL,
    event_id TEXT,
    note TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX plan_start ON plan (start_at);

ALTER TABLE card ADD COLUMN actions TEXT NOT NULL DEFAULT '[]';
ALTER TABLE card ADD COLUMN data TEXT;
",
    // Fireflies meetings now keep their summaries: read them again, from
    // the first read's window, on the next pass. Action items lose the
    // recording time ("(12:24)") that reads copied only sometimes, so drop
    // the copies that kept it.
    r"
DELETE FROM cursor WHERE source = 'connector' AND key IN ('fireflies', 'fireflies-since');

DELETE FROM message WHERE id IN (
    SELECT timed.id FROM message timed
    JOIN conversation c ON c.id = timed.conversation_id AND c.app = 'fireflies'
    JOIN message plain ON plain.conversation_id = timed.conversation_id
        AND plain.id != timed.id AND plain.sender = timed.sender
        AND timed.text GLOB plain.text || ' ([0-9]*:[0-9][0-9])'
    WHERE timed.id NOT IN (SELECT message_id FROM plan WHERE message_id IS NOT NULL)
);
",
    r"
-- Version 4: repeated work (`routines`). A pattern is something the person
-- does again and again, kept once by its key, with the fix Claude drafted
-- for it and what the person said. A job is a fix they approved: a skill
-- or a scheduled job. A job_run is one run of a scheduled job.

CREATE TABLE pattern (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    signature TEXT NOT NULL,
    evidence TEXT NOT NULL,
    score REAL NOT NULL,
    state TEXT NOT NULL,
    draft TEXT,
    note TEXT,
    first_seen_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE job (
    id INTEGER PRIMARY KEY,
    pattern_id INTEGER REFERENCES pattern (id),
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    schedule TEXT,
    servers TEXT NOT NULL DEFAULT '[]',
    path TEXT,
    hash TEXT,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE job_run (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES job (id),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    outcome TEXT,
    result TEXT,
    cost_usd REAL
);
CREATE INDEX job_run_job ON job_run (job_id, started_at);
",
    r"
-- Version 5: the notch's Up next line. A plan the person marked done, or
-- put away until later (until the next working day starts).
ALTER TABLE plan ADD COLUMN done_at INTEGER;
ALTER TABLE plan ADD COLUMN hidden_until INTEGER;
",
    r"
-- Version 6: what was said in a meeting, a speaker's turn per row, read
-- from Fireflies when a question asks for someone's words (`transcripts`).
CREATE TABLE meeting_line (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversation (id),
    at_seconds INTEGER NOT NULL,
    speaker TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX meeting_line_conversation ON meeting_line (conversation_id, at_seconds);

CREATE VIRTUAL TABLE meeting_line_fts USING fts5 (
    speaker, text,
    content = 'meeting_line', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER meeting_line_fts_insert AFTER INSERT ON meeting_line BEGIN
    INSERT INTO meeting_line_fts (rowid, speaker, text) VALUES (new.id, new.speaker, new.text);
END;
CREATE TRIGGER meeting_line_fts_delete AFTER DELETE ON meeting_line BEGIN
    INSERT INTO meeting_line_fts (meeting_line_fts, rowid, speaker, text)
    VALUES ('delete', old.id, old.speaker, old.text);
END;
",
    r"
-- Version 7: X (`x`). A post the person scrolled past, kept once by its
-- status ID, with what they did with it. An idea is what Claude found to
-- use in a post they bookmarked. A fit is an idea weighed against a project
-- they worked on, and what came of it.
CREATE TABLE x_post (
    id INTEGER PRIMARY KEY,
    status TEXT NOT NULL UNIQUE,
    author TEXT NOT NULL,
    name TEXT,
    text TEXT NOT NULL,
    posted_at INTEGER,
    links TEXT NOT NULL DEFAULT '[]',
    media INTEGER NOT NULL DEFAULT 0,
    quoted TEXT,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    seen_count INTEGER NOT NULL DEFAULT 1,
    opened_at INTEGER,
    liked_at INTEGER,
    bookmarked_at INTEGER,
    unbookmarked_at INTEGER
);
CREATE INDEX x_post_seen ON x_post (last_seen_at);
CREATE INDEX x_post_bookmarked ON x_post (bookmarked_at) WHERE bookmarked_at IS NOT NULL;

CREATE VIRTUAL TABLE x_post_fts USING fts5 (
    author, name, text,
    content = 'x_post', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER x_post_fts_insert AFTER INSERT ON x_post BEGIN
    INSERT INTO x_post_fts (rowid, author, name, text) VALUES (new.id, new.author, new.name, new.text);
END;
CREATE TRIGGER x_post_fts_delete AFTER DELETE ON x_post BEGIN
    INSERT INTO x_post_fts (x_post_fts, rowid, author, name, text)
    VALUES ('delete', old.id, old.author, old.name, old.text);
END;
CREATE TRIGGER x_post_fts_update AFTER UPDATE OF author, name, text ON x_post BEGIN
    INSERT INTO x_post_fts (x_post_fts, rowid, author, name, text)
    VALUES ('delete', old.id, old.author, old.name, old.text);
    INSERT INTO x_post_fts (rowid, author, name, text) VALUES (new.id, new.author, new.name, new.text);
END;

CREATE TABLE x_idea (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL UNIQUE REFERENCES x_post (id),
    state TEXT NOT NULL,
    kind TEXT,
    name TEXT,
    what TEXT,
    use_when TEXT,
    words TEXT,
    note TEXT,
    read_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX x_idea_state ON x_idea (state);

CREATE VIRTUAL TABLE x_idea_fts USING fts5 (
    name, what, use_when, words,
    content = 'x_idea', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER x_idea_fts_insert AFTER INSERT ON x_idea BEGIN
    INSERT INTO x_idea_fts (rowid, name, what, use_when, words)
    VALUES (new.id, new.name, new.what, new.use_when, new.words);
END;
CREATE TRIGGER x_idea_fts_delete AFTER DELETE ON x_idea BEGIN
    INSERT INTO x_idea_fts (x_idea_fts, rowid, name, what, use_when, words)
    VALUES ('delete', old.id, old.name, old.what, old.use_when, old.words);
END;
CREATE TRIGGER x_idea_fts_update AFTER UPDATE OF name, what, use_when, words ON x_idea BEGIN
    INSERT INTO x_idea_fts (x_idea_fts, rowid, name, what, use_when, words)
    VALUES ('delete', old.id, old.name, old.what, old.use_when, old.words);
    INSERT INTO x_idea_fts (rowid, name, what, use_when, words)
    VALUES (new.id, new.name, new.what, new.use_when, new.words);
END;

CREATE TABLE x_fit (
    id INTEGER PRIMARY KEY,
    idea_id INTEGER NOT NULL REFERENCES x_idea (id),
    project_id INTEGER NOT NULL REFERENCES project (id),
    verdict TEXT NOT NULL,
    why TEXT,
    step TEXT,
    checked_at INTEGER NOT NULL,
    answered_at INTEGER,
    UNIQUE (idea_id, project_id)
);
",
    r"
-- Version 8: Freebyrd opens bookmarked posts itself (`x::open`): X's public
-- data, stills across videos, linked pages, and the browser for threads and
-- long posts. What it found is kept per post, with a small picture for the
-- card. Ideas get a category, what their pictures show, and how to make or
-- use them; those read from text alone are read again.
CREATE TABLE x_open (
    post_id INTEGER PRIMARY KEY REFERENCES x_post (id),
    state TEXT NOT NULL,
    text TEXT,
    thread TEXT,
    pages TEXT NOT NULL DEFAULT '[]',
    looked TEXT NOT NULL DEFAULT '[]',
    thumb BLOB,
    note TEXT,
    tries INTEGER NOT NULL DEFAULT 0,
    first_tried_at INTEGER,
    tried_at INTEGER,
    opened_at INTEGER
);

ALTER TABLE x_idea ADD COLUMN category TEXT;
ALTER TABLE x_idea ADD COLUMN shows TEXT;
ALTER TABLE x_idea ADD COLUMN how TEXT;

DROP TRIGGER x_idea_fts_insert;
DROP TRIGGER x_idea_fts_delete;
DROP TRIGGER x_idea_fts_update;
DROP TABLE x_idea_fts;
CREATE VIRTUAL TABLE x_idea_fts USING fts5 (
    name, what, use_when, words, shows, how,
    content = 'x_idea', content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER x_idea_fts_insert AFTER INSERT ON x_idea BEGIN
    INSERT INTO x_idea_fts (rowid, name, what, use_when, words, shows, how)
    VALUES (new.id, new.name, new.what, new.use_when, new.words, new.shows, new.how);
END;
CREATE TRIGGER x_idea_fts_delete AFTER DELETE ON x_idea BEGIN
    INSERT INTO x_idea_fts (x_idea_fts, rowid, name, what, use_when, words, shows, how)
    VALUES ('delete', old.id, old.name, old.what, old.use_when, old.words, old.shows, old.how);
END;
CREATE TRIGGER x_idea_fts_update AFTER UPDATE OF name, what, use_when, words, shows, how ON x_idea BEGIN
    INSERT INTO x_idea_fts (x_idea_fts, rowid, name, what, use_when, words, shows, how)
    VALUES ('delete', old.id, old.name, old.what, old.use_when, old.words, old.shows, old.how);
    INSERT INTO x_idea_fts (rowid, name, what, use_when, words, shows, how)
    VALUES (new.id, new.name, new.what, new.use_when, new.words, new.shows, new.how);
END;
INSERT INTO x_idea_fts (x_idea_fts) VALUES ('rebuild');

UPDATE x_idea SET state = 'new' WHERE state IN ('idea', 'none', 'failed');
DELETE FROM x_fit WHERE verdict = 'no';
",
    r"
-- Version 9: ideas at the moment they are needed (`moments`). A moment is
-- what the person started in the app in front: a page or window by its
-- title, and in a chat app or Claude Code the last thing they typed. Each
-- one weighed is kept with what was picked and the card it made. A verdict
-- is the person's answer about one idea (an X bookmark, a page, a session,
-- a skill) in one place (a site or an app), or everywhere ('').
CREATE TABLE moment (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    place TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    ask TEXT,
    signature TEXT NOT NULL,
    weighed INTEGER NOT NULL DEFAULT 0,
    top_score REAL,
    asked_claude INTEGER NOT NULL DEFAULT 0,
    picks TEXT NOT NULL DEFAULT '[]',
    card_id INTEGER,
    note TEXT
);
CREATE INDEX moment_at ON moment (at);
CREATE INDEX moment_signature ON moment (signature);

CREATE TABLE idea_verdict (
    item TEXT NOT NULL,
    place TEXT NOT NULL,
    verdict TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (item, place, verdict)
);
",
    r"
-- Version 10: the to-do list (`todos`). A to-do with a date is a plan (a
-- promise, a Notion task, one typed with a date); a todo row is one without:
-- an action item from a meeting, the person's (`mine`) or someone else's
-- for them, or one the person typed. A meeting's item is listed once. Given
-- a date, a todo becomes a plan (`plan_id`) and the plan is listed instead.
-- No foreign keys: a meeting read again may replace its messages.
CREATE TABLE todo (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    mine INTEGER NOT NULL,
    person TEXT,
    origin TEXT NOT NULL,
    source TEXT NOT NULL,
    conversation_id INTEGER,
    message_id INTEGER UNIQUE,
    plan_id INTEGER,
    said_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    done_at INTEGER,
    hidden_until INTEGER,
    removed_at INTEGER
);
CREATE INDEX todo_open ON todo (said_at) WHERE done_at IS NULL AND removed_at IS NULL;
",
    r"
-- Version 11: people (`people`). A person is someone the person deals
-- with, seen under names and email addresses in WhatsApp, Gmail, Fireflies
-- meetings and plans, joined on this Mac by plain code. An alias is one such
-- name or address, normalized (`value`), as written (`shown`); an email
-- seen with a name keeps it (`pair`). One the person joined or split by hand
-- (`by_hand`) is never moved by code. An alias with no person is a first
-- name more than one person has. A sighting is a conversation an alias was
-- seen in: a chat with them, a message from them, a meeting they attended
-- or owe an action item from.
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    note TEXT,
    hidden INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE person_alias (
    value TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    shown TEXT NOT NULL,
    pair TEXT,
    person_id INTEGER REFERENCES person (id),
    by_hand INTEGER NOT NULL DEFAULT 0,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX person_alias_person ON person_alias (person_id);

CREATE TABLE person_seen (
    alias TEXT NOT NULL,
    conversation_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    last_at INTEGER NOT NULL,
    PRIMARY KEY (alias, conversation_id, role)
) WITHOUT ROWID;
CREATE INDEX person_seen_conversation ON person_seen (conversation_id);
",
    r"
-- Version 12: moments by conversation (`moments`). A chat or a Claude Code
-- session is weighed at its start; a later message only when Jev finds it
-- turned to something new (`turn`, its probability). One that did not is
-- kept as `skipped`, weighed against nothing.
ALTER TABLE moment ADD COLUMN conversation TEXT;
ALTER TABLE moment ADD COLUMN turn REAL;
ALTER TABLE moment ADD COLUMN skipped INTEGER NOT NULL DEFAULT 0;
CREATE INDEX moment_conversation ON moment (conversation);
",
    r"
-- Version 13: night shift (`night`). Work the person queues before they stop,
-- run while they are away, waiting in the notch when they are back. `kind`
-- is preview, final or list; `spec` what to do, `result` what came of it
-- (both JSON); `dir` its folder under Freebyrd's. A final's `parent_id` is
-- the preview it was approved from. `acted_at` is when the person kept,
-- approved or put it away; until then a finished job waits for them.
CREATE TABLE night_job (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    sub TEXT NOT NULL DEFAULT '',
    project TEXT,
    spec TEXT NOT NULL,
    state TEXT NOT NULL,
    position INTEGER NOT NULL,
    parent_id INTEGER REFERENCES night_job (id),
    dir TEXT,
    session_id TEXT,
    progress TEXT,
    result TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    acted_at INTEGER,
    action TEXT
);
CREATE INDEX night_job_state ON night_job (state, position);
",
    r"
-- Version 14: Waiting on also lists what is read from places that change
-- it: Notion tasks others owe the person (`connectors`), and work threads
-- where the person waits on the other side (`asks`). Such a to-do keeps
-- that place's id for it (`source_id`: notion:<page id>, thread:<conversation
-- id>), so a later read updates or closes it, and the date it has there
-- (`due_at`, local midnight).
ALTER TABLE todo ADD COLUMN source_id TEXT;
ALTER TABLE todo ADD COLUMN due_at INTEGER;
CREATE UNIQUE INDEX todo_source_id ON todo (source_id) WHERE source_id IS NOT NULL;
",
    r"
-- Version 15: what moved a to-do (`progress`). After a Claude Code session
-- goes quiet, or a reply lands in a thread tied to a to-do, Claude says which
-- items it moved; each development is kept by the item it is about (`item`:
-- todo:<id> or plan:<id>). `state` is moved, done or yours (something owed to
-- the person came back to them, with their `next` step); `source` is where
-- it happened and `at` when.
CREATE TABLE progress (
    id INTEGER PRIMARY KEY,
    item TEXT NOT NULL,
    state TEXT NOT NULL,
    note TEXT NOT NULL,
    next TEXT,
    source TEXT NOT NULL,
    at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX progress_item ON progress (item, at);
",
    r"
-- Version 16: who an email went to (`message.recipients`): the names in
-- Gmail's to and cc, the person left out, so a forward is known by who it
-- went to. Their addresses join their pages (`people`, role recipient).
-- Older reads carry none; a thread read again fills them in.
ALTER TABLE message ADD COLUMN recipients TEXT;
",
];

pub struct Store {
    conn: Connection,
}

/// One thing that happened, from one source.
#[derive(Debug, Default, Clone)]
pub struct NewActivity<'a> {
    pub source: &'a str,
    pub kind: &'a str,
    /// Milliseconds since the Unix epoch.
    pub ts: i64,
    pub duration_ms: Option<i64>,
    pub project_id: Option<i64>,
    pub session_id: Option<&'a str>,
    pub app: Option<&'a str>,
    pub title: Option<&'a str>,
    pub url: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub text_id: Option<i64>,
    /// Unique within the source, so the same thing is never stored twice.
    pub ext_id: &'a str,
}

impl Store {
    pub fn default_path() -> Option<PathBuf> {
        Some(crate::home()?.join("Library/Application Support/Freebyrd/memory.db"))
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path().context("HOME is not set")?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Opens the memory to read only (the brief, and a copy being looked
    /// at): SQLite's read-only flag and `query_only`, never a migration,
    /// never a new file. A memory of another schema version is refused,
    /// since the queries here are written for this one.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening {} to read", path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "query_only", "ON")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let known = i64::try_from(MIGRATIONS.len())?;
        if version != known {
            bail!(
                "the memory at {} is version {version} and this build reads version {known}",
                path.display()
            );
        }
        Ok(Self { conn })
    }

    fn init(conn: Connection) -> Result<Self> {
        // The app's background reader and the overlay use the file at the
        // same time; write-ahead logging lets them.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let applied = |conn: &Connection| -> rusqlite::Result<i64> {
            conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        };
        if applied(&self.conn)? >= i64::try_from(MIGRATIONS.len())? {
            return Ok(());
        }
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            let version = i64::try_from(index)? + 1;
            // The worker and the overlay open the memory at the same moment
            // at startup: each takes the write lock before reading the
            // version again, so one applies it and the other skips it
            // (2026-10-14 both ran version 16's ALTER, and one failed).
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            if applied(&tx)? >= version {
                continue;
            }
            tx.execute_batch(sql)
                .with_context(|| format!("upgrading the memory to version {version}"))?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Groups writes; much faster than one implicit transaction per row.
    pub fn transaction(&self) -> Result<Transaction<'_>> {
        Ok(self.conn.unchecked_transaction()?)
    }

    /// Stores `activity` unless its `ext_id` is already stored. Returns
    /// whether it was new.
    pub fn add_activity(&self, activity: &NewActivity) -> Result<bool> {
        let mut statement = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO activity
             (source, kind, ts, duration_ms, project_id, session_id, app, title, url, detail, text_id, ext_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        let added = statement.execute(params![
            activity.source,
            activity.kind,
            activity.ts,
            activity.duration_ms,
            activity.project_id,
            activity.session_id,
            activity.app,
            activity.title,
            activity.url,
            activity.detail,
            activity.text_id,
            activity.ext_id,
        ])?;
        Ok(added == 1)
    }

    /// The ID of `text` in `page_text`, storing it the first time it is seen.
    pub fn page_text_id(&self, text: &str) -> Result<i64> {
        let hash = fingerprint(text);
        self.conn
            .prepare_cached("INSERT OR IGNORE INTO page_text (hash, text) VALUES (?1, ?2)")?
            .execute(params![hash, text])?;
        Ok(self
            .conn
            .prepare_cached("SELECT id FROM page_text WHERE hash = ?1")?
            .query_row([&hash], |row| row.get(0))?)
    }

    pub fn project_id(&self, root: &str, name: &str) -> Result<i64> {
        self.conn
            .prepare_cached("INSERT OR IGNORE INTO project (root, name) VALUES (?1, ?2)")?
            .execute(params![root, name])?;
        Ok(self
            .conn
            .prepare_cached("SELECT id FROM project WHERE root = ?1")?
            .query_row([root], |row| row.get(0))?)
    }

    pub fn session_project(&self, id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached("SELECT project_id FROM session WHERE id = ?1")?
            .query_row([id], |row| row.get::<_, Option<i64>>(0))
            .optional()?
            .flatten())
    }

    /// Records that session `id` was active at `ts`. A session keeps the
    /// first project it was seen with.
    pub fn note_session(&self, id: &str, project_id: Option<i64>, ts: i64) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO session (id, project_id, started_at, last_at) VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT (id) DO UPDATE SET
                     project_id = coalesce(project_id, excluded.project_id),
                     started_at = min(coalesce(started_at, excluded.started_at), excluded.started_at),
                     last_at = max(coalesce(last_at, excluded.last_at), excluded.last_at)",
            )?
            .execute(params![id, project_id, ts])?;
        Ok(())
    }

    pub fn set_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO session (id, title) VALUES (?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET title = excluded.title",
            )?
            .execute(params![id, title])?;
        Ok(())
    }

    /// Where reading `key` of `source` stopped, and the identity of what was
    /// read (for a file, its inode), if it was read before.
    pub fn cursor(&self, source: &str, key: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .prepare_cached("SELECT position, identity FROM cursor WHERE source = ?1 AND key = ?2")?
            .query_row(params![source, key], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?)
    }

    pub fn set_cursor(&self, source: &str, key: &str, position: i64, identity: &str) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO cursor (source, key, position, identity) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (source, key) DO UPDATE SET
                     position = excluded.position, identity = excluded.identity",
            )?
            .execute(params![source, key, position, identity])?;
        Ok(())
    }
}

/// FNV-1a, 64 bits: a fingerprint that stays the same across runs and Rust
/// versions, for deduplicating text.
pub fn fingerprint(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_each_activity_once_and_searches_it() {
        let store = Store::open_in_memory().unwrap();
        let activity = NewActivity {
            source: "test",
            kind: "prompt",
            ts: 1,
            detail: Some("Fix the webhook retries"),
            ext_id: "a",
            ..Default::default()
        };
        assert!(store.add_activity(&activity).unwrap());
        assert!(!store.add_activity(&activity).unwrap());

        let found: i64 = store
            .conn()
            .query_row(
                "SELECT count(*) FROM activity_fts WHERE activity_fts MATCH 'webhook'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
        // Porter stemming: "fixing" finds "Fix".
        let stemmed: i64 = store
            .conn()
            .query_row(
                "SELECT count(*) FROM activity_fts WHERE activity_fts MATCH 'fixing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stemmed, 1);
    }

    #[test]
    fn page_text_is_stored_once() {
        let store = Store::open_in_memory().unwrap();
        let a = store.page_text_id("same text").unwrap();
        let b = store.page_text_id("same text").unwrap();
        let c = store.page_text_id("other text").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_session_keeps_its_first_project_and_widest_span() {
        let store = Store::open_in_memory().unwrap();
        let first = store.project_id("/a", "a").unwrap();
        let second = store.project_id("/b", "b").unwrap();
        store.set_session_title("s", "Title").unwrap();
        store.note_session("s", Some(first), 20).unwrap();
        store.note_session("s", Some(second), 10).unwrap();
        store.note_session("s", Some(second), 30).unwrap();
        assert_eq!(store.session_project("s").unwrap(), Some(first));
        let span: (i64, i64, String) = store
            .conn()
            .query_row(
                "SELECT started_at, last_at, title FROM session WHERE id = 's'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(span, (10, 30, "Title".to_owned()));
    }

    #[test]
    fn reopening_does_not_rerun_migrations() {
        let path = std::env::temp_dir().join(format!("freebyrd-memory-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Store::open(&path).unwrap();
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn openers_at_the_same_moment_take_turns_upgrading() {
        let path =
            std::env::temp_dir().join(format!("freebyrd-memory-race-{}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let start = std::sync::Arc::new(std::sync::Barrier::new(6));
        let openers: Vec<_> = (0..6)
            .map(|_| {
                let path = path.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    Store::open(&path).map(|_| ())
                })
            })
            .collect();
        for opener in openers {
            opener.join().unwrap().unwrap();
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(""), "cbf29ce484222325");
        assert_eq!(fingerprint("a"), "af63dc4c8601ec8c");
    }
}
