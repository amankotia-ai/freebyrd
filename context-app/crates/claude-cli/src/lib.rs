//! Runs the Claude Code CLI with the person's existing Claude login.
//!
//! `ask` and `stream` run in print mode with every tool, MCP server, plugin,
//! hook and skill turned off, from an empty folder, and without saving the
//! session. What Freebyrd sends them (screen text, pages, its memory) is
//! untrusted, so the model gets nothing it could act with.
//!
//! A `stream` may be given web search (`Tools::WebSearch`), and nothing
//! else. Anthropic runs the search, so a query reaches a search service
//! rather than an address the model picks: text on screen cannot make it
//! send the person's memory to a site of its choosing, as opening a page
//! (`WebFetch`) could. What comes back is untrusted.
//!
//! `fetch` is the one exception, for reading from the person's claude.ai
//! connectors (Fireflies, Notion): the prompt is Freebyrd's own, never
//! untrusted text; only the connector tools it names may run, every tool
//! that writes is denied by name, and anything else is denied by
//! `dontAsk`. What the tools return is untrusted, so its answer is data.
//! `run_job` is a `fetch` for a scheduled job the person approved: the
//! approved prompt, from the job's own folder, with a cap on what it spends.
//!
//! Not saving sessions keeps all these calls out of `~/.claude/projects`,
//! where Freebyrd's memory reads the person's own Claude Code sessions:
//! saved calls would come back as the person's activity.
//!
//! `agent` is the one call that may change files: a night shift job the
//! person queued (`memory::night`), with Claude Code's own file and shell
//! tools, inside Claude Code's sandbox. It runs from a folder under
//! Freebyrd's, writes only there, has no network (so nothing installs,
//! pushes, deploys or sends) and no MCP server, hook or plugin. Its session
//! is saved so the next item can resume it; being under Freebyrd's folder,
//! the memory skips it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

/// The fastest current model. Pinned by ID: older CLIs resolve the `haiku`
/// alias to Haiku 4.5.
pub const FAST_MODEL: &str = "claude-haiku-5-5";

const LOGIN_HELP: &str =
    "Claude Code's login has expired. Run `claude` in Terminal, type /login, then ask again.";

#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub system_prompt: &'a str,
    pub model: &'a str,
    pub effort: &'a str,
}

/// A picture sent with a streamed prompt, such as the window the person is
/// asking about.
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    /// `image/jpeg` or `image/png`.
    pub media_type: &'a str,
    pub data: &'a [u8],
}

/// What a streamed answer may use besides its prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    None,
    /// Anthropic's web search, and no other tool: no page is opened, no
    /// file read and no command run.
    WebSearch,
}

/// What a streamed answer reports while it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Text(String),
    /// A fallback model took over mid-answer.
    Fallback,
    /// The model is searching the web for this query.
    Searching(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Refused,
}

/// `FREEBYRD_CLAUDE`, or else the newest Claude Code on this Mac: `claude`
/// on `PATH` or in the usual install locations (an app opened from Finder
/// gets a minimal `PATH`), or the one the Claude desktop app keeps, which is
/// often newer. Connectors need a recent one (`fetch`). Found once, and
/// again if it disappears (the desktop app replaces its copy on update).
pub fn find_cli() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FREEBYRD_CLAUDE") {
        return Some(PathBuf::from(path));
    }
    static FOUND: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut found = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(path) = found.as_ref().filter(|path| path.is_file()) {
        return Some(path.clone());
    }
    *found = candidates()
        .into_iter()
        .filter(|path| path.is_file())
        .filter_map(|path| Some((version(&path)?, path)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path);
    found.clone()
}

fn candidates() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("claude"))
                .collect()
        })
        .unwrap_or_default();
    paths.extend([
        home.join(".local/bin/claude"),
        home.join(".npm-global/bin/claude"),
        home.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ]);
    // ~/Library/Application Support/Claude/claude-code/<version>/<build>/claude.app
    let desktop = home.join("Library/Application Support/Claude/claude-code");
    for version in std::fs::read_dir(desktop).into_iter().flatten().flatten() {
        for build in std::fs::read_dir(version.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            paths.push(build.path().join("claude.app/Contents/MacOS/claude"));
        }
    }
    paths
}

/// "2.1.288 (Claude Code)" as (2, 1, 288).
fn version(cli: &Path) -> Option<(u32, u32, u32)> {
    let out = Command::new(cli).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text
        .split_whitespace()
        .next()?
        .split('.')
        .map(|part| part.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// An empty folder to run in, so no project's CLAUDE.md is picked up.
fn work_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = Path::new(&home).join("Library/Application Support/Freebyrd/claude-cwd");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn command(request: &Request, output_format: &str, tools: Tools) -> Result<Command> {
    let cli = find_cli().context(
        "The Claude Code CLI was not found. Install it, or set FREEBYRD_CLAUDE to its path.",
    )?;
    let mut command = Command::new(&cli);
    command
        .current_dir(work_dir()?)
        .args(["--print", "--output-format", output_format])
        .args(tool_args(tools))
        .args(["--strict-mcp-config", "--setting-sources", ""])
        .args(["--disable-slash-commands", "--no-session-persistence"])
        .args(["--model", request.model, "--effort", request.effort])
        .args(["--system-prompt", request.system_prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

/// The flags for a call's tools: none, or web search alone. MCP servers
/// are denied either way, and with web search, opening pages too.
fn tool_args(tools: Tools) -> &'static [&'static str] {
    match tools {
        Tools::None => &["--tools", "", "--disallowedTools", "mcp__*"],
        Tools::WebSearch => &[
            "--tools",
            "WebSearch",
            "--allowedTools",
            "WebSearch",
            "--disallowedTools",
            "mcp__*,WebFetch",
            "--permission-mode",
            "dontAsk",
        ],
    }
}

/// Writes the prompt over stdin, not argv, so it never shows up in `ps`,
/// and starts draining stderr so a chatty CLI cannot block on a full pipe.
fn start(mut command: Command, prompt: &str) -> Result<(Child, JoinHandle<String>)> {
    let mut child = command.spawn().context("starting the Claude CLI")?;
    let mut stdin = child.stdin.take().context("no stdin for the CLI")?;
    stdin.write_all(prompt.as_bytes())?;
    drop(stdin);
    let mut stderr = child.stderr.take().context("no stderr for the CLI")?;
    let stderr = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    Ok((child, stderr))
}

/// The prompt as the one user message of `--input-format stream-json`, a
/// JSON line, with the pictures before the text, as Anthropic recommends.
fn user_message(prompt: &str, images: &[Image]) -> String {
    let mut content: Vec<Value> = images
        .iter()
        .map(|image| {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.media_type,
                    "data": BASE64.encode(image.data),
                },
            })
        })
        .collect();
    content.push(json!({ "type": "text", "text": prompt }));
    let message = json!({
        "type": "user",
        "message": { "role": "user", "content": content },
    });
    format!("{message}\n")
}

/// Streams an answer to `prompt` and any `images`, with `tools`; `on_event`
/// gets the text as it arrives.
pub fn stream(
    request: &Request,
    prompt: &str,
    images: &[Image],
    tools: Tools,
    mut on_event: impl FnMut(Event),
) -> Result<Outcome> {
    let mut command = command(request, "stream-json", tools)?;
    command.args(["--verbose", "--include-partial-messages"]);
    command.args(["--input-format", "stream-json"]);
    let (mut child, stderr) = start(command, &user_message(prompt, images))?;

    let stdout = child.stdout.take().context("no stdout for the CLI")?;
    let mut streamed = false;
    // Text written before a search and text after it are separate
    // paragraphs.
    let mut searched = false;
    let mut outcome = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.context("reading the CLI's output")?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match event["type"].as_str() {
            Some("stream_event") => {
                let inner = &event["event"];
                match inner["type"].as_str() {
                    Some("content_block_delta") if inner["delta"]["type"] == "text_delta" => {
                        if let Some(text) = inner["delta"]["text"].as_str() {
                            let after_search = std::mem::take(&mut searched);
                            if streamed && after_search {
                                on_event(Event::Text("\n\n".to_owned()));
                            }
                            streamed = true;
                            on_event(Event::Text(text.to_owned()));
                        }
                    }
                    Some("content_block_start") if inner["content_block"]["type"] == "fallback" => {
                        on_event(Event::Fallback);
                    }
                    _ => {}
                }
            }
            // The whole message, once its tool calls are complete.
            Some("assistant") => {
                for query in search_queries(&event) {
                    searched = true;
                    on_event(Event::Searching(query));
                }
            }
            Some("result") => outcome = Some(event),
            _ => {}
        }
    }
    let status = child.wait()?;
    let stderr = stderr.join().unwrap_or_default();

    let Some(result) = outcome else {
        bail!(
            "the Claude CLI exited ({status}) without an answer: {}",
            stderr.trim().lines().last().unwrap_or("no details")
        );
    };
    let text = answer_text(&result)?;
    if refused(&result) {
        return Ok(Outcome::Refused);
    }
    // Without partial messages (older CLIs), the whole answer arrives at the end.
    if !streamed && !text.is_empty() {
        on_event(Event::Text(text));
    }
    Ok(Outcome::Done)
}

/// Runs one request to completion and returns the answer.
pub fn ask(request: &Request, prompt: &str) -> Result<String> {
    let (mut child, stderr) = start(command(request, "json", Tools::None)?, prompt)?;
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .context("no stdout for the CLI")?
        .read_to_string(&mut stdout)
        .context("reading the CLI's output")?;
    let status = child.wait()?;
    let stderr = stderr.join().unwrap_or_default();

    let Some(result) = result_message(&stdout) else {
        bail!(
            "the Claude CLI exited ({status}) without an answer: {}",
            stderr.trim().lines().last().unwrap_or("no details")
        );
    };
    let text = answer_text(&result)?;
    if refused(&result) {
        bail!("Claude declined to answer");
    }
    Ok(text)
}

/// A read from the person's claude.ai connectors.
#[derive(Debug, Clone, Copy)]
pub struct Fetch<'a> {
    pub request: Request<'a>,
    /// Servers that must be connected before the model starts, such as
    /// "claude.ai Fireflies".
    pub servers: &'a [&'a str],
    /// The connector tools that may run, such as
    /// `mcp__claude_ai_Fireflies__fireflies_get_transcripts`.
    pub allowed: &'a [&'a str],
    /// Tools denied by name or pattern even if allowed somewhere else: every
    /// tool of these servers that writes.
    pub denied: &'a [&'a str],
    /// How long the whole read may take, waiting for servers included:
    /// `FETCH_TIMEOUT` in the background, less while the person waits.
    pub timeout: Duration,
}

const FETCH_ATTEMPTS: u32 = 4;
/// For reads nobody is waiting on.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(300);

/// Runs `prompt`, which must be Freebyrd's own, with the connector tools in
/// `fetch` and returns the answer. claude.ai connectors can still be
/// connecting when the CLI starts, so a run whose servers are not all
/// connected is stopped before the model is asked, and started again.
pub fn fetch(fetch: &Fetch, prompt: &str) -> Result<String> {
    answer_text(&fetch_result(fetch, prompt, &work_dir()?, None)?.0)
}

/// Like `fetch`, and also returns the text each tool call gave back, for a
/// read whose tool output is the data itself (a meeting's transcript), so
/// the model need not retype it. That text is untrusted too.
pub fn fetch_with_results(fetch: &Fetch, prompt: &str) -> Result<(String, Vec<String>)> {
    let (result, tool_results) = fetch_result(fetch, prompt, &work_dir()?, None)?;
    Ok((answer_text(&result)?, tool_results))
}

/// A scheduled job's run: a `fetch` of the prompt the person approved.
#[derive(Debug, Clone, Copy)]
pub struct Job<'a> {
    pub fetch: Fetch<'a>,
    /// The job's own folder, under Freebyrd's: were a session ever saved,
    /// it would be filed under this folder's name, which the memory skips.
    pub dir: &'a Path,
    /// The run stops once it has cost this many dollars.
    pub budget_usd: f64,
}

/// What a job's run answered, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct JobAnswer {
    pub text: String,
    pub cost_usd: Option<f64>,
}

/// Runs a scheduled job's prompt with its connectors' reading tools, like
/// `fetch`. The answer is untrusted data.
pub fn run_job(job: &Job, prompt: &str) -> Result<JobAnswer> {
    std::fs::create_dir_all(job.dir)?;
    let (result, _) = fetch_result(&job.fetch, prompt, job.dir, Some(job.budget_usd))?;
    if result["subtype"] == "error_max_budget_usd" {
        bail!("it stopped at its budget of ${:.2}", job.budget_usd);
    }
    Ok(JobAnswer {
        text: answer_text(&result)?,
        cost_usd: result["total_cost_usd"].as_f64(),
    })
}

/// A night shift job's Claude Code run (`agent`).
#[derive(Debug, Clone, Copy)]
pub struct Agent<'a> {
    /// The job's folder, under Freebyrd's: the only place it may write.
    pub dir: &'a Path,
    pub model: &'a str,
    pub effort: &'a str,
    /// The run stops once it has cost this many dollars.
    pub budget_usd: f64,
    /// The session to continue, for the next item of a list.
    pub resume: Option<&'a str>,
    /// How long the whole run may take before it is stopped.
    pub timeout: Duration,
    /// Rules added to Claude Code's own system prompt.
    pub rules: &'a str,
    /// Folders it may read besides its own (the person's project), never
    /// write: Claude Code's file tools ask before reading outside the
    /// working folder, and nobody is there to answer.
    pub read: &'a [&'a Path],
}

/// What an agent run answered.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRun {
    pub text: String,
    pub session_id: Option<String>,
    pub cost_usd: Option<f64>,
    /// Why it stopped early: its budget, a usage limit, an error.
    pub stopped: Option<String>,
}

/// Claude Code's settings for an agent run: its sandbox (checked live on
/// 2026-10-13 with 2.1.288: a shell write outside the folder, a file-tool
/// write outside it and a request to the internet were all refused; shell
/// commands run sandboxed without asking, and one that cannot be sandboxed
/// is refused), and reading the folders in `read`.
fn agent_settings(read: &[&Path]) -> String {
    let allow: Vec<String> = read
        .iter()
        .map(|dir| format!("Read(/{}/**)", dir.to_string_lossy().trim_end_matches('/')))
        .collect();
    json!({
        "disableAllHooks": true,
        "permissions": { "allow": allow },
        "sandbox": {
            "enabled": true,
            "autoAllowBashIfSandboxed": true,
            "allowUnsandboxedCommands": false,
            "failIfUnavailable": true,
            "network": { "allowedDomains": [] },
        },
    })
    .to_string()
}

/// Runs `prompt`, the person's queued job in Freebyrd's words, as Claude
/// Code with its file and shell tools, fenced to `agent.dir` (`agent_settings`).
/// Edits inside the folder are accepted; anything that would need asking is
/// refused, since nobody is there to ask.
pub fn agent(agent: &Agent, prompt: &str) -> Result<AgentRun> {
    std::fs::create_dir_all(agent.dir)?;
    let cli = find_cli().context("The Claude Code CLI was not found.")?;
    let mut command = Command::new(&cli);
    command
        .current_dir(agent.dir)
        .args(["--print", "--output-format", "json"])
        .args(["--setting-sources", "", "--strict-mcp-config"])
        .arg("--settings")
        .arg(agent_settings(agent.read))
        .args(["--tools", "Bash,Read,Edit,Write,Glob,Grep"])
        .args(["--permission-mode", "acceptEdits"])
        .args(["--disallowedTools", "mcp__*,WebFetch,WebSearch"])
        .arg("--disable-slash-commands")
        .args(["--model", agent.model, "--effort", agent.effort])
        .arg("--max-budget-usd")
        .arg(format!("{:.2}", agent.budget_usd))
        .args(["--append-system-prompt", agent.rules])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(session) = agent.resume {
        command.args(["--resume", session]);
    }
    let (mut child, stderr) = start(command, prompt)?;
    let mut stdout = child.stdout.take().context("no stdout for the CLI")?;
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + agent.timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "it ran over {} min and was stopped",
                agent.timeout.as_secs() / 60
            );
        }
        thread::sleep(Duration::from_millis(500));
    };
    let stdout = reader.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let Some(result) = result_message(&stdout) else {
        bail!(
            "the Claude CLI exited ({status}) without an answer: {}",
            stderr.trim().lines().last().unwrap_or("no details")
        );
    };
    let text = result["result"].as_str().unwrap_or_default().to_owned();
    let stopped = match result["subtype"].as_str() {
        Some("error_max_budget_usd") => {
            Some(format!("it reached its budget of ${:.2}", agent.budget_usd))
        }
        Some("success") | None => None,
        Some(other) => Some(other.replace('_', " ")),
    }
    .or_else(|| {
        result["is_error"]
            .as_bool()
            .unwrap_or(false)
            .then(|| text.lines().next().unwrap_or("an error").to_owned())
    });
    if stopped
        .as_deref()
        .is_some_and(|why| why.contains("authenticate") || why.contains("401"))
    {
        bail!(LOGIN_HELP);
    }
    Ok(AgentRun {
        text,
        session_id: result["session_id"].as_str().map(str::to_owned),
        cost_usd: result["total_cost_usd"].as_f64(),
        stopped,
    })
}

/// Runs `fetch` from `dir` until its servers are connected, and returns the
/// CLI's `result` message and the text of each successful tool result.
fn fetch_result(
    fetch: &Fetch,
    prompt: &str,
    dir: &Path,
    budget_usd: Option<f64>,
) -> Result<(Value, Vec<String>)> {
    let deadline = Instant::now() + fetch.timeout;
    for attempt in 1..=FETCH_ATTEMPTS {
        if let Some(result) = fetch_once(fetch, prompt, deadline, dir, budget_usd)? {
            return Ok(result);
        }
        if attempt < FETCH_ATTEMPTS && Instant::now() + Duration::from_secs(4) < deadline {
            thread::sleep(Duration::from_secs(2));
        } else {
            break;
        }
    }
    bail!("{} did not connect", fetch.servers.join(", "))
}

fn fetch_once(
    fetch: &Fetch,
    prompt: &str,
    deadline: Instant,
    dir: &Path,
    budget_usd: Option<f64>,
) -> Result<Option<(Value, Vec<String>)>> {
    let cli = find_cli().context("The Claude Code CLI was not found.")?;
    let mut command = Command::new(&cli);
    command
        .current_dir(dir)
        .args(["--print", "--output-format", "stream-json", "--verbose"])
        // The person's settings hold their connectors; their hooks do not run.
        .args(["--tools", "", "--setting-sources", "user"])
        .args(["--settings", r#"{"disableAllHooks": true}"#])
        // With a config given, print mode waits for servers to connect.
        .args(["--mcp-config", r#"{"mcpServers": {}}"#])
        .args(["--permission-mode", "dontAsk"])
        .arg("--allowedTools")
        .arg(fetch.allowed.join(","))
        .args(["--disable-slash-commands", "--no-session-persistence"])
        .args([
            "--model",
            fetch.request.model,
            "--effort",
            fetch.request.effort,
        ])
        .args(["--system-prompt", fetch.request.system_prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Every other server's tools are denied, which also keeps them out of
    // the model's context: the person's servers bring hundreds of tools.
    let mut denied: Vec<String> = mcp_servers(&cli)
        .into_iter()
        .filter(|name| !fetch.servers.contains(&name.as_str()))
        .map(|name| format!("mcp__{}__*", tool_prefix(&name)))
        .collect();
    denied.extend(fetch.denied.iter().map(|tool| (*tool).to_owned()));
    if !denied.is_empty() {
        command.arg("--disallowedTools").arg(denied.join(","));
    }
    if let Some(budget) = budget_usd {
        command.arg("--max-budget-usd").arg(format!("{budget:.2}"));
    }
    let (mut child, stderr) = start(command, prompt)?;
    let stdout = child.stdout.take().context("no stdout for the CLI")?;
    let servers: Vec<String> = fetch.servers.iter().map(|s| (*s).to_owned()).collect();

    // Lines are read on a thread so the whole run can time out.
    let (lines, events) = mpsc::channel::<Value>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(event) = serde_json::from_str::<Value>(&line)
                && lines.send(event).is_err()
            {
                break;
            }
        }
    });
    let mut result = None;
    let mut tool_results = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = match events.recv_timeout(left) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                bail!(
                    "reading from {} took over {} s",
                    servers.join(", "),
                    fetch.timeout.as_secs()
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match (event["type"].as_str(), event["subtype"].as_str()) {
            (Some("system"), Some("init")) => {
                let connected = |name: &str| {
                    event["mcp_servers"].as_array().is_some_and(|all| {
                        all.iter()
                            .any(|s| s["name"] == name && s["status"] == "connected")
                    })
                };
                if !servers.iter().all(|name| connected(name)) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(None);
                }
            }
            (Some("user"), _) => tool_results.extend(tool_result_texts(&event)),
            (Some("result"), _) => result = Some(event),
            _ => {}
        }
    }
    let status = child.wait()?;
    let stderr = stderr.join().unwrap_or_default();
    let Some(result) = result else {
        bail!(
            "the Claude CLI exited ({status}) without an answer: {}",
            stderr.trim().lines().last().unwrap_or("no details")
        );
    };
    Ok(Some((result, tool_results)))
}

/// The text of each successful tool result in a stream's `user` message;
/// a result's content is a string or a list of text blocks.
fn tool_result_texts(event: &Value) -> Vec<String> {
    let Some(blocks) = event["message"]["content"].as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block["type"] == "tool_result" && block["is_error"] != true)
        .map(|block| match &block["content"] {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        })
        .filter(|text| !text.is_empty())
        .collect()
}

/// The query of each web search in a stream's `assistant` message.
fn search_queries(event: &Value) -> Vec<String> {
    let Some(blocks) = event["message"]["content"].as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block["type"] == "tool_use" && block["name"] == "WebSearch")
        .filter_map(|block| block["input"]["query"].as_str())
        .map(str::to_owned)
        .collect()
}

/// The person's MCP servers by name ("claude.ai Fireflies", "rea").
fn mcp_servers(cli: &Path) -> Vec<String> {
    mcp_list(cli).into_iter().map(|(name, _)| name).collect()
}

/// The person's MCP servers that are connected now, such as
/// "claude.ai Notion". Servers that need a sign-in are left out.
pub fn connected_servers() -> Vec<String> {
    find_cli()
        .map(|cli| mcp_list(&cli))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, connected)| connected.then_some(name))
        .collect()
}

/// Each MCP server's name, and whether it is connected.
type Servers = Vec<(String, bool)>;

/// The servers from `claude mcp list`, which asks no model. Kept for an
/// hour.
fn mcp_list(cli: &Path) -> Servers {
    static SEEN: Mutex<Option<(Instant, Servers)>> = Mutex::new(None);
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, servers)) = seen.as_ref()
        && at.elapsed() < Duration::from_secs(3_600)
    {
        return servers.clone();
    }
    let out = Command::new(cli)
        .args(["mcp", "list"])
        .current_dir(work_dir().unwrap_or_else(|_| PathBuf::from("/")))
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let servers = parse_mcp_list(&out);
    *seen = Some((Instant::now(), servers.clone()));
    servers
}

/// "claude.ai Fireflies: https://api.fireflies.ai/mcp - ✔ Connected" as
/// ("claude.ai Fireflies", true); "! Needs authentication" is not connected.
fn parse_mcp_list(out: &str) -> Servers {
    out.lines()
        .filter_map(|line| {
            let (rest, status) = line.rsplit_once(" - ")?;
            let (name, _) = rest.split_once(": ")?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_owned(), status.trim().ends_with("Connected")))
        })
        .collect()
}

/// How a server's name appears in its tools' names: "claude.ai Fireflies"
/// becomes `claude_ai_Fireflies`, as in `mcp__claude_ai_Fireflies__...`.
fn tool_prefix(server: &str) -> String {
    server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The `result` message: the whole output with `--output-format json`, or
/// one element of it when the CLI prints every message as an array.
fn result_message(stdout: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(stdout.trim()).ok()?;
    match value {
        Value::Array(messages) => messages.into_iter().rev().find(|m| m["type"] == "result"),
        message if message["type"] == "result" => Some(message),
        _ => None,
    }
}

fn answer_text(result: &Value) -> Result<String> {
    let text = result["result"].as_str().unwrap_or_default().to_owned();
    if result["is_error"].as_bool().unwrap_or(false) {
        if text.contains("authenticate") || text.contains("401") {
            bail!(LOGIN_HELP);
        }
        bail!("Claude could not answer: {text}");
    }
    Ok(text)
}

fn refused(result: &Value) -> bool {
    result["stop_reason"] == "refusal"
}

/// Wraps untrusted text in an element that the system prompt calls data.
/// A closing tag inside the text cannot end the element early.
pub fn data_element(name: &str, attributes: &[(&str, &str)], body: Option<&str>) -> String {
    let mut out = format!("<{name}");
    for (key, value) in attributes {
        let value: String = value
            .chars()
            .map(|c| match c {
                '"' => '\'',
                '\n' | '\r' => ' ',
                c => c,
            })
            .collect();
        out.push_str(&format!(" {key}=\"{value}\""));
    }
    match body {
        None => out.push_str("/>"),
        Some(body) => {
            out.push_str(">\n");
            out.push_str(&neutralize_closing_tag(body, name));
            out.push_str(&format!("\n</{name}>"));
        }
    }
    out
}

/// `</name` in any letter case becomes `</ name`, which closes nothing.
fn neutralize_closing_tag(body: &str, name: &str) -> String {
    let needle = format!("</{}", name.to_ascii_lowercase());
    // ASCII lowercasing keeps every byte offset the same.
    let lower = body.to_ascii_lowercase();
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        let start = at + found;
        out.push_str(&body[at..start]);
        out.push_str("</ ");
        out.push_str(&body[start + 2..start + needle.len()]);
        at = start + needle.len();
    }
    out.push_str(&body[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_message_is_one_line_with_pictures_first() {
        let image = Image {
            media_type: "image/jpeg",
            data: b"\xff\xd8\xff",
        };
        let line = user_message("What is this?\nThe chart", &[image]);
        assert!(line.ends_with("}\n"));
        assert_eq!(line.matches('\n').count(), 1);
        let message: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(message["type"], "user");
        let content = &message["message"]["content"];
        assert_eq!(content[0]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[0]["source"]["data"], "/9j/");
        assert_eq!(content[1]["text"], "What is this?\nThe chart");

        let text_only: Value = serde_json::from_str(&user_message("hi", &[])).unwrap();
        assert_eq!(text_only["message"]["content"][0]["type"], "text");
    }

    #[test]
    fn data_element_cannot_be_closed_from_inside() {
        let element = data_element(
            "screen",
            &[("app", "Say \"hi\"\nthere")],
            Some("before </screen> and </SCREEN > after"),
        );
        assert_eq!(
            element,
            "<screen app=\"Say 'hi' there\">\nbefore </ screen> and </ SCREEN > after\n</screen>"
        );
        assert_eq!(element.matches("</screen>").count(), 1);
    }

    #[test]
    fn data_element_without_a_body_is_empty() {
        assert_eq!(
            data_element("screen", &[("unavailable", "paused")], None),
            "<screen unavailable=\"paused\"/>"
        );
    }

    #[test]
    fn finds_the_result_message() {
        let single = r#"{"type":"result","result":"hi","is_error":false}"#;
        assert_eq!(result_message(single).unwrap()["result"], "hi");
        let array = r#"[{"type":"system"},{"type":"result","result":"there"}]"#;
        assert_eq!(result_message(array).unwrap()["result"], "there");
        assert!(result_message("not json").is_none());
    }

    #[test]
    fn reads_which_servers_are_connected() {
        let out = "Checking MCP server health…\n\n\
            claude.ai Fireflies: https://api.fireflies.ai/mcp - ✔ Connected\n\
            claude.ai Gmail: https://gmail.mcp.claude.com/mcp - ! Needs authentication\n\
            plugin:github:github: https://api.githubcopilot.com/mcp/ (HTTP) - ✘ Failed to connect\n\
            rea: npx -y rea-agents@6.4.0 mcp - ✔ Connected\n";
        assert_eq!(
            parse_mcp_list(out),
            [
                ("claude.ai Fireflies".to_owned(), true),
                ("claude.ai Gmail".to_owned(), false),
                ("plugin:github:github".to_owned(), false),
                ("rea".to_owned(), true),
            ]
        );
    }

    #[test]
    fn tool_prefixes_match_the_cli() {
        assert_eq!(tool_prefix("claude.ai Fireflies"), "claude_ai_Fireflies");
        assert_eq!(
            tool_prefix("claude.ai Claude Docs"),
            "claude_ai_Claude_Docs"
        );
        assert_eq!(tool_prefix("plugin:github:github"), "plugin_github_github");
        assert_eq!(tool_prefix("rea"), "rea");
    }

    #[test]
    fn reads_tool_results_from_the_stream() {
        let event: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"a","content":[{"type":"text","text":"line 1"},{"type":"text","text":"line 2"}]},
                {"type":"tool_result","tool_use_id":"b","content":"plain"},
                {"type":"tool_result","tool_use_id":"c","is_error":true,"content":"denied"}]}}"#,
        )
        .unwrap();
        assert_eq!(tool_result_texts(&event), ["line 1\nline 2", "plain"]);
        assert!(tool_result_texts(&serde_json::json!({"type": "user"})).is_empty());
    }

    #[test]
    fn calls_get_no_tools_or_web_search_alone() {
        let none = tool_args(Tools::None);
        assert_eq!(none[..2], ["--tools", ""]);
        assert!(!none.contains(&"--allowedTools"));

        let web = tool_args(Tools::WebSearch);
        let after = |flag: &str| web[web.iter().position(|arg| *arg == flag).unwrap() + 1];
        assert_eq!(after("--tools"), "WebSearch");
        assert_eq!(after("--allowedTools"), "WebSearch");
        assert_eq!(after("--disallowedTools"), "mcp__*,WebFetch");
        assert_eq!(after("--permission-mode"), "dontAsk");
    }

    #[test]
    fn reads_search_queries_from_the_stream() {
        let event: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"content":[
                {"type":"text","text":"Checking."},
                {"type":"tool_use","name":"WebSearch","input":{"query":"Blender Open Data RTX PRO 6000","mode":"standard"}},
                {"type":"tool_use","name":"WebFetch","input":{"url":"https://example.com"}}]}}"#,
        )
        .unwrap();
        assert_eq!(search_queries(&event), ["Blender Open Data RTX PRO 6000"]);
        assert!(search_queries(&serde_json::json!({"type": "assistant"})).is_empty());
    }

    #[test]
    fn reports_an_expired_login() {
        let result: Value =
            serde_json::from_str(r#"{"is_error":true,"result":"401 OAuth token has expired"}"#)
                .unwrap();
        assert_eq!(answer_text(&result).unwrap_err().to_string(), LOGIN_HELP);
    }
}
