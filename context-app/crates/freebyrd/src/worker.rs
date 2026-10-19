//! Keeps the memory current in the background. Every few minutes it reads
//! what is new from Claude Code, browser history and saved pages (email
//! threads among them), looks for promises in work conversations and puts
//! the person's on the calendar, drafts what they owe the day before it is
//! due (`memory::drafts`), and once a working day is over (04:00), has
//! Claude write that day's recap. It joins the names it saw into people
//! (`memory::people`) and lists meeting action items with no date as
//! to-dos (`memory::todos`). Once a day it looks for work the person
//! repeats and suggests a fix (`memory::routines`). On a thread of its
//! own, Jev tags new things with a project, in shadow (`memory::jev`). Every
//! minute it checks whether night shift should pick tonight's work or start
//! (`memory::night`).
//! While a card is waiting to be seen, the notch glows.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Local;
use memory::{Store, cards, connectors, drafts, ingest, promises, recap, routines};

const INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Between passes, how often to look for a card added from outside the
/// app: a scheduled job's result, posted by its own process.
const CARD_CHECK: Duration = Duration::from_secs(20);
/// After a recap fails (an expired login, say), wait this long before
/// trying again.
const RECAP_RETRY: Duration = Duration::from_secs(60 * 60);
/// Jev's thread looks for due work this often (each job keeps its own pace),
/// starting after the first pass has opened (and upgraded) the memory.
const JEV_INTERVAL: Duration = Duration::from_secs(60);
const JEV_START: Duration = Duration::from_secs(45);
/// Whether night shift should start (away after 02:00, on power, jobs
/// queued) is checked this often. The runner is a process of its own.
const NIGHT_INTERVAL: Duration = Duration::from_secs(60);

/// Starts the background reader. After each pass, `after_pass` learns
/// whether a card is waiting to be seen.
pub fn start(after_pass: impl Fn(bool) + Send + 'static) {
    if std::env::var_os("FREEBYRD_NO_MEMORY").is_some() {
        eprintln!("freebyrd: memory is off (FREEBYRD_NO_MEMORY is set)");
        return;
    }
    let spawned = thread::Builder::new()
        .name("memory".to_owned())
        .spawn(move || {
            let mut recap_failed_at = None;
            loop {
                match run_once(&mut recap_failed_at) {
                    Ok(waiting) => after_pass(waiting),
                    Err(e) => eprintln!("freebyrd: memory: {e:#}"),
                }
                // Until the next pass, glow soon after a card arrives.
                let next_pass = Instant::now() + INTERVAL;
                let mut newest = newest_waiting();
                while Instant::now() < next_pass {
                    thread::sleep(
                        CARD_CHECK.min(next_pass.saturating_duration_since(Instant::now())),
                    );
                    let now_newest = newest_waiting();
                    if now_newest > newest {
                        after_pass(true);
                    }
                    newest = now_newest;
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("freebyrd: memory: could not start: {e}");
    }
    // Jev (TypeSafe), in shadow, tags new pages, meetings, chats and plans
    // with a project. A request to TypeSafe never holds up the pass above.
    let spawned = thread::Builder::new().name("jev".to_owned()).spawn(|| {
        thread::sleep(JEV_START);
        loop {
            match Store::open_default() {
                Ok(store) => {
                    for line in memory::jev::run_due(&store, Local::now()) {
                        eprintln!("freebyrd: memory: {line}");
                    }
                }
                Err(e) => eprintln!("freebyrd: memory: jev: {e:#}"),
            }
            thread::sleep(JEV_INTERVAL);
        }
    });
    if let Err(e) = spawned {
        eprintln!("freebyrd: memory: jev: could not start: {e}");
    }
    // Night shift starts once the person is away (`memory::night::run`).
    let spawned = thread::Builder::new().name("night".to_owned()).spawn(|| {
        thread::sleep(JEV_START);
        loop {
            match Store::open_default() {
                Ok(store) => {
                    // In the evening, tonight's work picked from what is owed
                    // and what the person was in the middle of.
                    if let Some(line) = memory::night::suggest::run_due(&store, Local::now()) {
                        eprintln!("freebyrd: memory: {line}");
                    }
                    // Near the usual stop, what is open, offered for the night.
                    if let Some(line) = memory::night::stop::run_due(&store, Local::now()) {
                        eprintln!("freebyrd: memory: {line}");
                    }
                    if let Some(line) = memory::night::run::start_if_due(&store, Local::now()) {
                        eprintln!("freebyrd: memory: {line}");
                    }
                }
                Err(e) => eprintln!("freebyrd: memory: night: {e:#}"),
            }
            thread::sleep(NIGHT_INTERVAL);
        }
    });
    if let Err(e) = spawned {
        eprintln!("freebyrd: memory: night: could not start: {e}");
    }
}

/// One pass. Returns whether a card is waiting to be seen.
fn run_once(recap_failed_at: &mut Option<Instant>) -> Result<bool> {
    let store = Store::open_default()?;
    let started = Instant::now();
    let reports = ingest::all(&store);
    let added: usize = reports.iter().map(|report| report.added).sum();
    if added > 0 || reports.iter().any(|report| !report.errors.is_empty()) {
        let details: Vec<String> = reports.iter().map(ToString::to_string).collect();
        eprintln!(
            "freebyrd: memory: {added} new in {} ms ({})",
            started.elapsed().as_millis(),
            details.join("; ")
        );
    }

    let may_try = recap_failed_at.is_none_or(|at| at.elapsed() >= RECAP_RETRY);
    if may_try && let Some(day) = recap::due(&store, Local::now())? {
        match recap::generate(&store, day) {
            Ok(_) => {
                *recap_failed_at = None;
                eprintln!("freebyrd: memory: wrote the recap of {day}");
            }
            Err(e) => {
                *recap_failed_at = Some(Instant::now());
                eprintln!("freebyrd: memory: the recap of {day} failed: {e:#}");
            }
        }
    }
    // Meetings from Fireflies and tasks from Notion, each every so often.
    for line in connectors::run_due(&store, Local::now()) {
        eprintln!("freebyrd: memory: {line}");
    }
    // Promises in work conversations go on the calendar.
    match promises::find(&store, Local::now()) {
        Ok(report) if !report.is_quiet() => eprintln!("freebyrd: memory: {report}"),
        Ok(_) => {}
        Err(e) => eprintln!("freebyrd: memory: looking for promises failed: {e:#}"),
    }
    // Work threads where the person waits on the other side join Waiting on.
    match memory::asks::check(&store, Local::now()) {
        Ok(report) if !report.is_quiet() => eprintln!("freebyrd: memory: {report}"),
        Ok(_) => {}
        Err(e) => eprintln!("freebyrd: memory: reading threads for Waiting on failed: {e:#}"),
    }
    // Names and addresses seen are joined into people, and meeting action
    // items with no date join the to-do list.
    match memory::people::sync(&store, memory::time::now_ms()) {
        Ok(0) => {}
        Ok(changed) => eprintln!("freebyrd: memory: people: {changed} names joined or added"),
        Err(e) => eprintln!("freebyrd: memory: people: {e:#}"),
    }
    match memory::todos::sync(&store, Local::now()) {
        Ok(0) => {}
        Ok(added) => eprintln!("freebyrd: memory: to-do list: {added} new from meetings"),
        Err(e) => eprintln!("freebyrd: memory: to-do list: {e:#}"),
    }
    // What the person's work moved on the list: Claude Code sessions gone
    // quiet, and replies in threads tied to an item.
    match memory::progress::check(&store, Local::now()) {
        Ok(report) if !report.is_quiet() => eprintln!("freebyrd: memory: {report}"),
        Ok(_) => {}
        Err(e) => eprintln!("freebyrd: memory: to-do progress failed: {e:#}"),
    }
    // From the working day before a promise or task is due, its draft:
    // one a pass, a few a day.
    if let Some(line) = drafts::run_due(&store, Local::now()) {
        eprintln!("freebyrd: memory: {line}");
    }
    // Once a day, repeated work becomes suggestions; nothing runs without
    // the person's Yes.
    for line in routines::run_due(&store, Local::now()) {
        eprintln!("freebyrd: memory: {line}");
    }
    cards::has_new(&store)
}

/// When the newest card waiting to be seen was added, if any.
fn newest_waiting() -> Option<i64> {
    Store::open_default()
        .and_then(|store| cards::newest_waiting(&store))
        .ok()
        .flatten()
}
