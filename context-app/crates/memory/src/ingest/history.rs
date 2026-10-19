//! Browser history, read from Chromium's own `History` database in each Arc
//! and Chrome profile. The browser keeps the file locked while it runs, so
//! it is opened read-only as immutable, which makes SQLite read without
//! taking locks. A read can then catch a write in progress and fail; the
//! next run tries again from the same cursor.
//!
//! History has every page, so the extension's exclusions are applied here
//! too. The first read goes back `BACKFILL_DAYS`.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, params};

use super::SourceReport;
use crate::redact::redact;
use crate::store::{NewActivity, Store};
use crate::{exclusions, home, time};

const SOURCE: &str = "history";
const BACKFILL_DAYS: i64 = 90;
const DAY_MS: i64 = 86_400_000;
const BATCH: i64 = 5_000;
/// Chromium counts microseconds from 1601-01-01 UTC.
const CHROMIUM_EPOCH_OFFSET_MS: i64 = 11_644_473_600_000;
/// A visit lasts until its tab navigates or closes, which for a tab left
/// open can be days.
const MAX_VISIT_MS: i64 = DAY_MS;
/// Transition types for pages loaded inside frames, not visited.
const AUTO_SUBFRAME: i64 = 3;
const MANUAL_SUBFRAME: i64 = 4;

#[derive(Debug)]
pub struct HistoryDb {
    pub browser: &'static str,
    pub profile: String,
    pub path: PathBuf,
}

/// The `History` files of every Arc and Chrome profile on this Mac.
pub fn discover() -> Vec<HistoryDb> {
    let Some(home) = home() else {
        return Vec::new();
    };
    let roots = [
        (
            "Arc",
            home.join("Library/Application Support/Arc/User Data"),
        ),
        (
            "Chrome",
            home.join("Library/Application Support/Google/Chrome"),
        ),
    ];
    let mut found = Vec::new();
    for (browser, root) in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let profile = entry.file_name().to_string_lossy().into_owned();
            if profile != "Default" && !profile.starts_with("Profile ") {
                continue;
            }
            let path = entry.path().join("History");
            if path.is_file() {
                found.push(HistoryDb {
                    browser,
                    profile,
                    path,
                });
            }
        }
    }
    found
}

pub fn ingest(store: &Store) -> SourceReport {
    let mut report = SourceReport::new("Browser history");
    for db in discover() {
        report.files += 1;
        match ingest_db(store, &db, time::now_ms()) {
            Ok(added) => report.added += added,
            Err(e) => report
                .errors
                .push(format!("{} {}: {e:#}", db.browser, db.profile)),
        }
    }
    report
}

struct Visit {
    id: i64,
    time: i64,
    duration: i64,
    transition: i64,
    url: String,
    title: Option<String>,
}

fn ingest_db(store: &Store, db: &HistoryDb, now_ms: i64) -> Result<usize> {
    let history = open_immutable(&db.path)?;
    let key = db.path.to_string_lossy();
    let max_id: i64 = history.query_row("SELECT ifnull(max(id), 0) FROM visits", [], |row| {
        row.get(0)
    })?;
    let mut last = match store.cursor(SOURCE, &key)? {
        // A cursor past the end means the database was replaced.
        Some((position, _)) if position <= max_id => position,
        _ => history.query_row(
            "SELECT ifnull(min(id), ?2 + 1) - 1 FROM visits WHERE visit_time >= ?1",
            params![to_chromium(now_ms - BACKFILL_DAYS * DAY_MS), max_id],
            |row| row.get(0),
        )?,
    };

    let tx = store.transaction()?;
    let mut statement = history.prepare(
        "SELECT v.id, v.visit_time, v.visit_duration, v.transition, u.url, u.title
         FROM visits v JOIN urls u ON u.id = v.url
         WHERE v.id > ?1 ORDER BY v.id LIMIT ?2",
    )?;
    let mut added = 0;
    loop {
        let visits = statement
            .query_map(params![last, BATCH], |row| {
                Ok(Visit {
                    id: row.get(0)?,
                    time: row.get(1)?,
                    duration: row.get(2)?,
                    transition: row.get(3)?,
                    url: row.get(4)?,
                    title: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(end) = visits.last().map(|visit| visit.id) else {
            break;
        };
        for visit in &visits {
            if record(store, db, visit)? {
                added += 1;
            }
        }
        last = end;
    }
    store.set_cursor(SOURCE, &key, last, "")?;
    tx.commit()?;
    Ok(added)
}

fn record(store: &Store, db: &HistoryDb, visit: &Visit) -> Result<bool> {
    let core = visit.transition & 0xFF;
    if core == AUTO_SUBFRAME || core == MANUAL_SUBFRAME || exclusions::reason(&visit.url).is_some()
    {
        return Ok(false);
    }
    let url = redact(&visit.url);
    let title = visit
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(redact);
    let ext_id = format!("{}/{}/{}", db.browser, db.profile, visit.id);
    store.add_activity(&NewActivity {
        source: SOURCE,
        kind: "visit",
        ts: from_chromium(visit.time),
        duration_ms: Some((visit.duration / 1_000).clamp(0, MAX_VISIT_MS)),
        app: Some(db.browser),
        title: title.as_deref(),
        url: Some(&url),
        ext_id: &ext_id,
        ..Default::default()
    })
}

fn open_immutable(path: &Path) -> Result<Connection> {
    let uri = format!("file:{}?immutable=1", uri_path(path));
    Ok(Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

/// A path for a `file:` URI: everything but unreserved characters and `/`
/// percent-encoded ("Application Support" has a space).
fn uri_path(path: &Path) -> String {
    path.as_os_str()
        .as_bytes()
        .iter()
        .map(|&byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn from_chromium(microseconds: i64) -> i64 {
    microseconds / 1_000 - CHROMIUM_EPOCH_OFFSET_MS
}

fn to_chromium(ms: i64) -> i64 {
    (ms + CHROMIUM_EPOCH_OFFSET_MS) * 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_chromium_time() {
        // 2026-10-12T00:00:00Z.
        let unix_ms = 1_791_763_200_000;
        assert_eq!(from_chromium(to_chromium(unix_ms)), unix_ms);
        assert_eq!(from_chromium(13_436_236_800_000_000), unix_ms);
    }

    #[test]
    fn encodes_paths_for_uris() {
        assert_eq!(
            uri_path(Path::new(
                "/Users/a/Library/Application Support/Arc/History"
            )),
            "/Users/a/Library/Application%20Support/Arc/History"
        );
        assert_eq!(uri_path(Path::new("/a?b#c%")), "/a%3Fb%23c%25");
    }

    #[test]
    fn reads_a_chromium_history_file() {
        let dir = std::env::temp_dir().join(format!("freebyrd history {}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("History");
        let history = Connection::open(&path).unwrap();
        let now = 1_791_763_200_000;
        let recent = to_chromium(now - DAY_MS);
        let old = to_chromium(now - 200 * DAY_MS);
        history
            .execute_batch(&format!(
                "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT);
                 CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER,
                                      transition INTEGER, visit_duration INTEGER);
                 INSERT INTO urls VALUES (1, 'https://example.com/a', 'Example'),
                                         (2, 'https://mybank.com/', 'Bank'),
                                         (3, 'https://ads.example.net/frame', '');
                 INSERT INTO visits VALUES (1, 1, {old}, 1, 0),
                                           (2, 1, {recent}, 805306368, 5000000),
                                           (3, 2, {recent}, 1, 0),
                                           (4, 3, {recent}, 3, 0);"
            ))
            .unwrap();
        drop(history);

        let store = Store::open_in_memory().unwrap();
        let db = HistoryDb {
            browser: "Arc",
            profile: "Default".into(),
            path: path.clone(),
        };
        // Only the recent visit: the old one is past the backfill, the bank
        // is excluded, and the frame was never visited.
        assert_eq!(ingest_db(&store, &db, now).unwrap(), 1);
        assert_eq!(ingest_db(&store, &db, now).unwrap(), 0);
        let (title, duration): (String, i64) = store
            .conn()
            .query_row("SELECT title, duration_ms FROM activity", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((title.as_str(), duration), ("Example", 5_000));

        fs::remove_dir_all(&dir).unwrap();
    }
}
