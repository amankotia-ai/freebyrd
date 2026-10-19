//! Where each of the person's Claude Code sessions stands: working, waiting
//! for them (Claude finished and they have not looked yet), blocked (Claude
//! asked them something, or is stuck on an error), or done. One reading for
//! the notch's Sessions tab, the closed notch beside it (`live.rs`) and the
//! iPad's board (`companion::board`), so they agree. A finished session in
//! a git repository whose files are not committed, or whose commits are not
//! pushed, offers Push: a new Claude Code session the person sees commits
//! and pushes them, as the push-only sessions they start themselves do.
//!
//! Plain code over the Claude desktop app's session records
//! (`claude-code-sessions/**/local_*.json`: the title, the folder, when the
//! person last looked) and Claude Code's transcripts, read only. Nothing is
//! sent anywhere and nothing is stored.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use memory::time;

/// How much of a transcript's end is read for where its last turn stands.
const TAIL: u64 = 512 * 1024;
/// A turn with no new entry for this long has stalled.
const QUIET_MS: i64 = 20 * 60_000;
/// Retrying the API for this long without an answer is being stuck.
const RETRY_MS: i64 = 60_000;
/// Background agents and commands started longer ago than this are
/// forgotten, in case their notice never came.
const BACKGROUND_MS: i64 = 3 * 3_600_000;
/// Finished turns older than this are done, looked at or not.
const WAITING_MS: i64 = 18 * 3_600_000;
/// Rows on the board, at most.
const ROWS: usize = 30;
/// Git gets this long to answer before Push is left off.
const GIT_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a session's changes are kept before git is asked again.
const CHANGES_FOR: Duration = Duration::from_secs(20);

/// What is kept between reads, by file or session.
type Cache<K, V> = LazyLock<Mutex<HashMap<K, V>>>;

// Where a session stands.

/// A session's state, as the board and the iPad show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `running`, `waiting` (finished, not looked at yet), `blocked` (asked
    /// the person something, or stuck) or `done`.
    pub state: &'static str,
    /// What it is doing or waiting for, in words: "Working for 12 min",
    /// "Done 4 min ago", "Asked you a question", "Out of usage".
    pub label: String,
    /// When the transcript was last written (ms).
    pub last_ms: i64,
}

/// The last turn of a transcript, as plain code reads it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Turn {
    /// When the person's last message (or a notice Claude answers) came.
    started_ms: Option<i64>,
    /// How and when the turn ended, if it did.
    ended: Option<(i64, Ending)>,
    /// A question (AskUserQuestion) or a plan (ExitPlanMode) waits for them.
    question: Option<&'static str>,
    /// Since when the API has been retried without an answer.
    retrying_ms: Option<i64>,
    /// Background agents and commands still running.
    agents: usize,
    commands: usize,
    /// The newest entry's time.
    last_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Claude finished; `asks` when its last words end with a question.
    Done { asks: bool },
    /// The person stopped it.
    Stopped,
    /// The turn ended on an API error.
    Error(Problem),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    SignedOut,
    OutOfUsage,
    Slept,
    Unreachable,
}

static AGENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"agentId: ([A-Za-z0-9_-]+)").expect("valid regex"));
static COMMAND_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"running in background with ID: ([A-Za-z0-9_-]+)").expect("valid regex")
});
static TASK_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<task-id>([A-Za-z0-9_-]+)</task-id>").expect("valid regex"));

/// Reads the last turn from transcript lines, oldest first. `now_ms` drops
/// background work started too long ago.
fn turn<'a>(lines: impl Iterator<Item = &'a str>, now_ms: i64) -> Turn {
    let mut turn = Turn::default();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut last_words = String::new();
    // Background work by ID: whether it is an agent, and when it started.
    let mut background: HashMap<String, (bool, i64)> = HashMap::new();
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = timestamp(&entry);
        if let Some(at) = at {
            turn.last_ms = Some(turn.last_ms.map_or(at, |last| last.max(at)));
        }
        // A background agent or command finished: its notice names it,
        // whether queued, attached or delivered as a message.
        if line.contains("<task-id>") {
            for found in TASK_ID.captures_iter(line) {
                background.remove(&found[1]);
            }
        }
        if entry["isSidechain"] == true {
            continue;
        }
        let at = at.unwrap_or_default();
        match entry["type"].as_str() {
            Some("user") => {
                let content = &entry["message"]["content"];
                let mut results = false;
                for block in content.as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    results = true;
                    let id = block["tool_use_id"].as_str().unwrap_or_default();
                    pending.retain(|(pending_id, _)| pending_id != id);
                    let said = text_of(&block["content"]);
                    if let Some(found) = AGENT_ID.captures(&said) {
                        background.insert(found[1].to_owned(), (true, at));
                    } else if let Some(found) = COMMAND_ID.captures(&said) {
                        background.insert(found[1].to_owned(), (false, at));
                    }
                }
                if results || entry["isMeta"] == true {
                    continue;
                }
                let said = text_of(content);
                let said = said.trim();
                if said.starts_with("[Request interrupted") {
                    turn.ended = Some((at, Ending::Stopped));
                    pending.clear();
                } else if starts_a_turn(said) {
                    turn.started_ms = Some(at);
                    turn.ended = None;
                    turn.retrying_ms = None;
                    pending.clear();
                    last_words.clear();
                }
            }
            Some("assistant") => {
                let message = &entry["message"];
                if entry["isApiErrorMessage"] == true {
                    let problem = problem(
                        entry["error"].as_str().unwrap_or_default(),
                        &text_of(&message["content"]),
                    );
                    turn.ended = Some((at, Ending::Error(problem)));
                    turn.retrying_ms = None;
                    continue;
                }
                turn.retrying_ms = None;
                for block in message["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("tool_use") => pending.push((
                            block["id"].as_str().unwrap_or_default().to_owned(),
                            block["name"].as_str().unwrap_or_default().to_owned(),
                        )),
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or_default().trim();
                            if !text.is_empty() {
                                text.clone_into(&mut last_words);
                            }
                        }
                        _ => {}
                    }
                }
                match message["stop_reason"].as_str() {
                    Some("end_turn") => {
                        let asks = last_words.ends_with('?');
                        turn.ended = Some((at, Ending::Done { asks }));
                    }
                    Some("tool_use") => turn.ended = None,
                    _ => {}
                }
            }
            Some("system") if entry["subtype"] == "api_error" => {
                turn.retrying_ms.get_or_insert(at);
            }
            _ => {}
        }
    }
    turn.question = pending
        .iter()
        .rev()
        .find_map(|(_, name)| match name.as_str() {
            "AskUserQuestion" => Some("question"),
            "ExitPlanMode" => Some("plan"),
            _ => None,
        });
    for (agent, started) in background.values() {
        if now_ms - started < BACKGROUND_MS {
            if *agent {
                turn.agents += 1;
            } else {
                turn.commands += 1;
            }
        }
    }
    turn
}

/// Whether a message of the person's starts a turn Claude answers: not a
/// local command's echo (`/model`, `/goal`) or its output.
fn starts_a_turn(said: &str) -> bool {
    !said.is_empty() && !said.starts_with("<command-") && !said.starts_with("<local-command")
}

fn problem(error: &str, text: &str) -> Problem {
    let text = text.to_lowercase();
    match error {
        "authentication_failed" => Problem::SignedOut,
        "rate_limit" => Problem::OutOfUsage,
        _ if text.contains("went to sleep") => Problem::Slept,
        _ if text.contains("usage") && text.contains("limit") => Problem::OutOfUsage,
        _ => Problem::Unreachable,
    }
}

/// Where a session stands, from its last turn, now, and whether the person
/// has looked at it since that turn ended (`seen`, given the end).
fn status_of(turn: &Turn, now_ms: i64, seen: impl FnOnce(i64) -> bool) -> Status {
    let last_ms = turn.last_ms.unwrap_or(0);
    let status = |state, label: String| Status {
        state,
        label,
        last_ms,
    };
    if let Some(kind) = turn.question {
        let label = if kind == "plan" {
            "Has a plan for you to approve"
        } else {
            "Asked you a question"
        };
        return status("blocked", label.to_owned());
    }
    match turn.ended {
        Some((at, Ending::Error(problem))) => {
            let label = match problem {
                Problem::SignedOut => "Signed out of Claude",
                Problem::OutOfUsage => "Out of usage",
                Problem::Slept => "The Mac slept mid-reply",
                Problem::Unreachable => "Couldn't reach Claude",
            };
            if now_ms - at >= WAITING_MS {
                status("done", format!("{label} {} ago", span(now_ms - at)))
            } else {
                status("blocked", label.to_owned())
            }
        }
        Some((at, Ending::Stopped)) => status("done", format!("Stopped {} ago", span(now_ms - at))),
        Some((at, Ending::Done { asks })) => {
            if turn.agents + turn.commands > 0 {
                return status("running", waiting_on(turn.agents, turn.commands));
            }
            let ago = format!("Done {} ago", span(now_ms - at));
            if now_ms - at >= WAITING_MS || seen(at) {
                status("done", ago)
            } else if asks {
                status("waiting", format!("{ago}, with a question"))
            } else {
                status("waiting", ago)
            }
        }
        None => {
            if let Some(since) = turn.retrying_ms.filter(|since| now_ms - since >= RETRY_MS) {
                return status(
                    "blocked",
                    format!("Can't reach Claude, retrying for {}", span(now_ms - since)),
                );
            }
            if turn.last_ms.is_some() && now_ms - last_ms >= QUIET_MS {
                if now_ms - last_ms >= WAITING_MS {
                    return status(
                        "done",
                        format!("Last active {} ago", span(now_ms - last_ms)),
                    );
                }
                return status(
                    "blocked",
                    format!("No progress for {}", span(now_ms - last_ms)),
                );
            }
            let label = turn.started_ms.map_or_else(
                || "Working".to_owned(),
                |since| format!("Working for {}", span(now_ms - since)),
            );
            status("running", label)
        }
    }
}

fn waiting_on(agents: usize, commands: usize) -> String {
    match (agents, commands) {
        (1, 0) => "Waiting on an agent".to_owned(),
        (n, 0) => format!("Waiting on {n} agents"),
        (0, 1) => "Waiting on a background command".to_owned(),
        (0, n) => format!("Waiting on {n} background commands"),
        (a, c) => format!("Waiting on {} background tasks", a + c),
    }
}

/// A stretch of time in words: "under a minute", "12 min", "2 h",
/// "1 h 20 min", "1 day", "3 days".
fn span(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "under a minute".to_owned(),
        1..=59 => format!("{minutes} min"),
        1440..=2879 => "1 day".to_owned(),
        2880.. => format!("{} days", minutes / 1440),
        _ if minutes % 60 == 0 || minutes >= 180 => format!("{} h", minutes / 60),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    }
}

fn timestamp(entry: &Value) -> Option<i64> {
    let text = entry["timestamp"].as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// The text of a message's content: a string, or its text blocks.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// The desktop app's sessions.

/// A Claude Code session in the desktop app's Code tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    /// The app's own ID (`local_…`), which its links take.
    pub id: String,
    /// Claude Code's session ID: its transcript's name.
    pub cli: String,
    pub cwd: String,
    pub title: String,
    pub last_activity_ms: i64,
    pub last_focused_ms: i64,
    /// When the person's latest message arrived, as the app noted it.
    pub latest_user_ms: i64,
}

fn desktop(record: &Value) -> Option<Desktop> {
    if record["isArchived"] == true {
        return None;
    }
    let id = record["sessionId"]
        .as_str()
        .filter(|id| is_desktop_id(id))?;
    Some(Desktop {
        id: id.to_owned(),
        cli: record["cliSessionId"].as_str()?.to_owned(),
        cwd: record["cwd"]
            .as_str()
            .or_else(|| record["originCwd"].as_str())?
            .to_owned(),
        title: record["title"].as_str().unwrap_or_default().to_owned(),
        last_activity_ms: record["lastActivityAt"].as_i64().unwrap_or(0),
        last_focused_ms: record["lastFocusedAt"].as_i64().unwrap_or(0),
        latest_user_ms: record["latestUserFrameAt"].as_i64().unwrap_or(0),
    })
}

/// A desktop session ID, as the app's links accept it.
fn is_desktop_id(id: &str) -> bool {
    id.strip_prefix("local_").is_some_and(|rest| {
        (1..=64).contains(&rest.len())
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn records_dir() -> Option<PathBuf> {
    Some(home()?.join("Library/Application Support/Claude/claude-code-sessions"))
}

/// Session records by file, kept while the file is unchanged.
static RECORDS: Cache<PathBuf, (SystemTime, Option<Desktop>)> = LazyLock::new(Mutex::default);

/// The desktop app's sessions active since `since_ms`, newest first.
pub fn desktop_sessions(since_ms: i64) -> Vec<Desktop> {
    let Some(dir) = records_dir() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    record_files(&dir, 0, &mut files);
    // A record is rewritten when its session changes; an hour's slack
    // covers a clock that disagrees.
    let since = SystemTime::UNIX_EPOCH
        + Duration::from_millis(u64::try_from(since_ms - 3_600_000).unwrap_or(0));
    let mut cache = RECORDS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut found = Vec::new();
    for (path, modified) in files {
        if modified < since {
            continue;
        }
        let entry = cache
            .entry(path.clone())
            .or_insert((SystemTime::UNIX_EPOCH, None));
        if entry.0 != modified {
            let record = fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok());
            *entry = (modified, record.as_ref().and_then(desktop));
        }
        if let Some(session) = &entry.1
            && session.last_activity_ms >= since_ms
        {
            found.push(session.clone());
        }
    }
    found.sort_by_key(|session| std::cmp::Reverse(session.last_activity_ms));
    found
}

fn record_files(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, SystemTime)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if depth < 3 {
                record_files(&path, depth + 1, out);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"))
            && let Ok(modified) = meta.modified()
        {
            out.push((path, modified));
        }
    }
}

/// Transcripts by Claude Code session ID.
static PATHS: LazyLock<Mutex<HashMap<String, PathBuf>>> = LazyLock::new(Mutex::default);

/// A Claude Code session's transcript, if there is one.
pub fn transcript(cli: &str) -> Option<PathBuf> {
    if cli.is_empty() || !cli.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let mut paths = PATHS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(path) = paths.get(cli).filter(|path| path.is_file()) {
        return Some(path.clone());
    }
    let name = format!("{cli}.jsonl");
    let found = fs::read_dir(memory::ingest::claude_code::projects_dir()?)
        .ok()?
        .flatten()
        .map(|project| project.path().join(&name))
        .find(|path| path.is_file())?;
    paths.insert(cli.to_owned(), found.clone());
    Some(found)
}

/// Last turns by transcript, kept while the file is unchanged.
static TURNS: Cache<PathBuf, (u64, SystemTime, Turn)> = LazyLock::new(Mutex::default);

fn read_turn(path: &Path, now_ms: i64) -> Option<Turn> {
    let meta = fs::metadata(path).ok()?;
    let (len, modified) = (meta.len(), meta.modified().ok()?);
    let mut cache = TURNS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached_len, cached_at, turn)) = cache.get(path)
        && *cached_len == len
        && *cached_at == modified
    {
        return Some(turn.clone());
    }
    let mut file = File::open(path).ok()?;
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let tail = String::from_utf8_lossy(&tail);
    // From the middle of a file, the first line is cut.
    let lines = tail.lines().skip(usize::from(start > 0));
    let turn = turn(lines, now_ms);
    cache.insert(path.to_owned(), (len, modified, turn.clone()));
    Some(turn)
}

/// What each session was last seen at: the end of the turn the person was
/// looking at it for, in the Claude app in front.
static LOOKED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Mutex::default);

/// Whether the person has looked at `session` since `ended_ms`: they picked
/// it in the app after that, or it was the session open in the app while
/// the app was in front (`open_now`) once the turn had ended.
fn seen(session: &Desktop, ended_ms: i64, open_now: bool) -> bool {
    let mut looked = LOOKED.lock().unwrap_or_else(PoisonError::into_inner);
    if open_now {
        looked.insert(session.id.clone(), ended_ms);
        return true;
    }
    session.last_focused_ms >= ended_ms || looked.get(&session.id).is_some_and(|&at| at >= ended_ms)
}

/// The session open in the Claude app, if the app is in front: the one the
/// person picked last.
fn open_in_front(sessions: &[Desktop]) -> Option<String> {
    let front = crate::context::frontmost_app()?;
    if front.name.as_deref() != Some(crate::claude_app::APP_NAME) {
        return None;
    }
    sessions
        .iter()
        .max_by_key(|session| session.last_focused_ms)
        .map(|session| session.id.clone())
}

/// A session with where it stands.
pub struct Entry {
    pub desktop: Desktop,
    pub status: Status,
    pub transcript: Option<PathBuf>,
}

/// Where today starts for the board: this working day (from 04:00), or the
/// last 18 hours if that reaches further back, so what finished last night
/// after the person stopped is still there when they come back.
fn since_ms(now: DateTime<Local>) -> i64 {
    let day = time::day_span(time::working_day(now)).start_ms;
    day.min(now.timestamp_millis() - WAITING_MS)
}

/// The desktop app's sessions of today (`since_ms`), newest first, with
/// where each stands.
pub fn today(now: DateTime<Local>) -> Vec<Entry> {
    entries(now, since_ms(now))
}

fn entries(now: DateTime<Local>, since: i64) -> Vec<Entry> {
    let now_ms = now.timestamp_millis();
    let sessions = desktop_sessions(since);
    let open = open_in_front(&sessions);
    sessions
        .into_iter()
        .map(|desktop| {
            let transcript = transcript(&desktop.cli);
            let mut turn = transcript
                .as_deref()
                .and_then(|path| read_turn(path, now_ms))
                .unwrap_or_default();
            // A long turn's start can lie before the part of the
            // transcript that was read; the app notes it too.
            if turn.started_ms.is_none() && turn.ended.is_none() && desktop.latest_user_ms > 0 {
                turn.started_ms = Some(desktop.latest_user_ms);
            }
            let open_now = open.as_deref() == Some(desktop.id.as_str());
            let mut status = status_of(&turn, now_ms, |ended| seen(&desktop, ended, open_now));
            if turn.last_ms.is_none() {
                status.last_ms = desktop.last_activity_ms;
            }
            Entry {
                desktop,
                status,
                transcript,
            }
        })
        .collect()
}

/// Where a Claude Code session stands, by its transcript's ID: for sessions
/// that are not the desktop app's (a terminal's), which no one is known to
/// have looked at.
pub fn status_of_transcript(cli: &str, now: DateTime<Local>) -> Option<Status> {
    let now_ms = now.timestamp_millis();
    let turn = read_turn(&transcript(cli)?, now_ms)?;
    Some(status_of(&turn, now_ms, |_| false))
}

/// A folder's name for the board: its last part, or "Scratch" for the
/// desktop app's scratch folders.
pub fn folder_name(cwd: &str) -> String {
    if cwd.contains("/Claude/scratch-workspaces/") {
        return "Scratch".to_owned();
    }
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(cwd)
        .to_owned()
}

// What the closed notch shows beside it.

/// How many sessions work, and how many need the person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub working: usize,
    pub needs: usize,
    /// Whether one of those is blocked, so the count shows as a warning.
    pub blocked: bool,
}

pub fn summary(now: DateTime<Local>) -> Option<Summary> {
    let entries = today(now);
    let summary = summary_of(entries.iter().map(|entry| &entry.status));
    (summary.working + summary.needs > 0).then_some(summary)
}

fn summary_of<'a>(statuses: impl Iterator<Item = &'a Status>) -> Summary {
    let mut summary = Summary::default();
    for status in statuses {
        match status.state {
            "running" => summary.working += 1,
            "waiting" => summary.needs += 1,
            "blocked" => {
                summary.needs += 1;
                summary.blocked = true;
            }
            _ => {}
        }
    }
    summary
}

// The board in the notch.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub rows: Vec<Row>,
    /// How many need the person, for the tab's count.
    pub needs: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// The desktop app's ID, which Open takes.
    pub id: String,
    pub title: String,
    pub folder: String,
    pub state: &'static str,
    pub label: String,
    /// `needs`, `working`, `push` (done, with work not pushed) or
    /// `earlier`: the heading it sits under.
    pub group: &'static str,
    pub push: Option<Push>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Push {
    /// Files the session changed that are not committed.
    pub files: usize,
    /// Commits the session made that are not pushed.
    pub commits: usize,
    /// "5 files not pushed".
    pub label: String,
}

/// How far back the board looks for sessions whose work is not pushed.
const PUSH_DAYS_MS: i64 = 3 * 24 * 3_600_000;

/// Today's sessions, and those of the last three days with work left to
/// push: working first (the person's ask, 2026-10-13), then blocked,
/// waiting, not pushed and earlier.
pub fn board(now: DateTime<Local>) -> Board {
    let today = since_ms(now);
    let mut rows: Vec<(u8, i64, Row)> = entries(now, now.timestamp_millis() - PUSH_DAYS_MS)
        .into_iter()
        .filter_map(|entry| {
            // Push only once Claude has finished there.
            let push = matches!(entry.status.state, "waiting" | "done" | "blocked")
                .then(|| changes(&entry.desktop, entry.transcript.as_deref()))
                .flatten()
                .map(|changes| changes.push());
            // Older than today: only as a loose end.
            if entry.status.last_ms < today && push.is_none() {
                return None;
            }
            let (group, rank) = match entry.status.state {
                "running" => ("working", 0),
                "blocked" => ("needs", 1),
                "waiting" => ("needs", 2),
                _ if push.is_some() => ("push", 3),
                _ => ("earlier", 4),
            };
            let row = Row {
                title: if entry.desktop.title.is_empty() {
                    "Untitled session".to_owned()
                } else {
                    entry.desktop.title.clone()
                },
                folder: folder_name(&entry.desktop.cwd),
                state: entry.status.state,
                label: entry.status.label,
                group,
                push,
                id: entry.desktop.id,
            };
            Some((rank, entry.status.last_ms, row))
        })
        .collect();
    rows.sort_by_key(|(rank, last, _)| (*rank, std::cmp::Reverse(*last)));
    let needs = rows.iter().filter(|(.., row)| row.group == "needs").count();
    Board {
        rows: rows.into_iter().take(ROWS).map(|(.., row)| row).collect(),
        needs,
    }
}

// Opening a session.

/// Opens a session in the Claude app's Code tab.
pub fn open(id: &str) -> Result<()> {
    if !is_desktop_id(id) {
        bail!("not a session of the Claude app");
    }
    open_link(&format!("claude://code/continue?session={id}"))
}

fn open_link(link: &str) -> Result<()> {
    let status = Command::new("/usr/bin/open")
        .arg(link)
        .status()
        .context("opening the Claude app")?;
    if !status.success() {
        bail!("the Claude app did not open ({status})");
    }
    Ok(())
}

// Push.

/// What a session changed that is not pushed yet, in its repository.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Changes {
    /// The repository's top folder.
    top: PathBuf,
    /// The session's files that are not committed, relative to `top`.
    files: Vec<String>,
    /// Commits not pushed, when the session committed.
    commits: usize,
}

impl Changes {
    fn push(&self) -> Push {
        let files = match self.files.len() {
            0 => None,
            1 => Some("1 file".to_owned()),
            n => Some(format!("{n} files")),
        };
        let commits = match self.commits {
            0 => None,
            1 => Some("1 commit".to_owned()),
            n => Some(format!("{n} commits")),
        };
        let what = match (files, commits) {
            (Some(files), Some(commits)) => format!("{files} and {commits}"),
            (Some(one), None) | (None, Some(one)) => one,
            (None, None) => String::new(),
        };
        Push {
            files: self.files.len(),
            commits: self.commits,
            label: format!("{what} not pushed"),
        }
    }
}

/// What a session edited, by transcript and its length.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Edits {
    files: BTreeSet<String>,
    /// Whether it ran `git commit`.
    committed: bool,
}

static EDITS: LazyLock<Mutex<HashMap<PathBuf, (u64, Edits)>>> = LazyLock::new(Mutex::default);
static CHANGES: Cache<String, (Instant, u64, Option<Changes>)> = LazyLock::new(Mutex::default);

/// The files a transcript's edits wrote (with the file tools; a script's
/// writes are not known), and whether it committed.
fn edits<'a>(lines: impl Iterator<Item = &'a str>) -> Edits {
    let mut edits = Edits::default();
    for line in lines {
        if !line.contains("\"tool_use\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["type"] != "assistant" {
            continue;
        }
        for block in entry["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_use" {
                continue;
            }
            let input = &block["input"];
            match block["name"].as_str() {
                Some("Edit" | "Write" | "MultiEdit") => {
                    if let Some(path) = input["file_path"].as_str() {
                        edits.files.insert(path.to_owned());
                    }
                }
                Some("NotebookEdit") => {
                    if let Some(path) = input["notebook_path"].as_str() {
                        edits.files.insert(path.to_owned());
                    }
                }
                Some("Bash")
                    if input["command"]
                        .as_str()
                        .is_some_and(|command| command.contains("git commit")) =>
                {
                    edits.committed = true;
                }
                _ => {}
            }
        }
    }
    edits
}

fn read_edits(path: &Path) -> Option<Edits> {
    let len = fs::metadata(path).ok()?.len();
    let mut cache = EDITS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached_len, edits)) = cache.get(path)
        && *cached_len == len
    {
        return Some(edits.clone());
    }
    let text = fs::read(path).ok()?;
    let edits = edits(String::from_utf8_lossy(&text).lines());
    cache.insert(path.to_owned(), (len, edits.clone()));
    Some(edits)
}

/// What a session changed that is not pushed, if it is in a git repository
/// and anything is left; asked of git at most every 20 s a session.
fn changes(session: &Desktop, transcript: Option<&Path>) -> Option<Changes> {
    let transcript = transcript?;
    let len = fs::metadata(transcript).ok()?.len();
    {
        let cache = CHANGES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, cached_len, changes)) = cache.get(&session.id)
            && *cached_len == len
            && at.elapsed() < CHANGES_FOR
        {
            return changes.clone();
        }
    }
    let found = find_changes(session, transcript);
    CHANGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session.id.clone(), (Instant::now(), len, found.clone()));
    found
}

fn find_changes(session: &Desktop, transcript: &Path) -> Option<Changes> {
    let edits = read_edits(transcript)?;
    if edits.files.is_empty() && !edits.committed {
        return None;
    }
    let top =
        PathBuf::from(git(Path::new(&session.cwd), &["rev-parse", "--show-toplevel"])?.trim());
    let files: Vec<String> = edits
        .files
        .iter()
        .filter_map(|file| Path::new(file).strip_prefix(&top).ok())
        .filter_map(|rel| rel.to_str())
        .filter(|rel| is_safe_path(rel))
        .map(str::to_owned)
        .collect();
    let mut dirty = Vec::new();
    if !files.is_empty() {
        let mut args = vec![
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
        ];
        args.extend(files.iter().map(String::as_str));
        dirty = porcelain_paths(&git(&top, &args)?);
        dirty.retain(|path| files.contains(path));
    }
    let commits = if edits.committed {
        git(&top, &["rev-list", "--count", "@{u}..HEAD"])
            .and_then(|count| count.trim().parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    (!dirty.is_empty() || commits > 0).then_some(Changes {
        top,
        files: dirty,
        commits,
    })
}

/// The paths in `git status --porcelain=v1 -z` output. A rename's or a
/// copy's entry is followed by the path it came from, which is skipped.
fn porcelain_paths(output: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        let Some((status, path)) = entry.split_at_checked(3) else {
            continue;
        };
        paths.push(path.to_owned());
        if status.starts_with(['R', 'C']) {
            entries.next();
        }
    }
    paths
}

/// A path that can go in a prompt as it is: relative, inside the
/// repository, one line, not too long.
fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 300
        && !path.starts_with('/')
        && !path.chars().any(char::is_control)
        && !Path::new(path)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

/// Runs git in `dir` without taking its optional locks (so the person's
/// sessions never meet a held index lock), within `GIT_TIMEOUT`. Its output,
/// or nothing if it failed or took too long.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if started.elapsed() > GIT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).into_owned())
}

/// What Push hands to Claude Code: the prompt and the folder it runs in.
pub struct Handoff {
    pub prompt: String,
    pub folder: String,
}

/// Push: opens a new Claude Code session in the session's repository with
/// a prompt to commit and push what it changed. The link only fills the
/// prompt in; the caller sends it (`claude_app::send_filled_prompt`).
pub fn push(id: &str) -> Result<Handoff> {
    let handoff = handoff(id)?;
    open_link(&format!(
        "claude://code/new?folder={}&q={}",
        encode(&handoff.folder),
        encode(&handoff.prompt)
    ))?;
    Ok(handoff)
}

/// What Push would hand over for a session, opening nothing.
pub fn handoff(id: &str) -> Result<Handoff> {
    let since = Local::now().timestamp_millis() - PUSH_DAYS_MS;
    let session = desktop_sessions(since)
        .into_iter()
        .find(|session| session.id == id)
        .context("that session is not among the last three days'")?;
    let transcript = transcript(&session.cli);
    // Ask git again: the board may be 20 s old.
    CHANGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&session.id);
    let changes = changes(&session, transcript.as_deref())
        .context("nothing of that session's is left to push")?;
    let folder = changes
        .top
        .to_str()
        .context("the repository's folder is not text")?
        .to_owned();
    let prompt = push_prompt(&changes.files, changes.commits);
    Ok(Handoff { prompt, folder })
}

/// The prompt for Push: fixed words and the session's file paths, nothing
/// else from the session.
fn push_prompt(files: &[String], commits: usize) -> String {
    const LISTED: usize = 40;
    let rebase = "If the push is rejected because the branch is behind, pull with rebase first \
                  (with --autostash, so other uncommitted work stays as it is). If that \
                  conflicts, stop and tell me.";
    let mut lines = Vec::new();
    if files.is_empty() {
        let commits = if commits == 1 {
            "1 commit".to_owned()
        } else {
            format!("{commits} commits")
        };
        lines.push(format!(
            "This branch has {commits} from a finished session that are not pushed yet. Push the current branch."
        ));
    } else {
        lines.push(
            "Commit and push the work a finished session left in this folder. Its files that are not committed yet:"
                .to_owned(),
        );
        lines.extend(files.iter().take(LISTED).map(|file| format!("- {file}")));
        if files.len() > LISTED {
            lines.push(format!(
                "- and {} more of its files (git status lists them)",
                files.len() - LISTED
            ));
        }
        lines.push(
            "Other uncommitted files here belong to other work: leave them out. Read the diff, write the commit message the way this repository's recent commits are written, commit only these files, then push the current branch."
                .to_owned(),
        );
        if commits > 0 {
            lines.push(format!(
                "The branch also has {} from that session that {} not pushed yet; push {} too.",
                if commits == 1 {
                    "1 commit".to_owned()
                } else {
                    format!("{commits} commits")
                },
                if commits == 1 { "is" } else { "are" },
                if commits == 1 { "it" } else { "them" },
            ));
        }
    }
    lines.push(rebase.to_owned());
    lines.join("\n")
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn at(ms: i64) -> String {
        DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    fn prompt(ms: i64, text: &str) -> String {
        serde_json::json!({"type": "user", "timestamp": at(ms), "message": {"role": "user", "content": text}})
            .to_string()
    }

    fn reply(ms: i64, text: &str, stop: &str) -> String {
        serde_json::json!({"type": "assistant", "timestamp": at(ms),
            "message": {"content": [{"type": "text", "text": text}], "stop_reason": stop}})
        .to_string()
    }

    fn tool(ms: i64, id: &str, name: &str) -> String {
        serde_json::json!({"type": "assistant", "timestamp": at(ms),
            "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": {}}], "stop_reason": "tool_use"}})
        .to_string()
    }

    fn result(ms: i64, id: &str, text: &str) -> String {
        serde_json::json!({"type": "user", "timestamp": at(ms),
            "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": text}]}})
        .to_string()
    }

    fn status(lines: &[String], seen: bool) -> Status {
        let turn = turn(lines.iter().map(String::as_str), NOW);
        status_of(&turn, NOW, |_| seen)
    }

    const MIN: i64 = 60_000;

    #[test]
    fn a_session_works_until_its_turn_ends() {
        let lines = [
            prompt(NOW - 12 * MIN, "fix the hero"),
            tool(NOW - MIN, "t1", "Bash"),
        ];
        let working = status(&lines, false);
        assert_eq!(working.state, "running");
        assert_eq!(working.label, "Working for 12 min");

        let mut lines = lines.to_vec();
        lines.push(result(NOW - MIN, "t1", "ok"));
        lines.push(reply(NOW - 4 * MIN, "Fixed it.", "end_turn"));
        let done = status(&lines, false);
        assert_eq!(
            (done.state, done.label.as_str()),
            ("waiting", "Done 4 min ago")
        );
        assert_eq!(status(&lines, true).state, "done");
    }

    #[test]
    fn a_question_or_a_plan_blocks_until_answered() {
        let mut lines = vec![
            prompt(NOW - 5 * MIN, "pick a layout"),
            tool(NOW - 4 * MIN, "q1", "AskUserQuestion"),
        ];
        let asked = status(&lines, true);
        assert_eq!(
            (asked.state, asked.label.as_str()),
            ("blocked", "Asked you a question")
        );
        lines.push(result(NOW - MIN, "q1", "Option A"));
        assert_eq!(status(&lines, true).state, "running");

        let plan = [
            prompt(NOW - 5 * MIN, "plan it"),
            tool(NOW - 4 * MIN, "p1", "ExitPlanMode"),
        ];
        assert_eq!(status(&plan, true).label, "Has a plan for you to approve");
    }

    #[test]
    fn a_reply_ending_in_a_question_says_so() {
        let lines = [
            prompt(NOW - 3 * MIN, "ideas?"),
            reply(NOW - 2 * MIN, "Want me to build it?", "end_turn"),
        ];
        assert_eq!(
            status(&lines, false).label,
            "Done 2 min ago, with a question"
        );
    }

    #[test]
    fn errors_block_with_what_went_wrong() {
        let error = |kind: &str, text: &str| {
            serde_json::json!({"type": "assistant", "timestamp": at(NOW - MIN), "isApiErrorMessage": true,
                "error": kind, "message": {"content": [{"type": "text", "text": text}]}})
            .to_string()
        };
        let label = |kind, text| {
            let lines = [prompt(NOW - 2 * MIN, "go"), error(kind, text)];
            let status = status(&lines, false);
            assert_eq!(status.state, "blocked");
            status.label
        };
        assert_eq!(
            label("authentication_failed", "Please run /login"),
            "Signed out of Claude"
        );
        assert_eq!(
            label("rate_limit", "You're out of usage credits."),
            "Out of usage"
        );
        assert_eq!(
            label(
                "server_error",
                "API Error: Your computer went to sleep mid-response."
            ),
            "The Mac slept mid-reply"
        );
        assert_eq!(
            label("server_error", "Can't reach the API server"),
            "Couldn't reach Claude"
        );
    }

    #[test]
    fn retrying_blocks_after_a_minute_and_an_answer_clears_it() {
        let retry = |ms: i64| {
            serde_json::json!({"type": "system", "subtype": "api_error", "timestamp": at(ms)})
                .to_string()
        };
        let mut lines = vec![
            prompt(NOW - 5 * MIN, "go"),
            retry(NOW - 3 * MIN),
            retry(NOW - MIN / 2),
        ];
        let stuck = status(&lines, false);
        assert_eq!(
            (stuck.state, stuck.label.as_str()),
            ("blocked", "Can't reach Claude, retrying for 3 min")
        );
        lines.push(tool(NOW - MIN / 4, "t1", "Read"));
        assert_eq!(status(&lines, false).state, "running");
        let fresh = [prompt(NOW - MIN, "go"), retry(NOW - MIN / 2)];
        assert_eq!(status(&fresh, false).state, "running");
    }

    #[test]
    fn a_quiet_turn_blocks_and_an_old_one_is_done() {
        let quiet = [
            prompt(NOW - 40 * MIN, "render"),
            tool(NOW - 25 * MIN, "t1", "Bash"),
        ];
        let stuck = status(&quiet, false);
        assert_eq!(
            (stuck.state, stuck.label.as_str()),
            ("blocked", "No progress for 25 min")
        );
        let old = [
            prompt(NOW - 30 * 60 * MIN, "render"),
            tool(NOW - 20 * 60 * MIN, "t1", "Bash"),
        ];
        assert_eq!(status(&old, false).state, "done");
    }

    #[test]
    fn stopping_it_is_done_and_needs_nothing() {
        let lines = [
            prompt(NOW - 10 * MIN, "audit"),
            tool(NOW - 9 * MIN, "t1", "Bash"),
            prompt(NOW - 5 * MIN, "[Request interrupted by user]"),
        ];
        let stopped = status(&lines, false);
        assert_eq!(
            (stopped.state, stopped.label.as_str()),
            ("done", "Stopped 5 min ago")
        );
    }

    #[test]
    fn background_agents_keep_it_working_until_their_notices_come() {
        let launched = |id: &str, agent: &str| {
            result(
                NOW - 9 * MIN,
                id,
                &format!("Async agent launched successfully.\nagentId: {agent} (internal ID)"),
            )
        };
        let mut lines = vec![
            prompt(NOW - 10 * MIN, "study my work"),
            tool(NOW - 9 * MIN, "a1", "Agent"),
            tool(NOW - 9 * MIN, "a2", "Agent"),
            launched("a1", "ad34"),
            launched("a2", "af97"),
            result(
                NOW - 9 * MIN,
                "b1",
                "Command running in background with ID: bsf7. Output is",
            ),
            reply(NOW - 8 * MIN, "Four agents are working.", "end_turn"),
        ];
        assert_eq!(status(&lines, false).label, "Waiting on 3 background tasks");
        lines.push(prompt(
            NOW - 5 * MIN,
            "<task-notification>\n<task-id>ad34</task-id>\n<status>completed",
        ));
        lines.push(reply(NOW - 5 * MIN, "One is back.", "end_turn"));
        lines.push(prompt(
            NOW - 4 * MIN,
            "<task-notification>\n<task-id>bsf7</task-id>",
        ));
        lines.push(reply(NOW - 4 * MIN, "The command finished.", "end_turn"));
        assert_eq!(status(&lines, false).label, "Waiting on an agent");
        lines.push(prompt(
            NOW - 3 * MIN,
            "<task-notification>\n<task-id>af97</task-id>",
        ));
        lines.push(reply(NOW - 2 * MIN, "All back.", "end_turn"));
        assert_eq!(status(&lines, false).state, "waiting");
    }

    #[test]
    fn local_commands_and_side_chains_do_not_start_a_turn() {
        let side =
            serde_json::json!({"type": "user", "isSidechain": true, "timestamp": at(NOW - MIN),
            "message": {"content": "a subagent's prompt"}})
            .to_string();
        let lines = [
            prompt(NOW - 6 * MIN, "go"),
            reply(NOW - 5 * MIN, "Done.", "end_turn"),
            prompt(NOW - 2 * MIN, "<command-name>/model</command-name>"),
            prompt(
                NOW - 2 * MIN,
                "<local-command-stdout>Set model</local-command-stdout>",
            ),
            side,
        ];
        assert_eq!(status(&lines, false).state, "waiting");
    }

    #[test]
    fn spans_read_as_words() {
        assert_eq!(span(20_000), "under a minute");
        assert_eq!(span(12 * MIN), "12 min");
        assert_eq!(span(120 * MIN), "2 h");
        assert_eq!(span(80 * MIN), "1 h 20 min");
        assert_eq!(span(200 * MIN), "3 h");
        assert_eq!(span(30 * 60 * MIN), "1 day");
        assert_eq!(span(50 * 60 * MIN), "2 days");
    }

    #[test]
    fn the_summary_counts_work_and_what_needs_the_person() {
        let of = |state| Status {
            state,
            label: String::new(),
            last_ms: 0,
        };
        let statuses = [of("running"), of("running"), of("waiting"), of("done")];
        assert_eq!(
            summary_of(statuses.iter()),
            Summary {
                working: 2,
                needs: 1,
                blocked: false
            }
        );
        let statuses = [of("blocked")];
        assert!(summary_of(statuses.iter()).blocked);
    }

    #[test]
    fn records_skip_archived_sessions_and_odd_ids() {
        let record = serde_json::json!({"sessionId": "local_7504a49e-82f5", "cliSessionId": "ab65",
            "cwd": "/w/repo", "title": "Fix hero", "lastActivityAt": 5, "lastFocusedAt": 3});
        let session = desktop(&record).unwrap();
        assert_eq!(
            (session.id.as_str(), session.cli.as_str()),
            ("local_7504a49e-82f5", "ab65")
        );
        let mut archived = record.clone();
        archived["isArchived"] = true.into();
        assert!(desktop(&archived).is_none());
        let mut odd = record;
        odd["sessionId"] = "local_x&y".into();
        assert!(desktop(&odd).is_none());
        assert!(!is_desktop_id("session_1") && is_desktop_id("local_ab-12"));
    }

    #[test]
    fn looking_marks_a_turn_seen() {
        let session = Desktop {
            id: "local_look-test".to_owned(),
            cli: "c".to_owned(),
            cwd: "/w".to_owned(),
            title: String::new(),
            last_activity_ms: 0,
            last_focused_ms: 100,
            latest_user_ms: 0,
        };
        assert!(seen(&session, 90, false));
        assert!(!seen(&session, 200, false));
        assert!(seen(&session, 200, true));
        // Still seen once the app goes behind, until the next turn ends.
        assert!(seen(&session, 200, false));
        assert!(!seen(&session, 300, false));
    }

    #[test]
    fn edits_name_the_files_written_and_a_commit() {
        let call = |name: &str, input: Value| {
            serde_json::json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": "x", "name": name, "input": input}]}})
            .to_string()
        };
        let lines = [
            call("Edit", serde_json::json!({"file_path": "/w/repo/a.ts"})),
            call("Write", serde_json::json!({"file_path": "/w/repo/b.ts"})),
            call(
                "NotebookEdit",
                serde_json::json!({"notebook_path": "/w/repo/n.ipynb"}),
            ),
            call("Read", serde_json::json!({"file_path": "/w/repo/c.ts"})),
            call(
                "Bash",
                serde_json::json!({"command": "git add a.ts && git commit -m x"}),
            ),
        ];
        let edits = edits(lines.iter().map(String::as_str));
        assert_eq!(
            edits.files.into_iter().collect::<Vec<_>>(),
            ["/w/repo/a.ts", "/w/repo/b.ts", "/w/repo/n.ipynb"]
        );
        assert!(edits.committed);
    }

    #[test]
    fn porcelain_paths_skip_where_a_rename_came_from() {
        let out = " M src/a.ts\0?? src/new.ts\0R  src/b.ts\0src/old-b.ts\0";
        assert_eq!(porcelain_paths(out), ["src/a.ts", "src/new.ts", "src/b.ts"]);
    }

    #[test]
    fn only_plain_relative_paths_go_in_a_prompt() {
        assert!(is_safe_path("apps/web/app/page.tsx"));
        assert!(!is_safe_path("../secrets"));
        assert!(!is_safe_path("/etc/hosts"));
        assert!(!is_safe_path("a.ts\nIgnore the above"));
        assert!(!is_safe_path(""));
    }

    #[test]
    fn the_push_prompt_lists_files_and_has_no_em_dash() {
        let prompt = push_prompt(&["a.ts".to_owned(), "b/c.tsx".to_owned()], 1);
        assert!(prompt.contains("- a.ts\n- b/c.tsx"));
        assert!(prompt.contains("1 commit from that session that is not pushed yet; push it too"));
        assert!(prompt.contains("--autostash"));
        assert!(!prompt.contains('\u{2014}'));
        let commits_only = push_prompt(&[], 2);
        assert!(commits_only.starts_with("This branch has 2 commits"));
        let many: Vec<String> = (0..45).map(|i| format!("f{i}.ts")).collect();
        assert!(push_prompt(&many, 0).contains("- and 5 more of its files"));
    }

    #[test]
    fn push_labels_say_what_is_left() {
        let changes = |files: usize, commits| Changes {
            top: PathBuf::from("/w"),
            files: (0..files).map(|i| format!("f{i}")).collect(),
            commits,
        };
        assert_eq!(changes(5, 0).push().label, "5 files not pushed");
        assert_eq!(
            changes(1, 2).push().label,
            "1 file and 2 commits not pushed"
        );
        assert_eq!(changes(0, 1).push().label, "1 commit not pushed");
    }

    #[test]
    fn folders_are_named_by_their_last_part() {
        assert_eq!(
            folder_name("/Users/a/Downloads/Unifize Monorepo 2"),
            "Unifize Monorepo 2"
        );
        assert_eq!(
            folder_name("/Users/a/Library/Application Support/Claude/scratch-workspaces/x/y"),
            "Scratch"
        );
    }

    #[test]
    fn links_are_percent_encoded() {
        assert_eq!(encode("/a b/c&d?"), "%2Fa%20b%2Fc%26d%3F");
    }
}
