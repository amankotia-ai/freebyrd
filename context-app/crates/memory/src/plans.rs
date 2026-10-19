//! Plans: calendar-able things Freebyrd found, and what happens to them.
//!
//! - A promise the person made in a work conversation (an email thread, a
//!   work chat, a meeting) goes on their calendar by itself, with an Undo
//!   card: they asked for no confirmation step.
//! - Anything else calendar-able, such as a meeting time seen in a chat, a
//!   deadline in an email or a launch date in Notion, becomes a nudge: a card
//!   with the event filled in, which the person can change and add with one
//!   press.
//! - What others promised the person is kept, for recall, without a card.
//!
//! A plan seen again (the same day, mostly the same words, from anywhere) is
//! not added twice, and one the person undid or dismissed stays off.

use std::collections::HashSet;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::agenda::{self, Agenda};
use crate::calendar::{self, Event};
use crate::cards::{self, Action, Card};
use crate::redact::redact;
use crate::store::Store;
use crate::{conversations, day, one_line, time};

/// All-day events alert at 09:00 on the day; timed ones half an hour before.
const DAY_ALERT_AFTER_S: i64 = 9 * 3_600;
const TIMED_ALERT_AFTER_S: i64 = -30 * 60;
pub(crate) const DEFAULT_MINUTES: i64 = 30;

#[derive(Debug, Clone, Default)]
pub struct NewPlan {
    /// `promise` (the person owes it), `theirs` (owed to the person), or
    /// `event` (a meeting, deadline or other event seen).
    pub kind: String,
    pub title: String,
    pub person: Option<String>,
    /// Milliseconds since the Unix epoch: local midnight for an all-day plan.
    pub start_at: i64,
    pub all_day: bool,
    pub minutes: Option<i64>,
    pub location: Option<String>,
    pub quote: String,
    /// Where it was seen: "your WhatsApp chat with Ben", "the email “Pricing”".
    pub source: String,
    pub conversation_id: Option<i64>,
    pub message_id: Option<i64>,
}

/// An event as the overlay shows it for editing, in local time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub title: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM`, or `None` for all day.
    pub time: Option<String>,
    pub minutes: Option<i64>,
    pub location: Option<String>,
    /// For a timed event, an alert this many minutes before it starts
    /// instead of the usual half hour (0: at the start). Only an event
    /// offered in an answer sets it (a flight's bag drop, say); the plan
    /// does not keep it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_minutes: Option<i64>,
}

impl Template {
    pub(crate) fn of(plan: &NewPlan) -> Self {
        let at = time::local(plan.start_at);
        Self {
            title: plan.title.clone(),
            date: at
                .map(|at| at.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            time: (!plan.all_day)
                .then(|| at.map(|at| at.format("%H:%M").to_string()))
                .flatten(),
            minutes: plan.minutes,
            location: plan.location.clone(),
            alert_minutes: None,
        }
    }

    /// The start in milliseconds and whether it is all day.
    fn start(&self) -> Option<(i64, bool)> {
        let date = NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d").ok()?;
        match self
            .time
            .as_deref()
            .map(str::trim)
            .filter(|time| !time.is_empty())
        {
            None => local_ms(date.and_time(NaiveTime::MIN)).map(|ms| (ms, true)),
            Some(time) => {
                let time = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
                local_ms(date.and_time(time)).map(|ms| (ms, false))
            }
        }
    }
}

fn local_ms(naive: NaiveDateTime) -> Option<i64> {
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|time| time.timestamp_millis())
}

/// Local midnight of `date`.
pub fn midnight_ms(date: NaiveDate) -> Option<i64> {
    local_ms(date.and_time(NaiveTime::MIN))
}

/// "Thu 15 Oct", or "Thu 15 Oct, 15:00".
pub fn when_label(start_at: i64, all_day: bool) -> String {
    let Some(at) = time::local(start_at) else {
        return String::new();
    };
    if all_day {
        at.format("%a %-d %b").to_string()
    } else {
        at.format("%a %-d %b, %H:%M").to_string()
    }
}

/// Reads a date (`YYYY-MM-DD`) and an optional time (`HH:MM`) as a plan's
/// start, in local time.
pub fn parse_start(date: &str, time_of_day: Option<&str>) -> Option<(i64, bool)> {
    Template {
        title: String::new(),
        date: date.to_owned(),
        time: time_of_day.map(str::to_owned),
        minutes: None,
        location: None,
        alert_minutes: None,
    }
    .start()
}

/// A promise of the person's own: stored and put on the calendar, with an
/// Undo card. `None` if it was already known.
pub fn add_promise(store: &Store, plan: &NewPlan, now_ms: i64) -> Result<Option<String>> {
    if duplicate(store, plan.start_at, &plan.title)? {
        return Ok(None);
    }
    let id = insert(store, plan, "new", now_ms)?;
    let reference = format!("plan:{id}");
    let when = when_label(plan.start_at, plan.all_day);
    match put_on_calendar(store, id, None) {
        Ok(written) => {
            cards::upsert_with(
                store,
                "due",
                &reference,
                &when,
                &format!("{}{}", added_body(plan, &when), written.card_lines()),
                &[Action::new("undo", "Undo")],
                None,
                now_ms,
            )?;
            Ok(Some(plan.title.clone()))
        }
        Err(e) => {
            cards::upsert_with(
                store,
                "due",
                &reference,
                &when,
                &format!(
                    "**{}**\nDue {when}, from {}. It could not go on your calendar: {e:#}",
                    plan.title, plan.source
                ),
                &[],
                None,
                now_ms,
            )?;
            Err(e)
        }
    }
}

/// An event seen somewhere: stored, with a nudge card holding the event to
/// edit and add. `None` if it was already known.
pub fn suggest(store: &Store, plan: &NewPlan, now_ms: i64) -> Result<Option<i64>> {
    if duplicate(store, plan.start_at, &plan.title)? {
        return Ok(None);
    }
    let id = insert(store, plan, "suggested", now_ms)?;
    let template = serde_json::to_value(Template::of(plan))?;
    cards::upsert_with(
        store,
        "event",
        &format!("plan:{id}"),
        &when_label(plan.start_at, plan.all_day),
        &format!("From {}: \u{201c}{}\u{201d}", plan.source, plan.quote),
        &[Action::new("add", "Add to calendar")],
        Some(&template),
        now_ms,
    )?;
    Ok(Some(id))
}

/// An event Claude offered in a notch answer, because the person asked for
/// something to go on their calendar: what they saw, as they edited it,
/// with where it was found and the details worth keeping in its notes.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    #[serde(flatten)]
    pub event: Template,
    /// Where it was found: "your IndiGo email in Outlook".
    #[serde(default)]
    pub source: Option<String>,
    /// A booking reference, when to be there, what to bring.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Puts an offered event on the calendar, as the person pressed Add on it
/// in an answer. It becomes a plan with an event card like a nudge's, seen
/// already (the press was theirs, so nothing glows), and is added through
/// the card, which then says so and offers Undo. Returns the card.
pub fn add_offered(store: &Store, offer: &Offer, now_ms: i64) -> Result<Card> {
    match offered(store, offer, now_ms)? {
        Offered::Added(card) => Ok(card),
        Offered::Waiting(card_id) => act(store, card_id, "add", Some(offer.event.clone())),
    }
}

/// An offered event as a plan and its card, before it goes on the calendar.
#[derive(Debug)]
enum Offered {
    /// The same event is on the calendar already: its card.
    Added(Card),
    /// The card to press Add on.
    Waiting(i64),
}

fn offered(store: &Store, offer: &Offer, now_ms: i64) -> Result<Offered> {
    let (start_at, all_day) = offer
        .event
        .start()
        .context("the date or time is not one Freebyrd can read")?;
    // From Claude's answer, which read the screen and the memory.
    let clean = |text: &str, max: usize| one_line(&redact(text), max);
    let plan = NewPlan {
        kind: "event".to_owned(),
        title: clean(&offer.event.title, 160),
        person: None,
        start_at,
        all_day,
        minutes: if all_day { None } else { offer.event.minutes },
        location: offer
            .event
            .location
            .as_deref()
            .map(|location| clean(location, 200))
            .filter(|location| !location.is_empty()),
        quote: clean(offer.notes.as_deref().unwrap_or_default(), 300),
        source: Some(clean(offer.source.as_deref().unwrap_or_default(), 120))
            .filter(|source| !source.is_empty())
            .unwrap_or_else(|| "your question in the notch".to_owned()),
        conversation_id: None,
        message_id: None,
    };
    if plan.title.is_empty() {
        bail!("the event needs a title");
    }
    // A nudge for it may be waiting already, or it was added, undone or
    // dismissed before: the press now is the person's say, so it is used.
    let id = match same_plan(store, start_at, &plan.title)? {
        Some((id, state)) if state == "added" => {
            if let Some(card) = card_for(store, id)? {
                return Ok(Offered::Added(card));
            }
            bail!("\u{201c}{}\u{201d} is on your calendar already", plan.title);
        }
        Some((id, _)) => {
            store.conn().execute(
                "UPDATE plan SET quote = ?2, source = ?3, state = 'suggested' WHERE id = ?1",
                params![id, plan.quote, plan.source],
            )?;
            id
        }
        None => insert(store, &plan, "suggested", now_ms)?,
    };
    let reference = format!("plan:{id}");
    cards::upsert_with(
        store,
        "event",
        &reference,
        &when_label(start_at, all_day),
        &format!("From {}: \u{201c}{}\u{201d}", plan.source, plan.quote),
        &[Action::new("add", "Add to calendar")],
        Some(&serde_json::to_value(Template::of(&plan))?),
        now_ms,
    )?;
    let card_id: i64 =
        store
            .conn()
            .query_row("SELECT id FROM card WHERE ref = ?1", [&reference], |row| {
                row.get(0)
            })?;
    cards::mark_seen(store, &[card_id], now_ms)?;
    Ok(Offered::Waiting(card_id))
}

/// The plan `duplicate` would find for this day and title, with its state.
fn same_plan(store: &Store, start_at: i64, title: &str) -> Result<Option<(i64, String)>> {
    let Some(day) = time::local(start_at).map(|at| at.date_naive()) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (midnight_ms(day), day.succ_opt().and_then(midnight_ms)) else {
        return Ok(None);
    };
    let mut statement = store.conn().prepare(
        "SELECT id, title, state FROM plan WHERE start_at >= ?1 AND start_at < ?2
               AND kind NOT LIKE 'block:%' ORDER BY id DESC",
    )?;
    let plans = statement
        .query_map(params![start, end], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(plans
        .into_iter()
        .find(|(_, other, _)| similar(other, title))
        .map(|(id, _, state)| (id, state)))
}

fn card_for(store: &Store, plan_id: i64) -> Result<Option<Card>> {
    let id: Option<i64> = store
        .conn()
        .query_row(
            "SELECT id FROM card WHERE ref = ?1",
            [format!("plan:{plan_id}")],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => cards::get(store, id),
        None => Ok(None),
    }
}

/// Someone else's promise: kept for recall, without a card.
pub fn keep_theirs(store: &Store, plan: &NewPlan, now_ms: i64) -> Result<bool> {
    if duplicate(store, plan.start_at, &plan.title)? {
        return Ok(false);
    }
    insert(store, plan, "kept", now_ms)?;
    Ok(true)
}

fn added_body(plan: &NewPlan, when: &str) -> String {
    format!(
        "**{}**\nOn your calendar for {when}. From {}: \u{201c}{}\u{201d}",
        plan.title, plan.source, plan.quote
    )
}

pub(crate) fn insert(store: &Store, plan: &NewPlan, state: &str, now_ms: i64) -> Result<i64> {
    store.conn().execute(
        "INSERT INTO plan (kind, title, person, start_at, all_day, minutes, location, quote, source,
                           conversation_id, message_id, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            plan.kind,
            plan.title,
            plan.person,
            plan.start_at,
            plan.all_day,
            plan.minutes,
            plan.location,
            plan.quote,
            plan.source,
            plan.conversation_id,
            plan.message_id,
            state,
            now_ms
        ],
    )?;
    Ok(store.conn().last_insert_rowid())
}

pub(crate) fn load(store: &Store, id: i64) -> Result<NewPlan> {
    store
        .conn()
        .query_row(
            "SELECT kind, title, person, start_at, all_day, minutes, location, quote, source,
                    conversation_id, message_id
             FROM plan WHERE id = ?1",
            [id],
            |row| {
                Ok(NewPlan {
                    kind: row.get(0)?,
                    title: row.get(1)?,
                    person: row.get(2)?,
                    start_at: row.get(3)?,
                    all_day: row.get(4)?,
                    minutes: row.get(5)?,
                    location: row.get(6)?,
                    quote: row.get(7)?,
                    source: row.get(8)?,
                    conversation_id: row.get(9)?,
                    message_id: row.get(10)?,
                })
            },
        )
        .optional()?
        .context("that plan is not in the memory any more")
}

/// The agenda an event got: none (a break, or a personal event), one, or
/// none because writing it failed, and why.
enum Written {
    Skipped,
    Agenda(Agenda),
    Failed(String),
}

impl Written {
    /// What the card adds after saying the event is on the calendar.
    fn card_lines(&self) -> String {
        match self {
            Self::Skipped => String::new(),
            Self::Agenda(agenda) => format!("\n\n{}", agenda::card_lines(agenda)),
            Self::Failed(why) => format!("\n\nNo agenda this time: {}", one_line(why, 200)),
        }
    }

    /// What the plan keeps: the agenda, empty when there is none to write,
    /// and nothing when writing failed, so a rewrite tries again.
    fn kept(&self) -> Option<String> {
        match self {
            Self::Skipped => Some(String::new()),
            Self::Agenda(agenda) => Some(agenda::text(agenda)),
            Self::Failed(_) => None,
        }
    }
}

/// Puts a plan on the Freebyrd calendar. Work gets an agenda in its notes
/// (`agenda`), which asks Claude; if that fails, the event goes on without.
/// `alert_minutes` before the start replaces a timed event's usual alert.
fn put_on_calendar(store: &Store, id: i64, alert_minutes: Option<i64>) -> Result<Written> {
    let plan = load(store, id)?;
    let written = if agenda::wanted(&plan) {
        match agenda::write(store, &plan, Local::now()) {
            Ok(Some(agenda)) => Written::Agenda(agenda),
            Ok(None) => Written::Skipped,
            Err(e) => Written::Failed(format!("{e:#}")),
        }
    } else {
        Written::Skipped
    };
    let notes = agenda::notes(
        &plan,
        match &written {
            Written::Agenda(agenda) => Some(agenda),
            _ => None,
        },
    );
    let start = plan.start_at / 1_000;
    let event = Event {
        title: plan.title.clone(),
        start,
        end: (!plan.all_day).then(|| start + plan.minutes.unwrap_or(DEFAULT_MINUTES) * 60),
        all_day: plan.all_day,
        notes,
        alert_after: Some(if plan.all_day {
            DAY_ALERT_AFTER_S
        } else if let Some(minutes) =
            alert_minutes.filter(|minutes| (0..=24 * 60).contains(minutes))
        {
            -minutes * 60
        } else if plan.kind.starts_with(day::PLAN_KIND_PREFIX) {
            // A block of the person's own day: when it starts.
            0
        } else {
            TIMED_ALERT_AFTER_S
        }),
    };
    match calendar::add(&event) {
        Ok(event_id) => {
            store.conn().execute(
                "UPDATE plan SET state = 'added', event_id = ?2, note = ?3 WHERE id = ?1",
                params![id, event_id, written.kept()],
            )?;
            Ok(written)
        }
        Err(e) => {
            store.conn().execute(
                "UPDATE plan SET state = 'failed', note = ?2 WHERE id = ?1",
                params![id, format!("{e:#}")],
            )?;
            Err(e)
        }
    }
}

/// Puts a plan that is only on the to-do list (`todos::NOTED`, one the
/// person typed with a date) on the calendar, with an agenda for work.
pub(crate) fn add_listed(store: &Store, id: i64) -> Result<()> {
    put_on_calendar(store, id, None).map(|_| ())
}

/// What a card's button does. Returns the card as it is now.
pub fn act(store: &Store, card_id: i64, action: &str, edited: Option<Template>) -> Result<Card> {
    let reference = cards::reference(store, card_id)?.context("that card is gone")?;
    if let Some(rest) = reference.strip_prefix("plan:") {
        let id: i64 = rest.parse()?;
        match action {
            "add" => add_suggested(store, card_id, id, edited)?,
            "undo" => undo(store, card_id, id)?,
            other => bail!("a plan has no action {other:?}"),
        }
    } else if let Some(conversation) = reference
        .strip_prefix("conversation:")
        .and_then(|rest| rest.strip_suffix(":work"))
    {
        let id: i64 = conversation.parse()?;
        let body = match action {
            "work" => {
                conversations::set_work(store, id, true)?;
                "Marked as work. Freebyrd will look for promises in it."
            }
            "personal" => {
                conversations::set_work(store, id, false)?;
                "Marked as personal. Its messages stay on your Mac."
            }
            other => bail!("this question has no answer {other:?}"),
        };
        let card = cards::get(store, card_id)?.context("that card is gone")?;
        cards::settle(store, card_id, &card.title, body, &[], None)?;
    } else if reference.starts_with("pattern:") || reference.starts_with("run:") {
        // A suggested skill or scheduled job, or one of a job's results.
        crate::routines::act(store, card_id, &reference, action)?;
    } else if reference.starts_with("moment:") {
        // Something that helps with what the person just started.
        crate::moments::act(store, card_id, &reference, action)?;
    } else if reference.starts_with("draft:") {
        // A draft prepared before something is due.
        crate::drafts::act(store, card_id, &reference, action)?;
    } else if reference.starts_with("stop:") {
        // The easy stop: hand what is open to the night shift.
        crate::night::stop::act(store, card_id, action)?;
    } else if reference.starts_with("access:") {
        // A tool asks to read a project (the context connector).
        crate::connector::act(store, card_id, &reference, action)?;
    } else {
        bail!("this card has no actions");
    }
    cards::get(store, card_id)?.context("that card is gone")
}

/// Adds a nudge's event, as edited, to the calendar.
fn add_suggested(store: &Store, card_id: i64, id: i64, edited: Option<Template>) -> Result<()> {
    let alert_minutes = edited.as_ref().and_then(|template| template.alert_minutes);
    if let Some(template) = edited {
        let (start_at, all_day) = template
            .start()
            .context("the date or time is not one Freebyrd can read")?;
        let title = one_line(&template.title, 160);
        if title.is_empty() {
            bail!("the event needs a title");
        }
        store.conn().execute(
            "UPDATE plan SET title = ?2, start_at = ?3, all_day = ?4, minutes = ?5, location = ?6
             WHERE id = ?1",
            params![
                id,
                title,
                start_at,
                all_day,
                template.minutes,
                template.location.as_deref().map(|l| one_line(l, 200))
            ],
        )?;
    }
    let written = put_on_calendar(store, id, alert_minutes)?;
    let plan = load(store, id)?;
    let when = when_label(plan.start_at, plan.all_day);
    if let Some(category) = plan.kind.strip_prefix(day::PLAN_KIND_PREFIX) {
        // Keeps its category, which colors the card.
        let end_at = plan.start_at + plan.minutes.unwrap_or(DEFAULT_MINUTES) * 60_000;
        return cards::settle(
            store,
            card_id,
            &day::range_label(plan.start_at, end_at),
            &format!(
                "**{}**\nOn your calendar for {when}.{}",
                plan.title,
                written.card_lines()
            ),
            &[Action::new("undo", "Undo")],
            Some(&serde_json::json!({ "category": category })),
        );
    }
    cards::settle(
        store,
        card_id,
        &when,
        &format!("{}{}", added_body(&plan, &when), written.card_lines()),
        &[Action::new("undo", "Undo")],
        None,
    )
}

/// What `rewrite_notes` does with each event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewrite {
    /// Lists them.
    List,
    /// Prints the notes each would get, calling Claude but not the calendar.
    Preview,
    /// Adds each again with the new notes and removes the old event.
    Apply,
}

/// Events on the calendar from before agendas, still to come: work gets
/// an agenda in its notes, breaks and life blocks their plain line. An
/// event moved in Calendar goes back to the time Freebyrd has, so this runs
/// by hand only (`freebyrd-memory agendas`).
pub fn rewrite_notes(store: &Store, now: DateTime<Local>, mode: Rewrite) -> Result<Vec<String>> {
    let today = midnight_ms(now.date_naive()).unwrap_or_default();
    let ids = store
        .conn()
        .prepare(
            "SELECT id FROM plan
             WHERE state = 'added' AND event_id IS NOT NULL AND note IS NULL
               AND start_at >= ?1 AND (all_day OR start_at >= ?2)
             ORDER BY start_at",
        )?
        .query_map(params![today, now.timestamp_millis()], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut lines = Vec::new();
    for id in ids {
        let plan = load(store, id)?;
        let label = format!(
            "#{id} {}: {}",
            when_label(plan.start_at, plan.all_day),
            plan.title
        );
        if mode == Rewrite::List {
            let what = if agenda::wanted(&plan) {
                "an agenda"
            } else {
                "a plain line"
            };
            lines.push(format!("{label} (would get {what})"));
            continue;
        }
        if mode == Rewrite::Preview {
            let written = if agenda::wanted(&plan) {
                agenda::write(store, &plan, now)?
            } else {
                None
            };
            lines.push(format!(
                "{label}\n{}\n",
                agenda::notes(&plan, written.as_ref())
            ));
            continue;
        }
        let old: String =
            store
                .conn()
                .query_row("SELECT event_id FROM plan WHERE id = ?1", [id], |row| {
                    row.get(0)
                })?;
        // Removed first: one the person deleted in Calendar stays off.
        if !calendar::remove(&old)? {
            store
                .conn()
                .execute("UPDATE plan SET state = 'undone' WHERE id = ?1", [id])?;
            lines.push(format!("{label}: left off, it was deleted in Calendar"));
            continue;
        }
        match put_on_calendar(store, id, None) {
            Ok(written) => {
                let what = match written {
                    Written::Agenda(agenda) => format!("an agenda of {} steps", agenda.steps.len()),
                    Written::Skipped => "a plain line".to_owned(),
                    Written::Failed(why) => format!("no agenda ({})", one_line(&why, 120)),
                };
                lines.push(format!("{label}: rewritten with {what}"));
            }
            // `put_on_calendar` marked it failed, with why.
            Err(e) => lines.push(format!("{label}: removed, but not added again ({e:#})")),
        }
    }
    Ok(lines)
}

/// Takes a plan off the calendar; it stays off.
fn undo(store: &Store, card_id: i64, id: i64) -> Result<()> {
    let (title, state, event_id): (String, String, Option<String>) = store
        .conn()
        .query_row(
            "SELECT title, state, event_id FROM plan WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .context("that plan is not in the memory any more")?;
    if state == "added"
        && let Some(event_id) = event_id
    {
        calendar::remove(&event_id)?;
    }
    store
        .conn()
        .execute("UPDATE plan SET state = 'undone' WHERE id = ?1", [id])?;
    let card = cards::get(store, card_id)?.context("that card is gone")?;
    cards::settle(
        store,
        card_id,
        &card.title,
        &format!("Took \u{201c}{title}\u{201d} off your calendar."),
        &[],
        card.data.as_ref(),
    )
}

/// The same plan seen again, or restated: the same day, mostly the same
/// words, from anywhere. Undone and dismissed ones count, so they stay off.
/// Blocks of a day plan (`day`) do not: they are time, not something seen.
pub fn duplicate(store: &Store, start_at: i64, title: &str) -> Result<bool> {
    let Some(day) = time::local(start_at).map(|at| at.date_naive()) else {
        return Ok(false);
    };
    let (Some(start), Some(end)) = (midnight_ms(day), day.succ_opt().and_then(midnight_ms)) else {
        return Ok(false);
    };
    let mut statement = store.conn().prepare(
        "SELECT title FROM plan WHERE start_at >= ?1 AND start_at < ?2
               AND kind NOT LIKE 'block:%'",
    )?;
    let titles = statement
        .query_map(params![start, end], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(titles.iter().any(|other| similar(other, title)))
}

/// Mostly the same words: shared words are at least half of all the words.
pub fn similar(a: &str, b: &str) -> bool {
    let words = |text: &str| -> HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 3)
            .map(str::to_lowercase)
            .collect()
    };
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    union > 0 && a.intersection(&b).count() * 2 >= union
}

/// How far back an overdue promise or task still shows in Up next. Older
/// ones have most likely been dealt with or dropped.
pub(crate) const OVERDUE_DAYS: i64 = 14;
/// How far ahead Up next looks for something due.
const AHEAD_DAYS: i64 = 7;
/// Something timed counts as starting soon from two hours before its start
/// until a quarter of an hour after.
const SOON_BEFORE_MS: i64 = 2 * 3_600_000;
const SOON_AFTER_MS: i64 = 15 * 60_000;
/// Notion tasks become nudges with titles like this (`connectors`).
pub(crate) const TASK_PREFIX: &str = "Due: ";

/// The one thing that most needs the person now, which the notch shows on
/// its Up next line when it opens: something timed starting within two
/// hours, then an overdue promise or task, then what is due today, then
/// the soonest due in the next week. Promises are the person's own; tasks
/// come from Notion; events count only once on the calendar, and only
/// while they are about to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpNext {
    pub id: i64,
    /// `promise`, `task` or `event`.
    pub kind: String,
    pub title: String,
    /// "Starts in 25 min", "Was due yesterday", "Due today", "Due Thu 15 Oct".
    pub when: String,
    pub overdue: bool,
    pub source: String,
    /// Where an event is: "Goa to Bengaluru (T1)".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// `ask` first (Draft it, Help me finish, and for an event what fits
    /// it, `event_ask`), then `hide` and, except for an event, `done`.
    pub actions: Vec<Action>,
    /// What `ask` asks. It names nothing from the plan: the plan goes with
    /// it inside the memory, as data (`focus`).
    pub question: String,
    /// How many more are waiting behind this one.
    pub more: usize,
}

/// A plan picked for Up next, ranked: 0 starting soon, 1 overdue, 2 due
/// today, 3 due later.
struct Ranked {
    rank: u8,
    id: i64,
    kind: &'static str,
    title: String,
    start_at: i64,
    all_day: bool,
    source: String,
    place: Option<String>,
    event: EventKind,
}

/// What an event is, for the button that helps before it starts. "Prep
/// me" (who it is with, what was discussed last time) fits a meeting, not
/// a flight: on 2026-10-16 the person's flight showed it, which made no
/// sense to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Meeting,
    Travel,
    Other,
}

/// A trip, by its title, place or the words that set it.
static TRAVEL_EVENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(flights?|flying|airport|airlines?|airways|trains?|railway|station|bus|coach|ferry|cruise|hotel|check-?in|boarding|pnr|terminal|departs?|departure|itinerary|e-?tickets?)\b",
    )
    .expect("valid pattern")
});

/// A meeting, by its title.
static MEETING_EVENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(meetings?|calls?|sync|demo|review|interview|stand-?up|1:1|one-on-one|catch-?up|kick-?off|retro|workshop|presentation|pitch|webinar|onboarding|discussion)\b",
    )
    .expect("valid pattern")
});

/// Reads what an event is from what the plan says, by plain code: a trip
/// by its words; a meeting by the agenda written for it (only work gets
/// one), its title's words or someone it is with; anything else (a
/// birthday, a dinner, a doctor) is other.
pub fn event_kind(
    title: &str,
    location: Option<&str>,
    quote: &str,
    note: Option<&str>,
    person: Option<&str>,
) -> EventKind {
    let words = format!("{title}\n{}\n{quote}", location.unwrap_or_default());
    if TRAVEL_EVENT.is_match(&words) {
        EventKind::Travel
    } else if note.is_some_and(|note| !note.trim().is_empty())
        || MEETING_EVENT.is_match(title)
        || person.is_some_and(|person| !person.trim().is_empty())
    {
        EventKind::Meeting
    } else {
        EventKind::Other
    }
}

/// The button that helps with an event before it starts, and what it asks
/// (naming nothing from the plan, which goes along as memory). None when
/// nothing would help: a birthday or a dinner needs no brief.
pub fn event_ask(kind: EventKind) -> Option<(&'static str, &'static str)> {
    match kind {
        EventKind::Meeting => Some((
            "Prep me",
            "Prep me for the event under \u{201c}Up next\u{201d} at the top of the memory: who it \
             is with, what we discussed last time, and what is still open.",
        )),
        EventKind::Travel => Some((
            "Trip details",
            "What do I need for the trip under \u{201c}Up next\u{201d} at the top of the memory? The \
             booking reference, the terminal or station, when check-in or bag drop closes, and \
             anything else the booking email says to have ready. Keep it short.",
        )),
        EventKind::Other => None,
    }
}

pub fn up_next(store: &Store, now: DateTime<Local>) -> Result<Option<UpNext>> {
    let now_ms = now.timestamp_millis();
    let today = now.date_naive();
    let today_ms = midnight_ms(today).unwrap_or(now_ms);
    let tomorrow_ms = today
        .succ_opt()
        .and_then(midnight_ms)
        .unwrap_or(today_ms + 86_400_000);
    // A nudge the person dismissed stays out; a promise stays in, since
    // dismissing its card only says they saw it went on the calendar.
    let mut statement = store.conn().prepare(
        "SELECT p.id, p.kind, p.title, p.start_at, p.all_day, p.state, p.source,
                p.location, p.quote, p.note, p.person FROM plan p
         LEFT JOIN card c ON c.ref = 'plan:' || p.id
         WHERE p.start_at >= ?1 AND p.start_at < ?2
           AND p.done_at IS NULL AND coalesce(p.hidden_until, 0) <= ?3
           AND p.kind IN ('promise', 'event') AND p.state NOT IN ('undone', 'dismissed')
           AND NOT (p.kind = 'event' AND c.dismissed_at IS NOT NULL)",
    )?;
    let rows = statement.query_map(
        params![
            today_ms - OVERDUE_DAYS * 86_400_000,
            now_ms + AHEAD_DAYS * 86_400_000,
            now_ms
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        },
    )?;
    let mut ranked = Vec::new();
    for row in rows {
        let (id, kind, title, start_at, all_day, state, source, location, quote, note, person) =
            row?;
        let event = event_kind(
            &title,
            location.as_deref(),
            &quote,
            note.as_deref(),
            person.as_deref(),
        );
        let soon =
            !all_day && start_at >= now_ms - SOON_AFTER_MS && start_at < now_ms + SOON_BEFORE_MS;
        let (kind, title) = match title.strip_prefix(TASK_PREFIX) {
            Some(task) if kind == "event" => ("task", task.to_owned()),
            _ if kind == "event" => ("event", title),
            _ => ("promise", title),
        };
        let rank = if kind == "event" {
            if state != "added" || !soon {
                continue;
            }
            0
        } else if soon {
            0
        } else if if all_day {
            start_at < today_ms
        } else {
            start_at < now_ms
        } {
            1
        } else if start_at < tomorrow_ms {
            2
        } else {
            3
        };
        ranked.push(Ranked {
            rank,
            id,
            kind,
            title,
            start_at,
            all_day,
            source,
            place: location.filter(|place| !place.trim().is_empty()),
            event,
        });
    }
    // Overdue: the most recent first. Otherwise the soonest first.
    ranked.sort_by_key(|plan| {
        let order = if plan.rank == 1 {
            -plan.start_at
        } else {
            plan.start_at
        };
        (plan.rank, order, plan.id)
    });
    let more = ranked.len().saturating_sub(1);
    let Some(plan) = ranked.into_iter().next() else {
        return Ok(None);
    };
    let ask = match plan.kind {
        "event" => event_ask(plan.event),
        "task" => Some((
            "Help me finish",
            "Help me with the task under \u{201c}Up next\u{201d} at the top of the memory: what I \
             have already done on it, what is left, and a draft of the next piece.",
        )),
        _ => Some((
            "Draft it",
            "Help me deliver the promise under \u{201c}Up next\u{201d} at the top of the memory: \
             what I have already done toward it, what is still missing, and a draft of it, or of \
             an update, that I can send.",
        )),
    };
    let mut actions: Vec<Action> = ask
        .iter()
        .map(|(label, _)| Action::new("ask", label))
        .collect();
    if plan.kind == "event" {
        actions.push(Action::new("hide", "Hide"));
    } else {
        actions.push(Action::new("hide", "Not today"));
        actions.push(Action::new("done", "Done"));
    }
    Ok(Some(UpNext {
        id: plan.id,
        kind: plan.kind.to_owned(),
        when: up_next_when(&plan, now),
        overdue: plan.rank == 1,
        title: plan.title,
        source: plan.source,
        place: if plan.kind == "event" {
            plan.place
        } else {
            None
        },
        actions,
        question: ask
            .map(|(_, question)| question)
            .unwrap_or_default()
            .to_owned(),
        more,
    }))
}

/// "Starts in 25 min", "Due now", "Was due yesterday", "Due today, 15:00",
/// "Due tomorrow", "Due Thu 15 Oct".
fn up_next_when(plan: &Ranked, now: DateTime<Local>) -> String {
    let at_time = |prefix: &str, ms: i64| {
        if plan.all_day {
            prefix.to_owned()
        } else {
            format!("{prefix}, {}", time::clock(ms))
        }
    };
    let today = now.date_naive();
    let day = time::local(plan.start_at).map_or(today, |at| at.date_naive());
    let minutes = (plan.start_at - now.timestamp_millis()).div_euclid(60_000);
    match plan.rank {
        0 => {
            let verb = if plan.kind == "event" {
                "Starts"
            } else {
                "Due"
            };
            match minutes {
                m if m >= 60 && m % 60 == 0 => format!("{verb} in {} h", m / 60),
                m if m >= 60 => format!("{verb} in {} h {} min", m / 60, m % 60),
                m if m > 0 => format!("{verb} in {m} min"),
                m if m > -2 => format!("{verb} now"),
                m if plan.kind == "event" => format!("Started {} min ago", -m),
                m => format!("Was due {} min ago", -m),
            }
        }
        1 if day == today => at_time("Was due today", plan.start_at),
        1 if today.pred_opt() == Some(day) => "Was due yesterday".to_owned(),
        1 => format!("Was due {}", time::short_day_label(day)),
        2 => at_time("Due today", plan.start_at),
        _ if today.succ_opt() == Some(day) => at_time("Due tomorrow", plan.start_at),
        _ => at_time(
            &format!("Due {}", time::short_day_label(day)),
            plan.start_at,
        ),
    }
}

/// A button on the Up next line: `done` marks the plan done; `hide` puts it
/// away until the next working day. Returns what is up next now.
pub fn set_up_next(
    store: &Store,
    id: i64,
    action: &str,
    now: DateTime<Local>,
) -> Result<Option<UpNext>> {
    let now_ms = now.timestamp_millis();
    let changed = match action {
        "done" => store.conn().execute(
            "UPDATE plan SET done_at = ?2 WHERE id = ?1",
            params![id, now_ms],
        )?,
        "hide" => store.conn().execute(
            "UPDATE plan SET hidden_until = ?2 WHERE id = ?1",
            params![id, time::day_span(time::working_day(now)).end_ms],
        )?,
        other => bail!("Up next has no action {other:?}"),
    };
    if changed == 0 {
        bail!("that plan is not in the memory any more");
    }
    up_next(store, now)
}

/// The plan the person pressed on the Up next line, for the memory sent
/// with its question (`memory`), and the words to find related memory by
/// (`topic`, used on the Mac only).
pub struct Focus {
    pub memory: String,
    pub topic: String,
}

pub fn focus(store: &Store, id: i64) -> Result<Focus> {
    focus_as(
        store,
        id,
        "Up next (the item the person pressed in the notch)",
    )
}

/// `focus`, under `heading`, which the question names (the to-do list's
/// "To-do").
pub fn focus_as(store: &Store, id: i64, heading: &str) -> Result<Focus> {
    let plan = load(store, id)?;
    let title = plan
        .title
        .strip_prefix(TASK_PREFIX)
        .unwrap_or(&plan.title)
        .to_owned();
    let when = when_label(plan.start_at, plan.all_day);
    let what = match plan.kind.as_str() {
        "promise" => format!("a promise of the person's, due {when}"),
        "event" if plan.title.starts_with(TASK_PREFIX) => {
            format!("a task assigned to the person, due {when}")
        }
        "theirs" => format!("something owed to the person, due {when}"),
        _ => format!("an event on the person's calendar, {when}"),
    };
    let who = plan
        .person
        .as_deref()
        .map(|person| format!("; with {person}"))
        .unwrap_or_default();
    let mut memory = format!(
        "## {heading}\n- {title}: {what}{who}; from {}: \u{201c}{}\u{201d}",
        plan.source, plan.quote
    );
    let written: Option<String> = store
        .conn()
        .query_row(
            "SELECT note FROM plan WHERE id = ?1 AND state = 'added'",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(written) = written.filter(|text| !text.is_empty()) {
        memory.push_str(&format!("\n\nThe agenda in its calendar event:\n{written}"));
    }
    // A draft Freebyrd prepared before it was due (`drafts`).
    if let Some(prepared) = crate::drafts::for_focus(store, id)? {
        memory.push_str(&format!("\n\n{prepared}"));
    }
    Ok(Focus {
        memory: redact(&memory),
        topic: format!("{title} {}", plan.person.unwrap_or_default()),
    })
}

/// Plans from `from_ms` on, for recall: what is due, and what was promised.
pub fn upcoming(store: &Store, from_ms: i64, until_ms: i64) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT kind, title, person, start_at, all_day, state, source, done_at IS NOT NULL
         FROM plan
         WHERE start_at >= ?1 AND start_at < ?2 AND state NOT IN ('undone', 'dismissed')
           AND NOT (kind LIKE 'block:%' AND state != 'added')
         ORDER BY start_at LIMIT 30",
    )?;
    let rows = statement.query_map(params![from_ms, until_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, bool>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, bool>(7)?,
        ))
    })?;
    let mut lines = Vec::new();
    for row in rows {
        let (kind, title, person, start_at, all_day, state, source, done) = row?;
        let who = person.map(|p| format!(" ({p})")).unwrap_or_default();
        let what = match (kind.as_str(), state.as_str()) {
            _ if done => "marked done by them",
            ("theirs", _) => "owed to them by someone",
            (kind, _) if kind.starts_with(day::PLAN_KIND_PREFIX) => {
                "time they planned for themselves, on their calendar"
            }
            ("promise", "added") | ("event", "added") => "on their calendar",
            ("promise", crate::todos::NOTED) => "on their to-do list, not on their calendar",
            ("event", "suggested") => "seen, not on their calendar yet",
            _ => "noted",
        };
        lines.push(format!(
            "- {}: {title}{who}, {what}; from {source}",
            when_label(start_at, all_day)
        ));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(kind: &str, title: &str, date: &str) -> NewPlan {
        let (start_at, all_day) = parse_start(date, Some("15:00")).unwrap();
        NewPlan {
            kind: kind.into(),
            title: title.into(),
            person: Some("Ben".into()),
            start_at,
            all_day,
            minutes: Some(45),
            location: Some("Zoom".into()),
            quote: "demo thursday 3pm?".into(),
            source: "your WhatsApp chat with Ben".into(),
            ..NewPlan::default()
        }
    }

    #[test]
    fn a_nudge_carries_its_event_and_is_made_once() {
        let store = Store::open_in_memory().unwrap();
        let demo = plan("event", "Demo with Ben's team", "2026-10-15");
        assert!(suggest(&store, &demo, 1).unwrap().is_some());
        // Seen again elsewhere, worded a little differently: one nudge.
        let again = plan("event", "Demo with the team of Ben", "2026-10-15");
        assert!(suggest(&store, &again, 2).unwrap().is_none());

        let card = &cards::for_overlay(&store, 3).unwrap()[0];
        assert_eq!(card.kind, "event");
        assert_eq!(card.title, "Thu 15 Oct, 15:00");
        assert_eq!(card.actions[0].id, "add");
        let template: Template = serde_json::from_value(card.data.clone().unwrap()).unwrap();
        assert_eq!(
            template,
            Template {
                title: "Demo with Ben's team".into(),
                date: "2026-10-15".into(),
                time: Some("15:00".into()),
                minutes: Some(45),
                location: Some("Zoom".into()),
                alert_minutes: None,
            }
        );
    }

    fn offer(title: &str, notes: &str) -> Offer {
        Offer {
            event: Template {
                title: title.into(),
                date: "2026-10-16".into(),
                time: Some("23:30".into()),
                minutes: Some(80),
                location: Some("Goa (Dabolim)".into()),
                alert_minutes: Some(60),
            },
            source: Some("your IndiGo email in Outlook".into()),
            notes: Some(notes.into()),
        }
    }

    #[test]
    fn an_offered_event_becomes_a_seen_card_to_add() {
        let store = Store::open_in_memory().unwrap();
        let flight = offer(
            "IndiGo 6E 901, Goa to Bengaluru",
            "PNR Y7TW2V; bag drop closes 22:30",
        );
        let Offered::Waiting(card_id) = offered(&store, &flight, 1).unwrap() else {
            panic!("a new event waits for its Add");
        };
        let card = cards::get(&store, card_id).unwrap().unwrap();
        assert_eq!(card.kind, "event");
        assert_eq!(card.title, "Fri 16 Oct, 23:30");
        assert!(card.seen_at.is_some(), "the person pressed it: no glow");
        assert!(
            card.body
                .starts_with("From your IndiGo email in Outlook: \u{201c}PNR")
        );
        let (title, state, minutes, location): (String, String, i64, String) = store
            .conn()
            .query_row(
                "SELECT title, state, minutes, location FROM plan",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(title, "IndiGo 6E 901, Goa to Bengaluru");
        assert_eq!((state.as_str(), minutes), ("suggested", 80));
        assert_eq!(location, "Goa (Dabolim)");

        // Offered again, or seen by the watcher first: one plan and card.
        let again = offer("IndiGo 6E 901 Goa to Bengaluru", "PNR Y7TW2V");
        let Offered::Waiting(same) = offered(&store, &again, 2).unwrap() else {
            panic!("still waiting");
        };
        assert_eq!(same, card_id);
        // Once on the calendar, the card says so instead.
        store
            .conn()
            .execute("UPDATE plan SET state = 'added'", [])
            .unwrap();
        assert!(matches!(
            offered(&store, &again, 3).unwrap(),
            Offered::Added(card) if card.id == card_id
        ));
        let count: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM plan", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn an_offer_needs_a_title_and_a_date() {
        let store = Store::open_in_memory().unwrap();
        assert!(offered(&store, &offer("  ", ""), 1).is_err());
        let mut undated = offer("Flight", "");
        undated.event.date = "16 Oct".into();
        assert!(offered(&store, &undated, 1).is_err());
        // Read from the answer as the overlay sends it.
        let sent: Offer = serde_json::from_value(serde_json::json!({
            "title": "Flight", "date": "2026-10-16", "time": "23:30", "minutes": 80,
            "location": null, "alertMinutes": 60, "source": "the email", "notes": "PNR"
        }))
        .unwrap();
        assert_eq!(sent.event.alert_minutes, Some(60));
        assert_eq!(sent.source.as_deref(), Some("the email"));
    }

    #[test]
    fn templates_read_dates_and_times() {
        let all_day = parse_start("2026-10-15", None).unwrap();
        assert!(all_day.1);
        assert_eq!(when_label(all_day.0, true), "Thu 15 Oct");
        let timed = parse_start("2026-10-15", Some("09:30")).unwrap();
        assert_eq!(when_label(timed.0, false), "Thu 15 Oct, 09:30");
        assert!(parse_start("15 Oct", None).is_none());
        assert!(parse_start("2026-10-15", Some("9am")).is_none());
    }

    #[test]
    fn similar_titles() {
        assert!(similar(
            "Send Ben the revised proposal",
            "Send the revised proposal to Ben"
        ));
        assert!(!similar(
            "Send Ben the revised proposal",
            "Call Raj about pricing"
        ));
    }

    fn add(
        store: &Store,
        kind: &str,
        title: &str,
        date: &str,
        at: Option<&str>,
        state: &str,
    ) -> i64 {
        let (start_at, all_day) = parse_start(date, at).unwrap();
        let plan = NewPlan {
            kind: kind.into(),
            title: title.into(),
            start_at,
            all_day,
            quote: "quoted".into(),
            source: "a test".into(),
            ..NewPlan::default()
        };
        insert(store, &plan, state, 0).unwrap()
    }

    #[test]
    fn events_are_read_as_meetings_trips_or_other() {
        let kind = |title: &str, location: Option<&str>, quote: &str, note: Option<&str>| {
            event_kind(title, location, quote, note, None)
        };
        // The person's flight, as the watcher saw it and as an answer offered it.
        assert_eq!(
            kind(
                "Flight 6E 901 Goa to Bengaluru",
                Some("Goa to Bengaluru (T1)"),
                "23:30 6E 901",
                Some("")
            ),
            EventKind::Travel
        );
        assert_eq!(
            kind(
                "IndiGo 6E 901, Goa to Bengaluru",
                Some("Goa (GOI) airport"),
                "PNR Y7TW2V",
                Some("")
            ),
            EventKind::Travel
        );
        assert_eq!(kind("Website review", None, "", None), EventKind::Meeting);
        assert_eq!(
            kind(
                "Pricing",
                None,
                "",
                Some("Agenda\n1. Walk through the tiers")
            ),
            EventKind::Meeting
        );
        assert_eq!(
            event_kind("Pricing", None, "", None, Some("Ben")),
            EventKind::Meeting
        );
        assert_eq!(kind("Sam's birthday", None, "", Some("")), EventKind::Other);
        assert_eq!(
            kind(
                "Dinner at Toit",
                Some("Indiranagar"),
                "dinner fri 8pm",
                Some("")
            ),
            EventKind::Other
        );
        assert_eq!(event_ask(EventKind::Meeting).unwrap().0, "Prep me");
        assert_eq!(event_ask(EventKind::Travel).unwrap().0, "Trip details");
        assert!(event_ask(EventKind::Other).is_none());
    }

    #[test]
    fn up_next_gives_each_event_the_button_that_fits() {
        let store = Store::open_in_memory().unwrap();
        // Friday 16 October, 23:09.
        let now = Local.with_ymd_and_hms(2026, 10, 16, 23, 9, 0).unwrap();
        let event = |title: &str, at: &str, location: Option<&str>, quote: &str| {
            let (start_at, all_day) = parse_start("2026-10-16", Some(at)).unwrap();
            let plan = NewPlan {
                kind: "event".into(),
                title: title.into(),
                start_at,
                all_day,
                minutes: Some(80),
                location: location.map(Into::into),
                quote: quote.into(),
                source: "a test".into(),
                ..NewPlan::default()
            };
            let id = insert(&store, &plan, "added", 0).unwrap();
            // Added as personal: no agenda.
            store
                .conn()
                .execute("UPDATE plan SET note = '' WHERE id = ?1", [id])
                .unwrap();
            id
        };
        let flight = event(
            "Flight 6E 901 Goa to Bengaluru",
            "23:30",
            Some("Goa to Bengaluru (T1)"),
            "23:30 6E 901",
        );
        let next = up_next(&store, now).unwrap().unwrap();
        assert_eq!((next.id, next.when.as_str()), (flight, "Starts in 21 min"));
        assert_eq!(next.place.as_deref(), Some("Goa to Bengaluru (T1)"));
        let labels: Vec<&str> = next.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Trip details", "Hide"]);
        assert!(next.question.contains("booking reference"));
        // The question names nothing from the plan.
        assert!(!next.question.contains("6E"));

        // A birthday party: nothing to ask, only Hide.
        set_up_next(&store, flight, "hide", now).unwrap();
        event("Sam's birthday dinner", "23:45", None, "dinner at 11:45");
        let next = up_next(&store, now).unwrap().unwrap();
        let ids: Vec<&str> = next.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["hide"]);
        assert!(next.question.is_empty() && next.place.is_none());
    }

    #[test]
    fn up_next_goes_soon_then_overdue_then_today_then_later() {
        let store = Store::open_in_memory().unwrap();
        // Tuesday 13 October, 11:00.
        let now = Local.with_ymd_and_hms(2026, 10, 13, 11, 0, 0).unwrap();
        assert!(up_next(&store, now).unwrap().is_none());

        let later = add(
            &store,
            "promise",
            "Send Ben the V2 pre-read",
            "2026-10-15",
            None,
            "added",
        );
        let task = add(
            &store,
            "event",
            "Due: Ship the pricing page",
            "2026-10-13",
            None,
            "suggested",
        );
        let overdue = add(
            &store,
            "promise",
            "Send Raj the invoice",
            "2026-10-12",
            None,
            "added",
        );
        let call = add(
            &store,
            "event",
            "Website review",
            "2026-10-13",
            Some("11:40"),
            "added",
        );
        // Left out: someone else's promise, a nudge not added, an old
        // promise, an undone one, one more than a week ahead.
        add(
            &store,
            "theirs",
            "Ben sends the assets",
            "2026-10-13",
            None,
            "kept",
        );
        add(
            &store,
            "event",
            "Coffee with Priya",
            "2026-10-13",
            Some("11:30"),
            "suggested",
        );
        add(
            &store,
            "promise",
            "Old promise",
            "2026-09-01",
            None,
            "added",
        );
        add(
            &store,
            "promise",
            "Undone promise",
            "2026-10-13",
            None,
            "undone",
        );
        add(
            &store,
            "promise",
            "Far promise",
            "2026-10-30",
            None,
            "added",
        );

        let first = up_next(&store, now).unwrap().unwrap();
        assert_eq!(
            (
                first.id,
                first.kind.as_str(),
                first.when.as_str(),
                first.more
            ),
            (call, "event", "Starts in 40 min", 3)
        );
        let ids: Vec<&str> = first.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["ask", "hide"]);
        assert_eq!(first.actions[0].label, "Prep me");

        let second = set_up_next(&store, call, "hide", now).unwrap().unwrap();
        assert_eq!(
            (second.id, second.when.as_str(), second.overdue),
            (overdue, "Was due yesterday", true)
        );
        assert_eq!(second.actions[0].label, "Draft it");
        assert!(!second.question.contains("Raj"));

        let third = set_up_next(&store, overdue, "done", now).unwrap().unwrap();
        assert_eq!(
            (third.id, third.kind.as_str(), third.title.as_str()),
            (task, "task", "Ship the pricing page")
        );
        assert_eq!((third.when.as_str(), third.overdue), ("Due today", false));

        let fourth = set_up_next(&store, task, "hide", now).unwrap().unwrap();
        assert_eq!(
            (fourth.id, fourth.when.as_str(), fourth.more),
            (later, "Due Thu 15 Oct", 0)
        );

        // Hidden ones come back when the next working day starts at 04:00;
        // done ones do not, and recall says so.
        let next_day = Local.with_ymd_and_hms(2026, 10, 14, 4, 30, 0).unwrap();
        let back = up_next(&store, next_day).unwrap().unwrap();
        assert_eq!((back.id, back.when.as_str()), (task, "Was due yesterday"));
        let lines = upcoming(&store, 0, i64::MAX).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Send Raj the invoice") && l.contains("marked done"))
        );
        assert!(set_up_next(&store, 999, "done", now).is_err());
    }

    #[test]
    fn up_next_drops_dismissed_nudges_but_keeps_promises() {
        let store = Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 13, 11, 0, 0).unwrap();
        let task = plan("event", "Due: Write the case study", "2026-10-13");
        let task = suggest(&store, &task, 1).unwrap().unwrap();
        let promise = add(
            &store,
            "promise",
            "Send Ben the deck",
            "2026-10-14",
            None,
            "added",
        );
        cards::upsert(&store, "due", &format!("plan:{promise}"), "Wed", "body", 2).unwrap();

        let first = up_next(&store, now).unwrap().unwrap();
        assert_eq!((first.id, first.when.as_str()), (task, "Due today, 15:00"));
        for card in cards::for_overlay(&store, 3).unwrap() {
            cards::dismiss(&store, card.id, 3).unwrap();
        }
        let left = up_next(&store, now).unwrap().unwrap();
        assert_eq!((left.id, left.when.as_str()), (promise, "Due tomorrow"));
    }

    #[test]
    fn focus_carries_the_plan_as_memory() {
        let store = Store::open_in_memory().unwrap();
        let task = add(
            &store,
            "event",
            "Due: Ship the pricing page",
            "2026-10-13",
            None,
            "suggested",
        );
        let focus = focus(&store, task).unwrap();
        assert!(focus.memory.starts_with("## Up next"));
        assert!(focus.memory.contains(
            "Ship the pricing page: a task assigned to the person, due Tue 13 Oct; from a test"
        ));
        assert_eq!(focus.topic.trim(), "Ship the pricing page");
    }

    #[test]
    fn theirs_are_kept_without_a_card_and_listed_for_recall() {
        let store = Store::open_in_memory().unwrap();
        let assets = plan("theirs", "Ben sends the brand assets", "2026-10-14");
        assert!(keep_theirs(&store, &assets, 1).unwrap());
        assert!(cards::for_overlay(&store, 2).unwrap().is_empty());
        let lines = upcoming(&store, 0, i64::MAX).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Ben sends the brand assets (Ben), owed to them by someone"));
    }
}
