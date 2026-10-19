//! Which model answers a notch question. Most questions are quick lookups
//! (what is on the screen, a date, a name), which Haiku 5.5 answers fastest.
//! Questions that ask for things to be put together (what a meeting
//! covered, a recap, a draft to send, a decision) go to Sonnet 5.5, which
//! reads a long memory more carefully. The choice is made on the Mac from
//! the question's words, so it adds no wait.
//!
//! Measured on "what was discussed in my last meeting with Ben?" with the
//! same 13,000-character memory (2026-10-13): Haiku 5.5 answered in 4.8 s,
//! Sonnet 5.5 in 5.6 s, both at low effort; Sonnet kept more of the summary.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub model: &'static str,
    pub effort: &'static str,
    /// The model's name, for the log.
    pub name: &'static str,
    /// Why the stronger model was picked; `None` for a quick lookup.
    pub reason: Option<&'static str>,
}

const QUICK: Choice = Choice {
    model: claude_cli::FAST_MODEL,
    effort: "low",
    name: "Haiku 5.5",
    reason: None,
};

/// Low effort keeps it nearly as fast as Haiku; Anthropic suggests starting
/// chat on Sonnet 5.5 at low.
const THOUGHTFUL_MODEL: &str = "claude-sonnet-5-5";

static SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(summar\w*|recap\w*|overview|catch me up|status|progress|this week|last week|so far|what did (i|we)|where (are|do) (we|things)|what('s| is| are) (left|open|pending|outstanding))\b",
    )
    .expect("valid pattern")
});

static DRAFT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(draft\w*|write|rewrite|reply|respond|compose|email|message (to|for)|follow[- ]?up|proposal)\b",
    )
    .expect("valid pattern")
});

/// Decisions, and whether what is on screen fits the person's work ("can we
/// use this in any project?"), which takes their projects into account.
static REASONING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(why|should (i|we)|prioriti\w*|plan\w*|compare|decide|decision|explain|pros and cons|trade-?offs?|next steps?|(can|could) (i|we) use|(would|will|does|could) (this|it|that) (help|work|fit)|worth (it|using|trying|paying|buying)|useful (for|to|in)|good fit|(any|my|our) projects?)\b",
    )
    .expect("valid pattern")
});

/// The model for `question`. A follow-up ("make it shorter") is judged with
/// the question it follows, `earlier`, so it stays with the same model.
pub fn for_question(question: &str, earlier: Option<&str>) -> Choice {
    let text = format!("{question} {}", earlier.unwrap_or_default());
    let reason = if memory::recall::about_meetings(&text) || memory::recall::about_words(&text) {
        Some("a question about meetings")
    } else if DRAFT.is_match(&text) {
        Some("something to write")
    } else if SUMMARY.is_match(&text) {
        Some("a summary")
    } else if REASONING.is_match(&text) {
        Some("a question that needs weighing up")
    } else {
        None
    };
    match reason {
        Some(reason) => Choice {
            model: THOUGHTFUL_MODEL,
            effort: "low",
            name: "Sonnet 5.5",
            reason: Some(reason),
        },
        None => QUICK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_stay_quick() {
        for question in [
            "what does this error mean?",
            "what time is the Acme demo?",
            "translate this",
            "who sent this?",
            "what does this tool do?",
            "how much does this cost?",
        ] {
            assert_eq!(for_question(question, None), QUICK, "{question}");
        }
    }

    #[test]
    fn putting_things_together_gets_sonnet() {
        for (question, reason) in [
            (
                "what was discussed in my last meeting with Ben?",
                "a question about meetings",
            ),
            ("draft a reply to Ben", "something to write"),
            ("catch me up on the website", "a summary"),
            ("what's left on Website V1?", "a summary"),
            (
                "should I do the persona pages first?",
                "a question that needs weighing up",
            ),
            (
                "can we use this in any project?",
                "a question that needs weighing up",
            ),
            (
                "would this help with my projects?",
                "a question that needs weighing up",
            ),
            (
                "is it worth paying for?",
                "a question that needs weighing up",
            ),
        ] {
            let choice = for_question(question, None);
            assert_eq!(choice.model, THOUGHTFUL_MODEL, "{question}");
            assert_eq!(choice.reason, Some(reason), "{question}");
        }
    }

    #[test]
    fn a_follow_up_keeps_the_model() {
        assert_eq!(
            for_question("make it shorter", Some("draft a reply to Ben")).model,
            THOUGHTFUL_MODEL
        );
        assert_eq!(
            for_question("and tomorrow?", Some("what's on screen?")),
            QUICK
        );
    }
}
