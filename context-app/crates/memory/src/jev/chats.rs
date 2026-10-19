//! Whether a conversation is about work. Freebyrd asks the person with a
//! Work / Personal card the first time a chat brings up a time, and its
//! messages wait for the answer (`promises`). Jev guesses from the chat's
//! name and its last few messages, and the guess is logged beside the card
//! (`jev/chats.jsonl`); `report` sets each guess beside the person's answer.
//! Live, a confident guess would settle the chat and only an unsure one
//! would ask. `backtest` guesses for every chat already settled, to see how
//! often Jev would have agreed.

use std::time::Instant;

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Store;
use crate::redact::redact;

const LOG: &str = "chats";
/// The last messages Jev reads, oldest first.
const MESSAGES: i64 = 8;
const MESSAGE_CHARS: usize = 300;
/// Live, a guess outside this band would settle the chat on its own.
pub const SURE_BELOW: f64 = 0.2;
pub const SURE_ABOVE: f64 = 0.8;

#[derive(Debug, Serialize, Deserialize)]
struct Guess {
    at: i64,
    conversation: i64,
    title: String,
    /// The probability that the chat is about work.
    work: f64,
    ms: u64,
}

/// Guesses for a chat the person was just asked about, and logs it. Returns
/// a line for the log; `None` without a key.
pub fn shadow(store: &Store, conversation: i64, now_ms: i64) -> Option<String> {
    let result = guess(store, conversation).and_then(|guess| {
        let Some((work, ms, title)) = guess else {
            return Ok(None);
        };
        super::append(
            LOG,
            &Guess {
                at: now_ms,
                conversation,
                title: title.clone(),
                work,
                ms,
            },
        )?;
        Ok(Some(format!(
            "jev: chat {title:?} looks {:.0}% work",
            work * 100.0
        )))
    });
    super::line("chats", result)
}

/// Jev's probability that `conversation` is about work, how long it took,
/// and the chat's name.
fn guess(store: &Store, conversation: i64) -> Result<Option<(f64, u64, String)>> {
    let (app, title): (String, String) = store.conn().query_row(
        "SELECT app, title FROM conversation WHERE id = ?1",
        params![conversation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let state = state(store, conversation, &app, &title)?;
    let mut questions = Map::new();
    questions.insert("work".into(), question());
    let started = Instant::now();
    let Some(response) = super::ask(LOG, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    Ok(response
        .answers
        .get("work")
        .and_then(|answer| answer.noul)
        .map(|work| (work, ms, title)))
}

/// The chat's name and its last messages, the person's marked as theirs.
fn state(store: &Store, conversation: i64, app: &str, title: &str) -> Result<Value> {
    let mut statement = store.conn().prepare(
        "SELECT sender, text FROM message WHERE conversation_id = ?1
         ORDER BY coalesce(sent_at, first_seen_at) DESC, id DESC LIMIT ?2",
    )?;
    let mut messages: Vec<Value> = statement
        .query_map(params![conversation, MESSAGES], |row| {
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
    let app = match app {
        "whatsapp" => "WhatsApp",
        "gmail" => "Gmail",
        other => other,
    };
    Ok(json!({
        "app": app,
        "chat_name": redact(title),
        "last_messages_oldest_first": messages,
    }))
}

fn question() -> Value {
    super::noul(
        json!(
            "Is this conversation about the person's work (clients, colleagues, projects, \
             business or paid work) rather than their personal life?"
        ),
        "Mostly about work: clients, colleagues, projects, deliverables, meetings, invoices \
         or business.",
        "Mostly personal: family, friends, social plans, errands or chit-chat.",
    )
}

/// Each logged guess beside the person's answer, and how often they agree.
pub fn report(store: &Store) -> Result<Vec<String>> {
    let guesses: Vec<Guess> = super::entries(LOG);
    let mut lines = Vec::new();
    let (mut agree, mut differ, mut open, mut unsure) = (0, 0, 0, 0);
    for guess in &guesses {
        let answer: Option<bool> = store
            .conn()
            .query_row(
                "SELECT work FROM conversation WHERE id = ?1",
                params![guess.conversation],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let said = match answer {
            Some(true) => "you said work",
            Some(false) => "you said personal",
            None => "not answered yet",
        };
        match answer {
            Some(work) if (guess.work >= 0.5) == work => agree += 1,
            Some(_) => differ += 1,
            None => open += 1,
        }
        if (SURE_BELOW..=SURE_ABOVE).contains(&guess.work) {
            unsure += 1;
        }
        lines.push(format!(
            "{}  {}: Jev {:.0}% work | {said}",
            super::when(guess.at),
            guess.title,
            guess.work * 100.0
        ));
    }
    lines.push(format!(
        "{} guesses: agreed {agree}, differed {differ}, waiting {open}; \
         {unsure} would still have asked (between {SURE_BELOW} and {SURE_ABOVE})",
        guesses.len()
    ));
    Ok(lines)
}

/// Guesses for every chat the person already called work or personal (not
/// meetings, which are always work), and how often Jev agrees. Sends those
/// chats' names and last messages to TypeSafe; logs nothing.
pub fn backtest(store: &Store) -> Result<Vec<String>> {
    let settled: Vec<(i64, bool)> = store
        .conn()
        .prepare(
            "SELECT id, work FROM conversation
             WHERE work IS NOT NULL AND app IN ('whatsapp', 'gmail') ORDER BY id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut lines = Vec::new();
    let (mut agree, mut sure_wrong) = (0, 0);
    for (conversation, work) in &settled {
        let Some((guess, ms, title)) = guess(store, *conversation)? else {
            anyhow::bail!("no TypeSafe key in the Keychain");
        };
        let right = (guess >= 0.5) == *work;
        agree += usize::from(right);
        let sure = !(SURE_BELOW..=SURE_ABOVE).contains(&guess);
        sure_wrong += usize::from(sure && !right);
        lines.push(format!(
            "{title}: Jev {:.0}% work | you said {} | {ms} ms",
            guess * 100.0,
            if *work { "work" } else { "personal" }
        ));
    }
    lines.push(format!(
        "{} settled chats: Jev agreed on {agree}; sure and wrong on {sure_wrong}",
        settled.len()
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_marks_the_persons_messages_and_keeps_the_last_few_in_order() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO conversation (id, app, key, title, work, last_seen_at)
                   VALUES (1, 'whatsapp', 'k', 'Ben', NULL, 1);",
            )
            .unwrap();
        for n in 0..10 {
            store
                .conn()
                .execute(
                    "INSERT INTO message (conversation_id, sender, text, sent_at, first_seen_at, hash)
                     VALUES (1, ?1, ?2, ?3, 1, ?4)",
                    params![
                        if n % 2 == 0 { "me" } else { "Ben" },
                        format!("message {n} sk-ant-api03-abcdefghijklmnopqrstuvwxyz"),
                        n,
                        format!("h{n}")
                    ],
                )
                .unwrap();
        }
        let state = state(&store, 1, "whatsapp", "Ben").unwrap();
        let messages = state["last_messages_oldest_first"].as_array().unwrap();
        assert_eq!(messages.len(), MESSAGES as usize);
        assert_eq!(messages[0]["text"], "message 2 [API key]");
        assert_eq!(messages[0]["from"], "the person");
        assert_eq!(messages[7]["from"], "Ben");
        assert_eq!(state["app"], "WhatsApp");
        assert!(shadow(&store, 1, 0).is_none(), "tests have no key");
    }
}
