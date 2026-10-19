//! Repeated work (step 6): Freebyrd spots work the person does again and
//! again, suggests a fix, and runs only the fixes they approve.
//!
//! - `detect` finds the patterns with plain code over the memory: the same
//!   ask to Claude, the same thing at the same time, the same steps in the
//!   same order, and open loops.
//! - `suggest` has Claude name each new pattern and draft its fix from
//!   minimal evidence (counts, dates, titles, the first words of what the
//!   person typed), and shows the draft as a card with Yes and No.
//! - `jobs` carries out a Yes: a skill in `~/.claude/skills`, or a
//!   scheduled job that launchd runs and that posts its result as a card,
//!   each with Pause and Remove.
//!
//! Littlebird's routines are also a prompt plus a schedule, suggested with
//! a rationale, with each run ending clear, found or error
//! (`littlebird/FINDINGS.md` §5.3); but its suggestions are made, and its
//! runs happen, on its servers. Here the detectors run on the Mac, only
//! the minimal evidence goes to Claude, and nothing is set up or run
//! without the person's Yes.

pub mod detect;
pub mod jobs;
mod launchd;
pub mod skills;
pub mod suggest;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveTime, TimeDelta, TimeZone};
use serde::{Deserialize, Serialize};

use crate::connectors;
use crate::store::Store;

const SOURCE: &str = "routines";
/// Repeated work is looked for once a day.
const EVERY_MS: i64 = 24 * 3_600_000;
/// At most this many new patterns are drafted a day.
pub const DRAFTS_A_DAY: usize = 3;

/// A connector a scheduled job may read with: its server and its reading
/// tools. Every tool of it that writes is denied by name.
#[derive(Debug)]
pub struct Reader {
    /// "claude.ai Notion".
    pub server: &'static str,
    /// "Notion".
    pub name: &'static str,
    /// What it reads, for the drafts.
    pub reads: &'static str,
    pub allowed: &'static [&'static str],
    pub denied: &'static [&'static str],
}

/// The connectors jobs may read with: the ones Freebyrd already reads, with
/// the same tools (`connectors`).
pub const READERS: &[Reader] = &[
    Reader {
        server: "claude.ai Fireflies",
        name: "Fireflies",
        reads: "the person's recorded meetings: the list of meetings, and each one's summary \
                with its action items",
        allowed: connectors::FIREFLIES_ALLOWED,
        denied: connectors::FIREFLIES_DENIED,
    },
    Reader {
        server: "claude.ai Notion",
        name: "Notion",
        reads: "Notion pages and databases: search, fetch a page by its link, query a database",
        allowed: connectors::NOTION_ALLOWED,
        denied: connectors::NOTION_DENIED,
    },
];

/// The reader called `name` ("Notion"), or served by it ("claude.ai Notion").
pub fn reader(name: &str) -> Option<&'static Reader> {
    let name = name.trim();
    READERS
        .iter()
        .find(|r| r.server.eq_ignore_ascii_case(name) || r.name.eq_ignore_ascii_case(name))
}

/// The readers whose connectors are connected now.
pub fn available_readers() -> Vec<&'static Reader> {
    let connected = claude_cli::connected_servers();
    READERS
        .iter()
        .filter(|r| connected.iter().any(|c| c == r.server))
        .collect()
}

/// When a scheduled job runs: on these weekdays, at a local time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// 0 is Sunday, as in launchd.
    pub weekdays: Vec<u32>,
    /// "09:00".
    pub time: String,
}

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

impl Schedule {
    /// `None` unless there is a day and the time reads as `HH:MM`.
    pub fn new(weekdays: &[u32], time: &str) -> Option<Self> {
        let mut weekdays: Vec<u32> = weekdays.iter().copied().filter(|d| *d <= 6).collect();
        weekdays.sort_unstable();
        weekdays.dedup();
        let at = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
        (!weekdays.is_empty()).then(|| Self {
            weekdays,
            time: at.format("%H:%M").to_string(),
        })
    }

    pub fn hour_minute(&self) -> (u32, u32) {
        let (hour, minute) = self.time.split_once(':').unwrap_or(("9", "0"));
        (hour.parse().unwrap_or(9), minute.parse().unwrap_or(0))
    }

    /// "every weekday at 09:00", "on Mondays and Thursdays at 09:00".
    pub fn describe(&self) -> String {
        let days = match self.weekdays.as_slice() {
            [1, 2, 3, 4, 5] => "every weekday".to_owned(),
            [0, 1, 2, 3, 4, 5, 6] => "every day".to_owned(),
            [0, 6] => "on weekends".to_owned(),
            [day] => format!("every {}", DAY_NAMES[*day as usize]),
            days => {
                let names: Vec<String> = days
                    .iter()
                    .map(|d| format!("{}s", DAY_NAMES[*d as usize]))
                    .collect();
                let (last, rest) = names.split_last().expect("at least two days");
                format!("on {} and {last}", rest.join(", "))
            }
        };
        format!("{days} at {}", self.time)
    }

    /// The next time it runs after `now`.
    pub fn next_after(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        let (hour, minute) = self.hour_minute();
        let at = NaiveTime::from_hms_opt(hour, minute, 0)?;
        (0..=7).find_map(|ahead| {
            let day = now.date_naive() + TimeDelta::days(ahead);
            if !self
                .weekdays
                .contains(&day.weekday().num_days_from_sunday())
            {
                return None;
            }
            Local
                .from_local_datetime(&day.and_time(at))
                .earliest()
                .filter(|when| *when > now)
        })
    }
}

/// Once a day: looks for repeated work and drafts fixes for a few new
/// patterns, as suggestion cards; and keeps the scheduled jobs' runner
/// current. Returns a line per thing done, for the log.
pub fn run_due(store: &Store, now: DateTime<Local>) -> Vec<String> {
    let mut lines = Vec::new();
    match jobs::Places::standard().map(|places| jobs::refresh_runner(store, &places)) {
        Some(Ok(true)) => lines.push("routines: updated the job runner".to_owned()),
        Some(Err(e)) => lines.push(format!("routines: could not update the job runner: {e:#}")),
        _ => {}
    }
    let last = store
        .cursor(SOURCE, "suggest")
        .ok()
        .flatten()
        .map_or(0, |(at, _)| at);
    if now.timestamp_millis() - last < EVERY_MS {
        return lines;
    }
    // Counted as done even if it fails, so a failure is not retried on
    // every pass.
    let _ = store.set_cursor(SOURCE, "suggest", now.timestamp_millis(), "");
    match suggest_now(store, now, DRAFTS_A_DAY) {
        Ok(more) => lines.extend(more),
        Err(e) => lines.push(format!("routines: {e:#}")),
    }
    lines
}

/// Looks for repeated work now and drafts fixes for up to `max` new
/// patterns.
pub fn suggest_now(store: &Store, now: DateTime<Local>, max: usize) -> Result<Vec<String>> {
    let dismissed = suggest::settle_dismissed(store, now.timestamp_millis())?;
    let found = detect::all(store, now)?;
    let added = suggest::remember(store, &found, now.timestamp_millis())?;
    let mut lines = vec![format!(
        "routines: {} patterns found, {added} of them new; {dismissed} dismissed suggestions \
         counted as No",
        found.len()
    )];
    lines.extend(suggest::draft_new(store, now, max)?);
    Ok(lines)
}

/// A button on a suggestion card (`pattern:<id>`) or on a job's result
/// (`run:<id>`).
pub fn act(store: &Store, card_id: i64, reference: &str, action: &str) -> Result<()> {
    let places = jobs::Places::standard().context("HOME is not set")?;
    jobs::act(store, &places, card_id, reference, action, Local::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_read_and_describe_themselves() {
        let weekdays = Schedule::new(&[5, 1, 2, 3, 4, 9], "9:05").unwrap();
        assert_eq!(weekdays.weekdays, [1, 2, 3, 4, 5]);
        assert_eq!(weekdays.describe(), "every weekday at 09:05");
        assert_eq!(
            Schedule::new(&[1, 4], "17:30").unwrap().describe(),
            "on Mondays and Thursdays at 17:30"
        );
        assert_eq!(
            Schedule::new(&[0], "03:33").unwrap().describe(),
            "every Sunday at 03:33"
        );
        assert!(Schedule::new(&[], "09:00").is_none());
        assert!(Schedule::new(&[1], "9am").is_none());
    }

    #[test]
    fn the_next_run_skips_days_off() {
        let schedule = Schedule::new(&[1, 2, 3, 4, 5], "09:00").unwrap();
        // Friday 16 October 2026, 10:00: the next run is Monday 09:00.
        let friday = Local
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
                    .unwrap()
                    .and_hms_opt(10, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap();
        let next = schedule.next_after(friday).unwrap();
        assert_eq!(next.format("%a %d %H:%M").to_string(), "Mon 19 09:00");
    }

    #[test]
    fn readers_are_found_by_name_or_server() {
        assert_eq!(reader("Notion").unwrap().server, "claude.ai Notion");
        assert_eq!(reader("claude.ai Fireflies").unwrap().name, "Fireflies");
        assert!(reader("Gmail").is_none());
    }
}
