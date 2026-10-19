//! Freebyrd's memory: what the person worked on, kept in one SQLite file on
//! their Mac (`store`).
//!
//! `ingest` reads sources where they already are: Claude Code sessions,
//! browser history, and the pages the browser extension saved. `digest`
//! condenses a day of activity, `recap` has Claude write it up as a card,
//! `cards` are what the notch shows, `recall` picks what the notch sends
//! along with a question, and `day` plans the person's day as blocks to add
//! to their calendar. `redact` takes secrets out before anything is
//! stored or sent. `routines` spots work the person repeats and suggests
//! skills and scheduled jobs for it. `moments` brings up ideas at the
//! moment they help. `jev` asks TypeSafe's Jev for typed judgments: it
//! ranks those ideas, and shadows other steps in trial.
//! `drafts` prepares what the person owes the day before it is due.
//! `todos` is the to-do list behind the notch's checklist icon, and
//! `people` joins the names and addresses Freebyrd sees into one page per
//! person. `brief` is what a Claude Code session starting in a project
//! folder would be told from the memory, and `threads` the same as a short
//! index of live threads with one thread's details on request; `connector`
//! hands them to Claude Code with the person's consent, receipts and a cap.

use std::path::PathBuf;

pub mod agenda;
pub mod asks;
pub mod brief;
pub mod calendar;
pub mod cards;
pub mod connector;
pub mod connectors;
pub mod conversations;
pub mod day;
pub mod digest;
pub mod drafts;
pub mod exclusions;
pub mod ingest;
pub mod jev;
pub mod moments;
pub mod night;
pub mod people;
pub mod plans;
pub mod progress;
pub mod promises;
pub mod quick;
pub mod recall;
pub mod recap;
pub mod redact;
pub mod routines;
pub mod screen;
pub mod store;
pub mod threads;
pub mod time;
pub mod todos;
pub mod transcripts;

pub use store::Store;

pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The person's full name, from their Mac account ("Abhishek Mankotia").
/// Meeting notes and emails name people; this says which one is them.
pub fn person_name() -> Option<String> {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let out = std::process::Command::new("/usr/bin/id")
            .arg("-F")
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !name.is_empty()).then_some(name)
    })
    .clone()
}

/// Cuts `text` to at most `max` characters, marking the cut with an ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_owned(),
    }
}

/// `text` on one line: runs of whitespace, newlines included, become one space.
pub fn one_line(text: &str, max: usize) -> String {
    truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_character_boundaries() {
        assert_eq!(truncate("héllo wörld", 5), "héllo…");
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(one_line("a\n  b\tc", 10), "a b c");
    }
}
