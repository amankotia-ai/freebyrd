//! The easy stop (the person's ask, 2026-10-14): around the time they
//! usually stop (03:00 for them), one tap hands what they have open to the
//! night shift, so stopping costs nothing.
//!
//! Example: at 02:40 they are still in the Unifize repository. Freebyrd
//! picks tonight's work again (`suggest::refresh`), now favouring the
//! folders they had open in the last three hours, and a card waits on the
//! right of the notch: "Stopping for tonight?", with "Finish the CMD+K
//! search tests" and Hand it to the night. One press queues it; the night
//! starts once they have been away 20 minutes, on power (`run::due`), and
//! the morning card says what it did. Nothing is pushed, sent or merged
//! without them, as with any night job.

use anyhow::{Result, bail};
use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, params};

use super::{select, suggest};
use crate::{Store, cards, day, time};

/// When the person's hours are not known: 03:00, in minutes from the
/// working day's start at 04:00.
const DEFAULT_END: i64 = 23 * 60;
/// The card is offered from this long before their usual stop...
const BEFORE_MIN: i64 = 30;
/// ...until this long after it, while they are still at the Mac.
const AFTER_MIN: i64 = 120;
/// At the Mac: input within this long.
const PRESENT_SECONDS: u64 = 5 * 60;
/// Open: a Claude Code session they worked in within this long.
pub const OPEN_MS: i64 = 3 * 3_600_000;

/// The card's reference: one a working day.
fn reference(now: DateTime<Local>) -> String {
    format!("stop:{}", time::working_day(now))
}

/// Whether `now` falls near the usual stop, `end` minutes after 04:00.
fn in_window(now: DateTime<Local>, end: i64) -> bool {
    let start = time::day_span(time::working_day(now)).start_ms;
    let minutes = (now.timestamp_millis() - start) / 60_000;
    minutes >= end - BEFORE_MIN && minutes <= end + AFTER_MIN
}

/// The worker's check, every minute: near the usual stop, with the person
/// at the Mac and work open, once a working day, picks tonight's work again
/// and offers it on a card. Returns a line for the log when it did
/// something.
pub fn run_due(store: &Store, now: DateTime<Local>) -> Option<String> {
    check(store, now, super::idle_seconds())
}

/// `run_due`, given how long the person has been away.
fn check(store: &Store, now: DateTime<Local>, idle_seconds: Option<u64>) -> Option<String> {
    let end = day::usual_hours(store, now)
        .ok()
        .flatten()
        .map_or(DEFAULT_END, |hours| hours.end);
    if !in_window(now, end) || idle_seconds.is_none_or(|idle| idle > PRESENT_SECONDS) {
        return None;
    }
    match offered(store, now) {
        Ok(true) => return None,
        Ok(false) => {}
        Err(e) => return Some(format!("night shift: easy stop: {e:#}")),
    }
    match open_work(store, now) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => return Some(format!("night shift: easy stop: {e:#}")),
    }
    Some(match offer(store, now) {
        Ok(0) => "night shift: easy stop: nothing open can run unattended".to_owned(),
        Ok(count) => format!("night shift: easy stop: {count} offered for tonight"),
        Err(e) => {
            // Marked as offered, so a failing call is not repeated all night.
            let _ = mark(store, now);
            format!("night shift: easy stop failed: {e:#}")
        }
    })
}

/// Whether tonight's card was offered already.
fn offered(store: &Store, now: DateTime<Local>) -> Result<bool> {
    Ok(store
        .conn()
        .query_row(
            "SELECT 1 FROM card WHERE ref = ?1",
            [reference(now)],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether the person worked in a Claude Code session in the last hours.
fn open_work(store: &Store, now: DateTime<Local>) -> Result<bool> {
    let since = now.timestamp_millis() - OPEN_MS;
    let open: i64 = store.conn().query_row(
        "SELECT count(*) FROM session WHERE last_at >= ?1",
        [since],
        |row| row.get(0),
    )?;
    Ok(open > 0)
}

/// Notes that tonight was dealt with, on a card that never shows.
fn mark(store: &Store, now: DateTime<Local>) -> Result<()> {
    let reference = reference(now);
    let now_ms = now.timestamp_millis();
    cards::add_once(store, "stop", &reference, "", "", &[], None, now_ms)?;
    if let Some(id) = card_id(store, &reference)? {
        cards::dismiss(store, id, now_ms)?;
    }
    Ok(())
}

fn card_id(store: &Store, reference: &str) -> Result<Option<i64>> {
    Ok(store
        .conn()
        .query_row("SELECT id FROM card WHERE ref = ?1", [reference], |row| {
            row.get(0)
        })
        .optional()?)
}

/// Picks tonight's work again, favouring what is open, and offers it on
/// the stop card. Returns how many jobs it offers; with none, tonight is
/// marked as dealt with and no card shows.
pub fn offer(store: &Store, now: DateTime<Local>) -> Result<usize> {
    suggest::refresh(store, now)?;
    offer_picks(store, now)
}

/// The card for the picks waiting now (`suggested`).
fn offer_picks(store: &Store, now: DateTime<Local>) -> Result<usize> {
    let picks = select(store, "state = 'suggested' ORDER BY position, id", [])?;
    if picks.is_empty() {
        mark(store, now)?;
        return Ok(0);
    }
    let queued = select(store, "state = 'queued' ORDER BY position, id", [])?;
    // Picks the check found already built, in this offer.
    let left_out = select(
        store,
        "state = 'skipped' AND json_extract(spec, '$.dropped') IS NOT NULL AND created_at >= ?1
         ORDER BY id",
        [now.timestamp_millis() - 10 * 60_000],
    )?;
    let mut lines = vec!["The night can carry on while you're away:".to_owned()];
    lines.extend(
        picks
            .iter()
            .map(|job| format!("- **{}**: {}", job.title, job.sub)),
    );
    if !left_out.is_empty() {
        let named: Vec<String> = left_out
            .iter()
            .map(|job| format!("{} ({})", job.title, job.sub))
            .collect();
        lines.push(format!("Left out, already there: {}.", named.join("; ")));
    }
    if !queued.is_empty() {
        let titles: Vec<&str> = queued.iter().map(|job| job.title.as_str()).collect();
        lines.push(format!("Already queued: {}.", titles.join(", ")));
    }
    lines.push(String::new());
    lines.push(
        "It starts once you've been away 20 minutes, on power, and leaves previews and \
         branches for the morning. Nothing is pushed, sent or merged without you."
            .to_owned(),
    );
    cards::upsert_with(
        store,
        "stop",
        &reference(now),
        "Stopping for tonight?",
        &lines.join("\n"),
        &[
            cards::Action::new("handoff", "Hand it to the night"),
            cards::Action::new("not", "Not tonight"),
        ],
        None,
        now.timestamp_millis(),
    )?;
    Ok(picks.len())
}

/// A button on the stop card: `handoff` queues every pick waiting now;
/// `not` leaves them under the moon for the person to add.
pub fn act(store: &Store, card_id: i64, action: &str) -> Result<()> {
    match action {
        "handoff" => {
            let picks = select(store, "state = 'suggested' ORDER BY position, id", [])?;
            for job in &picks {
                store.conn().execute(
                    "UPDATE night_job SET state = 'queued',
                         position = (SELECT coalesce(max(position), 0) + 1 FROM night_job)
                     WHERE id = ?1",
                    params![job.id],
                )?;
            }
            let body = match picks.len() {
                0 => "Nothing was waiting to hand over; what is queued under the moon still runs."
                    .to_owned(),
                count => {
                    let titles: Vec<&str> = picks.iter().map(|job| job.title.as_str()).collect();
                    format!(
                        "{} queued: {}. It starts once you've been away 20 minutes, on power; \
                         the morning card shows what it did. Good night.",
                        if count == 1 {
                            "1 job".to_owned()
                        } else {
                            format!("{count} jobs")
                        },
                        titles.join(", ")
                    )
                }
            };
            cards::settle(store, card_id, "Handed to the night", &body, &[], None)?;
        }
        "not" => cards::settle(
            store,
            card_id,
            "Not tonight",
            "Nothing new runs tonight. The picks wait under the moon if you change your mind.",
            &[],
            None,
        )?,
        other => bail!("the stop card has no action {other:?}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::super::{Kind, Plan};
    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 14, hour, minute, 0)
            .single()
            .unwrap()
    }

    fn pick(title: &str) -> suggest::Pick {
        suggest::Pick {
            plan: Plan {
                kind: Kind::List,
                project: "/work/unifize".to_owned(),
                title: title.to_owned(),
                sub: String::new(),
                what: String::new(),
                items: vec!["Write the tests".to_owned()],
                minutes: Some(30),
            },
            why: "For Ben: site search".to_owned(),
        }
    }

    #[test]
    fn the_window_opens_half_an_hour_before_the_usual_stop() {
        // A stop at 03:00 is 23 hours after 04:00.
        assert!(!in_window(at(2, 20), DEFAULT_END));
        assert!(in_window(at(2, 30), DEFAULT_END));
        assert!(in_window(at(3, 45), DEFAULT_END));
        assert!(!in_window(at(14, 0), DEFAULT_END));
        // Someone who stops at 22:00 (18 hours after 04:00).
        assert!(in_window(at(21, 40), 18 * 60));
        assert!(!in_window(at(2, 40), 18 * 60));
    }

    #[test]
    fn it_waits_for_the_person_and_open_work() {
        let store = Store::open_in_memory().unwrap();
        // Away for half an hour: nothing.
        assert!(check(&store, at(2, 40), Some(30 * 60)).is_none());
        // At the Mac but nothing worked on in the last hours: nothing.
        assert!(check(&store, at(2, 40), Some(10)).is_none());
        assert!(!offered(&store, at(2, 40)).unwrap());
    }

    #[test]
    fn the_card_lists_the_picks_and_one_press_queues_them() {
        let store = Store::open_in_memory().unwrap();
        let now = at(2, 40);
        suggest::save(
            &store,
            &[pick("Finish CMD+K tests"), pick("Contact form states")],
            now.timestamp_millis(),
        )
        .unwrap();
        assert_eq!(offer_picks(&store, now).unwrap(), 2);
        assert!(offered(&store, now).unwrap());
        let id = card_id(&store, &reference(now)).unwrap().unwrap();
        let card = cards::get(&store, id).unwrap().unwrap();
        assert_eq!(card.title, "Stopping for tonight?");
        assert!(
            card.body
                .contains("**Finish CMD+K tests**: For Ben: site search")
        );
        assert!(!card.body.contains('\u{2014}'));

        act(&store, id, "handoff").unwrap();
        let queued = select(&store, "state = 'queued'", []).unwrap();
        assert_eq!(queued.len(), 2);
        let card = cards::get(&store, id).unwrap().unwrap();
        assert_eq!(card.title, "Handed to the night");
        assert!(
            card.body
                .starts_with("2 jobs queued: Finish CMD+K tests, Contact form states.")
        );
        assert!(card.actions.is_empty());
    }

    #[test]
    fn picks_found_already_built_are_named_as_left_out() {
        let store = Store::open_in_memory().unwrap();
        let now = at(2, 40);
        suggest::save(
            &store,
            &[pick("Contact form states")],
            now.timestamp_millis(),
        )
        .unwrap();
        suggest::save_dropped(
            &store,
            &[(
                pick("Scaffold persona pages"),
                "Persona pages are built under explorations/personas".to_owned(),
            )],
            now.timestamp_millis(),
        )
        .unwrap();
        assert_eq!(offer_picks(&store, now).unwrap(), 1);
        let id = card_id(&store, &reference(now)).unwrap().unwrap();
        let card = cards::get(&store, id).unwrap().unwrap();
        assert!(card.body.contains("- **Contact form states**"));
        assert!(card.body.contains(
            "Left out, already there: Scaffold persona pages (Persona pages are built under explorations/personas)."
        ));
        // The dropped one is not handed over.
        act(&store, id, "handoff").unwrap();
        let queued = select(&store, "state = 'queued'", []).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].title, "Contact form states");
    }

    #[test]
    fn not_tonight_leaves_the_picks_waiting() {
        let store = Store::open_in_memory().unwrap();
        let now = at(2, 40);
        suggest::save(
            &store,
            &[pick("Finish CMD+K tests")],
            now.timestamp_millis(),
        )
        .unwrap();
        offer_picks(&store, now).unwrap();
        let id = card_id(&store, &reference(now)).unwrap().unwrap();
        act(&store, id, "not").unwrap();
        assert!(select(&store, "state = 'queued'", []).unwrap().is_empty());
        assert_eq!(select(&store, "state = 'suggested'", []).unwrap().len(), 1);
        assert!(act(&store, id, "maybe").is_err());
    }

    #[test]
    fn with_nothing_to_offer_the_night_is_marked_and_no_card_shows() {
        let store = Store::open_in_memory().unwrap();
        let now = at(2, 40);
        assert_eq!(offer_picks(&store, now).unwrap(), 0);
        assert!(offered(&store, now).unwrap());
        assert!(
            cards::for_overlay(&store, now.timestamp_millis())
                .unwrap()
                .is_empty()
        );
    }
}
