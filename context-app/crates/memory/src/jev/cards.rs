//! How much each waiting card needs the person today, so the notch could
//! show the one that matters first. Each new card the notch would show (not
//! Plan my day's blocks) is scored once on a scale from "can wait days" to
//! "someone is waiting within hours", from its kind, title and the start of
//! its body; scores go to `jev/cards.jsonl`. `report` lists the cards by
//! Jev's score beside when the person looked at or dismissed each.

use std::collections::HashSet;
use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Local};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Store;
use crate::redact::redact;

const LOG: &str = "cards";
const BODY_CHARS: usize = 300;
/// Cards made in this long are scored, at most this many a run.
const RECENT_MS: i64 = 7 * 86_400_000;
const MAX_CARDS: i64 = 40;

#[derive(Debug, Serialize, Deserialize)]
struct Scored {
    at: i64,
    card: i64,
    kind: String,
    title: String,
    /// 0 (can wait days) to 3 (urgent today).
    need: f64,
    ms: u64,
}

/// Scores cards not scored yet. A line for the log; none without a key or
/// new cards.
pub fn run_due(store: &Store, now: DateTime<Local>) -> Option<String> {
    super::line("cards", run(store, now))
}

fn run(store: &Store, now: DateTime<Local>) -> Result<Option<String>> {
    if super::key().is_none() {
        return Ok(None);
    }
    let now_ms = now.timestamp_millis();
    let done: HashSet<i64> = super::entries::<Scored>(LOG)
        .into_iter()
        .map(|scored| scored.card)
        .collect();
    let mut statement = store.conn().prepare(
        "SELECT id, kind, title, body FROM card
         WHERE kind <> 'block' AND dismissed_at IS NULL AND created_at > ?1
         ORDER BY created_at DESC LIMIT ?2",
    )?;
    let cards: Vec<(i64, String, String, String)> = statement
        .query_map(params![now_ms - RECENT_MS, MAX_CARDS], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .filter_map(Result::ok)
        .filter(|(id, ..)| !done.contains(id))
        .collect();
    if cards.is_empty() {
        return Ok(None);
    }
    let state = json!({
        "now": now.format("%A %-d %B, %H:%M").to_string(),
        "about": "Cards Freebyrd shows the person in the notch: recaps, things due, drafts, \
                  calendar nudges, questions and suggestions.",
    });
    let questions: Map<String, Value> = cards
        .iter()
        .map(|(id, kind, title, body)| (format!("c{id}"), question(kind, title, body)))
        .collect();
    let started = Instant::now();
    let Some(response) = super::ask(LOG, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    for (id, kind, title, _) in &cards {
        let Some(need) = response
            .answers
            .get(&format!("c{id}"))
            .and_then(|answer| answer.score)
        else {
            continue;
        };
        super::append(
            LOG,
            &Scored {
                at: now_ms,
                card: *id,
                kind: kind.clone(),
                title: crate::one_line(&redact(title), 120),
                need,
                ms,
            },
        )?;
    }
    Ok(Some(format!("jev: scored {} new cards", cards.len())))
}

fn question(kind: &str, title: &str, body: &str) -> Value {
    super::score(
        json!({
            "card": {
                "kind": kind,
                "title": redact(&crate::one_line(title, 160)),
                "body_start": redact(&crate::one_line(body, BODY_CHARS)),
            },
            "question": "How much does `card` need the person today?",
        }),
        &[
            "Can wait days: for reference, nothing to do or decide.",
            "Worth a look today, but nothing to do.",
            "Needs a decision, a reply or an action from the person today.",
            "Urgent: something is due or someone is waiting within hours.",
        ],
    )
}

/// The scored cards by Jev's score, beside when the person looked at or
/// dismissed each.
pub fn report(store: &Store) -> Result<Vec<String>> {
    let mut scored: Vec<Scored> = super::entries(LOG);
    scored.sort_by(|a, b| b.need.total_cmp(&a.need));
    let mut lines = Vec::new();
    for card in &scored {
        let (seen, dismissed): (Option<i64>, Option<i64>) = store
            .conn()
            .query_row(
                "SELECT seen_at, dismissed_at FROM card WHERE id = ?1",
                [card.card],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((None, None));
        let what = match (seen, dismissed) {
            (_, Some(at)) => format!("dismissed {}", super::when(at)),
            (Some(at), None) => format!("seen {}", super::when(at)),
            (None, None) => "waiting".to_owned(),
        };
        lines.push(format!(
            "{:.1}/3  {} ({}): {what}",
            card.need, card.title, card.kind
        ));
    }
    lines.push(format!("{} cards scored", scored.len()));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_carry_the_card_and_four_levels() {
        let q = question(
            "due",
            "Send Priya the proposal",
            "Due Thursday. sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
        );
        assert_eq!(q["type"], "score");
        assert_eq!(q["criteria"].as_array().unwrap().len(), 4);
        assert_eq!(
            q["instructions"]["card"]["body_start"],
            "Due Thursday. [API key]"
        );
        let store = Store::open_in_memory().unwrap();
        assert!(run_due(&store, Local::now()).is_none(), "tests have no key");
    }
}
