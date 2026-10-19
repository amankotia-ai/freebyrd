//! What only the iPad shows, read from the memory by plain code: the cards
//! of the last few days, today's plan blocks, meetings coming and past, a
//! board of the day's Claude Code sessions, the week, and the reading shelf
//! (links shared from the iPad; X bookmarks were on it until the X
//! suggestions were removed, 2026-10-16). Nothing here asks Claude; asking
//! about a meeting sends it inside the memory, as data (`meeting_focus`).

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Duration, Local};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use memory::cards::{self, Card};
use memory::people::{Chip, Directory};
use memory::plans::{self, Focus};
use memory::{Store, recap, time};

use super::state::{self, Shared};
use crate::sessions;

const CARD_DAYS: i64 = 3;

/// Cards not put away: new ones, and those from the last few days. More
/// than the notch's four, since the iPad has room. A day plan's blocks show
/// with the plan instead (`blocks`).
pub fn cards(store: &Store, now_ms: i64, limit: usize) -> Result<Vec<Card>> {
    let ids: Vec<i64> = store
        .conn()
        .prepare(
            "SELECT id FROM card
             WHERE dismissed_at IS NULL AND kind != 'block' AND (seen_at IS NULL OR created_at > ?1)
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?
        .query_map(
            params![now_ms - CARD_DAYS * 86_400_000, i64::try_from(limit)?],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut found = Vec::new();
    for id in ids {
        if let Some(card) = cards::get(store, id)? {
            found.push(card);
        }
    }
    Ok(found)
}

/// Today's day plan as cards, in time order: the working day's date, and
/// the next date for blocks after midnight.
pub fn blocks(store: &Store, now: DateTime<Local>) -> Result<Vec<Card>> {
    let day = time::working_day(now);
    let ids: Vec<i64> = store
        .conn()
        .prepare(
            "SELECT id FROM card
             WHERE kind = 'block' AND dismissed_at IS NULL
               AND json_extract(data, '$.date') IN (?1, ?2)
             ORDER BY json_extract(data, '$.date'), json_extract(data, '$.time')",
        )?
        .query_map(
            params![day.to_string(), (day + Duration::days(1)).to_string()],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut found = Vec::new();
    for id in ids {
        if let Some(card) = cards::get(store, id)? {
            found.push(card);
        }
    }
    Ok(found)
}

// Meetings.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meetings {
    pub upcoming: Vec<Upcoming>,
    pub recent: Vec<Meeting>,
}

/// An event coming up, from the plans Freebyrd keeps.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upcoming {
    pub id: i64,
    pub title: String,
    pub when: String,
    pub start_at: i64,
    pub on_calendar: bool,
    pub source: String,
    /// The agenda Freebyrd wrote into the event's notes, if any.
    pub agenda: Option<String>,
    pub people: Vec<Chip>,
    pub focus: String,
    /// The button that fits the event (`plans::event_ask`): Prep me for a
    /// meeting, Trip details for travel, none for a birthday or a dinner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

#[derive(Serialize)]
pub struct Ask {
    pub label: &'static str,
    pub question: &'static str,
}

/// A meeting Fireflies recorded.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: i64,
    pub title: String,
    pub when: String,
    pub at: i64,
    pub summary: String,
    pub people: Vec<Chip>,
    /// Action items: the person's own, and everyone's.
    pub mine: usize,
    pub items: usize,
    pub focus: String,
}

pub fn meetings(store: &Store, now: DateTime<Local>) -> Result<Meetings> {
    let now_ms = now.timestamp_millis();
    let directory = Directory::load(store)?;
    let mut upcoming = Vec::new();
    {
        let mut statement = store.conn().prepare(
            "SELECT id, title, start_at, all_day, state, source, note, person, location, quote
             FROM plan
             WHERE kind = 'event' AND state IN ('added', 'suggested')
               AND start_at >= ?1 AND start_at < ?2
             ORDER BY start_at LIMIT 20",
        )?;
        let rows = statement.query_map(
            params![now_ms - 2 * 3_600_000, now_ms + 7 * 86_400_000],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )?;
        for row in rows {
            let (id, title, start_at, all_day, state, source, note, person, location, quote) = row?;
            let kind = plans::event_kind(
                &title,
                location.as_deref(),
                &quote,
                note.as_deref(),
                person.as_deref(),
            );
            let ask = plans::event_ask(kind).map(|(label, question)| Ask { label, question });
            let people = person
                .as_deref()
                .and_then(|written| directory.resolve(written))
                .and_then(|id| directory.chip(id))
                .into_iter()
                .collect();
            upcoming.push(Upcoming {
                id,
                title,
                when: plans::when_label(start_at, all_day),
                start_at,
                on_calendar: state == "added",
                source,
                agenda: note.filter(|note| !note.trim().is_empty()),
                people,
                focus: format!("plan:{id}"),
                ask,
            });
        }
    }

    let mut recent = Vec::new();
    {
        let mut statement = store.conn().prepare(
            "SELECT c.id, c.title, coalesce(m.sent_at, m.first_seen_at) AS at, m.text
             FROM conversation c JOIN message m ON m.conversation_id = c.id AND m.sender = 'summary'
             WHERE c.app = 'fireflies'
             ORDER BY at DESC LIMIT 20",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, title, at, summary) = row?;
            let (mine, items): (usize, usize) = store.conn().query_row(
                "SELECT coalesce(sum(sender = ?2), 0), count(*) FROM message
                 WHERE conversation_id = ?1 AND sender != 'summary'",
                params![id, memory::conversations::ME],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as usize,
                        row.get::<_, i64>(1)? as usize,
                    ))
                },
            )?;
            recent.push(Meeting {
                id,
                title,
                when: time::moment(at),
                at,
                summary: clip(&summary, 700),
                people: directory
                    .attendees(id)
                    .into_iter()
                    .filter_map(|person| directory.chip(person))
                    .collect(),
                mine,
                items,
                focus: format!("meeting:{id}"),
            });
        }
    }
    Ok(Meetings { upcoming, recent })
}

/// A meeting the person pressed, for the memory sent with their question.
pub fn meeting_focus(store: &Store, id: i64) -> Result<Focus> {
    let (title, at): (String, i64) = store.conn().query_row(
        "SELECT c.title, coalesce(max(m.sent_at), max(m.first_seen_at), c.last_seen_at)
         FROM conversation c LEFT JOIN message m ON m.conversation_id = c.id
         WHERE c.id = ?1 AND c.app = 'fireflies'",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut lines = vec![
        "## Meeting (the one the person pressed on their iPad)".to_owned(),
        format!("{title}, {}", time::moment_with_year(at)),
    ];
    let mut statement = store.conn().prepare(
        "SELECT sender, text FROM message WHERE conversation_id = ?1 ORDER BY sender != 'summary', id",
    )?;
    let rows = statement.query_map([id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (sender, text) = row?;
        if sender == "summary" {
            lines.push(format!("Summary:\n{}", clip(&text, 4000)));
        } else {
            let who = if sender == memory::conversations::ME {
                "you"
            } else {
                sender.as_str()
            };
            items.push(format!("- {who}: {}", clip(&text, 300)));
        }
    }
    if !items.is_empty() {
        lines.push("Action items:".to_owned());
        lines.extend(items.into_iter().take(40));
    }
    Ok(Focus {
        memory: lines.join("\n"),
        topic: title,
    })
}

// The day's Claude Code sessions.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub sessions: Vec<Session>,
    /// Files more than one session changed in the last two hours.
    pub overlaps: Vec<Overlap>,
    pub since: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub project: String,
    pub title: String,
    pub first_ask: Option<String>,
    pub last_ask: Option<String>,
    pub started_at: Option<i64>,
    pub last_at: i64,
    /// `running`, `waiting` (Claude finished and the person has not looked
    /// yet), `blocked` (Claude asked them something, or is stuck on an
    /// error) or `done`; the notch's Sessions tab reads the same
    /// (`crate::sessions`).
    pub state: String,
    /// The state in words: "Done 4 min ago", "Asked you a question".
    pub label: String,
    pub asks: i64,
    pub files: Vec<String>,
    pub resume: Option<String>,
}

#[derive(Serialize)]
pub struct Overlap {
    pub file: String,
    pub sessions: Vec<String>,
}

pub fn work(store: &Store, now: DateTime<Local>) -> Result<Board> {
    work_with(store, now, sessions::today(now))
}

/// The board, given the desktop app's sessions of today (`sessions::today`):
/// where each stands, as the notch reads it, their titles, and the ones the
/// memory has not read yet.
fn work_with(store: &Store, now: DateTime<Local>, today: Vec<sessions::Entry>) -> Result<Board> {
    let now_ms = now.timestamp_millis();
    let span = time::day_span(time::working_day(now));
    let desktop: HashMap<String, sessions::Entry> = today
        .into_iter()
        .map(|entry| (entry.desktop.cli.clone(), entry))
        .collect();
    let mut statement = store.conn().prepare(
        "SELECT s.id, s.title, s.started_at, s.last_at, p.name, p.root
         FROM session s LEFT JOIN project p ON p.id = s.project_id
         WHERE s.last_at >= ?1 ORDER BY s.last_at DESC LIMIT 40",
    )?;
    let rows = statement
        .query_map([span.start_ms], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut sessions = Vec::new();
    let known: HashSet<String> = rows.iter().map(|row| row.0.clone()).collect();
    for (id, title, started_at, last_at, project, root) in rows {
        let ask = |order: &str| -> Result<Option<String>> {
            Ok(store
                .conn()
                .query_row(
                    &format!(
                        "SELECT detail FROM activity WHERE session_id = ?1 AND kind = 'prompt' AND ts >= ?2
                         ORDER BY ts {order} LIMIT 1"
                    ),
                    params![id, span.start_ms],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten()
                .map(|text| clip(&text, 140)))
        };
        let asks: i64 = store.conn().query_row(
            "SELECT count(*) FROM activity WHERE session_id = ?1 AND kind = 'prompt' AND ts >= ?2",
            params![id, span.start_ms],
            |row| row.get(0),
        )?;
        let files: Vec<String> = store
            .conn()
            .prepare(
                "SELECT detail FROM activity WHERE session_id = ?1 AND kind = 'edit' AND ts >= ?2
                 GROUP BY detail ORDER BY max(ts) DESC LIMIT 8",
            )?
            .query_map(params![id, span.start_ms], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .map(|path| short_path(&path))
            .collect();
        let entry = desktop.get(&id);
        let status = entry
            .map(|entry| entry.status.clone())
            .or_else(|| sessions::status_of_transcript(&id, now));
        let last_at = status.as_ref().map_or(last_at, |s| s.last_ms.max(last_at));
        let (state, label) = status.map_or_else(
            || ("done".to_owned(), String::new()),
            |s| (s.state.to_owned(), s.label),
        );
        let title = entry
            .map(|entry| entry.desktop.title.clone())
            .filter(|title| !title.is_empty())
            .or(title);
        sessions.push(Session {
            resume: root
                .as_ref()
                .map(|root| format!("cd {} && claude --resume {id}", shell_quote(root))),
            project: project.unwrap_or_else(|| "No project".to_owned()),
            title: title.map(|t| clip(&t, 120)).unwrap_or_default(),
            first_ask: ask("ASC")?,
            last_ask: ask("DESC")?,
            started_at,
            last_at,
            state,
            label,
            asks,
            files,
            id,
        });
    }
    // Sessions of today's the memory has not read yet (it is a few minutes
    // behind), from the desktop app.
    for entry in desktop.into_values() {
        if known.contains(&entry.desktop.cli) {
            continue;
        }
        let cwd = &entry.desktop.cwd;
        sessions.push(Session {
            resume: Some(format!(
                "cd {} && claude --resume {}",
                shell_quote(cwd),
                entry.desktop.cli
            )),
            project: sessions::folder_name(cwd),
            title: clip(&entry.desktop.title, 120),
            first_ask: None,
            last_ask: None,
            started_at: None,
            last_at: entry.status.last_ms.max(entry.desktop.last_activity_ms),
            state: entry.status.state.to_owned(),
            label: entry.status.label,
            asks: 0,
            files: Vec::new(),
            id: entry.desktop.cli,
        });
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_at));
    Ok(Board {
        overlaps: overlaps(store, now_ms - 2 * 3_600_000)?,
        sessions,
        since: time::clock(span.start_ms),
    })
}

fn overlaps(store: &Store, since_ms: i64) -> Result<Vec<Overlap>> {
    let mut statement = store.conn().prepare(
        "SELECT a.detail, group_concat(DISTINCT coalesce(s.title, a.session_id))
         FROM activity a LEFT JOIN session s ON s.id = a.session_id
         WHERE a.kind = 'edit' AND a.ts >= ?1 AND a.session_id IS NOT NULL
         GROUP BY a.detail HAVING count(DISTINCT a.session_id) > 1
         ORDER BY max(a.ts) DESC LIMIT 10",
    )?;
    let rows = statement.query_map([since_ms], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut found = Vec::new();
    for row in rows {
        let (file, sessions) = row?;
        found.push(Overlap {
            file: short_path(&file),
            sessions: sessions.split(',').map(|s| clip(s, 80)).collect(),
        });
    }
    Ok(found)
}

// The week.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Week {
    pub label: String,
    pub days: Vec<DayRow>,
    pub projects: Vec<ProjectRow>,
    pub promises: Promises,
    pub hours: f64,
    pub sessions: i64,
    pub asks: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayRow {
    pub label: String,
    pub short: String,
    /// Hours with activity: ten-minute stretches with something done.
    pub hours: f64,
    pub headline: Option<String>,
    pub today: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    pub name: String,
    pub sessions: i64,
    pub asks: i64,
    pub edits: i64,
    pub done: Vec<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Promises {
    pub kept: i64,
    pub slipped: Vec<String>,
    pub open: i64,
}

pub fn week(store: &Store, now: DateTime<Local>) -> Result<Week> {
    let today = time::working_day(now);
    let first = today - Duration::days(6);
    let start = time::day_span(first).start_ms;
    let end = time::day_span(today).end_ms;
    let mut days = Vec::new();
    let mut total = 0.0;
    let mut done_by_project: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for offset in 0..7 {
        let day = first + Duration::days(offset);
        let span = time::day_span(day);
        let stretches: i64 = store.conn().query_row(
            "SELECT count(DISTINCT ts / 600000) FROM activity
             WHERE ts >= ?1 AND ts < ?2 AND kind IN ('prompt', 'reply', 'edit', 'visit', 'page', 'window')",
            params![span.start_ms, span.end_ms],
            |row| row.get(0),
        )?;
        let hours = (stretches as f64 * 10.0 / 60.0 * 10.0).round() / 10.0;
        total += hours;
        let saved = recap::saved(store, day)?;
        if let Some(recap) = &saved {
            for project in &recap.projects {
                done_by_project
                    .entry(project.name.clone())
                    .or_default()
                    .extend(project.done.iter().cloned());
            }
        }
        days.push(DayRow {
            label: time::day_label(day),
            short: day.format("%a").to_string(),
            hours,
            headline: saved.map(|recap| recap.headline),
            today: day == today,
        });
    }
    let mut statement = store.conn().prepare(
        "SELECT coalesce(p.name, 'No project'), count(DISTINCT a.session_id),
                coalesce(sum(a.kind = 'prompt'), 0), coalesce(sum(a.kind = 'edit'), 0)
         FROM activity a LEFT JOIN project p ON p.id = a.project_id
         WHERE a.source = 'claude' AND a.ts >= ?1 AND a.ts < ?2
         GROUP BY 1 ORDER BY 3 DESC LIMIT 8",
    )?;
    let projects = statement
        .query_map(params![start, end], |row| {
            Ok(ProjectRow {
                name: row.get(0)?,
                sessions: row.get(1)?,
                asks: row.get(2)?,
                edits: row.get(3)?,
                done: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|mut row| {
            row.done = done_by_project
                .get(&row.name)
                .map(|done| done.iter().take(4).cloned().collect())
                .unwrap_or_default();
            row
        })
        .collect();
    let (sessions, asks): (i64, i64) = store.conn().query_row(
        "SELECT count(DISTINCT session_id), coalesce(sum(kind = 'prompt'), 0) FROM activity
         WHERE source = 'claude' AND ts >= ?1 AND ts < ?2",
        params![start, end],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let now_ms = now.timestamp_millis();
    let mut promises = Promises::default();
    let mut statement = store.conn().prepare(
        "SELECT title, start_at, done_at FROM plan
         WHERE kind IN ('promise', 'task', 'noted') AND state NOT IN ('undone', 'replaced')
           AND start_at >= ?1 AND start_at < ?2",
    )?;
    let rows = statement.query_map(params![start, end], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for row in rows {
        let (title, due, done) = row?;
        match done {
            Some(_) => promises.kept += 1,
            None if due < now_ms - 86_400_000 => promises.slipped.push(clip(&title, 100)),
            None => promises.open += 1,
        }
    }
    Ok(Week {
        label: format!(
            "{} to {}",
            time::short_day_label(first),
            time::short_day_label(today)
        ),
        days,
        projects,
        promises,
        hours: (total * 10.0).round() / 10.0,
        sessions,
        asks,
    })
}

// The reading shelf: links shared from the iPad.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shelf {
    pub items: Vec<Item>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    /// `shared` (sent from the iPad).
    pub kind: &'static str,
    pub url: Option<String>,
    pub title: String,
    pub text: Option<String>,
    pub at: i64,
    pub read: bool,
}

pub fn reading(read: &[String]) -> Shelf {
    let mut items: Vec<Item> = state::shared()
        .into_iter()
        .rev()
        .map(|shared| {
            let Shared {
                id,
                at,
                url,
                title,
                text,
            } = shared;
            Item {
                read: read.contains(&id),
                title: title
                    .or_else(|| text.as_deref().map(|t| first_line(t, 90)))
                    .or_else(|| url.clone())
                    .unwrap_or_default(),
                id,
                kind: "shared",
                url,
                text: text.map(|t| clip(&t, 400)),
                at,
            }
        })
        .collect();
    items.sort_by_key(|item| -item.at);
    Shelf { items }
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

fn first_line(text: &str, max: usize) -> String {
    clip(
        text.lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(text),
        max,
    )
}

/// `.../crates/memory/src/day.rs` as `memory/src/day.rs`.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    parts[parts.len().saturating_sub(3)..].join("/")
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_text_are_shortened() {
        assert_eq!(
            short_path("/Users/a/repo/crates/memory/src/day.rs"),
            "memory/src/day.rs"
        );
        assert_eq!(short_path("a.rs"), "a.rs");
        assert_eq!(clip("abcdef", 3), "abc\u{2026}");
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }

    #[test]
    fn the_week_and_the_shelf_read_an_empty_memory() {
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let week = week(&store, now).unwrap();
        assert_eq!(week.days.len(), 7);
        assert!(week.days.last().unwrap().today);
        assert!(reading(&[]).items.iter().all(|item| item.kind == "shared"));
        assert!(
            work_with(&store, now, Vec::new())
                .unwrap()
                .sessions
                .is_empty()
        );
        let meetings = meetings(&store, now).unwrap();
        assert!(meetings.upcoming.is_empty() && meetings.recent.is_empty());
        assert!(
            cards(&store, now.timestamp_millis(), 10)
                .unwrap()
                .is_empty()
        );
        assert!(blocks(&store, now).unwrap().is_empty());
    }
}
