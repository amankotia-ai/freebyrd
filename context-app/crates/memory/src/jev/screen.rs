//! Whether lines seen on screen set up an event. The screen watcher sends
//! lines that look like an event (a day or time and an event word nearby)
//! to Claude, which fills in an event for each real one (`screen::nudge`).
//! Many are not ("lunch was great on Thursday"). Jev judges the same lines
//! (`jev/screen.jsonl`), beside how many events Claude found in each; live,
//! lines Jev is sure hold no event would not be sent to Claude at all.
//! `report` counts the Claude calls that would have been saved and the
//! events that would have been missed.

use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::redact::redact;
use crate::screen::Snippet;

const LOG: &str = "screen";
/// Live, a group of lines below this would not go to Claude.
pub const SKIP_BELOW: f64 = 0.15;

#[derive(Debug, Serialize, Deserialize)]
struct Judged {
    at: i64,
    app: String,
    /// One per group of lines, in order.
    groups: Vec<Group>,
    ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Group {
    /// The probability that the lines set up or mention an event.
    event: f64,
    /// How many events Claude found in them.
    claude_found: usize,
}

/// Judges the lines Claude was just given (`found`: how many events Claude
/// found in each) and logs it. Returns a line for the log; `None` without a
/// key.
pub fn shadow(snippets: &[Snippet], found: &[usize], now: DateTime<Local>) -> Option<String> {
    let result = judge(snippets, now).and_then(|judged| {
        let Some((events, ms)) = judged else {
            return Ok(None);
        };
        let groups: Vec<Group> = events
            .iter()
            .zip(found)
            .map(|(event, claude_found)| Group {
                event: *event,
                claude_found: *claude_found,
            })
            .collect();
        let skipped = groups.iter().filter(|g| g.event < SKIP_BELOW).count();
        let line = format!(
            "jev: {} screen groups, {skipped} Jev would not send to Claude",
            groups.len()
        );
        super::append(
            LOG,
            &Judged {
                at: now.timestamp_millis(),
                app: snippets.first().map(|s| s.app.clone()).unwrap_or_default(),
                groups,
                ms,
            },
        )?;
        Ok(Some(line))
    });
    super::line("screen", result)
}

/// For each snippet, the probability that it sets up or mentions an event,
/// and how long that took.
fn judge(snippets: &[Snippet], now: DateTime<Local>) -> Result<Option<(Vec<f64>, u64)>> {
    if snippets.is_empty() {
        return Ok(None);
    }
    let state = json!({
        "now": now.format("%A %-d %B %Y, %H:%M").to_string(),
        "the_person": crate::person_name().unwrap_or_else(|| "the person".into()),
    });
    let questions: Map<String, Value> = snippets
        .iter()
        .enumerate()
        .map(|(index, snippet)| (format!("s{index}"), question(snippet)))
        .collect();
    let started = Instant::now();
    let Some(response) = super::ask(LOG, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    let events = (0..snippets.len())
        .map(|index| {
            response
                .answers
                .get(&format!("s{index}"))
                .and_then(|answer| answer.noul)
                .unwrap_or(1.0)
        })
        .collect();
    Ok(Some((events, ms)))
}

fn question(snippet: &Snippet) -> Value {
    super::noul(
        json!({
            "screen": {
                "app": snippet.app,
                "window": redact(&crate::one_line(&snippet.window, 120)),
                "lines": redact(&snippet.text),
            },
            "question": "Do the lines in `screen` set up or mention a specific event, meeting, \
                         call, deadline or plan at a date or time that has not passed, which the \
                         person might want on their calendar?",
        }),
        "A concrete upcoming event, meeting, call, deadline, booking or plan with a day or \
         time, proposed, agreed or announced.",
        "No upcoming event: past events, times without a plan, general talk about days, \
         timestamps, schedules of things the person would not attend, or interface text.",
    )
}

/// What the shadow saw, a pass a line, and what it would have changed.
pub fn report() -> Result<Vec<String>> {
    let passes: Vec<Judged> = super::entries(LOG);
    let mut lines = Vec::new();
    let (mut groups, mut saved, mut missed, mut calls_saved) = (0, 0, 0, 0);
    for pass in &passes {
        let skipped: Vec<&Group> = pass
            .groups
            .iter()
            .filter(|g| g.event < SKIP_BELOW)
            .collect();
        groups += pass.groups.len();
        saved += skipped.iter().filter(|g| g.claude_found == 0).count();
        missed += skipped.iter().map(|g| g.claude_found).sum::<usize>();
        if skipped.len() == pass.groups.len() {
            calls_saved += 1;
        }
        let shown: Vec<String> = pass
            .groups
            .iter()
            .map(|g| format!("Jev {:.2} / Claude {}", g.event, g.claude_found))
            .collect();
        lines.push(format!(
            "{}  {}: {} | {} ms",
            super::when(pass.at),
            pass.app,
            shown.join(", "),
            pass.ms
        ));
    }
    lines.push(format!(
        "{} passes, {groups} groups: below {SKIP_BELOW}, {saved} held no event for Claude \
         either and {missed} events Claude found would have been missed; {calls_saved} \
         Claude calls would not have been made",
        passes.len()
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_carry_each_group_redacted_and_no_key_means_no_shadow() {
        let snippet = Snippet {
            app: "WhatsApp".into(),
            window: "Ben".into(),
            text: "can we do the demo Thursday at 3pm? key sk-ant-api03-abcdefghijklmnopqrstuvwxyz"
                .into(),
        };
        let q = question(&snippet);
        assert_eq!(q["type"], "noul");
        assert_eq!(
            q["instructions"]["screen"]["lines"],
            "can we do the demo Thursday at 3pm? key [API key]"
        );
        assert!(shadow(&[snippet], &[1], Local::now()).is_none());
    }

    /// Made-up lines to TypeSafe:
    /// `FREEBYRD_JEV_LIVE=1 cargo test -p memory jev::screen::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        let snippet = |text: &str| Snippet {
            app: "WhatsApp".into(),
            window: "Sam".into(),
            text: text.into(),
        };
        let snippets = [
            snippet("Sam: can we do the product demo with Acme on Thursday at 3pm?\nme: works"),
            snippet("Sam: lunch was great on Thursday, thanks again!"),
            snippet("Last seen today at 14:02\nSam: ok"),
            snippet("me: I'll send the revised proposal by Friday"),
        ];
        let (events, ms) = judge(&snippets, Local::now()).unwrap().expect("a key");
        println!("{events:.2?} in {ms} ms");
        assert!(events[0] > 0.5 && events[1] < 0.5 && events[2] < 0.5);
    }
}
