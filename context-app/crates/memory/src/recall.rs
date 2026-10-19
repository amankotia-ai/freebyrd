//! What the notch sends with a question, besides the screen: today's
//! activity so far, the latest recaps, other cards, and past activity,
//! pages, messages and meetings that match the question or the title of the
//! window in front.

use std::collections::HashSet;
use std::fmt::Write as _;

use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};
use rusqlite::params;

use crate::digest::Digest;
use crate::redact::redact;
use crate::store::Store;
use crate::time::{self, Span};
use crate::{one_line, recap, truncate};

/// Roughly 5,000 tokens: enough to be useful, small enough to answer fast.
const BUDGET: usize = 20_000;
const TODAY_BUDGET: usize = 5_000;
const SEARCH_DAYS: i64 = 45;
const MAX_MATCHES: usize = 12;
const MAX_PAGES: usize = 3;
const MAX_MEETINGS: usize = 3;
/// Meetings go back further than other matches: Fireflies history is
/// filled in for half a year.
const MEETING_DAYS: i64 = 200;
/// Transcript lines go with a meeting when a question asks for someone's
/// words, up to this many characters a meeting.
const TRANSCRIPT_BUDGET: usize = 5_000;
/// A question for the calendar or about a booking looks through mail
/// opened in the browser this many days back, for at most this many mails,
/// each up to this many characters of its event-like lines.
const MAIL_DAYS: i64 = 14;
const MAX_MAILS: usize = 3;
const MAIL_CHARS: usize = 1_800;
/// Projects worked on in this many days are listed with every question.
const PROJECT_DAYS: i64 = 30;
const MAX_PROJECTS: usize = 8;
/// The latest session titles that say what each project is about.
const PROJECT_TITLES: usize = 3;

const STOPWORDS: &[&str] = &[
    "about",
    "after",
    "again",
    "all",
    "also",
    "and",
    "any",
    "are",
    "ask",
    "been",
    "before",
    "but",
    "can",
    "could",
    "did",
    "does",
    "doing",
    "done",
    "for",
    "from",
    "get",
    "give",
    "had",
    "has",
    "have",
    "her",
    "here",
    "him",
    "his",
    "how",
    "into",
    "its",
    "just",
    "know",
    "last",
    "like",
    "make",
    "more",
    "most",
    "need",
    "not",
    "now",
    "one",
    "our",
    "out",
    "over",
    "please",
    "said",
    "say",
    "should",
    "show",
    "some",
    "tell",
    "than",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "today",
    "tomorrow",
    "was",
    "were",
    "what",
    "when",
    "where",
    "which",
    "who",
    "why",
    "will",
    "with",
    "would",
    "yesterday",
    "you",
    "your",
];

/// The `<memory>` text for a question asked at `now`, with `window_title`
/// (of the app in front) as a hint to what it is about. Empty when there is
/// nothing to say.
pub fn context(
    store: &Store,
    question: &str,
    window_title: Option<&str>,
    now: DateTime<Local>,
) -> Result<String> {
    let mut sections = Vec::new();
    let now_ms = now.timestamp_millis();

    let today = time::working_day(now);
    let span = Span {
        start_ms: time::day_span(today).start_ms,
        end_ms: now_ms + 1,
    };
    let digest = Digest::build(
        store,
        span,
        format!("today so far, {}", time::day_label(today)),
        false,
    )?;
    if !digest.is_empty() {
        sections.push(format!("## Today so far\n{}", digest.render(TODAY_BUDGET)));
    }

    // "Can we use this in any project?" needs the projects by name, and a
    // question about the screen alone matches none of them.
    let projects = projects(store, now_ms - PROJECT_DAYS * 86_400_000)?;
    if !projects.is_empty() {
        sections.push(format!(
            "## Their projects (folders they worked in with Claude Code, last {PROJECT_DAYS} days, newest first)\n{}",
            projects.join("\n")
        ));
    }

    // A question about someone gets their page, from every app at once:
    // what is open between them and the person, what waits on a reply,
    // where they met (`people`).
    let mut pages = Vec::new();
    for person in crate::people::for_question(store, question, window_title)? {
        pages.push(crate::people::memory(store, person, now)?);
    }
    if !pages.is_empty() {
        sections.push(format!(
            "## People the question is about (their pages in Freebyrd: everywhere they were seen, and what is open with them)\n{}",
            pages.join("\n\n")
        ));
    }
    // A question about what is owed or left gets the to-do list.
    if ABOUT_TODOS.is_match(question) {
        let lines = crate::todos::memory_lines(store, now)?;
        if !lines.is_empty() {
            sections.push(format!(
                "## Their to-do list (what is open, from Freebyrd's list)\n{}",
                lines.join("\n")
            ));
        }
    }

    let terms = format!("{question} {}", window_title.unwrap_or_default());
    let since_ms = now_ms - SEARCH_DAYS * 86_400_000;
    // Ahead of the rest, which the budget cuts first: for a question about
    // a meeting, the meeting is the answer.
    let meetings = meetings(
        store,
        &terms,
        question,
        &time::named_days(question, now.date_naive()),
        now_ms - MEETING_DAYS * 86_400_000,
    )?;
    if !meetings.is_empty() {
        sections.push(format!(
            "## Meetings (Fireflies), newest first\n{}",
            meetings.join("\n\n")
        ));
    }

    // "Add my flight to my calendar": the ticket is in their mail, which
    // matches the question by none of its words.
    if about_calendar(question) {
        let mails = mail_events(store, &terms, now_ms - MAIL_DAYS * 86_400_000)?;
        if !mails.is_empty() {
            sections.push(format!(
                "## Event-like lines from email they opened in the browser (last {MAIL_DAYS} days; those matching the question first, then newest)\n{}",
                mails.join("\n")
            ));
        }
    }

    for (day, recap) in recap::recent(store, 2)? {
        sections.push(format!(
            "## Recap of {}\n{}",
            time::day_label(day),
            recap.to_markdown()
        ));
    }

    if let Some(query) = fts_query(&terms) {
        let matches = search_activity(store, &query, since_ms)?;
        if !matches.is_empty() {
            sections.push(format!(
                "## Past activity matching the question\n{}",
                matches.join("\n")
            ));
        }
        let pages = search_pages(store, &query)?;
        if !pages.is_empty() {
            sections.push(format!(
                "## Saved pages matching the question\n{}",
                pages.join("\n")
            ));
        }
        let messages = search_messages(store, &query, since_ms)?;
        if !messages.is_empty() {
            sections.push(format!(
                "## Messages matching the question (WhatsApp, email)\n{}",
                messages.join("\n")
            ));
        }
    }

    let coming = crate::plans::upcoming(store, now_ms - 7 * 86_400_000, now_ms + 14 * 86_400_000)?;
    if !coming.is_empty() {
        sections.push(format!(
            "## Plans from a week ago to two weeks ahead (promises, deadlines, events)\n{}",
            coming.join("\n")
        ));
    }

    if ABOUT_REPLIES.is_match(question) {
        let waiting = they_wrote_last(store, now_ms - 3 * 86_400_000)?;
        if !waiting.is_empty() {
            sections.push(format!(
                "## WhatsApp chats where the other side wrote last (last 3 days)\n{}",
                waiting.join("\n")
            ));
        }
    }

    Ok(truncate(&redact(&sections.join("\n\n")), BUDGET))
}

/// Questions about what is owed, due or left, which get the to-do list.
static ABOUT_TODOS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(to-?dos?|tasks?|list|owe[sd]?|pending|left|open|backlog|due|overdue|waiting on|plate)\b",
    )
    .expect("valid pattern")
});

/// Questions about replies and messages, which get the chats waiting on
/// the person.
static ABOUT_REPLIES: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(whatsapp|reply|replied|respond|answer|waiting|messag\w*|chats?|texts?|wrote|pending)\b")
        .expect("valid pattern")
});

/// Questions about meetings: they get the latest meetings even when nothing
/// else in them matches, and read Fireflies first (`connectors::refresh_fireflies`).
static ABOUT_MEETINGS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(meetings?|calls?|met|meet|sync|standup|stand-up|catch[- ]?up|1:1|discuss\w*|talked|spoke|minutes|fireflies|transcripts?)\b",
    )
    .expect("valid pattern")
});

pub fn about_meetings(question: &str) -> bool {
    ABOUT_MEETINGS.is_match(question)
}

/// Questions that ask for something to go on the calendar, or are about a
/// booking: they get the event-like lines of mail the person opened.
static ABOUT_CALENDAR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(calendars?|calender|cal|remind(er|ers)?|schedule|tickets?|e-?tickets?|flights?|trains?|bookings?|booked|reservations?|pnr|itinerar(y|ies)|appointments?|boarding|check-?in)\b",
    )
    .expect("valid pattern")
});

pub fn about_calendar(question: &str) -> bool {
    ABOUT_CALENDAR.is_match(question)
}

/// Web mail the person opened since `since_ms` (pages the extension saved),
/// each as its event-like lines (`screen::candidates`): those that share a
/// word with `terms` first, then the newest.
fn mail_events(store: &Store, terms: &str, since_ms: i64) -> Result<Vec<String>> {
    let words: Vec<String> = fts_query(terms)
        .map(|query| {
            query
                .split(" OR ")
                .map(|word| word.trim_matches('"').to_owned())
                .collect()
        })
        .unwrap_or_default();
    // SQLite takes the other columns from the row with the latest time.
    let mut statement = store.conn().prepare(
        "SELECT max(a.ts), a.title, a.url, p.text
         FROM activity a JOIN page_text p ON p.id = a.text_id
         WHERE a.kind = 'page' AND a.ts >= ?1
           AND (a.url LIKE 'https://mail.%' OR a.url LIKE 'https://outlook.%')
         GROUP BY a.text_id ORDER BY max(a.ts) DESC LIMIT 60",
    )?;
    let rows = statement.query_map([since_ms], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for row in rows {
        let (ts, title, url, text) = row?;
        if !crate::screen::is_mail(&url) {
            continue;
        }
        let Some(lines) = crate::screen::candidates(None, &text) else {
            continue;
        };
        // The same mail saved again as it was scrolled or reopened.
        if !seen.insert(lines.clone()) {
            continue;
        }
        let lower = text.to_lowercase();
        let matches = words.iter().any(|word| lower.contains(word.as_str()));
        found.push((matches, ts, title, url, lines));
    }
    // Stable: newest first within each.
    found.sort_by_key(|(matches, ..)| !matches);
    Ok(found
        .into_iter()
        .take(MAX_MAILS)
        .map(|(_, ts, title, url, lines)| {
            let site = crate::exclusions::site(&url).unwrap_or_default();
            let lines: Vec<String> = truncate(&lines, MAIL_CHARS)
                .lines()
                .map(|line| format!("  {line}"))
                .collect();
            format!(
                "- \"{}\" ({site}), opened {}:\n{}",
                one_line(title.as_deref().unwrap_or("Untitled"), 120),
                time::moment(ts),
                lines.join("\n")
            )
        })
        .collect())
}

/// Questions about what someone said, which get a meeting's transcript
/// lines: "what did Ben say about pricing", "quote Raj on the seal". Not
/// "tell me about the meeting", which the summary answers.
static ABOUT_WORDS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(said|say|says|saying|quote\w*|verbatim|word for word|exact words|exactly what|in (his|her|their|my) (own )?words|mention\w*|told|brought up|bring up)\b",
    )
    .expect("valid pattern")
});

/// Where words are kept other than meetings: a question about what someone
/// said there is not about a meeting unless it says so.
static OTHER_PLACES: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(whatsapp|e-?mails?|mail|chats?|messag\w*|texts?|texted|slack|dm)\b")
        .expect("valid pattern")
});

/// Whether a question asks what someone said in a meeting.
pub fn about_words(question: &str) -> bool {
    ABOUT_WORDS.is_match(question) && (about_meetings(question) || !OTHER_PLACES.is_match(question))
}

/// The meetings a question is about, as `context` picks them, best first;
/// for `connectors::before_question`, which reads the first one's
/// transcript when the question asks what someone said.
pub fn meetings_for_question(
    store: &Store,
    question: &str,
    now: DateTime<Local>,
) -> Result<Vec<i64>> {
    meeting_ids(
        store,
        question,
        about_meetings(question),
        &time::named_days(question, now.date_naive()),
        now.timestamp_millis() - MEETING_DAYS * 86_400_000,
    )
}

/// Fireflies meetings for a question, each with its summary and action
/// items, and the transcript lines that match when it asks what someone
/// said. Says so when it names days and none is kept for them.
fn meetings(
    store: &Store,
    terms: &str,
    question: &str,
    days: &[NaiveDate],
    since_ms: i64,
) -> Result<Vec<String>> {
    let ids = meeting_ids(store, terms, about_meetings(question), days, since_ms)?;
    if ids.is_empty() && !days.is_empty() {
        let named: Vec<String> = days.iter().map(|day| time::short_day_label(*day)).collect();
        return Ok(vec![format!(
            "No meeting is kept for {} (Fireflies had none, or could not be read).",
            named.join(" or ")
        )]);
    }
    // The words to look for in what was said: the question's, without
    // those about meetings or saying.
    let quotes = if about_words(question) {
        fts_query(&ABOUT_WORDS.replace_all(&ABOUT_MEETINGS.replace_all(terms, " "), " "))
    } else {
        None
    };
    ids.iter()
        .map(|id| describe_meeting(store, *id, quotes.as_deref()))
        .collect()
}

/// On the days a question names when it names any (those matching `terms`
/// first), none when none is kept for them; otherwise those that match
/// `terms` (a name, a topic), newest first. Words about meetings themselves
/// do not count toward a match, since every meeting would match them; a
/// question about meetings with nothing else to go on gets the latest ones.
fn meeting_ids(
    store: &Store,
    terms: &str,
    about: bool,
    days: &[NaiveDate],
    since_ms: i64,
) -> Result<Vec<i64>> {
    let matching = match fts_query(&ABOUT_MEETINGS.replace_all(terms, " ")) {
        Some(query) => matching_meetings(store, &query, since_ms)?,
        None => Vec::new(),
    };
    let mut ids: Vec<i64> = if days.is_empty() {
        let mut ids = matching;
        if ids.is_empty() && about {
            ids = meetings_between(store, since_ms, i64::MAX)?;
        }
        ids
    } else {
        let mut on_days = Vec::new();
        for day in days {
            let span = time::calendar_day(*day);
            on_days.extend(meetings_between(store, span.start_ms, span.end_ms)?);
        }
        // Stable: on each day, the newest first still.
        on_days.sort_by_key(|id| !matching.contains(id));
        on_days
    };
    ids.truncate(MAX_MEETINGS);
    Ok(ids)
}

/// Meetings with a message matching `query`, newest first.
fn matching_meetings(store: &Store, query: &str, since_ms: i64) -> Result<Vec<i64>> {
    let mut statement = store.conn().prepare(
        "SELECT c.id FROM message_fts
         JOIN message m ON m.id = message_fts.rowid
         JOIN conversation c ON c.id = m.conversation_id
         WHERE message_fts MATCH ?1 AND c.app = 'fireflies'
           AND coalesce(m.sent_at, m.first_seen_at) >= ?2
         GROUP BY c.id ORDER BY max(coalesce(m.sent_at, m.first_seen_at)) DESC LIMIT 50",
    )?;
    let rows = statement.query_map(params![query, since_ms], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Meetings that started from `from_ms` and before `until_ms`, newest first.
fn meetings_between(store: &Store, from_ms: i64, until_ms: i64) -> Result<Vec<i64>> {
    let mut statement = store.conn().prepare(
        "SELECT c.id FROM conversation c JOIN message m ON m.conversation_id = c.id
         WHERE c.app = 'fireflies' AND coalesce(m.sent_at, m.first_seen_at) >= ?1
           AND coalesce(m.sent_at, m.first_seen_at) < ?2
         GROUP BY c.id ORDER BY max(coalesce(m.sent_at, m.first_seen_at)) DESC LIMIT 50",
    )?;
    let rows = statement.query_map(params![from_ms, until_ms], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A meeting's title and time, its summary, and its action items by owner,
/// worded as tasks given to someone, never as something they said: "for the
/// person: Share the minutes" is a task Fireflies gave them, and once read as
/// "you asked to share the minutes".
fn describe_meeting(store: &Store, conversation_id: i64, quotes: Option<&str>) -> Result<String> {
    let mut statement = store.conn().prepare(
        "SELECT c.title, m.sender, m.text, coalesce(m.sent_at, m.first_seen_at)
         FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE m.conversation_id = ?1 ORDER BY m.id",
    )?;
    let rows = statement.query_map(params![conversation_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut heading = None;
    let mut summary = None;
    let mut items = Vec::new();
    for row in rows {
        let (title, sender, text, at) = row?;
        heading.get_or_insert_with(|| {
            format!("### \"{}\", {}", one_line(&title, 120), time::moment(at))
        });
        if sender == crate::connectors::SUMMARY {
            summary = Some(text);
        } else {
            let owner = if sender == crate::conversations::ME {
                "the person"
            } else {
                sender.as_str()
            };
            items.push(format!("- for {owner}: {}", one_line(&text, 300)));
        }
    }
    let mut out = heading.unwrap_or_default();
    match summary {
        Some(summary) => {
            let _ = write!(out, "\n{}", summary.trim());
        }
        None => out.push_str("\n(No summary was kept for this meeting.)"),
    }
    if !items.is_empty() {
        let _ = write!(
            out,
            "\nAction items from the call, as Fireflies assigned them (tasks, not things anyone said):\n{}",
            items.join("\n")
        );
    }
    if let Some(query) = quotes {
        if !crate::transcripts::has(store, conversation_id)? {
            out.push_str("\n(Its transcript is not kept, so its exact words are not known.)");
        } else {
            let lines =
                crate::transcripts::matching(store, conversation_id, query, TRANSCRIPT_BUDGET)?;
            if lines.is_empty() {
                out.push_str("\n(No line of its transcript matches the question.)");
            } else {
                let _ = write!(
                    out,
                    "\nWhat was said, from the transcript (minutes:seconds into the call; Fireflies' \
                     speech-to-text, which can mishear words), the lines that match the question \
                     with the one before each:\n{}",
                    lines.join("\n")
                );
            }
        }
    }
    Ok(out)
}

/// Messages in chats and email threads; meetings have their own section.
fn search_messages(store: &Store, query: &str, since_ms: i64) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT c.app, c.title, m.sender, m.text, coalesce(m.sent_at, m.first_seen_at) AS at
         FROM message_fts
         JOIN message m ON m.id = message_fts.rowid
         JOIN conversation c ON c.id = m.conversation_id
         WHERE message_fts MATCH ?1 AND coalesce(m.sent_at, m.first_seen_at) >= ?2
           AND c.app != 'fireflies'
         ORDER BY rank LIMIT 10",
    )?;
    let rows = statement.query_map(params![query, since_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut found = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    found.sort_by_key(|row| row.4);
    Ok(found
        .into_iter()
        .map(|(app, title, sender, text, at)| {
            format!(
                "- {}, {} \"{}\", {}: {}",
                time::moment(at),
                place(&app),
                one_line(&title, 60),
                sender,
                one_line(&text, 300)
            )
        })
        .collect())
}

/// WhatsApp chats whose newest message (since `since_ms`) is from the other side.
fn they_wrote_last(store: &Store, since_ms: i64) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT c.title, m.sender, m.text, coalesce(m.sent_at, m.first_seen_at) AS at
         FROM conversation c
         JOIN message m ON m.id = (
             SELECT id FROM message WHERE conversation_id = c.id
             ORDER BY coalesce(sent_at, first_seen_at) DESC, id DESC LIMIT 1)
         WHERE c.app = 'whatsapp' AND coalesce(m.sent_at, m.first_seen_at) >= ?1 AND m.sender != 'me'
         ORDER BY at DESC LIMIT 10",
    )?;
    let rows = statement.query_map(params![since_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut lines = Vec::new();
    for row in rows {
        let (title, sender, text, at) = row?;
        lines.push(format!(
            "- {title} ({sender}, {}): {}",
            time::moment(at),
            one_line(&text, 200)
        ));
    }
    Ok(lines)
}

/// One line a project worked on in Claude Code since `since_ms`, newest
/// first: its folder name, when it was last worked on, how many sessions,
/// and its latest session titles. The home folder and app folders (Claude's
/// scratch space) hold no one project and are left out.
fn projects(store: &Store, since_ms: i64) -> Result<Vec<String>> {
    let home = crate::home()
        .map(|home| home.display().to_string())
        .unwrap_or_default();
    let mut statement = store.conn().prepare(
        "SELECT p.id, p.name, p.root, max(a.ts), count(DISTINCT a.session_id)
         FROM activity a JOIN project p ON p.id = a.project_id
         WHERE a.source = 'claude' AND a.kind IN ('prompt', 'edit') AND a.ts > ?1
         GROUP BY p.id ORDER BY max(a.ts) DESC",
    )?;
    let found = statement
        .query_map([since_ms], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut titles = store.conn().prepare(
        "SELECT title FROM session WHERE project_id = ?1 AND last_at > ?2 AND title IS NOT NULL
         ORDER BY last_at DESC LIMIT 10",
    )?;
    let mut lines = Vec::new();
    for (id, name, root, last, sessions) in found {
        if root == home || root.contains("/Library/Application Support/") {
            continue;
        }
        let mut latest: Vec<String> = Vec::new();
        for title in titles.query_map(params![id, since_ms], |row| row.get::<_, String>(0))? {
            let title = one_line(&title?, 80);
            if !title.is_empty() && !latest.contains(&title) && latest.len() < PROJECT_TITLES {
                latest.push(title);
            }
        }
        let mut line = format!(
            "- {name}: last worked on {}, {sessions} session{}",
            time::local(last).map_or_else(String::new, |at| time::short_day_label(at.date_naive())),
            if sessions == 1 { "" } else { "s" }
        );
        if !latest.is_empty() {
            let quoted: Vec<String> = latest.iter().map(|t| format!("\"{t}\"")).collect();
            let _ = write!(line, "; lately {}", quoted.join(", "));
        }
        lines.push(line);
        if lines.len() == MAX_PROJECTS {
            break;
        }
    }
    Ok(lines)
}

fn place(app: &str) -> &'static str {
    match app {
        "whatsapp" => "WhatsApp chat",
        "gmail" => "email",
        _ => "conversation",
    }
}

/// An FTS5 query matching any meaningful word of `text`, or `None` if it
/// has none. Every word is quoted, so nothing in it is FTS syntax.
pub fn fts_query(text: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() >= 3 && !STOPWORDS.contains(&word.as_str()))
        .filter(|word| seen.insert(word.clone()))
        .take(12)
        .map(|word| format!("\"{word}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// Ranked by BM25 with a match in a title (a page's or a session's subject)
/// counting five times a match in the text, and a URL half. Littlebird's
/// local index weights titles too (`littlebird/FINDINGS.md` §5.0).
fn search_activity(store: &Store, query: &str, since_ms: i64) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT a.kind, a.ts, a.title, a.detail, a.url, p.name, s.title
         FROM activity_fts
         JOIN activity a ON a.id = activity_fts.rowid
         LEFT JOIN project p ON p.id = a.project_id
         LEFT JOIN session s ON s.id = a.session_id
         WHERE activity_fts MATCH ?1 AND a.ts >= ?2
         ORDER BY bm25(activity_fts, 5.0, 1.0, 0.5) LIMIT 60",
    )?;
    let rows = statement.query_map(params![query, since_ms], |row| {
        Ok(Match {
            kind: row.get(0)?,
            ts: row.get(1)?,
            title: row.get(2)?,
            detail: row.get(3)?,
            url: row.get(4)?,
            project: row.get(5)?,
            session: row.get(6)?,
        })
    })?;
    let mut seen = HashSet::new();
    let mut matches = Vec::new();
    for row in rows {
        let found = row?;
        // The same page visited many times is one match.
        if !seen.insert(found.dedup_key()) {
            continue;
        }
        matches.push(found);
        if matches.len() == MAX_MATCHES {
            break;
        }
    }
    matches.sort_by_key(|found| found.ts);
    Ok(matches.iter().map(Match::describe).collect())
}

struct Match {
    kind: String,
    ts: i64,
    title: Option<String>,
    detail: Option<String>,
    url: Option<String>,
    project: Option<String>,
    session: Option<String>,
}

impl Match {
    fn dedup_key(&self) -> String {
        match self.kind.as_str() {
            "visit" | "page" => format!(
                "page:{}",
                self.title
                    .as_deref()
                    .or(self.url.as_deref())
                    .unwrap_or_default()
            ),
            _ => format!(
                "{}:{}",
                self.kind,
                self.detail.as_deref().unwrap_or_default()
            ),
        }
    }

    fn describe(&self) -> String {
        let mut line = format!("- {}", time::moment(self.ts));
        if let Some(project) = &self.project {
            let _ = write!(line, ", {project}");
        }
        if let Some(session) = &self.session {
            let _ = write!(line, " (session \"{}\")", one_line(session, 60));
        }
        let detail = one_line(self.detail.as_deref().unwrap_or_default(), 300);
        let title = one_line(self.title.as_deref().unwrap_or_default(), 120);
        let url = self.url.as_deref().unwrap_or_default();
        let _ = match self.kind.as_str() {
            "prompt" => write!(line, ": they asked Claude: {detail}"),
            "reply" => write!(line, ": Claude replied: {detail}"),
            "edit" => write!(line, ": edited {detail}"),
            "ship" => write!(line, ": ran {detail}"),
            "notion" => write!(line, ": Notion through Claude: {detail}"),
            "skill" => write!(line, ": used the skill {detail}"),
            _ => write!(line, ": visited \"{title}\" ({url})"),
        };
        line
    }
}

fn search_pages(store: &Store, query: &str) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT snippet(page_fts, 0, '', '', ' … ', 40),
                (SELECT title FROM activity WHERE text_id = page_fts.rowid ORDER BY ts DESC LIMIT 1),
                (SELECT url FROM activity WHERE text_id = page_fts.rowid ORDER BY ts DESC LIMIT 1),
                (SELECT max(ts) FROM activity WHERE text_id = page_fts.rowid)
         FROM page_fts WHERE page_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let rows = statement.query_map(params![query, i64::try_from(MAX_PAGES)?], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    })?;
    let mut pages = Vec::new();
    for row in rows {
        let (snippet, title, url, ts) = row?;
        pages.push(format!(
            "- \"{}\" ({}), saved {}: {}",
            one_line(title.as_deref().unwrap_or("Untitled"), 120),
            url.unwrap_or_default(),
            ts.map(time::moment).unwrap_or_default(),
            one_line(&snippet, 400)
        ));
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewActivity;

    #[test]
    fn finds_messages_and_chats_waiting_on_the_person() {
        use crate::conversations::{self, Conversation, ME, NewMessage};
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let ts = now.timestamp_millis() - 3_600_000;
        let chat = |key: &'static str| Conversation {
            app: "whatsapp",
            key,
            title: key,
            work: None,
        };
        conversations::record(
            &store,
            &chat("Ben"),
            &[NewMessage {
                sender: "Ben",
                text: "Can you send the onboarding deck?",
                sent_label: None,
                sent_at: Some(ts),
            }],
            ts,
        )
        .unwrap();
        conversations::record(
            &store,
            &chat("Priya"),
            &[
                NewMessage {
                    sender: "Priya",
                    text: "Lunch?",
                    sent_label: None,
                    sent_at: Some(ts),
                },
                NewMessage {
                    sender: ME,
                    text: "Sure",
                    sent_label: None,
                    sent_at: Some(ts + 60_000),
                },
            ],
            ts,
        )
        .unwrap();

        let memory = context(
            &store,
            "Who is waiting on my reply about the deck?",
            None,
            now,
        )
        .unwrap();
        assert!(memory.contains("WhatsApp chat \"Ben\", Ben: Can you send the onboarding deck?"));
        assert!(memory.contains("- Ben (Ben, "));
        // Priya's chat ends with the person's message: not waiting.
        assert!(!memory.contains("- Priya (Priya"));
    }

    #[test]
    fn a_question_about_a_meeting_gets_the_whole_meeting() {
        use crate::conversations::{self, Conversation, ME, NewMessage};
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let meeting = |key: &'static str, title: &'static str, at: i64, messages: &[NewMessage]| {
            conversations::record(
                &store,
                &Conversation {
                    app: "fireflies",
                    key,
                    title,
                    work: Some(true),
                },
                messages,
                at,
            )
            .unwrap();
        };
        let day = 86_400_000;
        let old = now.timestamp_millis() - 5 * day;
        let recent = now.timestamp_millis() - day;
        meeting(
            "m1",
            "Website review",
            old,
            &[
                NewMessage {
                    sender: crate::connectors::SUMMARY,
                    text: "Website review, 45 min, with Ben Merton.\nThe team agreed the blog workflow.",
                    sent_label: None,
                    sent_at: Some(old),
                },
                NewMessage {
                    sender: "Ben Merton",
                    text: "Publish blogs through Content Library",
                    sent_label: None,
                    sent_at: Some(old),
                },
                NewMessage {
                    sender: ME,
                    text: "Share blog creation guide",
                    sent_label: None,
                    sent_at: Some(old),
                },
            ],
        );
        meeting(
            "m2",
            "Hiring sync",
            recent,
            &[NewMessage {
                sender: crate::connectors::SUMMARY,
                text: "Hiring sync, 20 min, with Raj.\nThis meeting covered two roles.",
                sent_label: None,
                sent_at: Some(recent),
            }],
        );

        // Ben's meeting, though a later one says "meeting" too.
        let memory = context(&store, "Tell me about my last meeting with Ben", None, now).unwrap();
        assert!(memory.contains("### \"Website review\""));
        assert!(memory.contains("The team agreed the blog workflow."));
        assert!(memory.contains("- for Ben Merton: Publish blogs through Content Library"));
        assert!(memory.contains("- for the person: Share blog creation guide"));
        assert!(!memory.contains("Hiring sync"));
        // Not again as loose messages.
        assert_eq!(memory.matches("Publish blogs").count(), 1);

        // Nothing named: the latest meetings, newest first.
        let memory = context(&store, "What happened on my last call?", None, now).unwrap();
        assert!(memory.find("Hiring sync").unwrap() < memory.find("Website review").unwrap());

        // A named day picks that day's meeting, though a later one matches too.
        let day = |ms: i64| time::local(ms).unwrap().format("%b %-d").to_string();
        let memory = context(
            &store,
            &format!("action items from my {} sync", day(old)),
            None,
            now,
        )
        .unwrap();
        assert!(memory.contains("Website review"));
        assert!(!memory.contains("Hiring sync"));
        let empty = now.timestamp_millis() - 3 * 86_400_000;
        let memory = context(&store, &format!("my {} call", day(empty)), None, now).unwrap();
        assert!(memory.contains("No meeting is kept for"));
        assert!(!memory.contains("Website review"));

        // What someone said: the transcript's matching lines, once kept.
        let ids = meetings_for_question(&store, "what did Ben say about the blog?", now).unwrap();
        assert_eq!(ids.len(), 1);
        let memory = context(&store, "what did Ben say about the blog?", None, now).unwrap();
        assert!(memory.contains("Its transcript is not kept"));
        crate::transcripts::save(
            &store,
            ids[0],
            &crate::transcripts::parse(
                "[40:12 - 40:20] Ben Merton: Publish every blog through the Content Library.\n\
                 [40:21 - 40:22] Abhishek Mankotia: Okay.",
            ),
        )
        .unwrap();
        let memory = context(&store, "what did Ben say about the blog?", None, now).unwrap();
        assert!(
            memory.contains("[40:12] Ben Merton: Publish every blog through the Content Library.")
        );
        // A summary question does not get transcript lines.
        let memory = context(&store, "tell me about my last meeting with Ben", None, now).unwrap();
        assert!(!memory.contains("[40:12]"));

        assert!(about_words("what did Ben say about pricing?"));
        assert!(about_words("did Raj mention the seal in the Sep 7 call?"));
        assert!(!about_words("what did Ben say on WhatsApp?"));
        assert!(!about_words("tell me about my last meeting with Ben"));
        assert!(about_meetings("what did we discuss with Ben?"));
        assert!(!about_meetings("what's open with Ben?"));
    }

    #[test]
    fn builds_safe_queries() {
        assert_eq!(
            fts_query("What should I tell Ben about the \"deck\" (v2)?").as_deref(),
            Some("\"ben\" OR \"deck\"")
        );
        assert_eq!(fts_query("what is it?"), None);
    }

    #[test]
    fn finds_matching_activity_and_pages() {
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let ts = now.timestamp_millis() - 3_600_000;
        store
            .add_activity(&NewActivity {
                source: "claude",
                kind: "prompt",
                ts,
                detail: Some("Draft the proposal for Ben's onboarding flow"),
                ext_id: "1",
                ..Default::default()
            })
            .unwrap();
        let text_id = store
            .page_text_id("Ben: can you send the onboarding proposal by Thursday?")
            .unwrap();
        store
            .add_activity(&NewActivity {
                source: "page",
                kind: "page",
                ts,
                title: Some("Inbox"),
                url: Some("https://mail.google.com/"),
                text_id: Some(text_id),
                ext_id: "2",
                ..Default::default()
            })
            .unwrap();

        let memory = context(&store, "What did I promise Ben?", None, now).unwrap();
        assert!(memory.contains("they asked Claude: Draft the proposal for Ben's onboarding flow"));
        assert!(memory.contains("\"Inbox\" (https://mail.google.com/)"));
        assert!(memory.contains("by Thursday"));

        // The window title alone can find what the question is about.
        let memory = context(&store, "what should I reply?", Some("Ben - WhatsApp"), now).unwrap();
        assert!(memory.contains("onboarding"));
    }

    #[test]
    fn a_calendar_question_gets_the_ticket_from_mail() {
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let mut n = 0;
        let mut page = |url: &str, title: &str, text: &str, hours_ago: i64| {
            let text_id = store.page_text_id(text).unwrap();
            n += 1;
            store
                .add_activity(&NewActivity {
                    source: "page",
                    kind: "page",
                    ts: now.timestamp_millis() - hours_ago * 3_600_000,
                    title: Some(title),
                    url: Some(url),
                    text_id: Some(text_id),
                    ext_id: &n.to_string(),
                    ..Default::default()
                })
                .unwrap();
        };
        // A made-up e-ticket, laid out as an airline's email is: nothing in
        // it says "calendar", and the date sits away from "flight".
        page(
            "https://outlook.live.com/mail/inbox/id/abc",
            "Mail - Outlook",
            "Your flight e-ticket\nFrom (Terminal)\nTo (Terminal)\nArrives\nFri\n23 Oct 26\nMumbai (T2)\n06:10\nAI 655\n(A320)\n05:10\nDelhi (T3)\n08:20",
            2,
        );
        // Newer mail with a date, and the airline's own site: not mail.
        page(
            "https://mail.google.com/mail/u/0/#inbox/def",
            "Team offsite",
            "Lunch on Friday 30 Oct after the review",
            1,
        );
        page(
            "https://www.airline.example/itinerary",
            "Itinerary",
            "Departs\n23 Oct 26\nMumbai 06:10",
            1,
        );

        let memory = context(&store, "add my flight to the calendar", None, now).unwrap();
        let mail = memory
            .split("## Event-like lines from email")
            .nth(1)
            .expect("a section of mail");
        assert!(mail.contains("\"Mail - Outlook\" (outlook.live.com)"));
        assert!(mail.contains("  AI 655") && mail.contains("  08:20"));
        assert!(mail.contains("Lunch on Friday 30 Oct"));
        assert!(!mail.contains("airline.example"));
        // The one that shares a word with the question comes first.
        assert!(mail.find("AI 655").unwrap() < mail.find("Lunch").unwrap());

        // Other questions do not get it.
        let memory = context(&store, "what did I work on?", None, now).unwrap();
        assert!(!memory.contains("Event-like lines from email"));
    }

    #[test]
    fn every_question_gets_the_projects_worked_on_lately() {
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let day = 86_400_000;
        let mut n = 0;
        let mut work = |root: &str, name: &str, session: &str, title: &str, days_ago: i64| {
            let project = store.project_id(root, name).unwrap();
            let ts = now.timestamp_millis() - days_ago * day;
            store.note_session(session, Some(project), ts).unwrap();
            store.set_session_title(session, title).unwrap();
            n += 1;
            store
                .add_activity(&NewActivity {
                    source: "claude",
                    kind: "prompt",
                    ts,
                    project_id: Some(project),
                    session_id: Some(session),
                    detail: Some("go on"),
                    ext_id: &n.to_string(),
                    ..Default::default()
                })
                .unwrap();
        };
        work("/w/site", "Website", "s1", "Pricing page copy", 2);
        work(
            "/w/agent",
            "Flow agent",
            "s2",
            "Browser tool for the agent",
            1,
        );
        work("/w/agent", "Flow agent", "s3", "Retry failed steps", 3);
        work("/w/old", "Old app", "s4", "Logo", 40);
        let scratch = "/Users/x/Library/Application Support/Claude/scratch";
        work(scratch, "Claude scratch", "s5", "Try a thing", 1);

        // Nothing in the question names a project.
        let memory = context(&store, "can we use this in any project?", None, now).unwrap();
        let at = |needle: &str| memory.find(needle).unwrap();
        assert!(memory.contains("## Their projects"));
        assert!(memory.contains("- Flow agent: last worked on "));
        assert!(memory.contains(
            ", 2 sessions; lately \"Browser tool for the agent\", \"Retry failed steps\""
        ));
        assert!(memory.contains("- Website: last worked on "));
        assert!(at("- Flow agent") < at("- Website"));
        // Older than the window, or Claude's own scratch space.
        assert!(!memory.contains("Old app"));
        assert!(!memory.contains("Claude scratch"));
    }
}
