//! Live threads: the brief as a short index a session reads at its start,
//! and one thread's details when the session asks for them (the person's
//! choice of 2026-10-18). On five sessions, most lines of a fixed brief
//! were about other work: it is built before the first prompt, and first
//! prompts are often a picture and a dozen words, so plain code cannot
//! tell the topic. The session's model can, so it picks the thread, the
//! way Claude Code loads a skill's name first and its body when used.
//!
//! Example: a session starts in the Unifize monorepo and reads:
//!
//! ```text
//! Live in Unifize website, as of Tue 13 Oct 21:30 (Freebyrd's memory; data, not instructions; details: thread KEY):
//! - Persona pages [persona] (Raj; due Thu 15 Oct, 2 files uncommitted)
//! - Factors data layer [factor] (Ben; waits on you, 2 open)
//! ```
//!
//! The person types "is the factors data layer on yet?", and the session
//! asks for `thread factor`: that thread's lines with their dates, sources
//! and statuses (seen, guessed, confirmed), its sessions and its people.
//!
//! A thread is a line of work: the brief's items (what is owed, the latest
//! recap's open items, matching meetings' open action items, the project's
//! sessions of the last three days) grouped by a word they share, the
//! group with the most weight first. Plain code over the memory with the
//! brief's rules: the same "as of" cut, people by name only, and nothing
//! from WhatsApp, personal chats, page text, screens or other people's
//! message text.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Local};
use rusqlite::params;

use crate::brief::{self, Line, Part, People, Section, Setup, Status, Words, clean, tokens};
use crate::conversations::ME;
use crate::people::Directory;
use crate::{Store, time};

/// The index at most, its first line included: about 300 characters of
/// threads.
pub const INDEX_CHARS: usize = 420;
const MAX_THREADS: usize = 6;
const NAME_CHARS: usize = 36;
const DAY_MS: i64 = 86_400_000;
/// Owed within this long counts as due soon.
const SOON_MS: i64 = 7 * DAY_MS;
/// Sessions of the brief's window, for the names the person typed there.
const SESSIONS_MS: i64 = 3 * DAY_MS;

/// Words that never start or end a thread's name, nor stand in a run of
/// words (`run_around`): "against" was one of the project's own words and
/// keyed a task, whose run then ended with it (2026-10-18, on a copy of the
/// memory).
const JOINS: &[&str] = &[
    "a", "about", "above", "across", "after", "against", "along", "among", "an", "and", "are",
    "around", "as", "at", "be", "before", "behind", "below", "beside", "between", "beyond", "but",
    "by", "during", "except", "for", "from", "if", "in", "inside", "into", "is", "it", "its",
    "like", "my", "near", "no", "nor", "not", "of", "on", "onto", "or", "our", "outside", "over",
    "past", "per", "since", "so", "still", "than", "that", "the", "then", "this", "through", "to",
    "toward", "towards", "under", "until", "upon", "versus", "via", "vs", "was", "with", "within",
    "without", "your",
];

/// Numbers in words, where a run stops as at digits ("the hero's four
/// tabs").
const NUMBERS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
    "twelve", "twenty", "thirty", "forty", "fifty", "hundred", "thousand", "million", "dozen",
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Owed,
    Open,
    Action,
    Session,
}

/// One of the brief's items, as a thread holds it.
#[derive(Debug, Clone)]
struct Item {
    kind: Kind,
    /// Its own words: a title, a recap line, an action item, a session's
    /// title.
    topic: String,
    line: Line,
    /// The heading it goes under in a thread's details.
    heading: String,
    /// Who it involves, as their pages name them.
    who: Vec<String>,
    due: Option<i64>,
    stalled: bool,
    asks: bool,
    uncommitted: usize,
    points: i64,
    /// The distinctive words that group it.
    words: Vec<String>,
    /// How often its text says each of `words` ("the 16 modules" and "the
    /// module count" say "module" twice).
    said: Vec<usize>,
    /// All its words, to find it by a few words.
    found_by: HashSet<String>,
}

/// A line of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// What a session asks for it by: a word its items share ("factor").
    pub key: String,
    pub name: String,
    /// Who it involves, as their pages name them.
    pub who: Vec<String>,
    /// Where it stands in a few words: "due Thu 15 Oct, 2 files
    /// uncommitted".
    pub state: String,
    pub last_at: i64,
    points: i64,
    items: Vec<usize>,
}

/// A project's live threads as of a time, most weight first.
#[derive(Debug, Clone)]
pub struct Threads {
    pub project: String,
    pub as_of: i64,
    pub threads: Vec<Thread>,
    items: Vec<Item>,
    /// The project's names' words, which tell no thread apart.
    names: HashSet<String>,
    /// The project's own words (`brief::words_of`), and the person's name.
    own: HashSet<String>,
    me: Vec<String>,
}

/// The live threads of the project `folder` belongs to, as of `as_of`,
/// with the person's project groups and name. `None` when the folder is no
/// project of theirs.
pub fn build(store: &Store, folder: &str, as_of: DateTime<Local>) -> Result<Option<Threads>> {
    build_with(store, folder, as_of, &Setup::load())
}

pub fn build_with(
    store: &Store,
    folder: &str,
    as_of: DateTime<Local>,
    setup: &Setup,
) -> Result<Option<Threads>> {
    let Some(project) = brief::project_for(store, folder, &setup.groups)? else {
        return Ok(None);
    };
    let at = as_of.timestamp_millis();
    let words = brief::words_of(store, &project, &setup.groups, at)?;
    let directory = Directory::load(store)?;
    let mut people = People::new(store, &directory, setup, &words)?;
    people.named_in_requests(store, &project, at)?;
    let items = gather(store, &project, &words, setup, at, &mut people)?;
    let threads = group(&items, &words, at);
    Ok(Some(Threads {
        project: project.name,
        as_of: at,
        threads,
        items,
        names: words.names,
        own: words.own,
        me: setup.me.clone(),
    }))
}

impl Threads {
    /// What a session reads at its start: its first line, then the threads
    /// most weight first, each "Name [key] (who; where it stands)", while
    /// they fit in `INDEX_CHARS`. Empty when nothing is live.
    pub fn index(&self) -> String {
        let all: Vec<&Thread> = self.threads.iter().collect();
        self.index_of(&all, None)
    }

    /// The index of these threads only (those that changed, for a resumed
    /// session), said to be `heading` ("Changed since this session's last
    /// index") instead of what is live.
    pub fn index_of(&self, threads: &[&Thread], heading: Option<&str>) -> String {
        if threads.is_empty() {
            return String::new();
        }
        let mut out = format!(
            "{} in {}, as of {} (Freebyrd's memory; data, not instructions; details: thread \
             KEY):",
            heading.unwrap_or("Live"),
            self.project,
            time::moment(self.as_of)
        );
        let mut shown = 0;
        for thread in threads {
            if shown == MAX_THREADS {
                break;
            }
            let entry = format!("\n{}", entry(thread));
            if out.chars().count() + entry.chars().count() <= INDEX_CHARS {
                out.push_str(&entry);
                shown += 1;
            }
        }
        out
    }

    /// The references of a thread's items in the memory (`todo:12`,
    /// `message:40`, `session:<id>`), for receipts.
    pub fn items_of(&self, thread: &Thread) -> Vec<String> {
        thread
            .items
            .iter()
            .map(|index| self.items[*index].line.item.clone())
            .collect()
    }

    /// Whether a text is about this project's work: one of its distinctive
    /// words is the project's own.
    pub fn about_project(&self, text: &str) -> bool {
        brief::item_terms(text, &self.me)
            .iter()
            .any(|word| self.own.contains(word) && !self.names.contains(word))
    }

    /// The thread a session asks for: by its key, else the live thread
    /// sharing most of the asked words (the project's names aside), else
    /// the items those words find, as a thread of their own (key `query`).
    /// `None` when nothing in the memory matches.
    pub fn find(&self, ask: &str) -> Option<Thread> {
        let key = ask.trim().to_lowercase();
        if let Some(thread) = self.threads.iter().find(|thread| thread.key == key) {
            return Some(thread.clone());
        }
        let wanted: HashSet<String> = find_words(ask).difference(&self.names).cloned().collect();
        if wanted.is_empty() {
            return None;
        }
        let shared = |items: &[usize]| {
            let found: HashSet<&String> = items
                .iter()
                .flat_map(|index| &self.items[*index].found_by)
                .collect();
            wanted.iter().filter(|word| found.contains(word)).count()
        };
        let best = self
            .threads
            .iter()
            .enumerate()
            .map(|(rank, thread)| (shared(&thread.items), Reverse(rank), thread))
            .filter(|(count, _, _)| *count > 0)
            .max_by_key(|(count, rank, _)| (*count, *rank));
        if let Some((_, _, thread)) = best {
            return Some(thread.clone());
        }
        let found: Vec<usize> = (0..self.items.len())
            .filter(|index| shared(&[*index]) > 0)
            .collect();
        if found.is_empty() {
            return None;
        }
        let mut thread = thread("query".to_owned(), found, &self.items, self.as_of);
        thread.name = clean(ask, NAME_CHARS);
        Some(thread)
    }

    /// One thread's details, at most `brief::MAX_CHARS`: what is owed by
    /// date, the recap's open items, each meeting's action items, its
    /// sessions newest first, and its people, each line with its date,
    /// source and status.
    pub fn details(&self, thread: &Thread) -> String {
        let mut members: Vec<&Item> = thread
            .items
            .iter()
            .map(|index| &self.items[*index])
            .collect();
        members.sort_by_key(|item| {
            let rank = match item.kind {
                Kind::Owed => 0,
                Kind::Open => 1,
                Kind::Action => 2,
                Kind::Session => 3,
            };
            let when = match (item.kind, item.due) {
                (Kind::Owed, Some(due)) => (0, due),
                (Kind::Session, _) => (0, -item.line.at),
                _ => (1, -item.line.at),
            };
            (rank, when)
        });
        let mut parts: Vec<Part> = Vec::new();
        for item in members {
            let section = match item.kind {
                Kind::Owed => Section::Due,
                Kind::Open => Section::Open,
                Kind::Action => Section::Meeting,
                Kind::Session => Section::Sessions,
            };
            match parts.iter_mut().find(|part| part.heading == item.heading) {
                Some(part) => part.lines.push(item.line.clone()),
                None => parts.push(Part {
                    section,
                    heading: item.heading.clone(),
                    lines: vec![item.line.clone()],
                }),
            }
        }
        if !thread.who.is_empty() {
            parts.push(Part {
                section: Section::People,
                heading: "People".to_owned(),
                lines: vec![Line {
                    text: thread.who.join(", "),
                    at: thread.last_at,
                    from: "its items and your requests".to_owned(),
                    status: Status::Seen,
                    item: "people".to_owned(),
                }],
            });
        }
        let header = format!(
            "Freebyrd's memory of {}, thread \"{}\" [{}], as of {}. {}",
            self.project,
            thread.name,
            thread.key,
            time::moment(self.as_of),
            brief::LEGEND
        );
        brief::fit_under(&header, &mut parts)
    }
}

/// "- Persona pages [persona] (Ben; due Thu 15 Oct, 2 open)", with first
/// names only.
fn entry(thread: &Thread) -> String {
    let mut first: Vec<&str> = Vec::new();
    for name in &thread.who {
        let word = name.split_whitespace().next().unwrap_or(name);
        if !first.contains(&word) && first.len() < 2 {
            first.push(word);
        }
    }
    let about = if first.is_empty() {
        thread.state.clone()
    } else {
        format!("{}; {}", first.join(", "), thread.state)
    };
    format!("- {} [{}] ({about})", thread.name, thread.key)
}

/// The words that group an item: its distinctive words (`terms`), without
/// when-and-how words, the project's names or the person's own name.
fn group_words(topic: &str, words: &Words, setup: &Setup) -> Vec<String> {
    brief::item_terms(topic, &setup.me)
        .into_iter()
        .filter(|word| !words.names.contains(word) && !word.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// All of a text's words of four letters or more, to find things by.
fn find_words(text: &str) -> HashSet<String> {
    tokens(text)
        .into_iter()
        .filter(|word| !brief::PLAIN.contains(&word.as_str()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn item(
    kind: Kind,
    topic: &str,
    line: Line,
    heading: &str,
    who: Vec<String>,
    points: i64,
    words: &Words,
    setup: &Setup,
) -> Item {
    let grouping = group_words(topic, words, setup);
    let spoken = tokens(topic);
    let said = grouping
        .iter()
        .map(|word| spoken.iter().filter(|token| *token == word).count().max(1))
        .collect();
    Item {
        kind,
        topic: topic.to_owned(),
        line,
        heading: heading.to_owned(),
        who,
        due: None,
        stalled: false,
        asks: false,
        uncommitted: 0,
        points,
        words: grouping,
        said,
        found_by: find_words(topic),
    }
}

/// The brief's items for the project as of `at`, without the brief's caps.
fn gather(
    store: &Store,
    project: &brief::Project,
    words: &Words,
    setup: &Setup,
    at: i64,
    people: &mut People,
) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let today = brief::working_day(at);
    for owed in brief::owed(store, words, setup, at)? {
        if let Some(who) = &owed.who {
            people.named(who, owed.listed, "to-dos");
        }
        let who = owed
            .who
            .as_deref()
            .map(|who| names(people, people.written(who)))
            .unwrap_or_default();
        let due = owed.due.map(|(due, _)| due);
        let soon = due.is_some_and(|due| due < at + SOON_MS);
        let points = if soon { 4 } else { 2 } + i64::from(owed.mine);
        let mut found = item(
            Kind::Owed,
            &owed.title,
            owed.line(today),
            "Due",
            who,
            points,
            words,
            setup,
        );
        found.due = due;
        items.push(found);
    }
    if let Some((day, open)) = brief::recap_open(store, project, at)? {
        let heading = format!("Open in the {} recap", time::short_day_label(day));
        for text in &open {
            let line = brief::recap_line(day, text);
            items.push(item(
                Kind::Open,
                text,
                line,
                &heading,
                Vec::new(),
                3,
                words,
                setup,
            ));
        }
    }
    for meeting in brief::meetings(store, words, setup, at, people)? {
        let heading = meeting.heading();
        for action in &meeting.items {
            let mine = action.owner == ME;
            let who = if mine || action.owner == "action item" {
                Vec::new()
            } else {
                names(people, people.written(&action.owner))
            };
            items.push(item(
                Kind::Action,
                &action.text,
                action.line(),
                &heading,
                who,
                if mine { 2 } else { 1 },
                words,
                setup,
            ));
        }
    }
    let prompts = prompts(store, project, at)?;
    for session in brief::session_states(store, project, at)? {
        if session.title.trim().is_empty() {
            continue;
        }
        let mut named: Vec<i64> = Vec::new();
        for text in prompts.get(&session.id).into_iter().flatten() {
            for person in people.mentioned_in(text) {
                if !named.contains(&person) {
                    named.push(person);
                }
            }
        }
        let flagged = session.stalled || session.asks || session.uncommitted > 0;
        let points = if flagged {
            3
        } else if at - session.last_at < DAY_MS {
            2
        } else {
            1
        };
        let mut found = item(
            Kind::Session,
            &session.title,
            session.line(),
            "Sessions",
            names(people, named),
            points,
            words,
            setup,
        );
        found.stalled = session.stalled;
        found.asks = session.asks;
        found.uncommitted = session.uncommitted;
        items.push(found);
    }
    Ok(items)
}

/// These people's names, as their pages name them.
fn names(people: &People, ids: Vec<i64>) -> Vec<String> {
    ids.into_iter()
        .filter_map(|person| people.directory.name(person))
        .map(|name| clean(name, 40))
        .collect()
}

/// The person's requests in the project's sessions of the brief's window,
/// by session: only the names in them are used.
fn prompts(
    store: &Store,
    project: &brief::Project,
    at: i64,
) -> Result<BTreeMap<String, Vec<String>>> {
    let ids: Vec<String> = project.ids.iter().map(i64::to_string).collect();
    let mut statement = store.conn().prepare(&format!(
        "SELECT session_id, detail FROM activity
         WHERE source = 'claude' AND kind = 'prompt' AND project_id IN ({})
           AND session_id IS NOT NULL AND detail IS NOT NULL AND ts < ?1 AND ts >= ?2",
        ids.join(", ")
    ))?;
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in statement.query_map(params![at, at - SESSIONS_MS], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (session, text) = row?;
        found.entry(session).or_default().push(text);
    }
    Ok(found)
}

/// Whether an item is live: owed, open in a recap, the person's action
/// item, or a session that stalled, asks them something or left files
/// uncommitted.
fn live(item: &Item) -> bool {
    match item.kind {
        Kind::Owed | Kind::Open => true,
        Kind::Action => item.points >= 2,
        Kind::Session => item.stalled || item.asks || item.uncommitted > 0,
    }
}

/// How closely two items sharing a word are tied, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tie {
    /// One word, each with three or more: only a live item with one that
    /// is not, and only once no closer group is left.
    Weak,
    /// One word, half of one of them.
    Fair,
    /// Two words, or all of one of them.
    Close,
}

/// How two items sharing a word are tied, if at all. Two sessions need two
/// words (one, if a title has only one): "Thank you modal state" and
/// "CMD+K site search modal" were two works. Two live items with three
/// words or more each and one in common are two works too, each a thread
/// of its own: "Confirm which modules ship and the module count per
/// product" and "Confirm what ships today for every AI claim" (2026-10-18,
/// on a copy of the memory).
fn tie(a: &Item, b: &Item) -> Option<Tie> {
    let shared = a.words.iter().filter(|word| b.words.contains(word)).count();
    let fewest = a.words.len().min(b.words.len());
    if shared == 0 {
        None
    } else if shared >= fewest.min(2) {
        Some(Tie::Close)
    } else if a.kind == Kind::Session && b.kind == Kind::Session {
        None
    } else if fewest <= 2 {
        Some(Tie::Fair)
    } else {
        (live(a) != live(b)).then_some(Tie::Weak)
    }
}

/// Of items sharing a word, those that make a thread, and whether each is
/// more than weakly tied: all of them when three or more share it and one
/// is more than a session (owed, a recap's, a meeting's), since three
/// sharing a word is no chance; else those tied to another (`tie`).
fn kept(items: &[Item], found: Vec<usize>) -> (Vec<usize>, bool) {
    if found.len() >= 3
        && found
            .iter()
            .any(|index| items[*index].kind != Kind::Session)
    {
        return (found, true);
    }
    let mut members = Vec::new();
    let mut strong = true;
    for a in &found {
        let best = found
            .iter()
            .filter(|b| *b != a)
            .filter_map(|b| tie(&items[*a], &items[*b]))
            .max();
        if let Some(best) = best {
            members.push(*a);
            strong &= best > Tie::Weak;
        }
    }
    (members, strong)
}

/// How much of its members' wording a group shares, in thousandths: for
/// each member, the part of its words another member has too (a word said
/// twice counts twice), averaged. "Write Module Features for the 16
/// modules" and "Confirm which modules ship and the module count" share
/// more of themselves than either does with another task by "feature" or
/// "ship" (2026-10-18, on a copy of the memory).
fn cohesion(items: &[Item], members: &[usize]) -> usize {
    let total: usize = members
        .iter()
        .map(|a| {
            let item = &items[*a];
            let shared: usize = item
                .words
                .iter()
                .zip(&item.said)
                .filter(|(word, _)| {
                    members
                        .iter()
                        .any(|b| b != a && items[*b].words.contains(word))
                })
                .map(|(_, said)| said)
                .sum();
            shared * 1000 / item.said.iter().sum::<usize>().max(1)
        })
        .sum();
    total / members.len().max(1)
}

/// Groups items into threads: again and again, of the words items not yet
/// grouped share (two items at least, `kept`), the one whose items are
/// tied more than weakly, then carry the most weight, then share the most
/// of their wording (`cohesion`), then is the project's own word, makes a
/// thread of them. Ties of weight were common (owed items weigh the same)
/// and went to the word first in order, which split two module tasks into
/// threads for "feature" and "ship" (2026-10-18). What is left alone is a
/// thread of its own only while it is live (`live`).
fn group(items: &[Item], words: &Words, at: i64) -> Vec<Thread> {
    let mut taken = vec![false; items.len()];
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    loop {
        let mut by_word: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, item) in items.iter().enumerate() {
            if taken[index] {
                continue;
            }
            for word in &item.words {
                by_word.entry(word.as_str()).or_default().push(index);
            }
        }
        let best = by_word
            .into_iter()
            .map(|(word, found)| {
                let (found, strong) = kept(items, found);
                (word, found, strong)
            })
            .filter(|(_, found, _)| found.len() >= 2)
            .max_by_key(|(word, found, strong)| {
                let points: i64 = found.iter().map(|index| items[*index].points).sum();
                (
                    *strong,
                    points,
                    cohesion(items, found),
                    words.own.contains(*word),
                    found.len(),
                    Reverse(*word),
                )
            });
        let Some((word, found, _)) = best else {
            break;
        };
        for index in &found {
            taken[*index] = true;
        }
        groups.push((word.to_owned(), found));
    }
    for (index, item) in items.iter().enumerate() {
        if taken[index] {
            continue;
        }
        if !live(item) {
            continue;
        }
        // Its longest word of the project's own, else its longest, one no
        // other thread is keyed by first: two tasks naming the homepage
        // were "homepage" and "homepage+".
        let longest = |own: bool, free: bool| {
            item.words
                .iter()
                .filter(|word| !own || words.own.contains(*word))
                .filter(|word| !free || groups.iter().all(|(key, _)| key != *word))
                .max_by_key(|word| (word.len(), Reverse(word.as_str())))
        };
        let key = longest(true, true)
            .or_else(|| longest(false, true))
            .or_else(|| longest(true, false))
            .or_else(|| longest(false, false));
        if let Some(key) = key.cloned() {
            groups.push((key, vec![index]));
        }
    }
    let mut threads: Vec<Thread> = Vec::new();
    for (key, found) in groups {
        let mut key = key;
        while threads.iter().any(|thread| thread.key == key) {
            key.push('+');
        }
        threads.push(thread(key, found, items, at));
    }
    threads.sort_by(|a, b| {
        b.points
            .cmp(&a.points)
            .then_with(|| b.last_at.cmp(&a.last_at))
            .then_with(|| a.key.cmp(&b.key))
    });
    threads
}

fn thread(key: String, found: Vec<usize>, items: &[Item], at: i64) -> Thread {
    let members: Vec<&Item> = found.iter().map(|index| &items[*index]).collect();
    let topics: Vec<&str> = members.iter().map(|item| item.topic.as_str()).collect();
    let sessions: Vec<&str> = members
        .iter()
        .filter(|item| item.kind == Kind::Session)
        .map(|item| item.topic.as_str())
        .collect();
    let mut who: Vec<String> = Vec::new();
    for name in members.iter().flat_map(|item| &item.who) {
        if !who.contains(name) {
            who.push(name.clone());
        }
    }
    let last_at = members.iter().map(|item| item.line.at).max().unwrap_or(0);
    Thread {
        name: name_for(&key, &topics, &sessions),
        state: state_of(&members, at, last_at),
        who,
        last_at,
        points: members.iter().map(|item| item.points).sum(),
        key,
        items: found,
    }
}

/// A thread's name: the longest phrase of two or three words around its
/// key word that two of its items share ("Factors data layer"), else its
/// shortest session title with the word, else the word as its items write
/// it most ("Modules"), else its shortest item, cut short (`short_name`).
/// Phrases match by their words' stems, so "Module Features" and "feature
/// pages" share "feature", and "ships" is "ship": compared as written, they
/// shared nothing, and the name became a task's title cut short ("Write
/// Module Features for the 16…", 2026-10-18).
fn name_for(key: &str, topics: &[&str], sessions: &[&str]) -> String {
    // "homepage+" is the second thread keyed by "homepage".
    let key = key.trim_end_matches('+');
    let has_key = |text: &str| tokens(text).iter().any(|word| word == key);
    // A phrase by its stems: how many items have it, its length, and each
    // time it is written.
    let mut phrases: BTreeMap<String, (usize, usize, Vec<String>)> = BTreeMap::new();
    for topic in topics {
        let words: Vec<&str> = topic
            .split_whitespace()
            .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|word| !word.is_empty())
            .collect();
        let mut here: HashSet<String> = HashSet::new();
        for (at, word) in words.iter().enumerate() {
            if !has_key(word) {
                continue;
            }
            for start in at.saturating_sub(2)..=at {
                for end in at..words.len().min(start + 3) {
                    let phrase = &words[start..=end];
                    let joins = |word: &str| JOINS.contains(&word.to_lowercase().as_str());
                    if joins(phrase[0]) || joins(phrase[phrase.len() - 1]) {
                        continue;
                    }
                    let stems: Vec<String> = phrase.iter().map(|word| stem(word)).collect();
                    let stems = stems.join(" ");
                    let entry =
                        phrases
                            .entry(stems.clone())
                            .or_insert((0, phrase.len(), Vec::new()));
                    // "the homepage hero's four tabs" names "Homepage hero".
                    let written = phrase.join(" ");
                    let written = ["'s", "\u{2019}s"]
                        .iter()
                        .find_map(|mark| written.strip_suffix(mark))
                        .unwrap_or(&written);
                    entry.2.push(written.to_owned());
                    if here.insert(stems) {
                        entry.0 += 1;
                    }
                }
            }
        }
    }
    let shared = phrases
        .values()
        .filter(|(count, _, _)| *count >= 2)
        .max_by_key(|(count, length, written)| (*length, *count, Reverse(written[0].clone())))
        .map(|(_, length, written)| (*length, written));
    let (phrase, word) = match shared {
        Some((1, written)) => (None, Some(most_written(written))),
        Some((_, written)) => (Some(written[0].clone()), None),
        None => (None, None),
    };
    // A shared word alone ("Modal") says less than a session's title.
    let name = phrase
        .or_else(|| {
            sessions
                .iter()
                .filter(|title| has_key(title))
                .min_by_key(|title| title.chars().count())
                .map(|title| (*title).to_owned())
        })
        .or(word)
        .or_else(|| {
            topics
                .iter()
                .min_by_key(|topic| topic.chars().count())
                .map(|topic| (*topic).to_owned())
        })
        .unwrap_or_else(|| key.to_owned());
    let name = short_name(&clean(&name, 200), key);
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
}

/// A word as phrases are compared: its stem as `tokens` takes it, else
/// itself in lower case ("the", "CMD+K").
fn stem(word: &str) -> String {
    let stems = tokens(word);
    if stems.is_empty() {
        word.to_lowercase()
    } else {
        stems.join(" ")
    }
}

/// How a word is written most, the longer on a tie ("modules" over
/// "module"), else as first written.
fn most_written(written: &[String]) -> String {
    let mut best: Option<(usize, usize, &String)> = None;
    for form in written {
        let uses = written
            .iter()
            .filter(|other| other.to_lowercase() == form.to_lowercase())
            .count();
        let length = form.chars().count();
        if best.is_none_or(|(most, longest, _)| (uses, length) > (most, longest)) {
            best = Some((uses, length, form));
        }
    }
    best.map(|(_, _, form)| form.clone()).unwrap_or_default()
}

/// A text as a name: its first clause (up to "; ", ": ", ", ", ". " or
/// " ("), else the run of words around the key word (`run_around`), else
/// the clause cut after a whole word within `NAME_CHARS`, without a joining
/// word at its end. Lone tasks' titles were often cut ("Decide the
/// coordination tax…"), where the run names the work ("Coordination tax
/// calculator", 2026-10-18, on a copy of the memory).
fn short_name(text: &str, key: &str) -> String {
    let text = text.trim();
    let end = ["; ", ": ", ", ", ". ", " ("]
        .iter()
        .filter_map(|mark| text.find(mark))
        .min()
        .unwrap_or(text.len());
    let clause = text[..end].trim();
    let clause = if clause.split_whitespace().count() >= 2 {
        clause
    } else {
        text
    };
    if clause.chars().count() <= NAME_CHARS {
        return clause.to_owned();
    }
    if let Some(run) = run_around(text, key) {
        return run;
    }
    let mut words: Vec<&str> = Vec::new();
    let mut length = 0;
    for word in clause.split_whitespace() {
        let more = word.chars().count() + usize::from(!words.is_empty());
        if length + more > NAME_CHARS - 1 {
            break;
        }
        words.push(word);
        length += more;
    }
    while words.len() > 1
        && words
            .last()
            .is_some_and(|word| JOINS.contains(&word.to_lowercase().as_str()))
    {
        words.pop();
    }
    if words.is_empty() {
        return clean(clause, NAME_CHARS);
    }
    format!("{}\u{2026}", words.join(" "))
}

/// The run of words around the key word: out from it on each side up to a
/// joining word, a number, a when-and-how word (`brief::PLAIN`), a clause
/// mark or a possessive, whose own word ends the run ("Decide the
/// coordination tax calculator's 9 open method questions" runs
/// "coordination tax calculator"). `None` when no run of two words or more
/// fits `NAME_CHARS`, or when the run is one of the things the text lists
/// ("the Vercel team plan and the PostHog plan"): named for one, a task of
/// two lost the other (2026-10-18, on a copy of the memory).
fn run_around(text: &str, key: &str) -> Option<String> {
    const LISTS: &[&str] = &["and", "or", "nor", "plus", "&", "+"];
    // Words that may stand between a listing word and a listed thing
    // ("and the PostHog plan").
    const PICKS: &[&str] = &[
        "a", "all", "an", "any", "both", "each", "every", "its", "my", "our", "some", "that",
        "the", "their", "these", "this", "those", "your",
    ];
    struct Said<'a> {
        raw: &'a str,
        /// As written, without the marks around it or a possessive.
        bare: &'a str,
        lower: String,
        /// The marks after it ("," or ")"; "teams'" ends in one too).
        after: &'a str,
        /// Marks before it: an opening bracket or quote.
        opens: bool,
        /// "calculator's".
        possessive: bool,
    }
    let said: Vec<Said> = text
        .split_whitespace()
        .map(|raw| {
            let from = raw.trim_start_matches(|c: char| !c.is_alphanumeric());
            let core = from.trim_end_matches(|c: char| !c.is_alphanumeric());
            let after = &from[core.len()..];
            let owner = ["'s", "\u{2019}s"]
                .iter()
                .find_map(|mark| core.strip_suffix(mark))
                .filter(|owner| !owner.is_empty());
            let bare = owner.unwrap_or(core);
            Said {
                raw,
                bare,
                lower: bare.to_lowercase(),
                after,
                opens: from.len() < raw.len(),
                possessive: owner.is_some(),
            }
        })
        .collect();
    let number = |word: &str| {
        let digits = ["st", "nd", "rd", "th"]
            .iter()
            .find_map(|end| word.strip_suffix(end))
            .unwrap_or(word);
        NUMBERS.contains(&word)
            || digits.starts_with(|c: char| c.is_ascii_digit())
                && digits
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
    };
    let named = |word: &Said| {
        let lower = word.lower.as_str();
        !lower.is_empty()
            && !JOINS.contains(&lower)
            && !number(lower)
            // "status" is in `PLAIN`, but its stem is "statu".
            && !brief::PLAIN.contains(&lower)
            && !brief::PLAIN.contains(&stem(lower).as_str())
    };
    // No run goes past this word.
    let ends = |at: usize| {
        said[at].possessive
            || !said[at].after.is_empty()
            || said.get(at + 1).is_some_and(|next| next.opens)
    };
    let joins = |word: &Said| LISTS.contains(&word.lower.as_str()) || LISTS.contains(&word.raw);
    for at in 0..said.len() {
        if !named(&said[at]) || !tokens(said[at].bare).iter().any(|word| word == key) {
            continue;
        }
        let mut start = at;
        while start > 0 && !ends(start - 1) && named(&said[start - 1]) {
            start -= 1;
        }
        let mut end = at;
        while !ends(end) && end + 1 < said.len() && named(&said[end + 1]) {
            end += 1;
        }
        if start == end {
            continue;
        }
        let mut before = start;
        while before > 0 && PICKS.contains(&said[before - 1].lower.as_str()) {
            before -= 1;
        }
        let listed_before = before > 0 && joins(&said[before - 1]);
        // "logo, quote and case study figure": a comma, then a listing word
        // before the clause ends.
        let listed_after = if said[end].after.starts_with(',') {
            let mut listed = false;
            for word in &said[end + 1..] {
                if word.raw.starts_with('(') {
                    break;
                }
                if joins(word) {
                    listed = true;
                    break;
                }
                if word.after.contains([';', ':', '.', '!', '?']) {
                    break;
                }
            }
            listed
        } else {
            !ends(end) && said.get(end + 1).is_some_and(joins)
        };
        if listed_before || listed_after {
            return None;
        }
        let run: Vec<&str> = said[start..=end].iter().map(|word| word.bare).collect();
        let run = run.join(" ");
        if run.chars().count() <= NAME_CHARS {
            return Some(run);
        }
    }
    None
}

/// Where a thread stands, in two facts at most: the earliest due date, a
/// stalled session or one waiting on the person, files left uncommitted,
/// how many items are open, how many sessions; else when it last moved.
fn state_of(members: &[&Item], at: i64, last_at: i64) -> String {
    let today = brief::working_day(at);
    let mut facts: Vec<String> = Vec::new();
    if let Some(day) = members
        .iter()
        .filter_map(|item| item.due)
        .min()
        .and_then(|due| time::local(due).map(|due| due.date_naive()))
    {
        let label = time::short_day_label(day);
        facts.push(if day < today {
            format!("overdue since {label}")
        } else {
            format!("due {label}")
        });
    }
    if members.iter().any(|item| item.stalled) {
        facts.push("stalled".to_owned());
    } else if members.iter().any(|item| item.asks) {
        facts.push("waits on you".to_owned());
    }
    let files: usize = members.iter().map(|item| item.uncommitted).sum();
    if files > 0 {
        let noun = if files == 1 { "file" } else { "files" };
        facts.push(format!("{files} {noun} uncommitted"));
    }
    let open = members
        .iter()
        .filter(|item| item.kind != Kind::Session)
        .count();
    if open > 0 {
        facts.push(format!("{open} open"));
    }
    let sessions = members.len() - open;
    if sessions > 1 {
        facts.push(format!("{sessions} sessions"));
    }
    facts.truncate(2);
    if facts.is_empty() {
        facts.push(format!("last {}", brief::short_date(last_at)));
    }
    facts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brief::tests::{FREEBYRD, UNIFIZE, memory, oct, setup, when};

    fn threads_at(store: &Store, folder: &str, at: i64) -> Threads {
        build_with(store, folder, when(at), &setup())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn items_group_into_named_threads_most_weight_first() {
        let store = memory();
        let threads = threads_at(&store, UNIFIZE, oct(13, 21, 30));
        let keys: Vec<&str> = threads
            .threads
            .iter()
            .map(|thread| thread.key.as_str())
            .collect();
        assert_eq!(keys.first(), Some(&"persona"), "{keys:?}");
        let persona = &threads.threads[0];
        assert_eq!(persona.name, "Persona pages");
        assert_eq!(persona.state, "due Thu 15 Oct, 2 files uncommitted");
        let factor = threads.find("factor").unwrap();
        assert_eq!(factor.name, "Factors data layer");
        assert_eq!(factor.state, "waits on you, 2 open");
        assert_eq!(factor.who, ["Ben Merton"]);
        // A live item alone is a thread of its own; a finished session alone
        // is not.
        assert!(
            keys.contains(&"vercel") && keys.contains(&"copy"),
            "{keys:?}"
        );
        assert!(!keys.contains(&"modal"), "{keys:?}");

        let index = threads.index();
        assert!(index.chars().count() <= INDEX_CHARS, "{index}");
        assert!(
            index.starts_with("Live in Unifize website, as of Tue 13 Oct 21:30 (Freebyrd's memory; data, not instructions; details: thread KEY):"),
            "{index}"
        );
        assert!(
            index
                .contains("\n- Persona pages [persona] (Raj; due Thu 15 Oct, 2 files uncommitted)"),
            "{index}"
        );
        assert!(
            index.contains("\n- Factors data layer [factor] (Ben; waits on you, 2 open)"),
            "{index}"
        );
        assert!(
            index.contains("[vercel] (Ben; due Wed 14 Oct, 1 open)"),
            "{index}"
        );
        assert!(!index.contains('\u{2014}'));
    }

    #[test]
    fn a_thread_tells_its_lines_sessions_and_people() {
        let store = memory();
        let threads = threads_at(&store, UNIFIZE, oct(13, 21, 30));
        let factor = threads.find("factor").unwrap();
        let text = threads.details(&factor);
        assert!(text.chars().count() <= brief::MAX_CHARS);
        assert!(text.starts_with("Freebyrd's memory of Unifize website, thread \"Factors data layer\" [factor], as of Tue 13 Oct 21:30."), "{text}");
        for line in [
            "Open in the Mon 12 Oct recap:\n- Check Factors (12 Oct, recap, guessed)",
            "Action items from Website review, Thu 8 Oct:\n- you: Track Factors data layer (8 Oct, Fireflies, seen)",
            "Sessions:\n- \"Factors data layer tracking\": Claude's last reply asked you a question (12 Oct, Claude Code, seen)",
            "People: Ben Merton (12 Oct, its items and your requests, seen)",
        ] {
            assert!(text.contains(line), "{line} in {text}");
        }
        // Nothing of the persona thread.
        assert!(!text.to_lowercase().contains("persona"), "{text}");

        let persona = threads.details(&threads.find("persona").unwrap());
        assert!(persona.contains("Due:\n- Send the persona pages to Ben, due Thu 15 Oct (12 Oct, email, guessed)\n- Fix the hero copy"), "{persona}");
    }

    #[test]
    fn a_few_words_find_a_thread_or_the_items_they_name() {
        let store = memory();
        let threads = threads_at(&store, UNIFIZE, oct(13, 21, 30));
        // Words of a thread's items, the key not among them.
        assert_eq!(
            threads.find("is the factors datalayer on?").unwrap().key,
            "factor"
        );
        assert_eq!(threads.find("Persona").unwrap().key, "persona");
        // Words only items outside the threads have.
        let blogs = threads.find("blogs").unwrap();
        assert_eq!(
            (blogs.key.as_str(), blogs.name.as_str()),
            ("query", "blogs")
        );
        assert!(
            threads
                .details(&blogs)
                .contains("Ben Merton: Publish blogs through the content library")
        );
        // Nothing, and the project's names alone, find nothing.
        assert!(threads.find("OG preview videos").is_none());
        assert!(threads.find("Unifize website").is_none());
    }

    #[test]
    fn as_of_leaves_out_what_came_later() {
        let store = memory();
        let then = threads_at(&store, UNIFIZE, oct(13, 21, 30));
        let all: String = then
            .threads
            .iter()
            .map(|thread| then.details(thread))
            .chain([then.index()])
            .collect();
        for later in [
            "LATER-SESSION",
            "LATER-RECAP",
            "LATER-MEETING",
            "Write the Factors persona brief",
            "DONE-EARLY",
            "Pricing",
        ] {
            assert!(!all.contains(later), "{later} in {all}");
        }
        assert!(then.find("pricing").is_none());

        let after = threads_at(&store, UNIFIZE, oct(16, 12, 0));
        let factor = after.details(&after.find("factor").unwrap());
        assert!(
            factor.contains("Write the Factors persona brief")
                || after
                    .details(&after.find("persona").unwrap())
                    .contains("Write the Factors persona brief")
        );
        assert!(after.find("LATER-MEETING").is_some());

        let before = threads_at(&store, UNIFIZE, oct(1, 0, 0));
        assert!(before.threads.is_empty() && before.index().is_empty());
    }

    #[test]
    fn personal_sources_never_appear() {
        let store = memory();
        for at in [oct(13, 21, 30), oct(16, 12, 0)] {
            for folder in [UNIFIZE, FREEBYRD] {
                let threads = threads_at(&store, folder, at);
                let mut all = threads.index();
                for thread in &threads.threads {
                    all.push_str(&threads.details(thread));
                }
                for ask in [
                    "persona", "secret", "whatsapp", "dinner", "page", "mona", "typesafe",
                ] {
                    if let Some(thread) = threads.find(ask) {
                        all.push_str(&threads.details(&thread));
                    }
                }
                for secret in [
                    "SECRET",
                    "WHATSAPP",
                    "PAGE-",
                    "Mona",
                    "Persona dinner",
                    "Persona block",
                    "Every Employee",
                    "Unifize Global",
                    "TypeSafe",
                    "SUMMARY-TEXT",
                ] {
                    assert!(!all.contains(secret), "{secret} in {all}");
                }
            }
        }
    }

    #[test]
    fn sessions_alone_group_only_by_two_shared_words() {
        let words = Words::default();
        let setup = Setup::default();
        let session = |title: &str| {
            let line = Line {
                text: title.to_owned(),
                at: 0,
                from: "Claude Code".to_owned(),
                status: Status::Seen,
                item: String::new(),
            };
            item(
                Kind::Session,
                title,
                line,
                "Sessions",
                Vec::new(),
                1,
                &words,
                &setup,
            )
        };
        let items = [
            session("Thank you modal state"),
            session("CMD+K site search modal"),
            session("Factors data layer status"),
            session("Factors.ai and data layer overview"),
            session("Factors review"),
        ];
        let threads = group(&items, &words, 0);
        let keys: Vec<(&str, usize)> = threads
            .iter()
            .map(|thread| (thread.key.as_str(), thread.items.len()))
            .collect();
        // Factors: two words shared, and a title of one word joins by it;
        // the two modals stay apart, and finished sessions alone are no
        // thread.
        assert_eq!(keys, [("factor", 3)]);

        // With an owed item, one shared word is enough.
        let mut owed = session("Fix the modal copy");
        owed.kind = Kind::Owed;
        let items = [
            session("Thank you modal state"),
            session("CMD+K site search modal"),
            owed,
        ];
        assert_eq!(group(&items, &words, 0)[0].items.len(), 3);
    }

    /// An item of this kind and weight, as `gather` makes it.
    fn made(kind: Kind, title: &str, points: i64) -> Item {
        let line = Line {
            text: title.to_owned(),
            at: 0,
            from: "Notion".to_owned(),
            status: Status::Seen,
            item: String::new(),
        };
        item(
            kind,
            title,
            line,
            "Due",
            Vec::new(),
            points,
            &Words::default(),
            &Setup::default(),
        )
    }

    #[test]
    fn tied_items_group_before_coincidences() {
        // Owed items weigh the same, so "note", "plugin" and "ship" all
        // join two of them; the plugin tasks say "plugin" twice each and
        // have the most in common.
        let items = [
            made(
                Kind::Owed,
                "Write Plugin Notes for the 9 plugins that have none",
                4,
            ),
            made(
                Kind::Owed,
                "Confirm which plugins ship and the plugin count per plan",
                4,
            ),
            made(
                Kind::Owed,
                "Ground the 12 Retail Stores note pages in Notion, then push them live",
                4,
            ),
            made(
                Kind::Owed,
                "Confirm what ships today for every AI promise, and mark the rest Beta",
                4,
            ),
            made(
                Kind::Owed,
                "Fill the missing regions and vendors rows in Notion",
                4,
            ),
            made(
                Kind::Owed,
                "Check its uptime promise against the proof rule",
                2,
            ),
        ];
        let threads = group(&items, &Words::default(), 0);
        let plugin = &threads[0];
        assert_eq!(
            (plugin.key.as_str(), plugin.name.as_str(), &plugin.items[..]),
            ("plugin", "Plugins", &[0, 1][..])
        );
        // Live tasks with several words and one in common ("ship",
        // "Notion", "promise") each stay a thread of their own.
        assert_eq!(threads.len(), 5, "{threads:?}");
        assert!(threads[1..].iter().all(|thread| thread.items.len() == 1));

        // A live task joins a session by one word once nothing closer is
        // left; two sessions alone still need two.
        let task = || {
            made(
                Kind::Owed,
                "Confirm what ships today for every AI promise, and mark the rest Beta",
                4,
            )
        };
        let items = [
            task(),
            made(Kind::Session, "Beta signup banner layout", 1),
            made(Kind::Session, "Signup form validation", 1),
        ];
        let threads = group(&items, &Words::default(), 0);
        assert_eq!(threads.len(), 1, "{threads:?}");
        assert_eq!(
            (threads[0].key.as_str(), &threads[0].items[..]),
            ("beta", &[0, 1][..])
        );
        // A session closer to another goes with it first.
        let items = [
            task(),
            made(Kind::Session, "Beta signup banner layout", 1),
            made(Kind::Session, "Signup emails", 1),
        ];
        let threads = group(&items, &Words::default(), 0);
        let found: Vec<(&str, &[usize])> = threads
            .iter()
            .map(|thread| (thread.key.as_str(), &thread.items[..]))
            .collect();
        assert_eq!(found, [("promise", &[0][..]), ("signup", &[1, 2][..])]);

        // Two tasks naming the homepage are two threads, keyed apart.
        let items = [
            made(Kind::Owed, "Decide the homepage hero's four tabs", 4),
            made(
                Kind::Owed,
                "Write the Quality domain page as the second page after the homepage",
                2,
            ),
        ];
        let keys: Vec<String> = group(&items, &Words::default(), 0)
            .into_iter()
            .map(|thread| thread.key)
            .collect();
        assert_eq!(keys, ["homepage", "quality"]);
    }

    #[test]
    fn names_match_by_stems_and_say_the_word_as_written() {
        // One word shared and no session: the word, not a title cut short.
        assert_eq!(
            name_for(
                "plugin",
                &[
                    "Write Plugin Notes for the 9 plugins that have none",
                    "Confirm which plugins ship and the plugin count per plan"
                ],
                &[]
            ),
            "Plugins"
        );
        assert_eq!(
            name_for(
                "feature",
                &[
                    "Write Plugin Features for the 9 plugins that have none",
                    "Ground the Retail feature pages in Notion"
                ],
                &[]
            ),
            "Features"
        );
        // Phrases match by stems, as first written.
        assert_eq!(
            name_for(
                "factor",
                &["Track Factor data layers", "Factors data layer status"],
                &[]
            ),
            "Factor data layers"
        );
        // Without a possessive at its end.
        assert_eq!(
            name_for(
                "hero",
                &[
                    "Decide the homepage hero's four tabs",
                    "Homepage hero document"
                ],
                &["Homepage hero document"]
            ),
            "Homepage hero"
        );
    }

    #[test]
    fn names_are_the_longest_phrase_two_items_share() {
        assert_eq!(
            name_for(
                "factor",
                &[
                    "Check Factors",
                    "Track Factors data layer",
                    "Factors data layer tracking"
                ],
                &["Factors data layer tracking"],
            ),
            "Factors data layer"
        );
        // Joining words never end a name; one shared word is enough.
        assert_eq!(
            name_for(
                "notion",
                &["Website tasks in Notion", "Add tasks to Notion"],
                &[]
            ),
            "Notion"
        );
        // Shared by none: the shortest session title with the word.
        assert_eq!(
            name_for(
                "modal",
                &["Thank you modal state", "CMD+K site search modal"],
                &["Thank you modal state", "CMD+K site search modal"],
            ),
            "Thank you modal state"
        );
        // One item: itself, cut short.
        assert_eq!(
            name_for(
                "vercel",
                &["buy the Vercel team plan and the PostHog plan for the year"],
                &[]
            ),
            "Buy the Vercel team plan…"
        );
    }

    #[test]
    fn lone_items_are_named_by_the_run_around_their_key() {
        let lone = |key: &str, title: &str| name_for(key, &[title], &[]);
        // The first clause does not fit: the run around the key, up to
        // joining words, numbers and when-and-how words, a possessive's
        // own word last.
        let planner = "Decide the calibration rig planner's 7 open sizing questions";
        assert_eq!(lone("calibration", planner), "Calibration rig planner");
        assert_eq!(lone("calibration+", planner), "Calibration rig planner");
        assert_eq!(
            lone(
                "feature",
                "Ground the 14 Retail Stores feature pages in Notion, then push them live"
            ),
            "Retail Stores feature pages"
        );
        assert_eq!(
            lone(
                "closing",
                "Rewrite every page's closing line before the spring launch"
            ),
            "Closing line"
        );
        let title = "Fix the Stripe checkout redirect loop on the pricing page for EU visitors";
        assert_eq!(
            name_for("checkout", &[title], &[title]),
            "Stripe checkout redirect loop"
        );
        // A preposition ends a run, and a key that is one has none.
        let rule = "Add the Looks Right panel to the QMS page and test its uptime promise against the audit rule";
        assert_eq!(lone("uptime", rule), "Uptime promise");
        assert_eq!(lone("against", rule), "Add the Looks Right panel…");
        // A first clause that fits stays as it is.
        assert_eq!(
            lone(
                "persona",
                "Review the 18 persona pages, then push them live"
            ),
            "Review the 18 persona pages"
        );
        // A word alone, one of two things listed or a run too long keeps
        // the title cut short.
        assert_eq!(
            lone(
                "mark",
                "Confirm what ships today for every AI promise, and mark the rest Beta"
            ),
            "Confirm what ships today for every…"
        );
        for key in ["figma", "linear"] {
            assert_eq!(
                lone(
                    key,
                    "Order the Figma org plan and the Linear plan for the year"
                ),
                "Order the Figma org plan…"
            );
        }
        assert_eq!(
            lone(
                "partner",
                "Confirm consent for every partner badge, testimonial and pricing figure"
            ),
            "Confirm consent for every partner…"
        );
        assert_eq!(
            lone(
                "pricing",
                "Fix the Enterprise Quality Management Suite Pricing Calculator bug"
            ),
            "Fix the Enterprise Quality…"
        );

        // As the index names a lone task.
        let threads = group(&[made(Kind::Owed, planner, 4)], &Words::default(), 0);
        assert_eq!(
            (threads[0].key.as_str(), threads[0].name.as_str()),
            ("calibration", "Calibration rig planner")
        );
    }
}
