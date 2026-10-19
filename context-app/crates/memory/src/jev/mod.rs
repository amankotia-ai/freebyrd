//! Jev, TypeSafe's System One model: typed judgments over text (the
//! probability of a yes, a pick from a set, a place on a scale) in a
//! fraction of a second, at $0.042 per million input tokens, output free.
//! It writes nothing: Claude still writes every card, recap and answer.
//!
//! The person approved it on 2026-10-13. One use is live: ranking ideas
//! for a moment (`moments`). The others below are shadows first: Jev's
//! answer goes to a log on this Mac (`jev/<use>.jsonl`) beside what
//! Freebyrd did, changes nothing the person sees, and a use goes live only
//! after they have read its report (`freebyrd-memory jev report USE`).
//! Two shadows for X bookmarks (`ideas`, `situations`) were removed with
//! the X suggestions on 2026-10-16.
//!
//! - `chats`: whether a chat is about work, guessed before the person
//!   answers the Work / Personal card.
//! - `screen`: whether lines the screen watcher sends Claude set up an event
//!   at all, so lines that do not could stay on the Mac.
//! - `promises`: whose each promise Claude found in a work chat is, the
//!   person's (it goes on their calendar) or someone else's.
//! - `suggest`: whether a repeated piece of work is already covered by a
//!   skill the person has, and how worth automating it is.
//! - `projects`: which project each page, meeting, chat and plan belongs
//!   to, for the context graph (`run_due`, on a thread of its own).
//! - `cards`: how much each waiting card needs the person today.
//!
//! Requests go to TypeSafe over HTTPS with `/usr/bin/curl`. The key lives in
//! the Keychain (service `freebyrd-typesafe`) and reaches curl on stdin,
//! never in its arguments. Without a key nothing is sent, and tests never
//! read it (except the live ones, run by hand). Each request is counted by
//! use in `jev/usage.json`; a day's requests stop at `DAILY_TOKENS`, and
//! shadows at their shares of it, so they never starve the live use.

pub mod cards;
pub mod chats;
pub mod projects;
pub mod promises;
pub mod screen;
pub mod suggest;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Local;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-latest";
const KEYCHAIN_SERVICE: &str = "freebyrd-typesafe";
/// A request may hold 64k tokens. Requests are cut well below that,
/// counting a token as 3 characters of JSON.
const MAX_REQUEST_CHARS: usize = 120_000;
const TIMEOUT_SECONDS: &str = "20";
/// Waits before trying again when TypeSafe is busy (429, 529).
const RETRY_AFTER: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];
/// A day's input tokens stop here, for every use together: $0.42 at today's
/// price. Not many times what the shadows use: on 2026-10-13 the day
/// reached 10.2 million by 13:43, about 5.1 million of it the projects
/// shadow tagging a week of pages at once and 2.3 million the X situations
/// shadow (removed since) asking again about ideas it had weighed (README
/// "Jev"), and moments ranked by shared words for the rest of the day.
const DAILY_TOKENS: u64 = 10_000_000;
/// Uses the person sees. Shadows cannot spend into what is left for them.
const LIVE: &[&str] = &["moments"];
/// All shadows, and backtests by hand, stop here together, so at least 6
/// million tokens a day stay for live uses.
const SHADOWS_TOKENS: u64 = 4_000_000;
/// One shadow stops here, so it cannot starve the other shadows either.
const SHADOW_TOKENS: u64 = 2_000_000;
/// Dollars per million input tokens (output is free).
const PRICE_PER_MILLION: f64 = 0.042;
/// Before a request is sent, it is weighed against the budget at a token
/// for every 3 characters of JSON, a little more than TypeSafe counts.
const CHARS_PER_TOKEN: usize = 3;
/// Set to 1 to let the live tests (`--ignored`) read the key.
const LIVE_TEST: &str = "FREEBYRD_JEV_LIVE";

/// The TypeSafe key from the Keychain, if the person added one. Tests get
/// none unless `FREEBYRD_JEV_LIVE=1`, so `cargo test` never reaches TypeSafe.
pub fn key() -> Option<String> {
    if cfg!(test) && std::env::var(LIVE_TEST).as_deref() != Ok("1") {
        return None;
    }
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let key = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !key.is_empty()).then_some(key)
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub model: String,
    pub answers: HashMap<String, Answer>,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// One answer. Which fields are set depends on the question's type: `noul`
/// for a yes/no, `choice` and `probabilities` for a pick, `score` for a
/// scale; `confidence` for the last two.
#[derive(Debug, Deserialize)]
pub struct Answer {
    pub noul: Option<f64>,
    pub choice: Option<String>,
    #[serde(default)]
    pub probabilities: HashMap<String, f64>,
    pub score: Option<f64>,
    pub confidence: Option<f64>,
}

/// Asks Jev `questions` about `state` for the use `name` (`moments`, or a
/// shadow's name) with the Keychain's key: `None` without a key. Nothing is
/// sent when the request would take the day past `DAILY_TOKENS`, or a
/// shadow past its share; each request sent is counted for `name`.
pub(crate) fn ask(
    name: &str,
    state: &Value,
    questions: &Map<String, Value>,
) -> Result<Option<Response>> {
    let Some(key) = key() else {
        return Ok(None);
    };
    let bodies = bodies(state, questions);
    let estimate = (bodies.iter().map(String::len).sum::<usize>() / CHARS_PER_TOKEN) as u64;
    if let Some(over) = refusal(&usage_on(&today()), name, estimate) {
        return Err(over.into());
    }
    // Counted on the day the answer came: one sent just before midnight
    // must not write over the new day's counts.
    send(&key, &bodies, &mut |tokens, chars| {
        let _ = record(&today(), name, tokens, chars);
    })
    .map(Some)
}

/// Why a request was not sent: the day's tokens, or a shadow's share of
/// them, would run out.
#[derive(Debug)]
pub(crate) struct OverBudget(String);

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OverBudget {}

/// Why a request of about `estimate` tokens for `name` may not be sent
/// after `usage`, if it may not.
fn refusal(usage: &DayUsage, name: &str, estimate: u64) -> Option<OverBudget> {
    let used = usage.input_tokens;
    if used + estimate > DAILY_TOKENS {
        return Some(OverBudget(format!(
            "Jev's daily budget of {DAILY_TOKENS} tokens is used up ({used} used, this request \
             about {estimate})"
        )));
    }
    if LIVE.contains(&name) {
        return None;
    }
    let mine = usage.by_use.get(name).map_or(0, |use_| use_.input_tokens);
    if mine + estimate > SHADOW_TOKENS {
        return Some(OverBudget(format!(
            "the {name} shadow's share of {SHADOW_TOKENS} tokens a day is used up ({mine} used, \
             this request about {estimate})"
        )));
    }
    let shadows = usage.shadows();
    if shadows + estimate > SHADOWS_TOKENS {
        return Some(OverBudget(format!(
            "the shadows' share of {SHADOWS_TOKENS} tokens a day is used up ({shadows} used, this \
             request about {estimate}); the rest is kept for moments"
        )));
    }
    None
}

/// A shadow's result as a log line. A failure is a line too: a shadow never
/// stops the step it watches.
pub(crate) fn line(name: &str, result: Result<Option<String>>) -> Option<String> {
    match result {
        Ok(line) => line,
        Err(error) => failed(name, &format!("jev: {name} shadow failed"), &error),
    }
}

/// The log line for a shadow's failure, `what` first. A budget refusal is
/// said once a day for each use, not at every pass that meets it.
pub(crate) fn failed(name: &str, what: &str, error: &anyhow::Error) -> Option<String> {
    static SAID: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    if error.downcast_ref::<OverBudget>().is_some() {
        let mut said = SAID.lock().unwrap_or_else(PoisonError::into_inner);
        if !said.insert(format!("{} {name}", today())) {
            return None;
        }
    }
    Some(format!("{what}: {error:#}"))
}

/// Prints a shadow's line where the app's log lines go, for steps whose
/// own result has no room for it.
pub(crate) fn say(line: Option<String>) {
    if let Some(line) = line {
        eprintln!("freebyrd: memory: {line}");
    }
}

/// A yes/no question; `yes` and `no` say where the line falls.
pub(crate) fn noul(instructions: Value, yes: &str, no: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": instructions,
        "criteria": { "true": yes, "false": no },
    })
}

/// A pick from `options`, each with what it means.
pub(crate) fn choice(instructions: Value, options: &[(&str, &str)]) -> Value {
    let criteria: Map<String, Value> = options
        .iter()
        .map(|(option, meaning)| ((*option).to_owned(), json!(meaning)))
        .collect();
    json!({ "type": "choice", "instructions": instructions, "criteria": criteria })
}

/// A place on a scale of `levels`, lowest first.
pub(crate) fn score(instructions: Value, levels: &[&str]) -> Value {
    json!({ "type": "score", "instructions": instructions, "criteria": levels })
}

/// Asks `questions` (TypeSafe question objects by id) about `state`. Large
/// sets go as several requests, each with the whole state; the answers come
/// back as one. Counted for no use: `ask` is the way in for Freebyrd.
pub fn evaluate(key: &str, state: &Value, questions: &Map<String, Value>) -> Result<Response> {
    send(key, &bodies(state, questions), &mut |_, _| {})
}

/// The requests for `questions` about `state`, as JSON.
fn bodies(state: &Value, questions: &Map<String, Value>) -> Vec<String> {
    batches(state, questions)
        .into_iter()
        .map(|batch| json!({ "state": state, "model": MODEL, "questions": batch }).to_string())
        .collect()
}

/// Sends `bodies` one after another, telling `count` each request's input
/// tokens and characters as it is answered, so a later failure still
/// counts what was spent.
fn send(key: &str, bodies: &[String], count: &mut dyn FnMut(u64, usize)) -> Result<Response> {
    let mut all: Option<Response> = None;
    for body in bodies {
        let text = post(key, body)?;
        let response: Result<Response> = serde_json::from_str(&text).with_context(|| {
            format!(
                "Jev's answer was not understood: {}",
                crate::truncate(&text, 300)
            )
        });
        // An answer that cannot be read was still a request TypeSafe counted.
        let tokens = response
            .as_ref()
            .map_or((body.len() / CHARS_PER_TOKEN) as u64, |response| {
                response.usage.input_tokens
            });
        count(tokens, body.len());
        let response = response?;
        match &mut all {
            None => all = Some(response),
            Some(all) => {
                all.answers.extend(response.answers);
                all.usage.input_tokens += response.usage.input_tokens;
                all.usage.output_tokens += response.usage.output_tokens;
            }
        }
    }
    all.context("no questions to ask Jev")
}

/// `questions` cut into groups that fit a request beside `state`.
fn batches(state: &Value, questions: &Map<String, Value>) -> Vec<Map<String, Value>> {
    let room = MAX_REQUEST_CHARS.saturating_sub(state.to_string().len());
    let mut batches = Vec::new();
    let mut batch = Map::new();
    let mut used = 0;
    for (id, question) in questions {
        let size = id.len() + question.to_string().len();
        if !batch.is_empty() && used + size > room {
            batches.push(std::mem::take(&mut batch));
            used = 0;
        }
        batch.insert(id.clone(), question.clone());
        used += size;
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

/// Posts one request: the body of TypeSafe's answer, once it answers 200.
fn post(key: &str, body: &str) -> Result<String> {
    let config = curl_config(key, body);
    let mut tries = RETRY_AFTER.iter();
    loop {
        let (status, text) = curl(&config)?;
        match status {
            200 => return Ok(text),
            429 | 529 => match tries.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => bail!("Jev stayed busy (HTTP {status})"),
            },
            _ => bail!("Jev said HTTP {status}: {}", crate::truncate(&text, 300)),
        }
    }
}

/// curl's settings for one request, read from stdin, so the key and the
/// person's data never show in the process list.
fn curl_config(key: &str, body: &str) -> String {
    format!(
        "url = \"{ENDPOINT}\"\n\
         header = \"Authorization: Bearer {}\"\n\
         header = \"Content-Type: application/json\"\n\
         data-binary = \"{}\"\n\
         write-out = \"\\n%{{http_code}}\"\n",
        quoted(key),
        quoted(body)
    )
}

/// `text` as the inside of a double-quoted value in a curl config file.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Runs curl with `config` on stdin: the HTTP status and the body.
fn curl(config: &str) -> Result<(u16, String)> {
    let mut child = Command::new("/usr/bin/curl")
        .args(["--silent", "--show-error", "--max-time", TIMEOUT_SECONDS])
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start curl")?;
    child
        .stdin
        .take()
        .context("curl has no stdin")?
        .write_all(config.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", &text));
    let status = status.trim().parse().context("curl gave no HTTP status")?;
    Ok((status, body.to_owned()))
}

/// Jev's folder, beside the memory. Tests use a folder of their own.
fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        let name = format!("freebyrd-jev-test-{}", std::process::id());
        return Some(std::env::temp_dir().join(name));
    }
    Some(crate::home()?.join("Library/Application Support/Freebyrd/jev"))
}

/// Adds `entry` to a use's shadow log, `jev/<name>.jsonl`.
pub(crate) fn append<T: Serialize>(name: &str, entry: &T) -> Result<()> {
    let dir = dir().context("no home folder")?;
    fs::create_dir_all(&dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{name}.jsonl")))?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

/// A use's shadow log, oldest first; lines it cannot read are left out.
pub(crate) fn entries<T: DeserializeOwned>(name: &str) -> Vec<T> {
    let Some(file) = dir().and_then(|dir| fs::File::open(dir.join(format!("{name}.jsonl"))).ok())
    else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// A day's input tokens (`jev/usage.json`), all together and by use.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DayUsage {
    day: String,
    input_tokens: u64,
    /// Kept since 2026-10-13; tokens counted before that are in
    /// `input_tokens` only.
    #[serde(default)]
    by_use: BTreeMap<String, UseUsage>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct UseUsage {
    input_tokens: u64,
    requests: u64,
    /// Characters of JSON sent, to check `CHARS_PER_TOKEN` against.
    chars: u64,
}

impl DayUsage {
    /// What the shadows and backtests used, together.
    fn shadows(&self) -> u64 {
        self.by_use
            .iter()
            .filter(|(name, _)| !LIVE.contains(&name.as_str()))
            .map(|(_, use_)| use_.input_tokens)
            .sum()
    }
}

/// The calendar day the budget counts, like `2026-10-13`.
fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// `day`'s usage so far: none on a new day.
fn usage_on(day: &str) -> DayUsage {
    dir()
        .and_then(|dir| fs::read_to_string(dir.join("usage.json")).ok())
        .and_then(|text| serde_json::from_str::<DayUsage>(&text).ok())
        .filter(|usage| usage.day == day)
        .unwrap_or_else(|| DayUsage {
            day: day.to_owned(),
            ..DayUsage::default()
        })
}

/// Adds one request of `name`'s to `day`. Threads take turns, so two
/// requests answered together are both counted.
fn record(day: &str, name: &str, tokens: u64, chars: usize) -> Result<()> {
    static TURN: Mutex<()> = Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let mut usage = usage_on(day);
    usage.input_tokens += tokens;
    let use_ = usage.by_use.entry(name.to_owned()).or_default();
    use_.input_tokens += tokens;
    use_.requests += 1;
    use_.chars += chars as u64;
    let dir = dir().context("no home folder")?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("usage.json"), serde_json::to_string(&usage)?)?;
    Ok(())
}

/// Today's tokens by use, beside the budget and the shares, with what they
/// cost: for `freebyrd-memory jev usage`.
pub fn usage_report() -> Vec<String> {
    let usage = usage_on(&today());
    let tokens = |n: u64| format!("{:.2}M (${:.3})", n as f64 / 1e6, dollars(n));
    let mut lines = vec![format!(
        "{}: {} of the day's {}",
        usage.day,
        tokens(usage.input_tokens),
        tokens(DAILY_TOKENS)
    )];
    let mut by_use: Vec<(&String, &UseUsage)> = usage.by_use.iter().collect();
    by_use.sort_by_key(|(_, use_)| std::cmp::Reverse(use_.input_tokens));
    let line = |name: &str, use_: &UseUsage, cap: &str| {
        format!(
            "    {name}: {}{cap} in {} requests, {:.1} characters a token",
            tokens(use_.input_tokens),
            use_.requests,
            use_.chars as f64 / use_.input_tokens.max(1) as f64
        )
    };
    lines.push("  live, with whatever the shadows leave:".to_owned());
    for name in LIVE {
        let use_ = usage.by_use.get(*name).cloned().unwrap_or_default();
        lines.push(line(name, &use_, ""));
    }
    lines.push(format!(
        "  shadows and backtests: {} of their {}",
        tokens(usage.shadows()),
        tokens(SHADOWS_TOKENS)
    ));
    let cap = format!(" of {}", tokens(SHADOW_TOKENS));
    for (name, use_) in by_use
        .iter()
        .filter(|(name, _)| !LIVE.contains(&name.as_str()))
    {
        lines.push(line(name, use_, &cap));
    }
    let counted: u64 = usage.by_use.values().map(|use_| use_.input_tokens).sum();
    if usage.input_tokens > counted {
        lines.push(format!(
            "  not counted by use (sent before uses were counted): {}",
            tokens(usage.input_tokens - counted)
        ));
    }
    lines
}

fn dollars(tokens: u64) -> f64 {
    tokens as f64 / 1e6 * PRICE_PER_MILLION
}

/// When `at` (ms) was, for a report line.
pub(crate) fn when(at: i64) -> String {
    use chrono::TimeZone;
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|at| at.format("%a %e %b %H:%M").to_string())
        .unwrap_or_default()
}

/// Jev's background work that is due: tagging new things with a project,
/// scoring new cards.
/// Lines for the log. Run on a thread of its own, so a slow request never
/// holds up the memory's pass.
pub fn run_due(store: &crate::Store, now: chrono::DateTime<Local>) -> Vec<String> {
    let mut lines = projects::run_due(store, now);
    lines.extend(cards::run_due(store, now));
    lines
}

/// The report for `name`, a use's shadow log beside what happened.
pub fn report(store: &crate::Store, name: &str) -> Result<Vec<String>> {
    match name {
        "chats" => chats::report(store),
        "screen" => screen::report(),
        "promises" => promises::report(store),
        "suggest" => suggest::report(store),
        "projects" => projects::report(),
        "cards" => cards::report(store),
        other => bail!(
            "no Jev report named {other:?}: chats, screen, promises, suggest, projects, cards"
        ),
    }
}

/// Runs a shadow over past cases that already have the person's answer,
/// and how often Jev agrees. Sends those cases to TypeSafe; logs nothing.
pub fn backtest(store: &crate::Store, name: &str) -> Result<Vec<String>> {
    match name {
        "chats" => chats::backtest(store),
        "projects" => projects::backtest(store),
        other => bail!("no Jev backtest named {other:?}: chats, projects"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_never_read_the_key() {
        if std::env::var(LIVE_TEST).as_deref() != Ok("1") {
            assert!(key().is_none());
        }
    }

    #[test]
    fn logs_and_usage_stay_in_a_test_folder() {
        let dir = dir().unwrap();
        assert!(dir.starts_with(std::env::temp_dir()));
        append("unit", &json!({ "n": 1 })).unwrap();
        append("unit", &json!({ "n": 2 })).unwrap();
        let read: Vec<Value> = entries("unit");
        assert_eq!(read.len(), 2);
        record("2000-01-01", "projects", 500, 1800).unwrap();
        record("2000-01-01", "projects", 100, 400).unwrap();
        record("2000-01-01", "moments", 50, 160).unwrap();
        let usage = usage_on("2000-01-01");
        assert_eq!(usage.input_tokens, 650);
        assert_eq!(usage.by_use["projects"].requests, 2);
        assert_eq!(usage.by_use["projects"].chars, 2200);
        assert_eq!(usage.shadows(), 600);
        assert_eq!(usage_on("2000-01-02").input_tokens, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_usage_file_from_before_uses_were_counted_still_reads() {
        let usage: DayUsage =
            serde_json::from_str(r#"{"day":"2026-10-13","input_tokens":10223673}"#).unwrap();
        assert_eq!(usage.input_tokens, 10_223_673);
        assert_eq!(usage.shadows(), 0);
        assert!(
            refusal(&usage, "moments", 1).is_some(),
            "the day is used up"
        );
    }

    fn used(total: u64, by_use: &[(&str, u64)]) -> DayUsage {
        DayUsage {
            day: "2000-01-01".to_owned(),
            input_tokens: total,
            by_use: by_use
                .iter()
                .map(|(name, tokens)| {
                    (
                        (*name).to_owned(),
                        UseUsage {
                            input_tokens: *tokens,
                            ..UseUsage::default()
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn a_shadow_can_never_spend_what_is_kept_for_moments() {
        // A shadow at its own share stops; the others and moments go on.
        let usage = used(SHADOW_TOKENS, &[("projects", SHADOW_TOKENS)]);
        assert!(refusal(&usage, "projects", 1).is_some());
        assert!(refusal(&usage, "screen", 1_000).is_none());
        assert!(refusal(&usage, "moments", 50_000).is_none());
        // Shadows together stop at theirs, however it is split.
        let usage = used(
            SHADOWS_TOKENS,
            &[("projects", SHADOW_TOKENS), ("screen", SHADOW_TOKENS)],
        );
        let over = refusal(&usage, "cards", 1).unwrap().to_string();
        assert!(over.contains("kept for moments"), "{over}");
        // Then moments still have the rest of the day.
        assert!(refusal(&usage, "moments", DAILY_TOKENS - SHADOWS_TOKENS).is_none());
        assert!(refusal(&usage, "moments", DAILY_TOKENS - SHADOWS_TOKENS + 1).is_some());
        // A request that would cross a share is not sent at all.
        let usage = used(0, &[]);
        assert!(refusal(&usage, "projects", SHADOW_TOKENS + 1).is_some());
    }

    #[test]
    fn a_budget_refusal_is_said_once_a_day_for_each_use() {
        let refused = || anyhow::Error::from(OverBudget("used up".to_owned()));
        assert!(failed("unit-a", "jev: unit-a shadow failed", &refused()).is_some());
        assert!(failed("unit-a", "jev: unit-a shadow failed", &refused()).is_none());
        assert!(failed("unit-b", "jev: unit-b shadow failed", &refused()).is_some());
        let other = || anyhow::anyhow!("Jev said HTTP 500");
        assert!(failed("unit-a", "jev: unit-a shadow failed", &other()).is_some());
        assert!(failed("unit-a", "jev: unit-a shadow failed", &other()).is_some());
    }

    #[test]
    fn the_budget_is_checked_on_the_requests_that_will_be_sent() {
        let state = json!("work");
        let questions: Map<String, Value> = (0..3)
            .map(|n| {
                (
                    format!("q{n}"),
                    json!({ "type": "noul", "instructions": "ok?" }),
                )
            })
            .collect();
        let bodies = bodies(&state, &questions);
        assert_eq!(bodies.len(), 1);
        let body: Value = serde_json::from_str(&bodies[0]).unwrap();
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["questions"].as_object().unwrap().len(), 3);
    }

    #[test]
    fn question_helpers_build_typesafe_questions() {
        let q = choice(
            json!("Who?"),
            &[("me", "the person"), ("them", "someone else")],
        );
        assert_eq!(q["type"], "choice");
        assert_eq!(q["criteria"]["them"], "someone else");
        let q = score(json!("How much?"), &["none", "some", "a lot"]);
        assert_eq!(q["criteria"][2], "a lot");
        let q = noul(json!("Is it?"), "yes means", "no means");
        assert_eq!(q["criteria"]["false"], "no means");
    }

    #[test]
    fn config_quotes_the_body_and_keeps_the_key_out_of_arguments() {
        let config = curl_config("k\"ey", "{\"a\":\"line\\nnext\"}");
        assert!(config.contains("header = \"Authorization: Bearer k\\\"ey\"\n"));
        assert!(config.contains("data-binary = \"{\\\"a\\\":\\\"line\\\\nnext\\\"}\"\n"));
        assert!(config.ends_with("write-out = \"\\n%{http_code}\"\n"));
    }

    #[test]
    fn large_question_sets_are_split_and_none_is_lost() {
        let state = json!("work");
        let long = "x".repeat(MAX_REQUEST_CHARS / 3);
        let questions: Map<String, Value> = (0..7)
            .map(|n| {
                (
                    format!("q{n}"),
                    json!({ "type": "noul", "instructions": long }),
                )
            })
            .collect();
        let batches = batches(&state, &questions);
        assert_eq!(batches.len(), 4);
        assert_eq!(batches.iter().map(Map::len).sum::<usize>(), 7);
        let small: Map<String, Value> = (0..50)
            .map(|n| {
                (
                    format!("q{n}"),
                    json!({ "type": "noul", "instructions": "ok?" }),
                )
            })
            .collect();
        assert_eq!(super::batches(&state, &small).len(), 1);
    }

    #[test]
    fn answers_of_each_type_are_read() {
        let response: Response = serde_json::from_str(
            r#"{"model":"jev-1.13.0","answers":{
                "a":{"type":"noul","noul":0.95},
                "b":{"type":"choice","choice":"billing","probabilities":{"billing":0.88,"sales":0.12},"confidence":0.81},
                "c":{"type":"score","score":1.05,"legend":{"0":"Calm"},"probabilities":{"0":1.0},"confidence":0.92}},
                "usage":{"input_tokens":283,"output_tokens":23}}"#,
        )
        .unwrap();
        assert_eq!(response.answers["a"].noul, Some(0.95));
        assert_eq!(response.answers["b"].choice.as_deref(), Some("billing"));
        assert_eq!(response.answers["c"].score, Some(1.05));
        assert_eq!(response.usage.input_tokens, 283);
    }
}
