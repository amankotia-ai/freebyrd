//! Which project each thing in the memory belongs to: the backbone of the
//! context graph. Claude Code work already carries its project (its
//! folder); pages the person visits, meetings, chats and plans do not. Jev
//! tags each new one with a project, or none, from its title and where it
//! was (a site, an app), against the projects described by their folders and
//! recent session titles. Tags go to `jev/projects.jsonl` for now and are
//! used by nothing; `report` shows them by project for the person to check,
//! and `backtest` measures them on Claude Code sessions, whose project is
//! known. Live, they would live in the memory and steer recall, recaps and
//! matching.

use std::collections::HashSet;
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Store;
use crate::redact::redact;

const LOG: &str = "projects";
const CURSOR: &str = "projects-cursor.json";
const GROUPS: &str = "project-groups.json";
/// At most this often, and this many new things a run.
const EVERY_MS: i64 = 10 * 60_000;
const MAX_ITEMS: usize = 150;
/// The first run looks back this far for pages. A week (3,600 page titles,
/// asked about at 150 a run) took about 5.1 million tokens on 2026-10-13,
/// half the day's budget: each thing is one question per project, and each
/// question carries that project's description.
const FIRST_DAYS: i64 = 1;
/// Session titles that describe each project.
const TITLES: usize = 12;
const NONE: &str = "none";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tagged {
    at: i64,
    kind: String,
    id: i64,
    title: String,
    project: String,
    p: f64,
}

/// Where the last run stopped, per source.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cursor {
    ran_at: i64,
    activity: i64,
    conversation: i64,
    plan: i64,
}

/// Something to tag.
#[derive(Debug, Clone)]
struct Item {
    kind: &'static str,
    id: i64,
    title: String,
    /// The site or app it was on.
    place: String,
}

/// A project as Jev sees it.
#[derive(Debug, Clone)]
struct Described {
    name: String,
    /// What it is about, when the person said.
    about: Option<String>,
    folder: String,
    recent_work: Vec<String>,
}

/// Tags what is new since the last run, when one is due. Lines for the
/// log; none without a key.
pub fn run_due(store: &Store, now: DateTime<Local>) -> Vec<String> {
    let result = run(store, now);
    super::line("projects", result).into_iter().collect()
}

fn run(store: &Store, now: DateTime<Local>) -> Result<Option<String>> {
    let now_ms = now.timestamp_millis();
    let mut cursor = read_cursor().unwrap_or_else(|| first_cursor(store, now_ms));
    if now_ms - cursor.ran_at < EVERY_MS || super::key().is_none() {
        return Ok(None);
    }
    // A run that fails waits like one that worked, and its things are asked
    // about at the next run: never again at every pass of the thread.
    save_cursor(&Cursor {
        ran_at: now_ms,
        ..cursor
    })?;
    let items = new_items(store, &mut cursor)?;
    cursor.ran_at = now_ms;
    if items.is_empty() {
        save_cursor(&cursor)?;
        return Ok(None);
    }
    let projects = projects(store, &HashSet::new(), &Groups::load())?;
    let Some((tags, ms, tokens)) = tag(LOG, &projects, &items)? else {
        return Ok(None);
    };
    let mut tagged = 0;
    for (item, (project, p)) in items.iter().zip(tags) {
        tagged += usize::from(project != NONE);
        super::append(
            LOG,
            &Tagged {
                at: now_ms,
                kind: item.kind.to_owned(),
                id: item.id,
                title: item.title.clone(),
                project,
                p,
            },
        )?;
    }
    save_cursor(&cursor)?;
    Ok(Some(format!(
        "jev: tagged {} new things in {:.1} s ({}k tokens), {tagged} to a project",
        items.len(),
        ms as f64 / 1000.0,
        tokens / 1000
    )))
}

fn read_cursor() -> Option<Cursor> {
    let text = fs::read_to_string(super::dir()?.join(CURSOR)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cursor(cursor: &Cursor) -> Result<()> {
    let dir = super::dir().context("no home folder")?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(CURSOR), serde_json::to_string(cursor)?)?;
    Ok(())
}

/// The first run starts a week back for pages, and at the start for the
/// few meetings, chats and plans.
fn first_cursor(store: &Store, now_ms: i64) -> Cursor {
    let activity = store
        .conn()
        .query_row(
            "SELECT coalesce(min(id), 1) - 1 FROM activity WHERE ts > ?1",
            [now_ms - FIRST_DAYS * 86_400_000],
            |row| row.get(0),
        )
        .unwrap_or_default();
    Cursor {
        activity,
        ..Cursor::default()
    }
}

/// New pages, meetings, chats and plans, up to `MAX_ITEMS`, moving `cursor`
/// past what is returned. Each is asked about once by its kind and title:
/// most visits are to pages seen before (the inbox, a dashboard), and a
/// weekly meeting keeps its title (70 repeats on 2026-10-13).
fn new_items(store: &Store, cursor: &mut Cursor) -> Result<Vec<Item>> {
    let mut seen: HashSet<(String, String)> = super::entries::<Tagged>(LOG)
        .into_iter()
        .map(|tag| (tag.kind, tag.title.to_lowercase()))
        .collect();
    let mut items = Vec::new();
    let mut statement = store
        .conn()
        .prepare("SELECT id, app, title FROM conversation WHERE id > ?1 ORDER BY id LIMIT ?2")?;
    let rows = statement.query_map(params![cursor.conversation, MAX_ITEMS as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, app, title) = row?;
        cursor.conversation = id;
        let (kind, place) = match app.as_str() {
            "fireflies" => ("meeting", "Fireflies"),
            "whatsapp" => ("chat", "WhatsApp"),
            "gmail" => ("chat", "Gmail"),
            _ => ("chat", "a chat app"),
        };
        if !seen.insert((kind.to_owned(), title.to_lowercase())) {
            continue;
        }
        items.push(Item {
            kind,
            id,
            title,
            place: place.to_owned(),
        });
    }
    let mut statement = store.conn().prepare(
        "SELECT id, title, coalesce(person, '') FROM plan
         WHERE id > ?1 AND kind NOT LIKE 'block:%' ORDER BY id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![cursor.plan, MAX_ITEMS as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, title, person) = row?;
        cursor.plan = id;
        let title = if person.is_empty() {
            title
        } else {
            format!("{title} (with {person})")
        };
        if !seen.insert(("plan".to_owned(), title.to_lowercase())) {
            continue;
        }
        items.push(Item {
            kind: "plan",
            id,
            title,
            place: "the person's calendar".to_owned(),
        });
    }
    let mut statement = store.conn().prepare(
        "SELECT id, coalesce(title, ''), coalesce(url, '') FROM activity
         WHERE id > ?1 AND source IN ('history', 'page') ORDER BY id LIMIT 2000",
    )?;
    let rows = statement.query_map([cursor.activity], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        if items.len() >= MAX_ITEMS {
            break;
        }
        let (id, title, url) = row?;
        cursor.activity = id;
        let title = title.trim().to_owned();
        if title.is_empty()
            || crate::exclusions::reason(&url).is_some()
            || !seen.insert(("page".to_owned(), title.to_lowercase()))
        {
            continue;
        }
        items.push(Item {
            kind: "page",
            id,
            title,
            place: crate::exclusions::site(&url).unwrap_or_default(),
        });
    }
    Ok(items)
}

/// The projects, each with its folder and latest session titles, leaving
/// out titles in `hold_out` (the backtest's questions).
/// Which folders make one project, and which folders are not projects,
/// as the person said (`jev/project-groups.json`). Any other folder is a
/// project of its own.
#[derive(Debug, Default, Deserialize)]
struct Groups {
    #[serde(default)]
    groups: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    not_projects: Vec<String>,
    /// What a project is about, in the person's words: session titles
    /// alone tagged website meetings to Freebyrd, whose work mentions
    /// meetings and websites as things it reads.
    #[serde(default)]
    about: std::collections::BTreeMap<String, String>,
}

impl Groups {
    fn load() -> Self {
        super::dir()
            .and_then(|dir| fs::read_to_string(dir.join(GROUPS)).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The project a folder (by name) counts toward; `None` when the person
    /// said it is not a project.
    fn project_of(&self, folder: &str) -> Option<String> {
        if self.not_projects.iter().any(|name| name == folder) {
            return None;
        }
        Some(
            self.groups
                .iter()
                .find(|(_, members)| members.iter().any(|name| name == folder))
                .map_or_else(|| folder.to_owned(), |(group, _)| group.clone()),
        )
    }
}

/// The projects, each with its folders and session titles spread over its
/// latest work, leaving out titles in `hold_out` (the backtest's questions).
fn projects(store: &Store, hold_out: &HashSet<String>, groups: &Groups) -> Result<Vec<Described>> {
    let home = crate::home()
        .map(|home| home.display().to_string())
        .unwrap_or_default();
    let mut statement = store.conn().prepare(
        "SELECT p.id, p.name, p.root FROM project p
         WHERE EXISTS (SELECT 1 FROM session s WHERE s.project_id = p.id)
         ORDER BY p.name",
    )?;
    let rows: Vec<(i64, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut titles = store.conn().prepare(
        "SELECT last_at, title FROM session WHERE project_id = ?1 AND coalesce(title, '') <> ''
         ORDER BY last_at DESC LIMIT 150",
    )?;
    // Folders by the project they count toward, in name order.
    let mut by_project: std::collections::BTreeMap<String, Vec<(i64, String)>> = Default::default();
    for (id, name, root) in rows {
        if root == home {
            continue;
        }
        if let Some(project) = groups.project_of(&name) {
            by_project.entry(project).or_default().push((id, root));
        }
    }
    let mut projects = Vec::new();
    for (name, folders) in by_project {
        let mut all: Vec<(i64, String)> = Vec::new();
        for (id, _) in &folders {
            all.extend(
                titles
                    .query_map([id], |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?.unwrap_or_default(),
                            row.get(1)?,
                        ))
                    })?
                    .filter_map(Result::ok)
                    .filter(|(_, title): &(i64, String)| !hold_out.contains(title)),
            );
        }
        all.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        // Spread over the project's last sessions, not only the newest few,
        // so one week's topic does not stand for the whole project.
        let step = all.len().div_ceil(TITLES).max(1);
        let recent_work: Vec<String> = all
            .iter()
            .step_by(step)
            .take(TITLES)
            .map(|(_, title)| redact(&crate::one_line(title, 120)))
            .collect();
        let folder = folders
            .iter()
            .map(|(_, root)| redact(&root.replacen(&home, "~", 1)))
            .collect::<Vec<_>>()
            .join(", ");
        projects.push(Described {
            about: groups.about.get(&name).map(|about| redact(about)),
            name,
            folder,
            recent_work,
        });
    }
    Ok(projects)
}

/// Each item's project (or "none") with Jev's probability for it.
type Tags = Vec<(String, f64)>;

/// Below this for every project, an item is tagged none.
const PART_OF_AT: f64 = 0.5;

/// One yes/no per item and project: is it part of that project? The item
/// goes to the project it is likeliest part of. A single pick from all the
/// projects did badly in the backtest (12, then 17 of 107): Jev favoured
/// the catch-all project, listed second, and a pick has to weigh every
/// description at once. With the tags: how long it took (ms) and the input
/// tokens, for the use `name`.
fn tag(name: &str, projects: &[Described], items: &[Item]) -> Result<Option<(Tags, u64, u64)>> {
    let state = json!({
        "about": "Things from the person's browser, meetings, chats and calendar, and the \
                  projects they work on, each with its folder and work done in it.",
    });
    let mut questions = Map::new();
    for (i, item) in items.iter().enumerate() {
        for (j, project) in projects.iter().enumerate() {
            questions.insert(format!("t{i}p{j}"), question(item, project));
        }
    }
    let started = Instant::now();
    let Some(response) = super::ask(name, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    let tags = (0..items.len())
        .map(|i| {
            let best = projects
                .iter()
                .enumerate()
                .filter_map(|(j, project)| {
                    let noul = response.answers.get(&format!("t{i}p{j}"))?.noul?;
                    Some((project.name.clone(), noul))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            match best {
                Some((name, p)) if p >= PART_OF_AT => (name, p),
                Some((_, p)) => (NONE.to_owned(), 1.0 - p),
                None => (NONE.to_owned(), 0.0),
            }
        })
        .collect();
    Ok(Some((tags, ms, response.usage.input_tokens)))
}

fn question(item: &Item, project: &Described) -> Value {
    let kind = match item.kind {
        "page" => "web page the person visited",
        "meeting" => "meeting",
        "chat" => "chat or email thread",
        "session" => "Claude Code session",
        _ => "plan on the person's calendar",
    };
    super::noul(
        json!({
            "item": {
                "kind": kind,
                "title": redact(&crate::one_line(&item.title, 200)),
                "where": item.place,
            },
            "project": {
                "name": project.name,
                "about": project.about,
                "folder": project.folder,
                "work_in_it": project.recent_work,
            },
            "question": "Is `item` part of the work in `project`?",
        }),
        "The item is about this project's work: the same product, client, codebase, \
         website or deliverable as its work.",
        "The item is about something else, or only shares a broad topic with it.",
    )
}

/// The tags so far by project, with a few titles each for the person to
/// check, and how sure Jev was.
pub fn report() -> Result<Vec<String>> {
    let tagged: Vec<Tagged> = super::entries(LOG);
    let mut by_project: std::collections::BTreeMap<&str, Vec<&Tagged>> = Default::default();
    for tag in &tagged {
        by_project
            .entry(tag.project.as_str())
            .or_default()
            .push(tag);
    }
    let mut lines = Vec::new();
    for (project, tags) in &by_project {
        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
        for tag in tags {
            *kinds.entry(tag.kind.as_str()).or_default() += 1;
        }
        let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}s")).collect();
        let unsure = tags.iter().filter(|tag| tag.p < 0.6).count();
        lines.push(format!("{project}: {} ({unsure} unsure)", kinds.join(", ")));
        for tag in tags.iter().rev().take(5) {
            lines.push(format!("    {} {:.2}: {}", tag.kind, tag.p, tag.title));
        }
    }
    lines.push(format!("{} things tagged", tagged.len()));
    Ok(lines)
}

/// Tags titles of older Claude Code sessions, whose project is known, with
/// projects described by newer ones, and counts how often Jev is right.
/// Sends those titles and the projects' descriptions to TypeSafe.
pub fn backtest(store: &Store) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT s.title, p.name FROM session s JOIN project p ON p.id = s.project_id
         WHERE coalesce(s.title, '') <> '' ORDER BY s.last_at DESC LIMIT 600",
    )?;
    let groups = Groups::load();
    // Each session's project as the person groups folders; sessions in
    // folders that are not projects are left out.
    let sessions: Vec<(String, String)> = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(Result::ok)
        .filter_map(|(title, folder)| Some((title, groups.project_of(&folder)?)))
        .collect();
    // Every third session after the newest few is a question; the others
    // can describe their project.
    let questions: Vec<(String, String)> = sessions
        .iter()
        .skip(20)
        .step_by(3)
        .take(MAX_ITEMS)
        .cloned()
        .collect();
    let hold_out: HashSet<String> = questions.iter().map(|(title, _)| title.clone()).collect();
    let projects = projects(store, &hold_out, &groups)?;
    let items: Vec<Item> = questions
        .iter()
        .enumerate()
        .map(|(id, (title, _))| Item {
            kind: "session",
            id: id as i64,
            title: title.clone(),
            place: "Claude Code".to_owned(),
        })
        .collect();
    let (tags, ms, _) =
        tag("backtest", &projects, &items)?.context("no TypeSafe key in the Keychain")?;
    let mut right = 0;
    let mut mixed: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut examples: Vec<String> = Vec::new();
    for ((title, truth), (project, p)) in questions.iter().zip(&tags) {
        if project == truth {
            right += 1;
            continue;
        }
        *mixed.entry((project.clone(), truth.clone())).or_default() += 1;
        if examples.len() < 10 {
            examples.push(format!("    {title}: Jev {project} {p:.2}, was {truth}"));
        }
    }
    let mut lines = vec![format!(
        "{right} of {} session titles tagged with their own project ({} projects, {ms} ms)",
        questions.len(),
        projects.len()
    )];
    let mut mixed: Vec<((String, String), usize)> = mixed.into_iter().collect();
    mixed.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    lines.push("Mixed up most (Jev's tag, the session's project):".to_owned());
    for ((jev, truth), count) in mixed.iter().take(8) {
        lines.push(format!("    {count}: {jev} for {truth}"));
    }
    lines.push("For example:".to_owned());
    lines.extend(examples);
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO project (id, name, root) VALUES (1, 'Website', '/w'), (2, 'App', '/a');
                 INSERT INTO session (id, project_id, title, started_at, last_at) VALUES
                   ('s1', 1, 'Pricing page copy', 1, 10), ('s2', 1, 'Hero section', 1, 20),
                   ('s3', 2, 'Notch glow', 1, 30);
                 INSERT INTO conversation (id, app, key, title, work, last_seen_at) VALUES
                   (1, 'fireflies', 'm', 'Website review', 1, 1), (2, 'whatsapp', 'c', 'Ben', NULL, 1),
                   (3, 'fireflies', 'm2', 'Website Review', 1, 2);
                 INSERT INTO activity (id, source, kind, ts, title, url, ext_id) VALUES
                   (1, 'history', 'visit', 100, 'Pricing - Website', 'https://www.example.com/pricing', 'e1'),
                   (2, 'history', 'visit', 101, 'Pricing - Website', 'https://www.example.com/pricing', 'e2'),
                   (3, 'history', 'visit', 102, 'Inbox', 'chrome://newtab', 'e3'),
                   (4, 'claude', 'prompt', 103, 'x', NULL, 'e4');",
            )
            .unwrap();
        store
    }

    #[test]
    fn new_items_take_each_title_once_and_move_the_cursor() {
        let store = store();
        let mut cursor = Cursor::default();
        let items = new_items(&store, &mut cursor).unwrap();
        let kinds: Vec<(&str, &str)> = items.iter().map(|i| (i.kind, i.place.as_str())).collect();
        assert_eq!(
            kinds,
            [
                ("meeting", "Fireflies"),
                ("chat", "WhatsApp"),
                ("page", "example.com")
            ]
        );
        assert_eq!((cursor.conversation, cursor.activity), (3, 3));
        assert!(new_items(&store, &mut cursor).unwrap().is_empty());
    }

    #[test]
    fn grouped_folders_make_one_project_and_not_projects_are_left_out() {
        let store = store();
        store
            .conn()
            .execute_batch(
                "INSERT INTO project (id, name, root) VALUES (3, 'Scratch', '/s');
                 INSERT INTO session (id, project_id, title, started_at, last_at)
                   VALUES ('s4', 3, 'Misc', 1, 40);",
            )
            .unwrap();
        let groups: Groups = serde_json::from_str(
            r#"{"groups": {"Product": ["Website", "App"]}, "not_projects": ["Scratch"],
                "about": {"Product": "The website and app"}}"#,
        )
        .unwrap();
        let projects = projects(&store, &HashSet::new(), &groups).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Product");
        assert_eq!(projects[0].folder, "/a, /w");
        assert_eq!(projects[0].about.as_deref(), Some("The website and app"));
        assert_eq!(
            projects[0].recent_work,
            ["Notch glow", "Hero section", "Pricing page copy"]
        );
        assert_eq!(groups.project_of("Scratch"), None);
        assert_eq!(groups.project_of("Other").as_deref(), Some("Other"));
    }

    #[test]
    fn projects_are_described_by_recent_sessions_minus_held_out_ones() {
        let store = store();
        let hold_out: HashSet<String> = ["Hero section".to_owned()].into();
        let projects = projects(&store, &hold_out, &Groups::default()).unwrap();
        let website = projects.iter().find(|p| p.name == "Website").unwrap();
        assert_eq!(website.recent_work, ["Pricing page copy"]);
        let q = question(
            &new_items(&store, &mut Cursor::default()).unwrap()[0],
            website,
        );
        assert_eq!(q["type"], "noul");
        assert_eq!(q["instructions"]["project"]["name"], "Website");
        assert_eq!(q["instructions"]["item"]["kind"], "meeting");
        assert!(
            run_due(&store, Local::now()).is_empty(),
            "tests have no key"
        );
    }
}
