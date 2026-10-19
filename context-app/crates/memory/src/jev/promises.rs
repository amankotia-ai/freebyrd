//! Who is making a promise. Claude reads new messages in work conversations
//! and says, for each promise, whether the person made it ("me", which goes
//! on their calendar by itself) or someone else did ("them"). Getting that
//! wrong puts someone else's task on the person's calendar. Jev asks the
//! same of each promise Claude found, from the message and the few before
//! it, and the answer is logged beside Claude's (`jev/promises.jsonl`).
//! Live, a disagreement would turn a calendar entry into a nudge that waits
//! for the person. `report` sets each beside what became of its plan.

use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Local};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Store;
use crate::redact::redact;

const LOG: &str = "promises";
/// The message and up to this many before it.
const BEFORE: i64 = 3;
const MESSAGE_CHARS: usize = 400;

#[derive(Debug, Serialize, Deserialize)]
struct Checked {
    at: i64,
    message: i64,
    what: String,
    /// "me" or "them", as Claude said.
    claude: String,
    /// "the_person", "someone_else" or "no_one".
    jev: String,
    probabilities: std::collections::HashMap<String, f64>,
    ms: u64,
}

/// A promise Claude found: the message it is in, whose Claude says it is,
/// and what is promised.
pub struct Claimed<'a> {
    pub message: i64,
    pub owner: &'a str,
    pub what: &'a str,
}

/// Asks Jev whose each of Claude's promises is, and logs both answers.
/// Returns a line for the log; `None` without a key or promises.
pub fn shadow(store: &Store, claimed: &[Claimed], now: DateTime<Local>) -> Option<String> {
    if claimed.is_empty() {
        return None;
    }
    let result = check(store, claimed).and_then(|answers| {
        let Some((answers, ms)) = answers else {
            return Ok(None);
        };
        let mut differ = 0;
        for (promise, (jev, probabilities)) in claimed.iter().zip(answers) {
            let agrees = matches!(
                (promise.owner, jev.as_str()),
                ("me", "the_person") | ("them", "someone_else")
            );
            differ += usize::from(!agrees);
            super::append(
                LOG,
                &Checked {
                    at: now.timestamp_millis(),
                    message: promise.message,
                    what: crate::one_line(&redact(promise.what), 160),
                    claude: promise.owner.to_owned(),
                    jev,
                    probabilities,
                    ms,
                },
            )?;
        }
        Ok(Some(format!(
            "jev: checked {} promises, {differ} where Jev and Claude differ on whose",
            claimed.len()
        )))
    });
    super::line("promises", result)
}

type Owner = (String, std::collections::HashMap<String, f64>);

fn check(store: &Store, claimed: &[Claimed]) -> Result<Option<(Vec<Owner>, u64)>> {
    let state = json!({
        "the_person": crate::person_name().unwrap_or_else(|| "the person".into()),
    });
    let mut questions = Map::new();
    for (index, promise) in claimed.iter().enumerate() {
        questions.insert(
            format!("p{index}"),
            question(&messages(store, promise.message)?, promise.what),
        );
    }
    let started = Instant::now();
    let Some(response) = super::ask(LOG, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    let owners = (0..claimed.len())
        .map(|index| {
            response
                .answers
                .get(&format!("p{index}"))
                .and_then(|answer| Some((answer.choice.clone()?, answer.probabilities.clone())))
                .unwrap_or_else(|| ("unknown".into(), Default::default()))
        })
        .collect();
    Ok(Some((owners, ms)))
}

/// The message and the few before it in its conversation, oldest first.
fn messages(store: &Store, message: i64) -> Result<Vec<Value>> {
    let mut statement = store.conn().prepare(
        "SELECT m.sender, m.text FROM message m
         JOIN message this ON this.id = ?1 AND m.conversation_id = this.conversation_id
         WHERE m.id <= ?1 ORDER BY m.id DESC LIMIT ?2",
    )?;
    let mut messages: Vec<Value> = statement
        .query_map(params![message, BEFORE + 1], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(Result::ok)
        .map(|(sender, text)| {
            let from = if sender == "me" {
                "the person".to_owned()
            } else {
                redact(&sender)
            };
            json!({ "from": from, "text": redact(&crate::one_line(&text, MESSAGE_CHARS)) })
        })
        .collect();
    messages.reverse();
    Ok(messages)
}

fn question(messages: &[Value], what: &str) -> Value {
    super::choice(
        json!({
            "messages_oldest_first": messages,
            "commitment": redact(what),
            "question": "In `messages_oldest_first`, who commits to doing `commitment`? The \
                         last message is the one it comes from; messages from \"the person\" \
                         are theirs.",
        }),
        &[
            (
                "the_person",
                "The person says they will do it (\"I'll send it Thursday\"), or agrees to do \
                 something asked of them.",
            ),
            (
                "someone_else",
                "Someone other than the person says they will do it, for the person or for \
                 the group.",
            ),
            (
                "no_one",
                "Nobody commits: a question, a request not yet accepted, a past event, or a \
                 plan with no one taking it on.",
            ),
        ],
    )
}

/// Each checked promise beside Claude's answer and what became of its plan.
pub fn report(store: &Store) -> Result<Vec<String>> {
    let checked: Vec<Checked> = super::entries(LOG);
    let mut lines = Vec::new();
    let (mut agree, mut differ) = (0, 0);
    for promise in &checked {
        let agrees = matches!(
            (promise.claude.as_str(), promise.jev.as_str()),
            ("me", "the_person") | ("them", "someone_else")
        );
        if agrees {
            agree += 1;
        } else {
            differ += 1;
        }
        let plan: Option<(String, String)> = store
            .conn()
            .query_row(
                "SELECT kind, state FROM plan WHERE message_id = ?1 ORDER BY id DESC LIMIT 1",
                params![promise.message],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let became = plan.map_or_else(
            || "no plan".to_owned(),
            |(kind, state)| format!("{kind}, {state}"),
        );
        let p = promise
            .probabilities
            .get(&promise.jev)
            .copied()
            .unwrap_or_default();
        lines.push(format!(
            "{}  {}: Claude {} | Jev {} {p:.2} | {became}",
            super::when(promise.at),
            promise.what,
            promise.claude,
            promise.jev
        ));
    }
    lines.push(format!(
        "{} promises: Jev and Claude agreed on whose {agree}, differed {differ}",
        checked.len()
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_the_promise_and_the_few_before_it_in_its_chat() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO conversation (id, app, key, title, work, last_seen_at)
                   VALUES (1, 'whatsapp', 'a', 'Priya', 1, 1), (2, 'whatsapp', 'b', 'Sam', 1, 1);",
            )
            .unwrap();
        for (id, conversation, sender, text) in [
            (1, 1, "Priya", "hi"),
            (2, 2, "Sam", "other chat"),
            (3, 1, "Priya", "can you send the proposal?"),
            (4, 1, "me", "sure, by Thursday"),
            (5, 1, "Priya", "after"),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO message (id, conversation_id, sender, text, first_seen_at, hash)
                     VALUES (?1, ?2, ?3, ?4, 1, ?1)",
                    params![id, conversation, sender, text],
                )
                .unwrap();
        }
        let messages = messages(&store, 4).unwrap();
        let texts: Vec<&str> = messages
            .iter()
            .map(|m| m["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            ["hi", "can you send the proposal?", "sure, by Thursday"]
        );
        assert_eq!(messages[2]["from"], "the person");
        let q = question(&messages, "Send the proposal");
        assert_eq!(q["type"], "choice");
        assert!(q["criteria"]["someone_else"].is_string());
        let claimed = [Claimed {
            message: 4,
            owner: "me",
            what: "Send the proposal",
        }];
        assert!(
            shadow(&store, &claimed, Local::now()).is_none(),
            "tests have no key"
        );
    }

    /// Made-up messages to TypeSafe:
    /// `FREEBYRD_JEV_LIVE=1 cargo test -p memory jev::promises::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        let mine = vec![
            json!({ "from": "Priya", "text": "can you send the revised proposal?" }),
            json!({ "from": "the person", "text": "sure, I'll send it by Thursday" }),
        ];
        let theirs = vec![
            json!({ "from": "the person", "text": "any update on the invoice?" }),
            json!({ "from": "Priya", "text": "I'll get it to you before Friday" }),
        ];
        let asked =
            vec![json!({ "from": "Priya", "text": "could you review the deck by Monday?" })];
        let mut questions = Map::new();
        questions.insert("mine".into(), question(&mine, "Send the revised proposal"));
        questions.insert("theirs".into(), question(&theirs, "Send the invoice"));
        questions.insert("asked".into(), question(&asked, "Review the deck"));
        let response = super::super::ask(LOG, &json!({ "the_person": "Alex" }), &questions)
            .unwrap()
            .expect("a key");
        for id in ["mine", "theirs", "asked"] {
            println!("{id}: {:?}", response.answers[id].choice);
        }
        assert_eq!(
            response.answers["mine"].choice.as_deref(),
            Some("the_person")
        );
        assert_eq!(
            response.answers["theirs"].choice.as_deref(),
            Some("someone_else")
        );
    }
}
