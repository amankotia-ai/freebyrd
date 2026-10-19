//! The detectors: plain code over the memory that finds work the person
//! repeats. No model is asked here. Each finding keeps only minimal
//! evidence for its draft: counts, dates, titles, project and folder names,
//! and the first words of what the person typed, redacted.
//!
//! - `asks`: the same ask to Claude Code across sessions ("push changes to
//!   github" in many sessions), or one ask repeated down a list by hand
//!   ("do the next case study"). A skill can do it.
//! - `times`: the same thing at the same time on weekdays, such as a
//!   meeting room opened at 13:00 or a Notion page every morning. A
//!   scheduled job can read it shortly before, when a connector reaches it.
//! - `steps`: the same steps in the same order across apps, such as a
//!   deploy, then the live site.
//! - `loops`: open loops: the person's meeting action items that have no
//!   date, so nothing reminds them. Those on the to-do list (`todos`) have
//!   a reminder and do not count.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, Timelike, Weekday};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::conversations::ME;
use crate::redact::redact;
use crate::store::Store;
use crate::{one_line, time, truncate};

const DAY_MS: i64 = 86_400_000;
/// How far back the detectors look: as far as the browser history goes.
const LOOKBACK_DAYS: i64 = 90;
/// What has not happened for this long is not a habit any more.
const RECENT_DAYS: i64 = 21;
/// A routine at a time of day must have happened within two weeks.
const RECENT_TIME_DAYS: i64 = 14;
/// The words that open a prompt identify its ask.
const OPENING_WORDS: usize = 6;
/// Prompts this alike (shared words over all words) are the same ask.
const MIN_SIMILARITY: f64 = 0.5;
/// An ask repeated in this many sessions, on more than one day, is a
/// habit. (Three sessions also let through one-off remarks repeated after
/// a fix that did not stick, such as "make the hero visual wider".)
const MIN_SESSIONS: usize = 4;
const MIN_DAYS: usize = 2;
/// One asked this many times in one or two sessions is a list worked
/// through by hand ("do the next case study").
const MIN_BY_HAND: usize = 5;
const BY_HAND_SESSIONS: usize = 2;
/// Groups of prompts this large make steps in a sequence.
const MIN_STEP_ASKS: usize = 3;
/// What happened this soon after an ask, in its session, came of it.
const FOLLOW_MS: i64 = 3_600_000;
/// A routine happens within this long a stretch of the day,
const WINDOW_MINUTES: u32 = 90;
/// on at least this many weekdays, over two weeks or more.
const MIN_WINDOW_DAYS: usize = 4;
/// Routines starting this close together go in one job.
const SLOT_MINUTES: u32 = 60;
/// Steps closer than this belong to one sequence.
const STEP_GAP_MS: i64 = 30 * 60_000;
/// Three steps in order on this many days, or two on more, are a routine.
const MIN_STEP_DAYS: usize = 4;
const MIN_PAIR_DAYS: usize = 6;
const MAX_STEP_PATTERNS: usize = 5;
/// A site open on this share of the days is background, not a step.
const BACKGROUND_SHARE: f64 = 0.6;
/// Open loops: the person's undated action items from the last month.
const LOOP_DAYS: i64 = 30;
const MIN_LOOP_ITEMS: usize = 3;
const MAX_EXAMPLES: usize = 6;
const EXAMPLE_CHARS: usize = 100;
/// Minutes from midnight to the start of a working day (04:00).
const DAY_START_MINUTES: u32 = time::DAY_START_HOUR * 60;

/// Words that carry no ask on their own: filler, pronouns, small verbs.
const FILLER: &[&str] = &[
    "a",
    "about",
    "actually",
    "after",
    "again",
    "ahead",
    "all",
    "already",
    "also",
    "am",
    "an",
    "and",
    "another",
    "any",
    "anything",
    "are",
    "as",
    "at",
    "basically",
    "be",
    "been",
    "being",
    "bit",
    "but",
    "by",
    "can",
    "claude",
    "cool",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "don",
    "done",
    "dont",
    "down",
    "each",
    "else",
    "even",
    "every",
    "everything",
    "for",
    "from",
    "get",
    "give",
    "go",
    "going",
    "gonna",
    "good",
    "got",
    "great",
    "had",
    "has",
    "have",
    "having",
    "he",
    "hello",
    "her",
    "here",
    "hey",
    "hi",
    "him",
    "his",
    "hmm",
    "how",
    "if",
    "im",
    "in",
    "into",
    "is",
    "it",
    "its",
    "just",
    "kind",
    "know",
    "let",
    "lets",
    "like",
    "little",
    "ll",
    "look",
    "make",
    "may",
    "me",
    "might",
    "mine",
    "more",
    "most",
    "must",
    "my",
    "need",
    "needs",
    "nice",
    "no",
    "not",
    "now",
    "of",
    "off",
    "ok",
    "okay",
    "on",
    "one",
    "only",
    "onto",
    "or",
    "other",
    "our",
    "out",
    "over",
    "please",
    "pls",
    "plz",
    "put",
    "re",
    "really",
    "right",
    "same",
    "see",
    "she",
    "should",
    "so",
    "some",
    "something",
    "sort",
    "still",
    "stuff",
    "such",
    "sure",
    "take",
    "thank",
    "thanks",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "think",
    "this",
    "those",
    "through",
    "to",
    "try",
    "uh",
    "uhh",
    "um",
    "umm",
    "up",
    "us",
    "ve",
    "very",
    "wanna",
    "want",
    "was",
    "we",
    "well",
    "were",
    "what",
    "whats",
    "when",
    "where",
    "which",
    "who",
    "whom",
    "why",
    "will",
    "with",
    "would",
    "yeah",
    "yep",
    "yes",
    "yet",
    "you",
    "your",
    "yours",
];

/// The kinds of repetition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Ask,
    Time,
    Steps,
    Loop,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Ask => "ask",
            Kind::Time => "time",
            Kind::Steps => "steps",
            Kind::Loop => "loop",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Kind::Ask => "the same ask to Claude Code",
            Kind::Time => "the same thing at the same time on weekdays",
            Kind::Steps => "the same steps in the same order",
            Kind::Loop => "an open loop",
        }
    }
}

/// A stretch of the day, on weekdays, when something keeps happening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// Local clock times, "12:45" and "13:30".
    pub from: String,
    pub to: String,
    /// The weekdays it happened on, 0 for Sunday to 6 for Saturday.
    pub weekdays: Vec<u32>,
    /// On how many days it happened in the window, of the weekdays it
    /// happened at all.
    pub hits: usize,
    pub of: usize,
}

impl Window {
    /// "between 12:45 and 13:30 on 4 of the 5 weekdays it happened".
    pub fn describe(&self) -> String {
        format!(
            "between {} and {} on {} of the {} weekdays it happened",
            self.from, self.to, self.hits, self.of
        )
    }
}

/// Something seen, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub at: i64,
    pub text: String,
}

/// Something the person repeats, as the detectors found it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Found {
    pub kind: Kind,
    /// Its identity from one run to the next, such as `ask:change git push`.
    pub key: String,
    /// The words or items that identify it, for knowing it again when its
    /// key drifts as new examples arrive.
    pub signature: Vec<String>,
    /// What it is, in a few words: "push changes to github".
    pub label: String,
    pub count: usize,
    pub sessions: usize,
    pub days: Vec<NaiveDate>,
    pub examples: Vec<Example>,
    /// Other facts for the draft, such as what usually came next.
    pub facts: Vec<String>,
    /// The connector servers that can read what it is about.
    pub readers: Vec<String>,
    pub window: Option<Window>,
    pub score: f64,
}

impl Found {
    /// "on 22 days from Tue 1 Sep to Mon 12 Oct".
    pub fn span(&self) -> String {
        match (self.days.first(), self.days.last()) {
            (Some(first), Some(last)) if first != last => format!(
                "on {} days from {} to {}",
                self.days.len(),
                time::short_day_label(*first),
                time::short_day_label(*last)
            ),
            (Some(day), _) => format!("on {}", time::short_day_label(*day)),
            _ => String::new(),
        }
    }
}

/// Everything the detectors find in the memory at `now`, strongest first.
pub fn all(store: &Store, now: DateTime<Local>) -> Result<Vec<Found>> {
    let since = now.timestamp_millis() - LOOKBACK_DAYS * DAY_MS;
    let prompts = load_prompts(store, since)?;
    let groups = cluster(&prompts);
    let visits = load_visits(store, since)?;
    let background = background_hosts(&visits);
    let mut found = asks(store, &prompts, &groups, now)?;
    found.extend(times(store, &visits, &background, since, now)?);
    found.extend(steps(
        store,
        &prompts,
        &groups,
        &visits,
        &background,
        since,
        now,
    )?);
    found.extend(loops(store, now)?);
    found.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
    Ok(found)
}

/// When the person usually starts work on weekdays, as "12:30": the middle
/// of each weekday's first activity over the last 30 days, rounded down to
/// a quarter hour.
pub fn usual_start(store: &Store, now: DateTime<Local>) -> Result<Option<String>> {
    let mut statement = store.conn().prepare(
        "SELECT ts FROM activity
         WHERE ts >= ?1 AND ((source = 'claude' AND kind = 'prompt') OR (source = 'history' AND kind = 'visit'))
         ORDER BY ts",
    )?;
    let rows = statement.query_map([now.timestamp_millis() - 30 * DAY_MS], |row| {
        row.get::<_, i64>(0)
    })?;
    let mut first: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    for ts in rows {
        if let Some((day, minute)) = weekday_minute(ts?) {
            first.entry(day).or_insert(minute);
        }
    }
    let mut minutes: Vec<u32> = first.into_values().collect();
    if minutes.len() < 5 {
        return Ok(None);
    }
    minutes.sort_unstable();
    let middle = minutes[minutes.len() / 2];
    Ok(Some(clock(middle - middle % 15)))
}

// Asks.

struct Prompt {
    ts: i64,
    session: String,
    project: Option<String>,
    text: String,
    words: Vec<String>,
}

fn load_prompts(store: &Store, since: i64) -> Result<Vec<Prompt>> {
    let mut statement = store.conn().prepare(
        "SELECT a.ts, a.session_id, p.name, a.detail FROM activity a
         LEFT JOIN project p ON p.id = a.project_id
         WHERE a.source = 'claude' AND a.kind = 'prompt' AND a.ts >= ?1
           AND a.session_id IS NOT NULL AND a.detail IS NOT NULL
         ORDER BY a.ts, a.id",
    )?;
    let rows = statement.query_map([since], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut prompts = Vec::new();
    for row in rows {
        let (ts, session, project, text) = row?;
        // Text that Claude Code or the desktop app put in a turn, such as a
        // scheduled task's prompt, is not the person's ask.
        if text.trim_start().starts_with('<') {
            continue;
        }
        let words = opening(&text);
        if words.len() < 2 {
            continue;
        }
        prompts.push(Prompt {
            ts,
            session,
            project,
            text,
            words,
        });
    }
    Ok(prompts)
}

/// The first content words of a prompt, which identify the ask: "Push all
/// changes to GitHub, please" gives ["push", "change", "github"]. Paths,
/// links and files mentioned with @ are left out.
pub fn opening(text: &str) -> Vec<String> {
    words(text, OPENING_WORDS)
}

/// The first `limit` content words of `text`, each once, as `opening`.
pub fn words(text: &str, limit: usize) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let text = MENTION.replace_all(text, " ").replace("[pasted text]", " ");
    for token in text.split_whitespace() {
        if is_link(token) || is_path(token) {
            continue;
        }
        for word in token.split(|c: char| !c.is_alphanumeric()) {
            let word = word.to_lowercase();
            if word.chars().count() < 2
                || word.chars().all(|c| c.is_ascii_digit())
                || FILLER.contains(&word.as_str())
            {
                continue;
            }
            let word = stem(&word);
            if !words.contains(&word) {
                words.push(word);
            }
            if words.len() == limit {
                return words;
            }
        }
    }
    words
}

fn is_link(token: &str) -> bool {
    let token = token.trim_matches(|c: char| "\"'()[]<>,".contains(c));
    token.starts_with("http") || token.contains("://") || token.starts_with("www.")
}

fn is_path(token: &str) -> bool {
    let token = token.trim_matches(|c: char| "\"'()[]<>,".contains(c));
    token.starts_with('@')
        || token.starts_with('/')
        || token.starts_with('~')
        || (token.contains('/') && token.contains('.'))
}

/// Plural endings off: "changes" and "change" are one word.
fn stem(word: &str) -> String {
    if word.len() > 4
        && word.ends_with('s')
        && !word.ends_with("ss")
        && !word.ends_with("us")
        && !word.ends_with("is")
    {
        word[..word.len() - 1].to_owned()
    } else {
        word.to_owned()
    }
}

/// Two words are the same when equal, or when one starts with the other
/// and both have three letters or more ("git", "github").
pub fn same_word(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 3 && (a.starts_with(b) || b.starts_with(a)))
}

/// Shared words over all words. Fewer than two shared words is no match.
pub fn similarity(a: &[String], b: &[String]) -> f64 {
    let shared = a
        .iter()
        .filter(|x| b.iter().any(|y| same_word(x, y)))
        .count();
    if shared < 2 {
        return 0.0;
    }
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// Groups prompts that open with mostly the same words, starting with the
/// same one (usually the verb: "push", "remove"). Each group has a leader,
/// its first prompt; a prompt joins the group whose leader it is most like,
/// so a group cannot drift through a chain of near matches.
fn cluster(prompts: &[Prompt]) -> Vec<Vec<usize>> {
    let mut leaders: Vec<&[String]> = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_word: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, prompt) in prompts.iter().enumerate() {
        let mut candidates: Vec<usize> = prompt
            .words
            .iter()
            .filter_map(|word| by_word.get(word.as_str()))
            .flatten()
            .copied()
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        let best = candidates
            .into_iter()
            .filter(|group| same_word(&prompt.words[0], &leaders[*group][0]))
            .map(|group| (group, similarity(&prompt.words, leaders[group])))
            .filter(|(_, alike)| *alike >= MIN_SIMILARITY)
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        match best {
            Some((group, _)) => groups[group].push(index),
            None => {
                for word in &prompt.words {
                    by_word.entry(word).or_default().push(groups.len());
                }
                leaders.push(&prompt.words);
                groups.push(vec![index]);
            }
        }
    }
    groups
}

fn asks(
    store: &Store,
    prompts: &[Prompt],
    groups: &[Vec<usize>],
    now: DateTime<Local>,
) -> Result<Vec<Found>> {
    let recent = now.timestamp_millis() - RECENT_DAYS * DAY_MS;
    let mut asked_in: HashMap<&str, Vec<i64>> = HashMap::new();
    for prompt in prompts {
        asked_in.entry(&prompt.session).or_default().push(prompt.ts);
    }
    let mut found = Vec::new();
    for members in groups {
        let sessions: BTreeSet<&str> = members
            .iter()
            .map(|&i| prompts[i].session.as_str())
            .collect();
        let days: BTreeSet<NaiveDate> = members
            .iter()
            .filter_map(|&i| day_of(prompts[i].ts))
            .collect();
        let last = members
            .iter()
            .map(|&i| prompts[i].ts)
            .max()
            .unwrap_or_default();
        let habit = sessions.len() >= MIN_SESSIONS && days.len() >= MIN_DAYS;
        let by_hand = members.len() >= MIN_BY_HAND && sessions.len() <= BY_HAND_SESSIONS;
        if !(habit || by_hand) || last < recent {
            continue;
        }
        let signature = signature(prompts, members);
        if signature.len() < 2 {
            continue;
        }
        let mut facts = ask_facts(store, prompts, members, &asked_in)?;
        if !habit {
            facts.push(by_hand_fact(prompts, members));
        }
        let times: Vec<i64> = members.iter().map(|&i| prompts[i].ts).collect();
        let window = routine(&times, now).map(|(window, _)| window);
        if let Some(window) = &window {
            facts.push(format!("On weekdays it was asked {}.", window.describe()));
        }
        let score =
            2.0 * sessions.len() as f64 + 2.0 * days.len() as f64 + members.len() as f64 / 2.0;
        found.push(Found {
            kind: Kind::Ask,
            key: key(Kind::Ask, &signature),
            label: label(prompts, members),
            signature,
            count: members.len(),
            sessions: sessions.len(),
            days: days.into_iter().collect(),
            examples: spread(members)
                .into_iter()
                .map(|i| Example {
                    at: prompts[i].ts,
                    text: example(&prompts[i].text),
                })
                .collect(),
            facts,
            readers: Vec::new(),
            window,
            score,
        });
    }
    Ok(found)
}

/// The words most of a group's prompts open with, most common first.
fn signature(prompts: &[Prompt], members: &[usize]) -> Vec<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for &i in members {
        for word in &prompts[i].words {
            match counts.iter_mut().find(|(known, _)| same_word(known, word)) {
                Some((_, count)) => *count += 1,
                None => counts.push((word.clone(), 1)),
            }
        }
    }
    let half = members.len().div_ceil(2);
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
        .into_iter()
        .filter(|(_, count)| *count >= half)
        .map(|(word, _)| word)
        .take(OPENING_WORDS)
        .collect()
}

fn key(kind: Kind, signature: &[String]) -> String {
    let mut parts: Vec<&str> = signature.iter().map(String::as_str).collect();
    parts.sort_unstable();
    format!("{}:{}", kind.as_str(), parts.join(" "))
}

/// The way the person most often put a group's ask, as typed.
fn label(prompts: &[Prompt], members: &[usize]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for &i in members {
        *counts
            .entry(one_line(&example(&prompts[i].text).to_lowercase(), 60))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.len().cmp(&a.0.len())))
        .map(|(text, _)| text)
        .unwrap_or_default()
}

/// The start of a prompt as evidence: on one line, with paths and links
/// replaced and secrets out.
fn example(text: &str) -> String {
    let text = MENTION
        .replace_all(text, " [file] ")
        .replace("[pasted text]", "[pasted]");
    let mut words: Vec<&str> = Vec::new();
    for token in text.split_whitespace() {
        let word = if is_link(token) {
            "[link]"
        } else if is_path(token) {
            "[file]"
        } else {
            token
        };
        if !(word.starts_with('[') && words.last() == Some(&word)) {
            words.push(word);
        }
    }
    let joined = words.join(" ");
    truncate(&redact(&joined), EXAMPLE_CHARS)
}

/// At most `MAX_EXAMPLES` of `members`, spread from the first to the last.
fn spread<T: Clone>(members: &[T]) -> Vec<T> {
    if members.len() <= MAX_EXAMPLES {
        return members.to_vec();
    }
    (0..MAX_EXAMPLES)
        .map(|k| members[k * (members.len() - 1) / (MAX_EXAMPLES - 1)].clone())
        .collect()
}

pub(super) fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

/// Where the asks were made, and what came of them in the same session.
fn ask_facts(
    store: &Store,
    prompts: &[Prompt],
    members: &[usize],
    asked_in: &HashMap<&str, Vec<i64>>,
) -> Result<Vec<String>> {
    let mut facts = Vec::new();
    let mut projects: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for &i in members {
        if let Some(project) = prompts[i].project.as_deref() {
            projects
                .entry(project)
                .or_default()
                .insert(prompts[i].session.as_str());
        }
    }
    if !projects.is_empty() {
        let mut list: Vec<(&str, usize)> = projects.iter().map(|(p, s)| (*p, s.len())).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let list: Vec<String> = list
            .iter()
            .take(3)
            .map(|(project, n)| format!("{project} ({})", plural(*n, "session")))
            .collect();
        facts.push(format!("Projects: {}.", list.join(", ")));
    }
    let sessions: BTreeSet<&str> = members
        .iter()
        .map(|&i| prompts[i].session.as_str())
        .collect();
    let titles = session_titles(store, &sessions)?;
    if !titles.is_empty() {
        facts.push(format!("Session titles: {}.", titles.join("; ")));
    }

    // What came of each ask: until the person's next real ask in that
    // session, or an hour.
    let mut statement = store.conn().prepare_cached(
        "SELECT kind, detail FROM activity
         WHERE source = 'claude' AND session_id = ?1 AND ts > ?2 AND ts < ?3
           AND kind IN ('ship', 'skill', 'notion', 'edit')",
    )?;
    let mut next: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut folders: BTreeMap<String, usize> = BTreeMap::new();
    let mut edits = 0;
    for &i in members {
        let prompt = &prompts[i];
        let until = asked_in
            .get(prompt.session.as_str())
            .and_then(|times| times.iter().find(|&&ts| ts > prompt.ts))
            .map_or(prompt.ts + FOLLOW_MS, |&ts| ts.min(prompt.ts + FOLLOW_MS));
        let rows = statement.query_map(params![prompt.session, prompt.ts, until], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (kind, detail) = row?;
            let detail = detail.unwrap_or_default();
            match kind.as_str() {
                "edit" => {
                    edits += 1;
                    *folders.entry(folder_of(&detail)).or_default() += 1;
                }
                "ship" => {
                    next.entry(ship_step(&detail))
                        .or_default()
                        .insert(&prompt.session);
                }
                "skill" => {
                    next.entry(format!("the {} skill", one_line(&detail, 40)))
                        .or_default()
                        .insert(&prompt.session);
                }
                "notion" => {
                    next.entry(notion_step(&detail).to_owned())
                        .or_default()
                        .insert(&prompt.session);
                }
                _ => {}
            }
        }
    }
    if !next.is_empty() {
        let mut list: Vec<(String, usize)> =
            next.into_iter().map(|(s, set)| (s, set.len())).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let list: Vec<String> = list
            .iter()
            .take(5)
            .map(|(step, n)| format!("{step} ({})", plural(*n, "session")))
            .collect();
        facts.push(format!(
            "What came next in the same session: {}.",
            list.join(", ")
        ));
    }
    if edits > 0 {
        let mut list: Vec<(String, usize)> =
            folders.into_iter().filter(|(f, _)| !f.is_empty()).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let list: Vec<String> = list
            .iter()
            .take(2)
            .map(|(folder, n)| format!("{folder} ({n})"))
            .collect();
        facts.push(format!(
            "{} after it{}.",
            plural(edits, "file edit"),
            if list.is_empty() {
                String::new()
            } else {
                format!(", mostly in {}", list.join(" and "))
            }
        ));
    }
    Ok(facts)
}

fn session_titles(store: &Store, sessions: &BTreeSet<&str>) -> Result<Vec<String>> {
    let mut statement = store
        .conn()
        .prepare_cached("SELECT title FROM session WHERE id = ?1 AND title IS NOT NULL")?;
    let mut titles: Vec<String> = Vec::new();
    for session in sessions {
        let found: Vec<String> = statement
            .query_map([session], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        for title in found {
            let title = one_line(&redact(&title), 60);
            if !title.is_empty() && !titles.contains(&title) {
                titles.push(title);
            }
        }
        if titles.len() == 3 {
            break;
        }
    }
    Ok(titles)
}

/// The last two folders of an edited file's path: "src/components".
fn folder_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 | 1 => String::new(),
        2 => parts[0].to_owned(),
        n => format!("{}/{}", parts[n - 3], parts[n - 2]),
    }
}

/// A shipping command as a step: "git commit", "deploy to Vercel (prod)".
fn ship_step(detail: &str) -> String {
    let detail = detail.trim();
    if detail.starts_with("git commit") {
        "git commit".to_owned()
    } else if detail.starts_with("git push") {
        "git push".to_owned()
    } else if detail.starts_with("vercel") && detail.contains("--prod") {
        "deploy to Vercel (prod)".to_owned()
    } else if detail.starts_with("vercel") {
        "deploy to Vercel".to_owned()
    } else if detail.starts_with("gh pr") {
        "a GitHub pull request".to_owned()
    } else {
        detail
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn notion_step(detail: &str) -> &'static str {
    let action = detail.split_whitespace().next().unwrap_or_default();
    if [
        "notion-fetch",
        "notion-search",
        "notion-query",
        "notion-get",
        "notion-list",
    ]
    .iter()
    .any(|read| action.starts_with(read))
    {
        "Notion read through Claude"
    } else {
        "Notion changed through Claude"
    }
}

fn by_hand_fact(prompts: &[Prompt], members: &[usize]) -> String {
    let sessions: BTreeSet<&str> = members
        .iter()
        .map(|&i| prompts[i].session.as_str())
        .collect();
    let days: BTreeSet<NaiveDate> = members
        .iter()
        .filter_map(|&i| day_of(prompts[i].ts))
        .collect();
    let mut times: Vec<i64> = members.iter().map(|&i| prompts[i].ts).collect();
    times.sort_unstable();
    let mut gaps: Vec<i64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let gap = gaps.get(gaps.len() / 2).copied().unwrap_or_default() / 60_000;
    let days: Vec<String> = days.iter().map(|d| time::short_day_label(*d)).collect();
    format!(
        "Asked {} times in {} on {}, a median of {} minutes apart.",
        members.len(),
        plural(sessions.len(), "session"),
        days.join(", "),
        gap
    )
}

// Times.

struct Visit {
    ts: i64,
    url: String,
    title: Option<String>,
}

fn load_visits(store: &Store, since: i64) -> Result<Vec<Visit>> {
    let mut statement = store.conn().prepare(
        "SELECT ts, url, title FROM activity
         WHERE source = 'history' AND kind = 'visit' AND ts >= ?1 AND url IS NOT NULL
         ORDER BY ts",
    )?;
    let visits = statement
        .query_map([since], |row| {
            Ok(Visit {
                ts: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(visits)
}

/// Sites open on most days, such as a feed or a search engine: background,
/// not a routine or a step.
fn background_hosts(visits: &[Visit]) -> HashSet<String> {
    let mut days_by_host: HashMap<String, HashSet<NaiveDate>> = HashMap::new();
    let mut all_days = HashSet::new();
    for visit in visits {
        let (Some(day), Some((host, _))) = (day_of(visit.ts), host_and_path(&visit.url)) else {
            continue;
        };
        all_days.insert(day);
        days_by_host.entry(host).or_default().insert(day);
    }
    let needed = all_days.len() as f64 * BACKGROUND_SHARE;
    days_by_host
        .into_iter()
        .filter(|(_, days)| all_days.len() >= 10 && days.len() as f64 >= needed)
        .map(|(host, _)| host)
        .collect()
}

/// A thing the person opens: a Notion page, a meeting room, a chat, a page.
#[derive(Debug, Clone)]
struct Item {
    key: String,
    /// "Website Tasks (Notion page)".
    label: String,
    reader: Option<&'static str>,
    /// How a job finds it: a Notion page's link, a meeting's title.
    handle: Option<String>,
}

const NOTION_HOSTS: &[&str] = &["app.notion.com", "notion.so", "notion.site"];
const NOTION: &str = "claude.ai Notion";
const FIREFLIES: &str = "claude.ai Fireflies";

/// A file mentioned in a prompt the way Claude Code writes it, quoted when
/// its path has spaces: `@"/Users/x/Screen Recording 1.mov"`.
static MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"@"[^"]*"|@'[^']*'"#).expect("valid pattern"));
static MEET_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]{3}-[a-z]{4}-[a-z]{3}$").expect("valid pattern"));
static COUNTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\(\d+\+?\)\s*").expect("valid pattern"));

fn item_for(url: &str, title: Option<&str>, background: &HashSet<String>) -> Option<Item> {
    let (host, path) = host_and_path(url)?;
    let title = title.map(clean_title).filter(|t| !t.is_empty());
    if NOTION_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
    {
        let id = notion_id(&path)?;
        return Some(Item {
            key: format!("notion:{id}"),
            label: format!(
                "{} (Notion page)",
                title.unwrap_or_else(|| "a page".to_owned())
            ),
            reader: Some(NOTION),
            handle: Some(format!("https://www.notion.so/{id}")),
        });
    }
    if host == "meet.google.com" {
        let code = path.trim_start_matches('/').split('/').next()?;
        if !MEET_CODE.is_match(code) {
            return None;
        }
        let name = title
            .as_deref()
            .and_then(meeting_name)
            .filter(|name| name != code);
        return Some(Item {
            key: format!("meet:{code}"),
            label: format!(
                "{} (Google Meet)",
                name.clone().unwrap_or_else(|| format!("the call {code}"))
            ),
            reader: name.is_some().then_some(FIREFLIES),
            handle: name,
        });
    }
    if host.is_empty() || background.contains(&host) {
        return None;
    }
    let page = format!("{host}{}", path.trim_end_matches('/'));
    Some(Item {
        label: match title {
            Some(title) => format!("{title} ({host})"),
            None => page.clone(),
        },
        key: format!("page:{page}"),
        reader: None,
        handle: None,
    })
}

/// "https://www.notion.so/a/b?x=1#c" as ("notion.so", "/a/b").
fn host_and_path(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let rest = &rest[..end];
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    Some((host, path.to_owned()))
}

/// The 32 hexadecimal digits that end a Notion page's path.
fn notion_id(path: &str) -> Option<String> {
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    let digits: String = last.chars().filter(|c| *c != '-').collect();
    let id = digits.get(digits.len().checked_sub(32)?..)?;
    id.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| id.to_lowercase())
}

/// "Meet – Website Rebuild" as "Website Rebuild".
fn meeting_name(title: &str) -> Option<String> {
    let name = ["Meet – ", "Meet - ", "Meet: "]
        .iter()
        .find_map(|prefix| title.strip_prefix(prefix))?
        .trim();
    (!name.is_empty() && name != "Google Meet").then(|| name.to_owned())
}

/// A page title without unread counters or the app's name.
fn clean_title(title: &str) -> String {
    let title = COUNTER.replace(title.trim(), "");
    let title = [" | Notion", " - Notion", " – Notion"]
        .iter()
        .find_map(|suffix| title.strip_suffix(suffix))
        .unwrap_or(&title);
    one_line(&redact(title), 80)
}

struct Routine {
    item: Item,
    window: Window,
    start: u32,
    days: Vec<NaiveDate>,
}

fn times(
    store: &Store,
    visits: &[Visit],
    background: &HashSet<String>,
    since: i64,
    now: DateTime<Local>,
) -> Result<Vec<Found>> {
    let mut things: BTreeMap<String, (Item, Vec<i64>)> = BTreeMap::new();
    for visit in visits {
        let Some(item) = item_for(&visit.url, visit.title.as_deref(), background) else {
            continue;
        };
        // The latest title wins: pages get renamed.
        let entry = things
            .entry(item.key.clone())
            .or_insert_with(|| (item.clone(), Vec::new()));
        entry.0 = item;
        entry.1.push(visit.ts);
    }
    // Chats the person writes in at the same time each day.
    let mut statement = store.conn().prepare(
        "SELECT c.id, c.title, m.sent_at FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE c.app = 'whatsapp' AND m.sender = ?1 AND m.sent_at >= ?2",
    )?;
    let rows = statement.query_map(params![ME, since], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (id, title, at) = row?;
        let key = format!("chat:{id}");
        things
            .entry(key.clone())
            .or_insert_with(|| {
                (
                    Item {
                        key,
                        label: format!("your WhatsApp chat with {}", one_line(&title, 40)),
                        reader: None,
                        handle: None,
                    },
                    Vec::new(),
                )
            })
            .1
            .push(at);
    }

    let mut routines: Vec<Routine> = things
        .into_values()
        .filter_map(|(item, times)| {
            let (window, days) = routine(&times, now)?;
            let start = minutes_after_start(&window.from);
            Some(Routine {
                item,
                window,
                start,
                days,
            })
        })
        .collect();
    routines.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.item.key.cmp(&b.item.key))
    });

    // Routines at the same time of day go together: one job reads them all.
    let mut slots: Vec<Vec<Routine>> = Vec::new();
    for routine in routines {
        match slots.last_mut() {
            Some(slot) if routine.start - slot[0].start <= SLOT_MINUTES => slot.push(routine),
            _ => slots.push(vec![routine]),
        }
    }
    Ok(slots.into_iter().map(slot_found).collect())
}

fn slot_found(slot: Vec<Routine>) -> Found {
    let mut keys: Vec<String> = slot.iter().map(|r| r.item.key.clone()).collect();
    keys.sort();
    let from = slot
        .iter()
        .map(|r| r.window.from.clone())
        .min()
        .unwrap_or_default();
    let to = slot
        .iter()
        .map(|r| r.window.to.clone())
        .max()
        .unwrap_or_default();
    let mut weekdays: Vec<u32> = slot
        .iter()
        .flat_map(|r| r.window.weekdays.clone())
        .collect();
    weekdays.sort_unstable();
    weekdays.dedup();
    let days: BTreeSet<NaiveDate> = slot.iter().flat_map(|r| r.days.clone()).collect();
    let mut readers: Vec<String> = slot
        .iter()
        .filter_map(|r| r.item.reader.map(str::to_owned))
        .collect();
    readers.sort();
    readers.dedup();
    let labels: Vec<&str> = slot.iter().map(|r| r.item.label.as_str()).collect();
    let facts = slot
        .iter()
        .map(|r| {
            let handle = r
                .item
                .handle
                .as_deref()
                .map(|h| {
                    if h.starts_with("http") {
                        format!(" Link: {h}")
                    } else {
                        format!(" Meeting title: {h}")
                    }
                })
                .unwrap_or_default();
            format!("{}: opened {}.{handle}", r.item.label, r.window.describe())
        })
        .collect();
    let examples = slot
        .iter()
        .flat_map(|r| {
            r.days.iter().rev().take(2).map(|day| Example {
                at: time::day_span(*day).start_ms,
                text: r.item.label.clone(),
            })
        })
        .collect();
    let hits = slot.iter().map(|r| r.window.hits).max().unwrap_or_default();
    let of = slot.iter().map(|r| r.window.of).max().unwrap_or_default();
    Found {
        kind: Kind::Time,
        key: format!("time:{}", keys.join(",")),
        label: format!("around {from} on weekdays: {}", labels.join(", ")),
        signature: keys,
        count: slot.iter().map(|r| r.window.hits).sum(),
        sessions: 0,
        days: days.into_iter().collect(),
        examples,
        facts,
        readers,
        window: Some(Window {
            from,
            to,
            weekdays,
            hits,
            of,
        }),
        score: 2.0 * hits as f64 + slot.len() as f64,
    }
}

fn minutes_after_start(clock_time: &str) -> u32 {
    let (hour, minute) = clock_time.split_once(':').unwrap_or(("0", "0"));
    let minutes = hour.parse::<u32>().unwrap_or(0) * 60 + minute.parse::<u32>().unwrap_or(0);
    (minutes + 1440 - DAY_START_MINUTES) % 1440
}

/// The weekday stretch of the day that most of `times` fall in, if it is a
/// routine: at least `MIN_WINDOW_DAYS` days, in two weeks or more, lately,
/// and at least half of the weekdays it happened at all. With the days.
fn routine(times: &[i64], now: DateTime<Local>) -> Option<(Window, Vec<NaiveDate>)> {
    let mut visits: Vec<(NaiveDate, u32)> = times
        .iter()
        .filter_map(|&ms| weekday_minute(ms))
        .map(|(day, minute)| (day, minute - minute % 5))
        .collect();
    visits.sort_unstable();
    visits.dedup();
    let all_days: BTreeSet<NaiveDate> = visits.iter().map(|v| v.0).collect();
    if all_days.len() < MIN_WINDOW_DAYS {
        return None;
    }
    let (start, end, days) = best_window(&visits)?;
    let weeks: BTreeSet<(i32, u32)> = days
        .iter()
        .map(|d| (d.iso_week().year(), d.iso_week().week()))
        .collect();
    let recent = time::working_day(now) - TimeDelta::days(RECENT_TIME_DAYS);
    let last = *days.iter().next_back()?;
    if days.len() < MIN_WINDOW_DAYS
        || weeks.len() < 2
        || last < recent
        || days.len() * 2 < all_days.len()
    {
        return None;
    }
    let mut weekdays: Vec<u32> = days
        .iter()
        .map(|d| d.weekday().num_days_from_sunday())
        .collect();
    weekdays.sort_unstable();
    weekdays.dedup();
    let window = Window {
        from: clock(start - start % 15),
        to: clock(end.div_ceil(15) * 15),
        weekdays,
        hits: days.len(),
        of: all_days.len(),
    };
    Some((window, days.into_iter().collect()))
}

/// The `WINDOW_MINUTES` of the day that the most days have something in:
/// its first and last minute after 04:00, and those days.
fn best_window(visits: &[(NaiveDate, u32)]) -> Option<(u32, u32, BTreeSet<NaiveDate>)> {
    let mut starts: Vec<u32> = visits.iter().map(|v| v.1).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut best: Option<(u32, u32, BTreeSet<NaiveDate>)> = None;
    for start in starts {
        let inside = visits
            .iter()
            .filter(|v| v.1 >= start && v.1 < start + WINDOW_MINUTES);
        let days: BTreeSet<NaiveDate> = inside.clone().map(|v| v.0).collect();
        if best.as_ref().is_none_or(|b| days.len() > b.2.len()) {
            let end = inside.map(|v| v.1).max().unwrap_or(start);
            best = Some((start, end, days));
        }
    }
    best
}

/// The working day of `ms` and its minutes after 04:00, on weekdays only.
fn weekday_minute(ms: i64) -> Option<(NaiveDate, u32)> {
    let at = time::local(ms)?;
    let day = time::working_day(at);
    if matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
        return None;
    }
    let minute = (at.hour() * 60 + at.minute() + 1440 - DAY_START_MINUTES) % 1440;
    Some((day, minute))
}

/// Minutes after 04:00 as a clock time: 540 is "13:00".
fn clock(minutes_after_start: u32) -> String {
    let minutes = (minutes_after_start + DAY_START_MINUTES) % 1440;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn day_of(ms: i64) -> Option<NaiveDate> {
    time::local(ms).map(time::working_day)
}

// Steps.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum App {
    Browser,
    Claude,
}

#[derive(Debug, Clone)]
struct Step {
    label: String,
    app: App,
    /// What the step is about. Opening Notion and reading it through Claude
    /// are one thing done twice, not two steps.
    subject: String,
    start: i64,
    end: i64,
    /// The Claude Code session of a step taken in one.
    session: Option<String>,
    /// The Notion pages it touched, by ID.
    pages: BTreeSet<String>,
}

#[derive(Default)]
struct Gram {
    days: BTreeSet<NaiveDate>,
    count: usize,
    last: i64,
    examples: Vec<(i64, String)>,
    sessions: BTreeSet<String>,
    pages: BTreeSet<String>,
}

/// Something done, as a step in a sequence.
struct Event {
    ts: i64,
    label: String,
    app: App,
    subject: String,
    session: Option<String>,
    page: Option<String>,
}

fn steps(
    store: &Store,
    prompts: &[Prompt],
    groups: &[Vec<usize>],
    visits: &[Visit],
    background: &HashSet<String>,
    since: i64,
    now: DateTime<Local>,
) -> Result<Vec<Found>> {
    // A prompt is a step only when it is an ask the person repeats.
    let mut asks: HashMap<(&str, i64), String> = HashMap::new();
    for members in groups
        .iter()
        .filter(|members| members.len() >= MIN_STEP_ASKS)
    {
        let label = label(prompts, members);
        for &i in members {
            asks.insert((prompts[i].session.as_str(), prompts[i].ts), label.clone());
        }
    }
    let mut events: Vec<Event> = Vec::new();
    // Notion pages' titles, from the browser, for the pages Claude read.
    let mut page_titles: HashMap<String, String> = HashMap::new();
    for visit in visits {
        let Some((host, path)) = host_and_path(&visit.url) else {
            continue;
        };
        let site = site_name(&host);
        let page = (site == "Notion").then(|| notion_id(&path)).flatten();
        if let (Some(id), Some(title)) = (&page, visit.title.as_deref()) {
            let title = clean_title(title);
            if !title.is_empty() {
                page_titles.insert(id.clone(), title);
            }
        }
        if !host.is_empty() && !background.contains(&host) {
            events.push(Event {
                ts: visit.ts,
                label: format!("open {site}"),
                app: App::Browser,
                subject: site,
                session: None,
                page,
            });
        }
    }
    let mut statement = store.conn().prepare(
        "SELECT ts, kind, session_id, detail FROM activity
         WHERE source = 'claude' AND kind IN ('prompt', 'ship', 'skill', 'notion') AND ts >= ?1",
    )?;
    let rows = statement.query_map([since], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (ts, kind, session, detail) = row?;
        let detail = detail.unwrap_or_default();
        let label = match kind.as_str() {
            "prompt" => session
                .as_deref()
                .and_then(|s| asks.get(&(s, ts)))
                .map(|label| format!("ask Claude \u{201c}{label}\u{201d}")),
            "ship" => Some(ship_step(&detail)),
            "skill" => Some(format!("use the {} skill", one_line(&detail, 40))),
            "notion" => Some(notion_step(&detail).to_owned()),
            _ => None,
        };
        if let Some(label) = label {
            let notion = kind == "notion";
            events.push(Event {
                ts,
                subject: if notion {
                    "Notion".to_owned()
                } else {
                    label.clone()
                },
                label,
                app: App::Claude,
                session,
                page: notion
                    .then(|| detail.split_whitespace().nth(1).and_then(notion_id))
                    .flatten(),
            });
        }
    }
    events.sort_by_key(|e| e.ts);

    // Each working day as a sequence of steps, repeats run together.
    let mut days: BTreeMap<NaiveDate, Vec<Step>> = BTreeMap::new();
    for event in events {
        let Some(day) = day_of(event.ts) else {
            continue;
        };
        let steps = days.entry(day).or_default();
        let step = match steps.last_mut() {
            Some(last) if last.label == event.label => {
                last.end = event.ts;
                last
            }
            _ => {
                steps.push(Step {
                    label: event.label,
                    app: event.app,
                    subject: event.subject,
                    start: event.ts,
                    end: event.ts,
                    session: None,
                    pages: BTreeSet::new(),
                });
                steps.last_mut().expect("just pushed")
            }
        };
        if event.session.is_some() {
            step.session = event.session;
        }
        step.pages.extend(event.page);
    }

    let mut grams: HashMap<Vec<String>, Gram> = HashMap::new();
    for (day, steps) in &days {
        for n in [2, 3] {
            for run in steps.windows(n) {
                let close = run.windows(2).all(|w| w[1].start - w[0].end <= STEP_GAP_MS);
                let labels: Vec<String> = run.iter().map(|s| s.label.clone()).collect();
                let distinct = run.iter().map(|s| &s.subject).collect::<HashSet<_>>().len() == n;
                let both = run.iter().any(|s| s.app == App::Browser)
                    && run.iter().any(|s| s.app == App::Claude);
                if !(close && distinct && both) {
                    continue;
                }
                let gram = grams.entry(labels).or_default();
                gram.count += 1;
                gram.last = gram.last.max(run[0].start);
                for step in run {
                    gram.sessions.extend(step.session.clone());
                    gram.pages.extend(step.pages.iter().cloned());
                }
                if gram.days.insert(*day) {
                    let text = run
                        .iter()
                        .map(|s| format!("{} {}", time::clock(s.start), s.label))
                        .collect::<Vec<_>>()
                        .join(", then ");
                    gram.examples.push((run[0].start, text));
                }
            }
        }
    }
    let recent = now.timestamp_millis() - RECENT_DAYS * DAY_MS;
    let mut candidates: Vec<(Vec<String>, Gram)> = grams
        .into_iter()
        .filter(|(labels, gram)| {
            let needed = if labels.len() == 3 {
                MIN_STEP_DAYS
            } else {
                MIN_PAIR_DAYS
            };
            gram.days.len() >= needed && gram.last >= recent
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.1.days
            .len()
            .cmp(&a.1.days.len())
            .then_with(|| b.0.len().cmp(&a.0.len()))
            .then_with(|| a.0.cmp(&b.0))
    });
    // Longer sequences first among equals; drop ones a kept one already
    // covers (sharing two steps).
    let mut kept: Vec<(Vec<String>, Gram)> = Vec::new();
    for (labels, gram) in candidates {
        let covered = kept
            .iter()
            .any(|(other, _)| labels.iter().filter(|l| other.contains(l)).count() >= 2);
        if !covered {
            kept.push((labels, gram));
        }
        if kept.len() == MAX_STEP_PATTERNS {
            break;
        }
    }
    let mut found = Vec::new();
    for (labels, gram) in kept {
        let starts: Vec<i64> = gram.examples.iter().map(|e| e.0).collect();
        let window = routine(&starts, now).map(|(window, _)| window);
        let mut facts = vec![format!(
            "Each step came within {} minutes of the one before.",
            STEP_GAP_MS / 60_000
        )];
        let sessions: BTreeSet<&str> = gram.sessions.iter().map(String::as_str).collect();
        let projects = session_projects(store, &sessions)?;
        if !projects.is_empty() {
            facts.push(format!("Claude Code projects: {}.", projects.join(", ")));
        }
        let titles = session_titles(store, &sessions)?;
        if !titles.is_empty() {
            facts.push(format!("Session titles: {}.", titles.join("; ")));
        }
        let mut pages: Vec<&str> = gram
            .pages
            .iter()
            .filter_map(|id| page_titles.get(id).map(String::as_str))
            .collect();
        pages.sort_unstable();
        pages.dedup();
        if !pages.is_empty() {
            let pages: Vec<String> = pages
                .iter()
                .take(5)
                .map(|p| format!("\u{201c}{p}\u{201d}"))
                .collect();
            facts.push(format!("Notion pages involved: {}.", pages.join(", ")));
        }
        if let Some(window) = &window {
            facts.push(format!("On weekdays it happened {}.", window.describe()));
        }
        found.push(Found {
            kind: Kind::Steps,
            key: format!("steps:{}", labels.join(" > ")),
            label: labels.join(", then "),
            count: gram.count,
            sessions: 0,
            days: gram.days.iter().copied().collect(),
            examples: spread(&gram.examples)
                .into_iter()
                .map(|(at, text)| Example { at, text })
                .collect(),
            facts,
            readers: Vec::new(),
            window,
            score: 2.0 * gram.days.len() as f64 + labels.len() as f64,
            signature: labels,
        });
    }
    Ok(found)
}

/// The projects of these sessions, the busiest first: "Unifize Monorepo 2
/// (12 sessions)".
fn session_projects(store: &Store, sessions: &BTreeSet<&str>) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare_cached(
        "SELECT p.name FROM session s JOIN project p ON p.id = s.project_id WHERE s.id = ?1",
    )?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for session in sessions {
        let names: Vec<String> = statement
            .query_map([session], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        for name in names {
            *counts.entry(name).or_default() += 1;
        }
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(list
        .into_iter()
        .take(3)
        .map(|(name, n)| format!("{name} ({})", plural(n, "session")))
        .collect())
}

fn site_name(host: &str) -> String {
    match host {
        "app.notion.com" | "notion.so" => "Notion".to_owned(),
        "meet.google.com" => "Google Meet".to_owned(),
        "mail.google.com" => "Gmail".to_owned(),
        "calendar.google.com" => "Google Calendar".to_owned(),
        // Every port is the same thing: the site being built, run locally.
        other if other.starts_with("localhost") || other.starts_with("127.0.0.1") => {
            "the local dev site".to_owned()
        }
        other => other.to_owned(),
    }
}

// Open loops.

fn loops(store: &Store, now: DateTime<Local>) -> Result<Vec<Found>> {
    let since = now.timestamp_millis() - LOOP_DAYS * DAY_MS;
    let mut statement = store.conn().prepare(
        "SELECT c.title, count(*), max(coalesce(m.sent_at, m.first_seen_at))
         FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE c.app = 'fireflies' AND m.sender = ?1 AND m.checked_at IS NOT NULL
           AND coalesce(m.sent_at, m.first_seen_at) >= ?2
           AND NOT EXISTS (SELECT 1 FROM plan p WHERE p.message_id = m.id)
           AND NOT EXISTS (SELECT 1 FROM todo t WHERE t.message_id = m.id)
         GROUP BY c.id ORDER BY 3 DESC",
    )?;
    let meetings: Vec<(String, usize, i64)> = statement
        .query_map(params![ME, since], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as usize,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let total: usize = meetings.iter().map(|m| m.1).sum();
    if total < MIN_LOOP_ITEMS {
        return Ok(Vec::new());
    }
    let days: BTreeSet<NaiveDate> = meetings.iter().filter_map(|m| day_of(m.2)).collect();
    Ok(vec![Found {
        kind: Kind::Loop,
        key: "loop:meeting action items".to_owned(),
        signature: vec!["meeting action items".to_owned()],
        label: "your meeting action items with no date".to_owned(),
        count: total,
        sessions: meetings.len(),
        days: days.into_iter().collect(),
        examples: meetings
            .iter()
            .take(MAX_EXAMPLES)
            .map(|(title, count, at)| Example {
                at: *at,
                text: format!(
                    "\u{201c}{}\u{201d}: {} of yours",
                    one_line(&redact(title), 60),
                    plural(*count, "action item")
                ),
            })
            .collect(),
        facts: vec![
            "Fireflies records these meetings. Their action items for the person have no due \
             date, so none went on the calendar and nothing reminds them."
                .to_owned(),
            "New meetings keep adding action items, so a reminder should cover every meeting \
             of the last 7 days, not only the ones named here."
                .to_owned(),
        ],
        readers: vec![FIREFLIES.to_owned()],
        window: None,
        // Things owed matter more than habits of the same count.
        score: 3.0 * total as f64,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{self, Conversation, NewMessage};
    use crate::store::NewActivity;
    use chrono::{NaiveTime, TimeZone};

    /// Local time on the `weeks_ago`-th week's `weekday` (Monday 0) before
    /// the week of `now`, at `hh:mm`.
    fn at(now: DateTime<Local>, weeks_ago: i64, weekday: i64, hh: u32, mm: u32) -> i64 {
        let monday = now.date_naive()
            - TimeDelta::days(i64::from(now.weekday().num_days_from_monday()))
            - TimeDelta::weeks(weeks_ago)
            + TimeDelta::days(weekday);
        Local
            .from_local_datetime(&monday.and_time(NaiveTime::from_hms_opt(hh, mm, 0).unwrap()))
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    fn now() -> DateTime<Local> {
        // A Wednesday afternoon, so this week has days behind it.
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 10, 14)
                    .unwrap()
                    .and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
            )
            .earliest()
            .unwrap()
    }

    fn add(store: &Store, id: &str, kind: &str, ts: i64, session: &str, detail: &str) {
        store
            .add_activity(&NewActivity {
                source: "claude",
                kind,
                ts,
                session_id: Some(session),
                detail: Some(detail),
                ext_id: id,
                ..Default::default()
            })
            .unwrap();
    }

    fn visit(store: &Store, id: &str, ts: i64, url: &str, title: &str) {
        store
            .add_activity(&NewActivity {
                source: "history",
                kind: "visit",
                ts,
                url: Some(url),
                title: Some(title),
                ext_id: id,
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn openings_keep_the_words_that_say_what_is_asked() {
        assert_eq!(
            opening("Push all changes to GitHub, please"),
            ["push", "change", "github"]
        );
        assert_eq!(
            opening("@\"/Users/x/Downloads/clip.mp4\" go through this product/customer video"),
            ["product", "customer", "video"]
        );
        assert_eq!(opening("remove this"), ["remove"]);
        // A quoted file with spaces in its name says nothing about the ask.
        assert_eq!(
            opening(
                "@\"/Users/x/Screen Recording 2026-09-26 at 7.21.34 PM.mov\" can we make a loop"
            ),
            ["loop"]
        );
        assert_eq!(
            example("@\"/Users/x/Screen Recording 1.mov\" can we make a loop"),
            "[file] can we make a loop"
        );
        assert_eq!(
            opening("um, can you look at https://x.com/a and fix src/app/page.tsx"),
            ["fix"]
        );
    }

    #[test]
    fn similar_asks_match_and_different_ones_do_not() {
        let a = opening("push changes to github");
        assert!(similarity(&a, &opening("push all changes to git")) >= MIN_SIMILARITY);
        assert!(similarity(&a, &opening("Push changes")) >= MIN_SIMILARITY);
        assert_eq!(
            similarity(
                &a,
                &opening("commit and push this and also deploy on vercel")
            ),
            0.0
        );
        assert!(
            similarity(
                &opening("on the platform page, we need a pricing section"),
                &opening("on the platform page, the hero animation is broken")
            ) < MIN_SIMILARITY
        );
    }

    #[test]
    fn finds_an_ask_repeated_across_sessions_and_what_came_of_it() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        let asks = [
            "push changes to github",
            "Push all changes to git",
            "push changes to github",
            "push changes to GitHub please",
        ];
        for (n, text) in asks.iter().enumerate() {
            let ts = at(now, 2 - n as i64 / 2, n as i64 % 2 * 2, 17, 0);
            let session = format!("s{n}");
            add(&store, &format!("p{n}"), "prompt", ts, &session, text);
            add(
                &store,
                &format!("c{n}"),
                "ship",
                ts + 60_000,
                &session,
                "git commit",
            );
            add(
                &store,
                &format!("e{n}"),
                "edit",
                ts + 30_000,
                &session,
                "/r/app/src/components/Hero.tsx",
            );
        }
        // Noise: one-off asks and a one-word one.
        add(
            &store,
            "x1",
            "prompt",
            at(now, 1, 1, 12, 0),
            "x",
            "write the pricing page copy",
        );
        add(
            &store,
            "x2",
            "prompt",
            at(now, 1, 2, 12, 0),
            "y",
            "remove this",
        );

        let found = all(&store, now).unwrap();
        let asks: Vec<&Found> = found.iter().filter(|f| f.kind == Kind::Ask).collect();
        assert_eq!(asks.len(), 1, "{asks:?}");
        let ask = asks[0];
        assert_eq!(ask.label, "push changes to github");
        assert_eq!((ask.count, ask.sessions, ask.days.len()), (4, 4, 4));
        assert_eq!(ask.key, "ask:change github push");
        assert!(
            ask.facts
                .iter()
                .any(|f| f.contains("git commit (4 sessions)")),
            "{:?}",
            ask.facts
        );
        assert!(ask.facts.iter().any(|f| f.contains("src/components (4)")));
    }

    #[test]
    fn a_list_worked_by_hand_in_one_session_counts() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        for n in 0..6 {
            let ts = at(now, 0, 1, 15, 10 * n);
            add(
                &store,
                &format!("p{n}"),
                "prompt",
                ts,
                "s",
                "do the next case study",
            );
        }
        let found = all(&store, now).unwrap();
        let ask = found.iter().find(|f| f.kind == Kind::Ask).unwrap();
        assert_eq!((ask.count, ask.sessions), (6, 1));
        assert!(
            ask.facts
                .iter()
                .any(|f| f.starts_with("Asked 6 times in 1 session")
                    && f.contains("10 minutes apart")),
            "{:?}",
            ask.facts
        );
    }

    #[test]
    fn old_habits_are_not_suggested() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        for n in 0..4 {
            let ts = at(now, 6 + n, 1, 17, 0);
            add(
                &store,
                &format!("p{n}"),
                "prompt",
                ts,
                &format!("s{n}"),
                "push changes to github",
            );
        }
        assert!(all(&store, now).unwrap().is_empty());
    }

    #[test]
    fn finds_a_meeting_and_a_page_at_the_same_time_on_weekdays() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        let mut n = 0;
        for (weeks_ago, weekday) in [(2, 0), (2, 2), (1, 1), (1, 3), (0, 0)] {
            n += 1;
            visit(
                &store,
                &format!("m{n}"),
                at(now, weeks_ago, weekday, 13, 0),
                "https://meet.google.com/amh-vtqd-nuc?authuser=0",
                "Meet – Website Rebuild",
            );
            visit(
                &store,
                &format!("n{n}"),
                at(now, weeks_ago, weekday, 12, 40),
                "https://app.notion.com/p/Website-Tasks-bee201da52b44c1d849647ce57872d9d",
                "(9+) Website Tasks | Notion",
            );
        }
        // Once at another time does not break it.
        visit(
            &store,
            "late",
            at(now, 1, 4, 22, 0),
            "https://meet.google.com/amh-vtqd-nuc",
            "Meet – Website Rebuild",
        );
        let found = all(&store, now).unwrap();
        let slot = found.iter().find(|f| f.kind == Kind::Time).unwrap();
        assert_eq!(
            slot.signature,
            [
                "meet:amh-vtqd-nuc",
                "notion:bee201da52b44c1d849647ce57872d9d"
            ]
        );
        assert_eq!(slot.readers, ["claude.ai Fireflies", "claude.ai Notion"]);
        let window = slot.window.as_ref().unwrap();
        assert_eq!(
            (window.from.as_str(), window.to.as_str()),
            ("12:30", "13:00")
        );
        assert_eq!(window.weekdays, [1, 2, 3, 4]);
        assert!(slot.label.contains("Website Tasks (Notion page)"));
        assert!(
            slot.facts
                .iter()
                .any(|f| f.contains("Link: https://www.notion.so/bee201da52b44c1d849647ce57872d9d"))
        );
        assert!(
            slot.facts
                .iter()
                .any(|f| f.contains("Meeting title: Website Rebuild"))
        );
    }

    #[test]
    fn a_page_opened_at_random_times_is_not_a_routine() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        for (n, (weeks_ago, weekday, hour)) in
            [(2, 0, 9), (2, 2, 13), (1, 1, 17), (1, 3, 21), (0, 0, 11)]
                .into_iter()
                .enumerate()
        {
            visit(
                &store,
                &format!("v{n}"),
                at(now, weeks_ago, weekday, hour, 0),
                "https://app.notion.com/p/bee201da52b44c1d849647ce57872d9d",
                "Website Tasks",
            );
        }
        assert!(
            all(&store, now)
                .unwrap()
                .iter()
                .all(|f| f.kind != Kind::Time)
        );
    }

    #[test]
    fn finds_the_same_steps_across_apps() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        for (n, (weeks_ago, weekday)) in [(2, 0), (2, 3), (1, 1), (1, 4), (0, 1)]
            .into_iter()
            .enumerate()
        {
            let start = at(now, weeks_ago, weekday, 16, 0);
            add(
                &store,
                &format!("c{n}"),
                "ship",
                start,
                &format!("s{n}"),
                "git commit",
            );
            add(
                &store,
                &format!("d{n}"),
                "ship",
                start + 120_000,
                &format!("s{n}"),
                "vercel deploy --prod",
            );
            visit(
                &store,
                &format!("v{n}"),
                start + 300_000,
                "https://www.unifize.com/platform",
                "Platform",
            );
            // Opening Notion and then reading it through Claude is one thing.
            let later = at(now, weeks_ago, weekday, 20, 0);
            visit(
                &store,
                &format!("n{n}"),
                later,
                "https://app.notion.com/p/bee201da52b44c1d849647ce57872d9d",
                "Tasks",
            );
            add(
                &store,
                &format!("r{n}"),
                "notion",
                later + 60_000,
                &format!("s{n}"),
                "notion-fetch bee201da",
            );
        }
        let found = all(&store, now).unwrap();
        let steps: Vec<&Found> = found.iter().filter(|f| f.kind == Kind::Steps).collect();
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert_eq!(
            steps[0].label,
            "git commit, then deploy to Vercel (prod), then open unifize.com"
        );
        assert_eq!(steps[0].days.len(), 5);
        assert!(
            steps[0].examples[0]
                .text
                .contains("16:00 git commit, then 16:02 deploy")
        );
    }

    #[test]
    fn finds_undated_meeting_action_items() {
        let store = Store::open_in_memory().unwrap();
        let now = now();
        let meeting = Conversation {
            app: "fireflies",
            key: "m1",
            title: "Website review",
            work: Some(true),
        };
        let sent = at(now, 0, 0, 15, 30);
        let items = ["Send Ben the deck", "Fix the hero", "Book the venue"];
        let messages: Vec<NewMessage> = items
            .iter()
            .map(|text| NewMessage {
                sender: ME,
                text,
                sent_label: None,
                sent_at: Some(sent),
            })
            .collect();
        conversations::record(&store, &meeting, &messages, sent).unwrap();
        // Not looked at for promises yet: not an open loop yet.
        assert!(loops(&store, now).unwrap().is_empty());
        store
            .conn()
            .execute("UPDATE message SET checked_at = 1", [])
            .unwrap();
        let found = loops(&store, now).unwrap();
        assert_eq!(found[0].count, 3);
        assert_eq!(found[0].readers, ["claude.ai Fireflies"]);
        assert!(
            found[0].examples[0]
                .text
                .contains("3 action items of yours")
        );
    }

    #[test]
    fn reads_page_identities() {
        assert_eq!(
            notion_id("/Website-3-0-32c860e6b45e81118f3ce0e6b459bb7f").as_deref(),
            Some("32c860e6b45e81118f3ce0e6b459bb7f")
        );
        assert_eq!(
            notion_id("/p/bee201da-52b4-4c1d-8496-47ce57872d9d").as_deref(),
            Some("bee201da52b44c1d849647ce57872d9d")
        );
        assert_eq!(notion_id("/settings"), None);
        assert_eq!(
            meeting_name("Meet – Website Rebuild").as_deref(),
            Some("Website Rebuild")
        );
        assert_eq!(meeting_name("Google Meet"), None);
        assert_eq!(clean_title("(9+) Website 3.0 | Notion"), "Website 3.0");
        assert_eq!(
            host_and_path("https://www.Notion.so/a/b?x=1#c"),
            Some(("notion.so".to_owned(), "/a/b".to_owned()))
        );
    }

    #[test]
    fn windows_and_clock_times() {
        assert_eq!(clock(0), "04:00");
        assert_eq!(clock(540), "13:00");
        assert_eq!(clock(1260), "01:00");
        assert_eq!(minutes_after_start("13:00"), 540);
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let visits = [
            (day(5), 530),
            (day(6), 545),
            (day(7), 600),
            (day(8), 900),
            (day(9), 540),
        ];
        let (start, end, days) = best_window(&visits).unwrap();
        assert_eq!((start, end, days.len()), (530, 600, 4));
    }
}
