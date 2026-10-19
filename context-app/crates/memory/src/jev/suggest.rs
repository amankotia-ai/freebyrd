//! Whether a repeated piece of work deserves a suggestion. Once a day
//! Claude drafts a skill or a scheduled job for up to three patterns the
//! detectors found (`routines::suggest`), and says when a skill the person
//! has already does it or no fix is worth it. Jev answers the same two
//! things for each pattern from the same minimal evidence (its label, how
//! often, the first 100 characters of examples, facts): which installed
//! skill already covers it, picked from all of them, and how worth
//! automating it is. Both go to `jev/suggest.jsonl` beside Claude's outcome;
//! `report` adds what the person did with the card. Live, patterns Jev finds
//! covered or not worth it would not be drafted.

use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Store;
use crate::redact::redact;
use crate::routines::detect::Found;
use crate::routines::skills::{self, Installed};

const LOG: &str = "suggest";
/// A Choice holds 255 options: the skills and "none".
const MAX_SKILLS: usize = 254;
const NONE: &str = "none";
/// Live, a pattern below this would not be drafted.
pub const WORTH_AT: f64 = 1.5;

#[derive(Debug, Serialize, Deserialize)]
struct Judged {
    at: i64,
    pattern: i64,
    label: String,
    /// The installed skill Jev thinks already does it, or "none".
    covered_by: String,
    covered_p: f64,
    /// 0 (not worth it) to 3 (very useful).
    worth: f64,
    claude: String,
    ms: u64,
}

/// Judges the patterns Claude was just asked to draft for (`outcomes`: what
/// came of each, by pattern ID) and logs it. Returns a line for the log;
/// `None` without a key.
pub fn shadow(
    batch: &[(i64, Found)],
    installed: &[Installed],
    outcomes: &[(i64, String)],
    now_ms: i64,
) -> Option<String> {
    if batch.is_empty() {
        return None;
    }
    let result = judge(batch, installed).and_then(|judged| {
        let Some((answers, ms)) = judged else {
            return Ok(None);
        };
        let mut skip = 0;
        for ((id, found), (covered_by, covered_p, worth)) in batch.iter().zip(answers) {
            skip += usize::from(covered_by != NONE || worth < WORTH_AT);
            let claude = outcomes
                .iter()
                .find(|(pattern, _)| pattern == id)
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or_default();
            super::append(
                LOG,
                &Judged {
                    at: now_ms,
                    pattern: *id,
                    label: found.label.clone(),
                    covered_by,
                    covered_p,
                    worth,
                    claude,
                    ms,
                },
            )?;
        }
        Ok(Some(format!(
            "jev: weighed {} patterns, {skip} Jev would not have drafted",
            batch.len()
        )))
    });
    super::line("suggest", result)
}

type Judgment = (String, f64, f64);

fn judge(batch: &[(i64, Found)], installed: &[Installed]) -> Result<Option<(Vec<Judgment>, u64)>> {
    let state = json!({
        "about": "Work the person repeats, found in their Claude Code sessions, browser \
                  history and meetings. A skill is a saved instruction for Claude Code; a \
                  scheduled job runs one at set times.",
    });
    let mut questions = Map::new();
    for (index, (_, found)) in batch.iter().enumerate() {
        let pattern = pattern(found);
        questions.insert(format!("c{index}"), covered(&pattern, found, installed));
        questions.insert(format!("w{index}"), worth(&pattern));
    }
    let started = Instant::now();
    let Some(response) = super::ask(LOG, &state, &questions)? else {
        return Ok(None);
    };
    let ms = started.elapsed().as_millis() as u64;
    let answers = (0..batch.len())
        .map(|index| {
            let covered = response.answers.get(&format!("c{index}"));
            let covered_by = covered
                .and_then(|answer| answer.choice.clone())
                .unwrap_or_else(|| NONE.to_owned());
            let covered_p = covered
                .and_then(|answer| answer.probabilities.get(&covered_by).copied())
                .unwrap_or_default();
            let worth = response
                .answers
                .get(&format!("w{index}"))
                .and_then(|answer| answer.score)
                .unwrap_or_default();
            (covered_by, covered_p, worth)
        })
        .collect();
    Ok(Some((answers, ms)))
}

/// The pattern as the drafting prompt describes it.
fn pattern(found: &Found) -> Value {
    json!({
        "kind": found.kind.describe(),
        "label": redact(&found.label),
        "how_often": format!("{} times in {} sessions, {}", found.count, found.sessions, found.span()),
        "seen": found.examples.iter().take(8).map(|example| redact(&example.text)).collect::<Vec<_>>(),
        "facts": found.facts.iter().map(|fact| redact(fact)).collect::<Vec<_>>(),
    })
}

/// Which installed skill already does it, from all of them (the ones that
/// share the most words with it when there are too many), or none.
fn covered(pattern: &Value, found: &Found, installed: &[Installed]) -> Value {
    let picked: Vec<&Installed> = if installed.len() <= MAX_SKILLS {
        installed.iter().collect()
    } else {
        let mut picked = skills::relevant(installed, &found.signature, MAX_SKILLS);
        for skill in installed {
            if picked.len() == MAX_SKILLS {
                break;
            }
            if !picked.iter().any(|known| known.name == skill.name) {
                picked.push(skill);
            }
        }
        picked
    };
    let descriptions: Vec<(String, String)> = picked
        .iter()
        .map(|skill| (skill.name.clone(), crate::one_line(&skill.description, 200)))
        .collect();
    let mut options: Vec<(&str, &str)> = descriptions
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();
    options.push((NONE, "No installed skill already does this work."));
    super::choice(
        json!({
            "pattern": pattern,
            "question": "Which installed skill already does the work in `pattern`, so that a new \
                         skill or scheduled job for it would repeat one the person has?",
        }),
        &options,
    )
}

fn worth(pattern: &Value) -> Value {
    super::score(
        json!({
            "pattern": pattern,
            "question": "How much would a skill or scheduled job for `pattern` help the person?",
        }),
        &[
            "Not worth it: rare, already quick, or too vague to hand to Claude.",
            "Minor: a small saving now and then.",
            "Useful: saves noticeable time or attention most weeks.",
            "Very useful: frequent, tedious and clear enough to hand to a skill or a \
             scheduled job.",
        ],
    )
}

/// Each judged pattern beside Claude's outcome and the person's answer.
pub fn report(store: &Store) -> Result<Vec<String>> {
    let judged: Vec<Judged> = super::entries(LOG);
    let mut lines = Vec::new();
    for pattern in &judged {
        let state: String = store
            .conn()
            .query_row(
                "SELECT state FROM pattern WHERE id = ?1",
                [pattern.pattern],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "gone".into());
        let covered = if pattern.covered_by == NONE {
            "no skill covers it".to_owned()
        } else {
            format!(
                "covered by {} ({:.2})",
                pattern.covered_by, pattern.covered_p
            )
        };
        lines.push(format!(
            "{}  {}: Jev {covered}, worth {:.1}/3 | Claude: {} | now {state}",
            super::when(pattern.at),
            pattern.label,
            pattern.worth,
            pattern.claude
        ));
    }
    let skipped = judged
        .iter()
        .filter(|p| p.covered_by != NONE || p.worth < WORTH_AT)
        .count();
    lines.push(format!(
        "{} patterns: Jev would not have drafted {skipped} (covered, or worth under {WORTH_AT})",
        judged.len()
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::detect::{Example, Kind};

    fn found() -> Found {
        Found {
            kind: Kind::Ask,
            key: "ask:push".into(),
            signature: vec!["push".into(), "github".into()],
            label: "push changes to github".into(),
            count: 12,
            sessions: 9,
            days: Vec::new(),
            examples: vec![Example {
                at: 0,
                text: "push changes to github".into(),
            }],
            facts: Vec::new(),
            readers: Vec::new(),
            window: None,
            score: 1.0,
        }
    }

    #[test]
    fn covered_offers_every_skill_and_none() {
        let installed: Vec<Installed> = (0..300)
            .map(|n| Installed {
                name: format!("skill-{n}"),
                description: if n == 299 {
                    "push to github".into()
                } else {
                    "other".into()
                },
            })
            .collect();
        let found = found();
        let q = covered(&pattern(&found), &found, &installed);
        let options = q["criteria"].as_object().unwrap();
        assert_eq!(options.len(), MAX_SKILLS + 1);
        assert!(
            options.contains_key("skill-299"),
            "the skill sharing words is kept"
        );
        assert!(options.contains_key(NONE));
        assert_eq!(
            worth(&pattern(&found))["criteria"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert!(
            shadow(&[(1, found)], &installed, &[], 0).is_none(),
            "tests have no key"
        );
    }
}
