//! A span of activity, condensed: per project, the Claude Code sessions
//! (what was asked and what came of it), files edited, commits and what
//! shipped; then the sites and pages visited. `render` turns it into the
//! text Claude reads for a recap, within a size budget.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::params;

use crate::store::Store;
use crate::time::{self, Span};
use crate::{exclusions, one_line, truncate};

const MAX_SITES: usize = 10;
const MAX_PAGES: usize = 15;
const MAX_FILES: usize = 25;
const MAX_COMMITS: usize = 25;

#[derive(Debug, Default)]
pub struct Digest {
    pub label: String,
    pub span: Span,
    pub projects: Vec<ProjectDay>,
    pub sites: Vec<(String, usize)>,
    pub pages: Vec<PageVisits>,
}

#[derive(Debug, Default)]
pub struct ProjectDay {
    pub name: String,
    pub root: String,
    pub sessions: Vec<SessionDay>,
    pub files: Vec<String>,
    pub shipped: Vec<String>,
    pub notion: usize,
    pub skills: Vec<String>,
    pub commits: Vec<String>,
}

#[derive(Debug, Default)]
pub struct SessionDay {
    pub title: Option<String>,
    pub first_ms: i64,
    pub last_ms: i64,
    pub turns: Vec<Turn>,
}

/// One prompt and the last reply to it. A reply with no prompt is the end
/// of a turn that started before the span.
#[derive(Debug, Default)]
pub struct Turn {
    pub asked: Option<String>,
    pub result: Option<String>,
}

#[derive(Debug)]
pub struct PageVisits {
    pub title: String,
    pub site: String,
    pub visits: usize,
}

impl Digest {
    /// The working day `day`, with commits from each project's git log.
    pub fn for_day(store: &Store, day: NaiveDate) -> Result<Self> {
        Self::build(store, time::day_span(day), time::day_label(day), true)
    }

    pub fn build(store: &Store, span: Span, label: String, with_commits: bool) -> Result<Self> {
        let mut digest = Self {
            label,
            span,
            ..Self::default()
        };
        digest.load_claude(store)?;
        if with_commits {
            for project in &mut digest.projects {
                project.commits = git_commits(&project.root, span);
            }
        }
        digest.load_web(store)?;
        Ok(digest)
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.sites.is_empty()
    }

    fn load_claude(&mut self, store: &Store) -> Result<()> {
        let mut statement = store.conn().prepare(
            "SELECT a.kind, a.ts, a.detail, a.session_id, a.project_id, p.name, p.root, s.title
             FROM activity a
             LEFT JOIN project p ON p.id = a.project_id
             LEFT JOIN session s ON s.id = a.session_id
             WHERE a.source = 'claude' AND a.ts >= ?1 AND a.ts < ?2
             ORDER BY a.project_id, a.session_id, a.ts, a.id",
        )?;
        let mut rows = statement.query(params![self.span.start_ms, self.span.end_ms])?;
        let mut projects: Vec<ProjectDay> = Vec::new();
        let mut index_of: HashMap<Option<i64>, usize> = HashMap::new();
        let mut last_session: Option<Option<String>> = None;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let ts: i64 = row.get(1)?;
            let detail: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let session_id: Option<String> = row.get(3)?;
            let project_id: Option<i64> = row.get(4)?;

            let index = match index_of.get(&project_id) {
                Some(index) => *index,
                None => {
                    projects.push(ProjectDay {
                        name: row
                            .get::<_, Option<String>>(5)?
                            .unwrap_or_else(|| "Other".into()),
                        root: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        ..ProjectDay::default()
                    });
                    index_of.insert(project_id, projects.len() - 1);
                    projects.len() - 1
                }
            };
            let project = &mut projects[index];
            // Rows come grouped by project, then session.
            if last_session.as_ref() != Some(&session_id) || project.sessions.is_empty() {
                project.sessions.push(SessionDay {
                    title: row.get(7)?,
                    first_ms: ts,
                    last_ms: ts,
                    turns: Vec::new(),
                });
                last_session = Some(session_id);
            }
            let session = project
                .sessions
                .last_mut()
                .expect("a session was just added");
            session.last_ms = ts;
            match kind.as_str() {
                "prompt" => session.turns.push(Turn {
                    asked: Some(detail),
                    result: None,
                }),
                "reply" => match session.turns.last_mut() {
                    Some(turn) => turn.result = Some(detail),
                    None => session.turns.push(Turn {
                        asked: None,
                        result: Some(detail),
                    }),
                },
                "edit" if !is_scratch(&detail) => {
                    push_unique(&mut project.files, relative(&detail, &project.root));
                }
                "ship" => push_unique(&mut project.shipped, detail),
                "notion" => project.notion += 1,
                "skill" => push_unique(&mut project.skills, detail),
                _ => {}
            }
        }
        for project in &mut projects {
            project.sessions.sort_by_key(|session| session.first_ms);
        }
        // Busiest first.
        projects.sort_by_key(|p| {
            std::cmp::Reverse(p.sessions.iter().map(|s| s.turns.len()).sum::<usize>())
        });
        self.projects = projects;
        Ok(())
    }

    fn load_web(&mut self, store: &Store) -> Result<()> {
        let mut statement = store.conn().prepare(
            "SELECT title, url FROM activity
             WHERE source = 'history' AND ts >= ?1 AND ts < ?2",
        )?;
        let rows = statement.query_map(params![self.span.start_ms, self.span.end_ms], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?;
        let mut sites: HashMap<String, usize> = HashMap::new();
        let mut pages: HashMap<(String, String), usize> = HashMap::new();
        for row in rows {
            let (title, url) = row?;
            let Some(site) = url.as_deref().and_then(exclusions::site) else {
                continue;
            };
            *sites.entry(site.clone()).or_default() += 1;
            if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
                *pages.entry((title, site)).or_default() += 1;
            }
        }
        self.sites = most(sites, MAX_SITES);
        self.pages = most(pages, MAX_PAGES)
            .into_iter()
            .map(|((title, site), visits)| PageVisits {
                title,
                site,
                visits,
            })
            .collect();
        Ok(())
    }

    /// The digest as text of at most `budget` characters. Detail is cut
    /// first: fewer turns per session, then shorter turns.
    pub fn render(&self, budget: usize) -> String {
        for (turns, chars) in [(14, 400), (10, 300), (6, 240), (4, 180), (2, 140), (1, 100)] {
            let text = self.render_with(turns, chars);
            if text.chars().count() <= budget {
                return text;
            }
        }
        truncate(&self.render_with(1, 80), budget)
    }

    fn render_with(&self, max_turns: usize, max_chars: usize) -> String {
        let mut out = format!("Activity for {} (local times).\n", self.label);
        for project in &self.projects {
            let _ = writeln!(out, "\nProject: {}", project.name);
            for session in &project.sessions {
                let title = session
                    .title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", one_line(t, 80)))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  Claude Code session{title}, {} to {}",
                    time::clock(session.first_ms),
                    time::clock(session.last_ms)
                );
                let skipped = session.turns.len().saturating_sub(max_turns);
                for (index, turn) in session.turns.iter().enumerate() {
                    // Keep the first turn (what the session was for) and the latest ones.
                    if index > 0 && index <= skipped {
                        if index == 1 {
                            let _ = writeln!(out, "    ({skipped} more turns)");
                        }
                        continue;
                    }
                    if let Some(asked) = &turn.asked {
                        let _ = writeln!(out, "    Asked: {}", one_line(asked, max_chars));
                    }
                    if let Some(result) = &turn.result {
                        let _ = writeln!(out, "    Result: {}", one_line(result, max_chars));
                    }
                }
            }
            if !project.files.is_empty() {
                let _ = writeln!(
                    out,
                    "  Files edited ({}): {}",
                    project.files.len(),
                    first(&project.files, MAX_FILES, ", ")
                );
            }
            if !project.commits.is_empty() {
                let _ = writeln!(
                    out,
                    "  Commits: {}",
                    first(&project.commits, MAX_COMMITS, "; ")
                );
            }
            if !project.shipped.is_empty() {
                let _ = writeln!(out, "  Shipped: {}", first(&project.shipped, 10, "; "));
            }
            if project.notion > 0 {
                let _ = writeln!(
                    out,
                    "  Notion pages read or changed through Claude: {}",
                    project.notion
                );
            }
            if !project.skills.is_empty() {
                let _ = writeln!(out, "  Skills used: {}", project.skills.join(", "));
            }
        }
        if !self.sites.is_empty() {
            let sites: Vec<String> = self
                .sites
                .iter()
                .map(|(site, visits)| format!("{site} ({visits})"))
                .collect();
            let _ = writeln!(
                out,
                "\nWeb (browser history)\n  Sites: {}",
                sites.join(", ")
            );
            if !self.pages.is_empty() {
                let _ = writeln!(out, "  Pages:");
                for page in &self.pages {
                    let _ = writeln!(
                        out,
                        "    {} ({}, {} visits)",
                        one_line(&page.title, 120),
                        page.site,
                        page.visits
                    );
                }
            }
        }
        out
    }
}

/// Edits that are not project work: temporary files, and Claude Code's own
/// notes and plans under `~/.claude`.
fn is_scratch(path: &str) -> bool {
    const TEMPORARY: &[&str] = &[
        "/private/tmp/",
        "/tmp/",
        "/private/var/folders/",
        "/var/folders/",
    ];
    TEMPORARY.iter().any(|prefix| path.starts_with(prefix))
        || crate::home()
            .is_some_and(|home| path.starts_with(&format!("{}/.claude/", home.display())))
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !item.is_empty() && !list.contains(&item) {
        list.push(item);
    }
}

fn first(items: &[String], max: usize, separator: &str) -> String {
    let mut text = items
        .iter()
        .take(max)
        .map(|item| one_line(item, 160))
        .collect::<Vec<_>>()
        .join(separator);
    if items.len() > max {
        let _ = write!(text, "{separator}and {} more", items.len() - max);
    }
    text
}

/// `path` relative to the project root when inside it.
fn relative(path: &str, root: &str) -> String {
    if root.is_empty() {
        return path.to_owned();
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_owned()
}

/// The `max` largest counts, largest first; ties in key order.
fn most<K: Ord + Clone>(counts: HashMap<K, usize>, max: usize) -> Vec<(K, usize)> {
    let mut sorted: Vec<(K, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.truncate(max);
    sorted
}

/// Subjects of the person's own commits in `span`, on any branch. Only
/// their own: a teammate's commits are not theirs to recap.
fn git_commits(root: &str, span: Span) -> Vec<String> {
    if root.is_empty() || !Path::new(root).join(".git").exists() {
        return Vec::new();
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // The identity git signs this person's commits with: the configured
    // email, or git's default (user@host) when none is set.
    let Some(email) = git(&["var", "GIT_AUTHOR_IDENT"])
        .and_then(|ident| Some(ident.split_once('<')?.1.split_once('>')?.0.to_owned()))
        .filter(|email| !email.is_empty())
    else {
        return Vec::new();
    };
    let at = |ms: i64| {
        time::local(ms).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M:%S").to_string())
    };
    let Some(log) = git(&[
        "log",
        "--all",
        "--no-merges",
        &format!("--since={}", at(span.start_ms)),
        &format!("--until={}", at(span.end_ms)),
        &format!("--author={email}"),
        "--format=%s",
        "-n",
        "60",
    ]) else {
        return Vec::new();
    };
    let mut subjects = Vec::new();
    for subject in log.lines() {
        push_unique(&mut subjects, subject.trim().to_owned());
    }
    subjects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewActivity;

    fn add(store: &Store, kind: &str, ts: i64, detail: &str, session: &str, project: Option<i64>) {
        let ext_id = format!("{kind}-{ts}-{detail}");
        store
            .add_activity(&NewActivity {
                source: "claude",
                kind,
                ts,
                project_id: project,
                session_id: Some(session),
                detail: Some(detail),
                ext_id: &ext_id,
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn groups_turns_by_project_and_session() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.project_id("/repo", "repo").unwrap();
        store.set_session_title("s1", "Webhooks").unwrap();
        add(
            &store,
            "reply",
            10,
            "Finished the migration.",
            "s1",
            Some(repo),
        );
        add(&store, "prompt", 20, "Fix retries", "s1", Some(repo));
        add(&store, "reply", 30, "Looking.", "s1", Some(repo));
        add(&store, "reply", 40, "Fixed; tests pass.", "s1", Some(repo));
        add(&store, "edit", 35, "/repo/src/hook.rs", "s1", Some(repo));
        add(&store, "edit", 36, "/repo/src/hook.rs", "s1", Some(repo));
        add(&store, "ship", 45, "git push origin main", "s1", Some(repo));
        add(&store, "prompt", 50, "Out of range", "s1", Some(repo));
        let span = Span {
            start_ms: 0,
            end_ms: 50,
        };
        let digest = Digest::build(&store, span, "test".into(), false).unwrap();

        assert_eq!(digest.projects.len(), 1);
        let project = &digest.projects[0];
        assert_eq!(project.files, ["src/hook.rs"]);
        assert_eq!(project.shipped, ["git push origin main"]);
        let session = &project.sessions[0];
        assert_eq!(session.title.as_deref(), Some("Webhooks"));
        assert_eq!(session.turns.len(), 2);
        assert_eq!(session.turns[0].asked, None);
        assert_eq!(session.turns[1].asked.as_deref(), Some("Fix retries"));
        assert_eq!(
            session.turns[1].result.as_deref(),
            Some("Fixed; tests pass.")
        );

        let text = digest.render(10_000);
        assert!(text.contains("Project: repo"));
        assert!(text.contains("Asked: Fix retries"));
        assert!(text.contains("Result: Fixed; tests pass."));
        assert!(!text.contains("Out of range"));
    }

    #[test]
    fn render_stays_within_budget() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..200 {
            add(&store, "prompt", i, &"long prompt ".repeat(50), "s", None);
        }
        let digest = Digest::build(
            &store,
            Span {
                start_ms: 0,
                end_ms: 1_000,
            },
            "test".into(),
            false,
        )
        .unwrap();
        let text = digest.render(2_000);
        assert!(text.chars().count() <= 2_000);
        assert!(text.contains("more turns"));
    }

    #[test]
    fn counts_sites_and_pages() {
        let store = Store::open_in_memory().unwrap();
        for (i, (title, url)) in [
            ("Board", "https://www.notion.so/a"),
            ("Board", "https://www.notion.so/a"),
            ("Repo", "https://github.com/x"),
        ]
        .iter()
        .enumerate()
        {
            store
                .add_activity(&NewActivity {
                    source: "history",
                    kind: "visit",
                    ts: 5,
                    title: Some(title),
                    url: Some(url),
                    ext_id: &i.to_string(),
                    ..Default::default()
                })
                .unwrap();
        }
        let digest = Digest::build(
            &store,
            Span {
                start_ms: 0,
                end_ms: 10,
            },
            "test".into(),
            false,
        )
        .unwrap();
        assert_eq!(
            digest.sites,
            [("notion.so".to_owned(), 2), ("github.com".to_owned(), 1)]
        );
        assert_eq!(digest.pages[0].title, "Board");
        assert_eq!(digest.pages[0].visits, 2);
    }
}
