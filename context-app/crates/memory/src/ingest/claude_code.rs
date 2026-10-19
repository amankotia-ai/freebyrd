//! Claude Code sessions, read where Claude Code keeps them:
//! `~/.claude/projects/<folder>/<session>.jsonl`, one JSON object per line.
//!
//! Kept: what the person asked (without the reminders, skill text and tool
//! output that Claude Code adds to a turn), what Claude replied, which files
//! were edited, commits, pushes and deploys, Notion pages touched, and skills
//! used. Subagent transcripts (in subfolders) and tool output are skipped.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use super::{SourceReport, read_new_lines};
use crate::redact::redact;
use crate::store::{NewActivity, Store};
use crate::{home, truncate};

const SOURCE: &str = "claude";
const MAX_PROMPT_CHARS: usize = 2_000;
const MAX_REPLY_CHARS: usize = 2_000;
const MAX_COMMAND_CHARS: usize = 200;

/// Where the Claude desktop app runs its quick sessions. They are grouped as
/// one project.
const SCRATCH_MARKER: &str = "/Library/Application Support/Claude/scratch-workspaces";

/// Turn text that Claude Code or the desktop app added, not the person.
const INJECTED_PREFIXES: &[&str] = &[
    "<task-notification",
    "<local-command-",
    "<command-",
    "<bash-",
    "Base directory for this skill",
    "[Request interrupted",
    "This session is being continued",
    "Caveat:",
    "Another Claude session",
    "[Cross-session",
    // A scheduled task's prompt, which the desktop app starts on its own.
    "<scheduled-task",
];

/// Commands that ship work.
const SHIP_COMMANDS: &[&str] = &[
    "git commit",
    "git push",
    "gh pr create",
    "gh pr merge",
    "vercel --prod",
    "vercel deploy",
    "npm publish",
    "cargo publish",
    "fly deploy",
    "wrangler deploy",
    "firebase deploy",
    "netlify deploy",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    uuid: Option<String>,
    session_id: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(default)]
    is_meta: bool,
    #[serde(default)]
    is_compact_summary: bool,
    #[serde(default)]
    is_visible_in_transcript_only: bool,
    origin: Option<Value>,
    message: Option<Message>,
    custom_title: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<Value>,
}

pub fn projects_dir() -> Option<PathBuf> {
    Some(home()?.join(".claude/projects"))
}

/// What the person last typed to Claude Code, read straight from the
/// transcripts rather than the memory, which is a few minutes behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestPrompt {
    pub session_id: String,
    pub cwd: String,
    /// Redacted, without what Claude Code adds to a turn.
    pub text: String,
    pub at: i64,
}

/// How much of the end of a transcript is read for its last prompt.
const TAIL_BYTES: u64 = 512 * 1024;

/// The newest prompt the person typed, in a transcript written since
/// `since_ms`. Other sessions' messages, tool results and Freebyrd's own
/// runs are not the person's and are skipped.
pub fn latest_prompt(since_ms: i64) -> Option<LatestPrompt> {
    use std::io::{Read, Seek, SeekFrom};
    let dir = projects_dir()?;
    let since = std::time::UNIX_EPOCH + std::time::Duration::from_millis(since_ms.max(0) as u64);
    let mut newest: Option<LatestPrompt> = None;
    for folder in fs::read_dir(dir).ok()?.flatten() {
        if folder
            .file_name()
            .to_string_lossy()
            .contains("Application-Support-Freebyrd")
        {
            continue;
        }
        let Ok(files) = fs::read_dir(folder.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let fresh = file
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified >= since);
            if !fresh || path.extension().is_none_or(|ext| ext != "jsonl") {
                continue;
            }
            let Ok(mut handle) = fs::File::open(&path) else {
                continue;
            };
            let length = handle.metadata().map(|meta| meta.len()).unwrap_or(0);
            let start = length.saturating_sub(TAIL_BYTES);
            if handle.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if handle.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            // A tail that starts mid-line drops its first, partial line.
            let lines = text.lines().skip(usize::from(start > 0));
            for raw in lines {
                if !raw.contains("\"type\":\"user\"") || raw.contains("\"toolUseResult\"") {
                    continue;
                }
                let Ok(line) = serde_json::from_str::<Line>(raw) else {
                    continue;
                };
                if line.kind != "user" || line.is_sidechain {
                    continue;
                }
                let (Some(at), Some(session_id), Some(cwd)) = (
                    line.timestamp.as_deref().and_then(parse_ms),
                    line.session_id.clone(),
                    line.cwd.clone(),
                ) else {
                    continue;
                };
                if at < since_ms || newest.as_ref().is_some_and(|found| found.at >= at) {
                    continue;
                }
                let content = line.message.as_ref().and_then(|m| m.content.as_ref());
                if let Some(text) = prompt(&line, content) {
                    newest = Some(LatestPrompt {
                        session_id,
                        cwd,
                        text,
                        at,
                    });
                }
            }
        }
    }
    newest
}

pub fn ingest(store: &Store) -> SourceReport {
    let mut report = SourceReport::new("Claude Code");
    let Some(dir) = projects_dir() else {
        return report;
    };
    let Ok(folders) = fs::read_dir(&dir) else {
        report.errors.push(format!("cannot read {}", dir.display()));
        return report;
    };
    let mut projects = Projects::default();
    for folder in folders.flatten() {
        // Freebyrd's own calls never save a session; anything run from its
        // folders would be Freebyrd's activity, not the person's.
        if folder
            .file_name()
            .to_string_lossy()
            .contains("Application-Support-Freebyrd")
        {
            continue;
        }
        let Ok(files) = fs::read_dir(folder.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|ext| ext != "jsonl") || !path.is_file() {
                continue;
            }
            report.files += 1;
            let result = read_new_lines(store, SOURCE, &path, |line| {
                if !worth_parsing(line) {
                    return Ok(0);
                }
                match serde_json::from_str::<Line>(line) {
                    Ok(line) => record(store, &line, &mut projects),
                    Err(_) => Ok(0),
                }
            });
            match result {
                Ok(added) => report.added += added,
                Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
            }
        }
    }
    report
}

/// Skips most of a transcript (tool output, thinking, attachments) without
/// parsing it. These keys appear unescaped only in a line's structure:
/// inside strings, their quotes are escaped.
fn worth_parsing(line: &str) -> bool {
    if line.contains("\"toolUseResult\"") || line.contains("\"type\":\"thinking\"") {
        return false;
    }
    line.contains("\"type\":\"user\"")
        || line.contains("\"type\":\"assistant\"")
        || line.contains("\"type\":\"custom-title\"")
}

fn record(store: &Store, line: &Line, projects: &mut Projects) -> Result<usize> {
    if line.kind == "custom-title" {
        if let (Some(id), Some(title)) = (&line.session_id, &line.custom_title)
            && !title.trim().is_empty()
        {
            store.set_session_title(id, title.trim())?;
        }
        return Ok(0);
    }
    if line.is_sidechain {
        return Ok(0);
    }
    let (Some(uuid), Some(session), Some(ts)) = (
        &line.uuid,
        &line.session_id,
        line.timestamp.as_deref().and_then(parse_ms),
    ) else {
        return Ok(0);
    };
    let content = line.message.as_ref().and_then(|m| m.content.as_ref());
    let items = match line.kind.as_str() {
        "user" => prompt(line, content).map_or_else(Vec::new, |text| vec![("prompt", text)]),
        "assistant" => assistant_items(content),
        _ => Vec::new(),
    };
    if items.is_empty() {
        return Ok(0);
    }

    let project_id = projects.for_session(store, session, line.cwd.as_deref())?;
    store.note_session(session, project_id, ts)?;
    let mut added = 0;
    for (index, (kind, detail)) in items.iter().enumerate() {
        let ext_id = format!("{uuid}:{index}");
        let activity = NewActivity {
            source: SOURCE,
            kind,
            ts,
            project_id,
            session_id: Some(session),
            detail: Some(detail),
            ext_id: &ext_id,
            ..Default::default()
        };
        if store.add_activity(&activity)? {
            added += 1;
        }
    }
    Ok(added)
}

fn parse_ms(timestamp: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// What the person typed, or `None` for anything else on a user line: tool
/// results, notices, compaction summaries, messages from other sessions.
fn prompt(line: &Line, content: Option<&Value>) -> Option<String> {
    if line.is_meta || line.is_compact_summary || line.is_visible_in_transcript_only {
        return None;
    }
    // Newer transcripts say who started a turn; only the person's count.
    if let Some(kind) = line.origin.as_ref().and_then(|o| o["kind"].as_str())
        && kind != "human"
    {
        return None;
    }
    let text = match content? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    clean_prompt(&text)
}

fn clean_prompt(text: &str) -> Option<String> {
    let start = text.trim_start();
    if INJECTED_PREFIXES.iter().any(|p| start.starts_with(p)) {
        return None;
    }
    let text = replace_blocks(text, "<system-reminder", "</system-reminder", "");
    let text = replace_blocks(
        &text,
        "<pasted_content",
        "</pasted_content",
        "[pasted text]",
    );
    let text = remove_image_markers(&text);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(truncate(&redact(text), MAX_PROMPT_CHARS))
}

/// Replaces each `<open ...>...</close ...>` block with `with`. A block that
/// never closes runs to the end of the text.
fn replace_blocks(text: &str, open: &str, close: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        out.push_str(with);
        let inside = &rest[start + open.len()..];
        rest = inside
            .find(close)
            .map(|end| &inside[end + close.len()..])
            .and_then(|after| after.find('>').map(|gt| &after[gt + 1..]))
            .unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// Drops `[Image: ...]` and `[Image #1]` markers the app puts in for pictures.
fn remove_image_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[Image") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find(']')
            .map_or("", |end| &rest[start + end + 1..]);
    }
    out.push_str(rest);
    out
}

fn assistant_items(content: Option<&Value>) -> Vec<(&'static str, String)> {
    let Some(Value::Array(blocks)) = content else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| match block["type"].as_str()? {
            "text" => {
                let text = block["text"].as_str()?.trim();
                (!text.is_empty()).then(|| ("reply", truncate(&redact(text), MAX_REPLY_CHARS)))
            }
            "tool_use" => notable_tool(block["name"].as_str()?, &block["input"]),
            _ => None,
        })
        .collect()
}

fn notable_tool(name: &str, input: &Value) -> Option<(&'static str, String)> {
    match name {
        "Edit" | "Write" | "MultiEdit" => Some(("edit", input["file_path"].as_str()?.to_owned())),
        "NotebookEdit" => Some(("edit", input["notebook_path"].as_str()?.to_owned())),
        "Skill" => Some(("skill", input["skill"].as_str()?.to_owned())),
        "Bash" => shipped(input["command"].as_str()?).map(|command| ("ship", command)),
        _ if name.contains("notion-") => {
            let action = name.rsplit("__").next().unwrap_or(name);
            let target = ["page_id", "id", "url"]
                .iter()
                .find_map(|key| input[*key].as_str())
                .unwrap_or_default();
            Some(("notion", format!("{action} {target}").trim().to_owned()))
        }
        _ => None,
    }
}

/// The part of a shell command that ships work, if any, in short form.
fn shipped(command: &str) -> Option<String> {
    command
        .split(['\n', ';'])
        .flat_map(|part| part.split("&&"))
        .map(short_command)
        .find(|short| SHIP_COMMANDS.iter().any(|c| short.starts_with(c)))
        .map(|short| truncate(&redact(&short), MAX_COMMAND_CHARS))
}

/// A command without option values, messages or redirections, which carry
/// noise and sometimes tokens: `vercel deploy --prod --yes --scope team_x >
/// log` becomes `vercel deploy --prod`. Commit messages come from git log.
fn short_command(command: &str) -> String {
    const ENDS_AT: &[&str] = &["-m", "-F", "--message", "--file", "--title", "--body"];
    const TAKES_VALUE: &[&str] = &[
        "-C", "--scope", "--token", "-t", "--team", "--cwd", "--base", "--head",
    ];
    let mut words = Vec::new();
    let mut skip_value = false;
    for word in command.split_whitespace() {
        if skip_value {
            skip_value = false;
            continue;
        }
        if ENDS_AT.contains(&word)
            || word.contains("<<")
            || word.starts_with(['>', '<', '|', '"', '\''])
            || word.starts_with("2>")
        {
            break;
        }
        if TAKES_VALUE.contains(&word) {
            skip_value = true;
        } else if !word.starts_with('-') || word == "--prod" {
            words.push(word);
        }
        if words.len() == 5 {
            break;
        }
    }
    words.join(" ")
}

#[derive(Default)]
struct Projects {
    by_cwd: HashMap<String, i64>,
    by_session: HashMap<String, i64>,
}

impl Projects {
    fn for_session(
        &mut self,
        store: &Store,
        session: &str,
        cwd: Option<&str>,
    ) -> Result<Option<i64>> {
        if let Some(id) = self.by_session.get(session) {
            return Ok(Some(*id));
        }
        // A session stays with the project it started in, even after `cd`.
        let id = match store.session_project(session)? {
            Some(id) => id,
            None => match cwd {
                Some(cwd) => self.for_cwd(store, cwd)?,
                None => return Ok(None),
            },
        };
        self.by_session.insert(session.to_owned(), id);
        Ok(Some(id))
    }

    fn for_cwd(&mut self, store: &Store, cwd: &str) -> Result<i64> {
        if let Some(id) = self.by_cwd.get(cwd) {
            return Ok(*id);
        }
        let (root, name) = project_root(Path::new(cwd));
        let id = store.project_id(&root, &name)?;
        self.by_cwd.insert(cwd.to_owned(), id);
        Ok(id)
    }
}

/// The project a folder belongs to: its git repository, else the folder.
/// Returns the project's root and name.
pub fn project_root(cwd: &Path) -> (String, String) {
    let text = cwd.to_string_lossy();
    if let Some(at) = text.find(SCRATCH_MARKER) {
        return (
            text[..at + SCRATCH_MARKER.len()].to_owned(),
            "Claude scratch".to_owned(),
        );
    }
    let home = home();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        if home.as_deref() == Some(current) {
            break;
        }
        if let Some(root) = repository_root(current) {
            return (root.display().to_string(), folder_name(&root));
        }
        dir = current.parent();
    }
    if home.as_deref() == Some(cwd) {
        return (cwd.display().to_string(), "Home".to_owned());
    }
    (cwd.display().to_string(), folder_name(cwd))
}

/// The repository that `dir` is the top of, if it has a `.git`. A worktree
/// (Claude Code makes them under `.claude/worktrees`) has a `.git` file
/// pointing into the main repository's `.git/worktrees`; its work counts
/// toward the main repository.
fn repository_root(dir: &Path) -> Option<PathBuf> {
    let git = dir.join(".git");
    if git.is_dir() {
        return Some(dir.to_path_buf());
    }
    let pointer = fs::read_to_string(&git).ok()?;
    let gitdir = pointer.trim().strip_prefix("gitdir:")?.trim();
    Some(
        gitdir
            .split_once("/.git/worktrees/")
            .map_or_else(|| dir.to_path_buf(), |(main, _)| PathBuf::from(main)),
    )
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(json: &str) -> Line {
        serde_json::from_str(json).unwrap()
    }

    fn items(json: &str) -> Vec<(&'static str, String)> {
        let line = line(json);
        let content = line.message.as_ref().and_then(|m| m.content.as_ref());
        match line.kind.as_str() {
            "user" => prompt(&line, content).map_or_else(Vec::new, |t| vec![("prompt", t)]),
            "assistant" => assistant_items(content),
            _ => Vec::new(),
        }
    }

    #[test]
    fn keeps_what_the_person_typed() {
        let typed = r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"Fix the login bug\n<system-reminder>noise</system-reminder>"}}"#;
        assert_eq!(items(typed), [("prompt", "Fix the login bug".to_owned())]);

        let blocks = r#"{"type":"user","message":{"content":[{"type":"text","text":"[Image: source: /tmp/a.png] What is this?"},{"type":"image"}]}}"#;
        assert_eq!(items(blocks), [("prompt", "What is this?".to_owned())]);

        let pasted = r#"{"type":"user","message":{"content":"Summarise <pasted_content id=x>long text</pasted_content id=x> please"}}"#;
        assert_eq!(
            items(pasted),
            [("prompt", "Summarise [pasted text] please".to_owned())]
        );
    }

    #[test]
    fn skips_what_the_person_did_not_type() {
        for json in [
            r#"{"type":"user","toolUseResult":{},"message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"Caveat: local commands"}}"#,
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>done</task-notification>"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"user","message":{"content":"Base directory for this skill: /x"}}"#,
            r#"{"type":"user","message":{"content":"<system-reminder>only noise</system-reminder>"}}"#,
            r#"{"type":"user","message":{"content":"<scheduled-task name=\"watch\">Check the files</scheduled-task>"}}"#,
        ] {
            assert!(items(json).is_empty(), "kept {json}");
        }
    }

    #[test]
    fn keeps_replies_edits_and_shipping() {
        let reply = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed it. Tests pass."}]}}"#;
        assert_eq!(
            items(reply),
            [("reply", "Fixed it. Tests pass.".to_owned())]
        );

        let edit = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/src/a.rs"}}]}}"#;
        assert_eq!(items(edit), [("edit", "/repo/src/a.rs".to_owned())]);

        let push = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cd /repo && cargo test && git push origin main"}}]}}"#;
        assert_eq!(items(push), [("ship", "git push origin main".to_owned())]);

        for (command, short) in [
            ("git commit -q -F - <<'EOF'\nFix retries\nEOF", "git commit"),
            (
                "git -C /repo push -u origin main 2>&1 | tail",
                "git push origin main",
            ),
            (
                "vercel deploy --prod --yes --scope team_x > $SP/log",
                "vercel deploy --prod",
            ),
            ("gh pr create --title \"Fix it\" --body x", "gh pr create"),
        ] {
            assert_eq!(shipped(command).as_deref(), Some(short), "{command}");
        }

        let ls = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#;
        assert!(items(ls).is_empty());

        let notion = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__abc__notion-update-page","input":{"page_id":"123"}}]}}"#;
        assert_eq!(
            items(notion),
            [("notion", "notion-update-page 123".to_owned())]
        );
    }

    #[test]
    fn prefilter_matches_compact_json() {
        assert!(worth_parsing(
            r#"{"type":"user","message":{"content":"hi"}}"#
        ));
        assert!(!worth_parsing(
            r#"{"type":"user","toolUseResult":{},"message":{"content":[]}}"#
        ));
        assert!(!worth_parsing(
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"x"}]}}"#
        ));
        assert!(!worth_parsing(r#"{"type":"attachment"}"#));
    }

    #[test]
    fn records_a_session_end_to_end() {
        let store = Store::open_in_memory().unwrap();
        let mut projects = Projects::default();
        let lines = [
            r#"{"type":"custom-title","customTitle":"Webhooks","sessionId":"s1"}"#,
            r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-10-12T09:00:00Z","cwd":"/nonexistent/repo","origin":{"kind":"human"},"message":{"content":"Fix retries"}}"#,
            r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-10-12T09:05:00Z","cwd":"/nonexistent/repo/sub","message":{"content":[{"type":"text","text":"Done."}]}}"#,
        ];
        let mut added = 0;
        for json in lines {
            added += record(&store, &line(json), &mut projects).unwrap();
        }
        assert_eq!(added, 2);
        // Reading the same lines again adds nothing.
        for json in lines {
            assert_eq!(
                record(&store, &line(json), &mut Projects::default()).unwrap(),
                0
            );
        }
        let (title, project): (String, String) = store
            .conn()
            .query_row(
                "SELECT s.title, p.name FROM session s JOIN project p ON p.id = s.project_id WHERE s.id = 's1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((title.as_str(), project.as_str()), ("Webhooks", "repo"));
    }

    #[test]
    fn worktrees_count_toward_their_repository() {
        let base = std::env::temp_dir().join(format!("freebyrd-repo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let main = base.join("My Repo");
        let worktree = main.join(".claude/worktrees/quiet-fox");
        fs::create_dir_all(main.join(".git/worktrees/quiet-fox")).unwrap();
        fs::create_dir_all(worktree.join("src")).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}/.git/worktrees/quiet-fox\n", main.display()),
        )
        .unwrap();

        let (root, name) = project_root(&worktree.join("src"));
        assert_eq!(root, main.display().to_string());
        assert_eq!(name, "My Repo");
        assert_eq!(project_root(&main).1, "My Repo");

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn scratch_sessions_share_a_project() {
        let (root, name) = project_root(Path::new(
            "/Users/x/Library/Application Support/Claude/scratch-workspaces/a/b/scratch-1",
        ));
        assert_eq!(
            root,
            "/Users/x/Library/Application Support/Claude/scratch-workspaces"
        );
        assert_eq!(name, "Claude scratch");
    }
}
