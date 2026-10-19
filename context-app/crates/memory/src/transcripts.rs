//! What was said in a meeting: its Fireflies transcript, kept as one row per
//! speaker's turn, so the notch can quote people. Read on demand, when a
//! question asks for someone's words (`connectors::before_question`): most
//! questions need only the summary, and a transcript is long (a 12-minute
//! call is about 5,000 characters, a 2-hour one about 70,000).

use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use rusqlite::params;

use crate::redact::redact;
use crate::store::Store;
use crate::{one_line, truncate};

/// A turn this long starts a new row even if the same person goes on.
const MAX_TURN_CHARS: usize = 600;
/// Turns shorter than this ("Yeah.", "Okay, sorry.") are never picked as
/// matches, only shown around one.
const MIN_MATCH_CHARS: usize = 20;
const MAX_MATCHES: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub at_seconds: i64,
    pub speaker: String,
    pub text: String,
}

/// Fireflies' transcript text, as turns: "[00:36 - 00:41] Abhishek
/// Mankotia: I can hear you", one sentence a line, with consecutive
/// sentences by the same speaker joined. Everything else in the text (the
/// title, participants, signed links to the recording) is left out.
pub fn parse(text: &str) -> Vec<Line> {
    static SENTENCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?m)^(?:Sentences:\s*)?\[(\d{1,3}):(\d{2})(?::(\d{2}))?\s*-[^\]]*\]\s*([^:\n]{1,80}):\s*(.*)$",
        )
        .expect("valid pattern")
    });
    let mut lines: Vec<Line> = Vec::new();
    for found in SENTENCE.captures_iter(text) {
        let number = |i: usize| found.get(i).and_then(|m| m.as_str().parse::<i64>().ok());
        // "12:24" is minutes and seconds; "1:02:10" adds hours.
        let at_seconds = match (number(1), number(2), number(3)) {
            (Some(h), Some(m), Some(s)) => h * 3_600 + m * 60 + s,
            (Some(m), Some(s), None) => m * 60 + s,
            _ => continue,
        };
        let speaker = one_line(&found[4], 80);
        let sentence = one_line(&found[5], MAX_TURN_CHARS);
        if speaker.is_empty() || sentence.is_empty() {
            continue;
        }
        match lines.last_mut() {
            Some(turn)
                if turn.speaker == speaker && turn.text.len() + sentence.len() < MAX_TURN_CHARS =>
            {
                turn.text.push(' ');
                turn.text.push_str(&sentence);
            }
            _ => lines.push(Line {
                at_seconds,
                speaker,
                text: sentence,
            }),
        }
    }
    lines
}

/// Whether a meeting's transcript has been kept.
pub fn has(store: &Store, conversation_id: i64) -> Result<bool> {
    Ok(store.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM meeting_line WHERE conversation_id = ?1)",
        params![conversation_id],
        |row| row.get(0),
    )?)
}

/// Keeps a meeting's transcript in place of any read before. Returns how
/// many turns were kept.
pub fn save(store: &Store, conversation_id: i64, lines: &[Line]) -> Result<usize> {
    let tx = store.transaction()?;
    store.conn().execute(
        "DELETE FROM meeting_line WHERE conversation_id = ?1",
        params![conversation_id],
    )?;
    let mut insert = store.conn().prepare_cached(
        "INSERT INTO meeting_line (conversation_id, at_seconds, speaker, text)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for line in lines {
        insert.execute(params![
            conversation_id,
            line.at_seconds,
            redact(&line.speaker),
            redact(&line.text)
        ])?;
    }
    drop(insert);
    tx.commit()?;
    Ok(lines.len())
}

/// The turns of a meeting that match `query` (an FTS query: a name matches
/// that person's turns, a word the turns that say it), each with the turns
/// just before and after it, since what answers a question often follows
/// the line that names its subject. The best matches are taken first until
/// `budget` characters are used, then shown in time order, as "[12:24] Ben
/// Merton: …".
pub fn matching(
    store: &Store,
    conversation_id: i64,
    query: &str,
    budget: usize,
) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT l.id FROM meeting_line_fts
         JOIN meeting_line l ON l.id = meeting_line_fts.rowid
         WHERE meeting_line_fts MATCH ?1 AND l.conversation_id = ?2 AND length(l.text) >= ?3
         ORDER BY rank LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![
            query,
            conversation_id,
            i64::try_from(MIN_MATCH_CHARS)?,
            i64::try_from(MAX_MATCHES)?
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let matches = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut neighbours = store.conn().prepare(
        "SELECT
           (SELECT max(id) FROM meeting_line WHERE conversation_id = ?1 AND id < ?2),
           (SELECT min(id) FROM meeting_line WHERE conversation_id = ?1 AND id > ?2)",
    )?;
    let mut line = store
        .conn()
        .prepare("SELECT at_seconds, speaker, text FROM meeting_line WHERE id = ?1")?;
    let mut chosen: Vec<(i64, String)> = Vec::new();
    let mut used = 0;
    'matches: for id in matches {
        let (before, after): (Option<i64>, Option<i64>) = neighbours
            .query_row(params![conversation_id, id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        for id in [before, Some(id), after].into_iter().flatten() {
            if chosen.iter().any(|(kept, _)| *kept == id) {
                continue;
            }
            let (at, speaker, text): (i64, String, String) = line
                .query_row(params![id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
            let formatted = format!(
                "[{}] {speaker}: {}",
                clock(at),
                truncate(&text, MAX_TURN_CHARS)
            );
            if used + formatted.len() + 1 > budget {
                break 'matches;
            }
            used += formatted.len() + 1;
            chosen.push((id, formatted));
        }
    }
    chosen.sort_unstable_by_key(|(id, _)| *id);
    Ok(chosen.into_iter().map(|(_, line)| line).collect())
}

/// "12:24", or "1:02:10" past the first hour.
fn clock(seconds: i64) -> String {
    let (h, m, s) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{self, Conversation, NewMessage};

    const TRANSCRIPT: &str = "Id: 01M208J9BENJ28W8YYDR0PDB4A
Speakers: Shilpa V, Abhishek Mankotia
Sentences: [00:00 - 00:00] Shilpa V: Sa.
[00:26 - 00:26] Abhishek Mankotia: Hello?
[01:11 - 01:19] Abhishek Mankotia: The deal stage is not happening for me because I don't have developer access.
[01:37 - 01:38] Shilpa V: Okay, sorry.
[02:43 - 02:45] Abhishek Mankotia: Sure.
[03:20 - 03:23] Shilpa V: So is it okay if I give you that access after this call?
[1:02:10 - 1:02:12] Abhishek Mankotia: We'll swap the Make account for yours once the pricing works.
Title: Website Form Automation Review
Audio Url: https://cdn.fireflies.ai/x/audio.mp3?Expires=1&Signature=abc
";

    #[test]
    fn reads_turns_and_drops_the_rest() {
        let lines = parse(TRANSCRIPT);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].speaker, "Shilpa V");
        // Consecutive sentences by one speaker are one turn.
        assert_eq!(lines[1].at_seconds, 26);
        assert!(lines[1].text.starts_with("Hello? The deal stage"));
        assert_eq!(lines[5].at_seconds, 3_730);
        assert!(lines.iter().all(|line| !line.text.contains("Signature")));
    }

    #[test]
    fn finds_what_someone_said() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = conversations::record(
            &store,
            &Conversation {
                app: "fireflies",
                key: "m1",
                title: "Website Form Automation Review",
                work: Some(true),
            },
            &[NewMessage {
                sender: "summary",
                text: "A call about Attio access.",
                sent_label: None,
                sent_at: Some(1),
            }],
            1,
        )
        .unwrap();
        assert!(!has(&store, id).unwrap());
        assert_eq!(save(&store, id, &parse(TRANSCRIPT)).unwrap(), 6);
        // Read again: kept once.
        save(&store, id, &parse(TRANSCRIPT)).unwrap();
        assert!(has(&store, id).unwrap());

        let lines = matching(&store, id, "\"shilpa\" OR \"access\"", 2_000).unwrap();
        assert_eq!(
            lines,
            [
                "[0:00] Shilpa V: Sa.",
                "[0:26] Abhishek Mankotia: Hello? The deal stage is not happening for me because I don't have developer access.",
                "[1:37] Shilpa V: Okay, sorry.",
                "[2:43] Abhishek Mankotia: Sure.",
                "[3:20] Shilpa V: So is it okay if I give you that access after this call?",
                "[1:02:10] Abhishek Mankotia: We'll swap the Make account for yours once the pricing works.",
            ]
        );
        let lines = matching(&store, id, "\"pricing\"", 2_000).unwrap();
        assert_eq!(
            lines[1],
            "[1:02:10] Abhishek Mankotia: We'll swap the Make account for yours once the pricing works."
        );
        assert!(matching(&store, id, "\"pricing\"", 10).unwrap().is_empty());
    }
}
