//! Conversations: WhatsApp chats and email threads, and their messages, as
//! Freebyrd's readers see them (WhatsApp through Accessibility, Gmail
//! through the browser extension). A message is stored once, by a
//! fingerprint of who sent it, when and what, so reading the same chat again
//! adds only what is new.
//!
//! A conversation is work, personal, or not decided yet. Promises are looked
//! for only in work conversations, and only their messages go to Claude.

use anyhow::Result;
use chrono::{Local, NaiveDateTime, TimeZone};
use rusqlite::params;

use crate::redact::redact;
use crate::store::{Store, fingerprint};
use crate::{one_line, truncate};

/// The sender name for the person's own messages.
pub const ME: &str = "me";
const MAX_MESSAGE_CHARS: usize = 5_000;

#[derive(Debug, Clone)]
pub struct Conversation<'a> {
    /// `whatsapp` or `gmail`.
    pub app: &'a str,
    /// The app's identity for it: a chat's name, a thread's ID.
    pub key: &'a str,
    /// What to call it: the person or group, or the subject.
    pub title: &'a str,
    /// Work or personal when the app says (email is work); `None` to ask.
    pub work: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    /// A name, or `ME`.
    pub sender: &'a str,
    pub text: &'a str,
    /// The app's own label for when it was sent ("14:02", "Tue, 13 Oct 2026, 14:02").
    pub sent_label: Option<&'a str>,
    /// Milliseconds since the Unix epoch, when the label could be read.
    pub sent_at: Option<i64>,
}

/// Stores what was read of a conversation at `seen_at`. Returns the
/// conversation's ID and how many messages were new. Callers that store
/// many conversations wrap this in a transaction.
pub fn record(
    store: &Store,
    conversation: &Conversation,
    messages: &[NewMessage],
    seen_at: i64,
) -> Result<(i64, usize)> {
    store.conn().execute(
        "INSERT INTO conversation (app, key, title, work, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (app, key) DO UPDATE SET title = excluded.title, last_seen_at = excluded.last_seen_at",
        params![
            conversation.app,
            conversation.key,
            conversation.title,
            conversation.work,
            seen_at
        ],
    )?;
    let id: i64 = store.conn().query_row(
        "SELECT id FROM conversation WHERE app = ?1 AND key = ?2",
        params![conversation.app, conversation.key],
        |row| row.get(0),
    )?;
    let mut added = 0;
    let mut insert = store.conn().prepare_cached(
        "INSERT OR IGNORE INTO message
         (conversation_id, sender, text, sent_at, sent_label, first_seen_at, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for message in messages {
        let text = truncate(&redact(message.text.trim()), MAX_MESSAGE_CHARS);
        if text.is_empty() {
            continue;
        }
        let hash = hash(message, &text);
        added += insert.execute(params![
            id,
            message.sender,
            text,
            message.sent_at,
            message.sent_label,
            seen_at,
            hash
        ])?;
    }
    Ok((id, added))
}

/// What tells a message apart in its conversation: who sent it, the app's
/// label for when, and its stored text.
fn hash(message: &NewMessage, text: &str) -> String {
    fingerprint(&format!(
        "{}\u{1f}{}\u{1f}{text}",
        message.sender,
        message.sent_label.unwrap_or_default()
    ))
}

/// Who a message `record` stored went to ("Ben Merton, Raj Patel"), also
/// for one stored before recipients were read.
pub fn set_recipients(
    store: &Store,
    conversation_id: i64,
    message: &NewMessage,
    recipients: &str,
) -> Result<()> {
    let text = truncate(&redact(message.text.trim()), MAX_MESSAGE_CHARS);
    let recipients = one_line(&redact(recipients), 200);
    if text.is_empty() || recipients.is_empty() {
        return Ok(());
    }
    store.conn().execute(
        "UPDATE message SET recipients = ?3 WHERE conversation_id = ?1 AND hash = ?2",
        params![conversation_id, hash(message, &text), recipients],
    )?;
    Ok(())
}

pub fn set_work(store: &Store, conversation_id: i64, work: bool) -> Result<()> {
    store.conn().execute(
        "UPDATE conversation SET work = ?2 WHERE id = ?1",
        params![conversation_id, work],
    )?;
    Ok(())
}

/// Reads a full date and time as apps print them ("Tue, 13 Oct 2026, 14:02",
/// "Tue, Oct 13, 2026 at 2:02 PM"), in local time.
pub fn parse_full_time(label: &str) -> Option<i64> {
    const FORMATS: &[&str] = &[
        "%d %b %Y, %H:%M",
        "%d %b %Y %H:%M",
        "%b %d, %Y, %I:%M %p",
        "%b %d, %Y %I:%M %p",
        "%b %d, %Y, %H:%M",
        "%d %B %Y, %H:%M",
        "%B %d, %Y, %I:%M %p",
        "%d/%m/%Y, %H:%M",
    ];
    // Drop a leading weekday and the word "at"; normalize narrow spaces.
    let cleaned = label
        .replace(['\u{202f}', '\u{a0}'], " ")
        .replace(" at ", ", ");
    let cleaned = match cleaned.split_once(", ") {
        Some((first, rest)) if first.chars().all(char::is_alphabetic) => rest.to_owned(),
        _ => cleaned,
    };
    let cleaned = cleaned.trim();
    FORMATS.iter().find_map(|format| {
        NaiveDateTime::parse_from_str(cleaned, format)
            .ok()
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .map(|time| time.timestamp_millis())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_each_message_once() {
        let store = Store::open_in_memory().unwrap();
        let chat = Conversation {
            app: "whatsapp",
            key: "Ben",
            title: "Ben",
            work: None,
        };
        let messages = [
            NewMessage {
                sender: "Ben",
                text: "Can you send it?",
                sent_label: Some("14:00"),
                sent_at: None,
            },
            NewMessage {
                sender: ME,
                text: "Sure, by Thursday",
                sent_label: Some("14:02"),
                sent_at: None,
            },
        ];
        let (id, added) = record(&store, &chat, &messages, 1).unwrap();
        assert_eq!(added, 2);
        let (again, added) = record(&store, &chat, &messages, 2).unwrap();
        assert_eq!((again, added), (id, 0));

        set_work(&store, id, true).unwrap();
        // Reading it again keeps the decision.
        record(&store, &chat, &messages, 3).unwrap();
        let work: Option<bool> = store
            .conn()
            .query_row("SELECT work FROM conversation WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(work, Some(true));
    }

    #[test]
    fn reads_app_time_labels() {
        let expected = Local
            .with_ymd_and_hms(2026, 10, 13, 14, 2, 0)
            .unwrap()
            .timestamp_millis();
        for label in [
            "Tue, 13 Oct 2026, 14:02",
            "Tue, Oct 13, 2026, 2:02 PM",
            "Tue, Oct 13, 2026 at 2:02\u{202f}PM",
            "13 Oct 2026, 14:02",
        ] {
            assert_eq!(parse_full_time(label), Some(expected), "{label}");
        }
        assert_eq!(parse_full_time("14:02"), None);
    }
}
