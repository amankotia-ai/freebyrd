//! Sending a task to Claude Code from the iPad: the person types what to do
//! and picks a project, and a new session starts in the Claude app on this
//! Mac, in the Code tab, where they see and steer it. The same path as Run
//! in Claude Code (`night_action` "code"): a `claude://code/new` link fills
//! the prompt in, and `claude_app::send_filled_prompt` presses Return only
//! if the app's input holds exactly that prompt.
//!
//! Off until the person turns it on, on the Mac (`Settings::claude_code`):
//! whoever holds the iPad's token could start sessions, and over the Wi-Fi
//! that token travels unencrypted. Only folders the person already trusts
//! in Claude Code and worked in lately can be picked; a folder
//! the iPad names is checked against that list, never taken as given.

use std::path::Path;
use std::thread;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{Companion, lock};

/// The longest task the iPad may send, as night shift's links.
const MAX_CHARS: usize = 5000;
const KEPT: usize = 12;

/// A project the iPad may start a session in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub name: String,
    pub folder: String,
    pub last_at: i64,
}

/// A task sent from the iPad, and what became of it on the Mac.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handoff {
    pub id: i64,
    pub at: i64,
    pub project: String,
    pub task: String,
    /// `opening`, `started`, or `waiting` (the prompt is filled in on the
    /// Mac, waiting for Return: the Claude app was not in front, or the
    /// Mac was locked).
    pub state: String,
    pub note: String,
}

/// Projects with Claude Code sessions in the last 30 days whose folders the
/// person trusts in Claude Code (`~/.claude.json`), most recent first.
/// Never Freebyrd's own folders. The desktop app's own trust dialog is
/// pressed for them only where `claude_app` allows it; otherwise it waits on
/// the Mac.
pub fn projects(store: &memory::Store, now_ms: i64) -> Result<Vec<Project>> {
    let trusted = trusted_folders();
    let mut statement = store.conn().prepare(
        "SELECT p.name, p.root, max(s.last_at) AS last FROM project p JOIN session s ON s.project_id = p.id
         WHERE s.last_at >= ?1 GROUP BY p.id ORDER BY last DESC LIMIT 30",
    )?;
    let rows = statement
        .query_map([now_ms - 30 * 86_400_000], |row| {
            Ok(Project {
                name: row.get(0)?,
                folder: row.get(1)?,
                last_at: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|project| {
            !project.folder.contains("Application Support/Freebyrd")
                && project.name != "Claude scratch"
                && Path::new(&project.folder).is_dir()
                && trusted.contains(&project.folder)
        })
        .take(12)
        .collect())
}

/// Starts a session in `folder` with `task` as its prompt. Returns the
/// handoff, whose state the iPad watches.
pub fn send(c: &Companion, store: &memory::Store, folder: &str, task: &str) -> Result<Handoff> {
    let task = task.trim();
    if task.is_empty() {
        bail!("type the task first");
    }
    if task.chars().count() > MAX_CHARS {
        bail!("that task is longer than {MAX_CHARS} characters; shorten it");
    }
    let now = memory::time::now_ms();
    let project = projects(store, now)?
        .into_iter()
        .find(|project| project.folder == folder)
        .context("that project is not one you've trusted and worked in with Claude Code")?;
    let link = link(&project.folder, task);
    std::process::Command::new("/usr/bin/open")
        .arg(&link)
        .status()
        .context("opening Claude Code")?;
    let handoff = {
        let mut handoffs = lock(&c.handoffs);
        let id = handoffs.last().map_or(1, |last| last.id + 1);
        let handoff = Handoff {
            id,
            at: now,
            project: project.name.clone(),
            task: clip(task, 160),
            state: "opening".to_owned(),
            note: "Opening Claude on your Mac".to_owned(),
        };
        handoffs.push(handoff.clone());
        let extra = handoffs.len().saturating_sub(KEPT);
        handoffs.drain(..extra);
        handoff
    };
    eprintln!("freebyrd: companion: a task from the iPad, for {}", project.name);
    let (id, prompt, folder) = (handoff.id, task.to_owned(), project.folder);
    thread::spawn(move || {
        let what = crate::claude_app::send_filled_prompt(&prompt, Some(&folder));
        eprintln!("freebyrd: companion: Claude Code session {what}");
        let started = what.starts_with("started");
        if let Some(handoff) = lock(&super::companion().handoffs).iter_mut().find(|h| h.id == id) {
            handoff.state = if started { "started" } else { "waiting" }.to_owned();
            handoff.note = if started {
                "Started in the Claude app on your Mac".to_owned()
            } else {
                "Filled in on your Mac but not sent: the Claude app wasn't in front, or the Mac is locked. Press Return there.".to_owned()
            };
        }
    });
    Ok(handoff)
}

/// The folders Claude Code has the person's trust for.
fn trusted_folders() -> std::collections::HashSet<String> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Default::default();
    };
    std::fs::read_to_string(home.join(".claude.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|config| config["projects"].as_object().cloned())
        .map(|projects| {
            projects
                .into_iter()
                .filter(|(_, project)| project["hasTrustDialogAccepted"] == true)
                .map(|(folder, _)| folder)
                .collect()
        })
        .unwrap_or_default()
}

/// The newest first.
pub fn recent(c: &Companion) -> Vec<Handoff> {
    lock(&c.handoffs).iter().rev().cloned().collect()
}

/// The link the desktop app opens a new Code tab session for, its prompt
/// filled in; Claude Code in a terminal if there is no desktop app.
fn link(folder: &str, prompt: &str) -> String {
    if Path::new("/Applications/Claude.app").is_dir() {
        format!("claude://code/new?folder={}&q={}", encode(folder), encode(prompt))
    } else {
        format!("claude-cli://open?cwd={}&q={}", encode(folder), encode(prompt))
    }
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_the_folder_and_task_encoded() {
        let link = link("/Users/a/My Repo", "Fix the glow & spacing\nthen test");
        assert!(link.ends_with("My%20Repo&q=Fix%20the%20glow%20%26%20spacing%0Athen%20test"));
        // Nothing in the task can add a parameter of its own.
        assert_eq!(link.matches('&').count(), 1);
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn a_folder_is_one_from_the_list_or_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(projects(&store, memory::time::now_ms()).unwrap().is_empty());
    }
}
