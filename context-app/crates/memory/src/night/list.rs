//! A list of tasks in a code project, worked through one item at a time:
//! what the person does by typing "next" (20 of 21 such steps in their
//! sessions needed nothing else from them, 2026-10-13).
//!
//! The work happens apart from the person's folder, in the job's folder:
//! a git worktree of the project on a branch of its own, started from their
//! uncommitted work as well as their last commit (they leave much of a day's
//! work uncommitted), or, for a project without git (Freebyrd itself), a
//! copy of it that Freebyrd keeps in a git repository of its own. One Claude
//! Code session does the items in order, resumed for each. After each, the
//! project's own check runs in the sandbox (its type check: package.json or
//! tsconfig.json, or `cargo check` for Rust); a failed check gets one try to
//! fix, then the list stops there. Each passing item is committed by
//! Freebyrd. Keep applies what the night changed to the person's folder as
//! changes they have not committed, all of it or nothing (if a file changed
//! since, nothing is applied); Discard deletes the worktree or the copy.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::blender::{Outcome, RULES};
use super::{Job, sandbox};
use crate::Store;

const ITEM_BUDGET_USD: f64 = 5.0;
/// A copy or carried-over file larger than this is left out.
const MAX_FILE_BYTES: u64 = 20 << 20;
/// A project larger than this is not copied.
const MAX_COPY_BYTES: u64 = 500 << 20;
/// Folders a copy leaves out: build output, packages (linked instead),
/// git's own, Claude Code's worktrees.
const NOT_COPIED: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".next",
    ".turbo",
    ".DS_Store",
];
const ITEM_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10 * 60);

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        // The project's hooks are not run: they can reach the network.
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        // The person's sessions never meet a lock held by a read.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The branch a job works on.
fn branch(job: &Job) -> String {
    let day = super::local(job.created_at).format("%m%d");
    format!("night/{day}-{}", job.id)
}

/// git's whole output to a file: a patch must keep its last newline.
fn git_to_file(dir: &Path, args: &[&str], path: &Path) -> Result<()> {
    let out = std::fs::File::create(path)?;
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdout(out)
        .stderr(std::process::Stdio::null())
        .status()
        .context("running git")?;
    if !status.success() {
        bail!("git {} failed", args.first().unwrap_or(&""));
    }
    Ok(())
}

pub(crate) fn run(store: &Store, job: &Job, dir: &Path) -> Result<Outcome> {
    let project = PathBuf::from(job.project.clone().unwrap_or_default());
    if !project.is_dir() {
        return Ok(failure(
            "This project's folder is not there",
            &project.to_string_lossy(),
        ));
    }
    // A git checkout gets a worktree; a folder without git, a copy.
    let repo = git(&project, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from);
    let source = repo.clone().unwrap_or_else(|| project.clone());
    let branch = branch(job);
    let tree = dir.join("repo");
    let base_file = dir.join("base");
    let start_file = dir.join("start-note");
    if !tree.is_dir() {
        let note = match &repo {
            Some(repo) => start_worktree(repo, &tree, &branch, dir)?,
            None => match start_copy(&project, &tree) {
                Ok(note) => note,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tree);
                    return Ok(failure(
                        "The project could not be copied",
                        &format!("{e:#}"),
                    ));
                }
            },
        };
        std::fs::write(&start_file, note.unwrap_or_default())?;
        std::fs::write(&base_file, git(&tree, &["rev-parse", "HEAD"])?)?;
    }
    let base = std::fs::read_to_string(&base_file)
        .context("the night's starting point is missing")?
        .trim()
        .to_owned();
    let start_note = std::fs::read_to_string(&start_file).unwrap_or_default();
    let place = match &repo {
        Some(_) => format!(
            "a git worktree of it on the branch {branch}, started from their uncommitted work \
             as well as their last commit"
        ),
        None => "a copy of it (the project has no git)".to_owned(),
    };
    link_modules(&source, &tree);
    let check = check_command(&tree);
    // A check that already fails before any change cannot judge the items.
    let baseline = check
        .as_deref()
        .map(|command| run_check(command, &tree, dir).0);
    let items: Vec<String> = job.spec["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect();
    let mut session = job.session_id.clone();
    let mut cost = 0.0;
    let mut results: Vec<Value> = Vec::new();
    let mut stopped = None;
    for (index, item) in items.iter().enumerate() {
        let _ = super::set_progress(
            store,
            job.id,
            &json!({
                "text": format!("Item {} of {}", index + 1, items.len()),
                "done": index,
                "of": items.len(),
            }),
        );
        let prompt = if index == 0 {
            format!(
                "The person queued a list for tonight in their project. This folder is {place}; \
                 work only here. I will send the items one at a time; do each fully, as they \
                 would want it, then stop. Do not commit: Freebyrd checks and commits each \
                 item.\n\nItem 1 of {}: \
                 {item}\n\nEnd with one line: DONE, or BLOCKED: what you need from the person.",
                items.len()
            )
        } else {
            format!(
                "Item {} of {}: {item}\n\nEnd with one line: DONE, or BLOCKED: what you need from \
                 the person.",
                index + 1,
                items.len()
            )
        };
        let before = git(&tree, &["rev-parse", "HEAD"])?;
        let answer = ask(&tree, &source, session.as_deref(), &prompt);
        let answer = match answer {
            Ok(answer) => answer,
            Err(e) => {
                results.push(json!({ "text": item, "state": "error", "note": format!("{e:#}") }));
                stopped = Some(format!("{e:#}"));
                break;
            }
        };
        cost += answer.cost_usd.unwrap_or(0.0);
        if let Some(id) = &answer.session_id {
            session = Some(id.clone());
            super::set_session(store, job.id, id)?;
        }
        if let Some(why) = &answer.stopped {
            results.push(json!({ "text": item, "state": "error", "note": why }));
            stopped = Some(why.clone());
            break;
        }
        let last = answer
            .text
            .trim()
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if let Some(why) = last.strip_prefix("BLOCKED:") {
            // The item waits for the person; the next ones may not need it.
            let _ = git(&tree, &["reset", "-q", "--hard", &before]);
            let _ = git(&tree, &["clean", "-fdq"]);
            results.push(
                json!({ "text": item, "state": "warn", "note": super::no_dashes(why.trim()) }),
            );
            continue;
        }
        let mut note = summary_line(&answer.text);
        if let (Some(command), Some(true)) = (check.as_deref(), baseline) {
            let (passed, output) = run_check(command, &tree, dir);
            if !passed {
                let fix = format!(
                    "The project's check `{command}` failed after that item:\n{}\nFix it without \
                     undoing the item. End with DONE or BLOCKED: why.",
                    tail_lines(&output, 60)
                );
                if let Ok(fixed) = ask(&tree, &source, session.as_deref(), &fix) {
                    cost += fixed.cost_usd.unwrap_or(0.0);
                    note = summary_line(&fixed.text);
                }
                let (passed, output) = run_check(command, &tree, dir);
                if !passed {
                    results.push(json!({
                        "text": item,
                        "state": "error",
                        "note": format!("The check still fails: {}", tail_lines(&output, 2)),
                    }));
                    stopped = Some("a check failed".into());
                    break;
                }
            }
        }
        git(&tree, &["add", "-A"])?;
        if git(&tree, &["diff", "--cached", "--quiet"]).is_err() {
            git(
                &tree,
                &["commit", "-q", "-m", &format!("Night shift: {item}")],
            )?;
        }
        // What the item changed, however it was committed.
        let after = git(&tree, &["rev-parse", "HEAD"])?;
        let commit = (after != before).then(|| after.chars().take(7).collect::<String>());
        let files = commit
            .as_ref()
            .and_then(|_| git(&tree, &["diff", "--name-only", &before, &after]).ok())
            .map(|names| names.lines().count())
            .unwrap_or(0);
        results.push(json!({
            "text": item,
            "state": "ok",
            "note": note,
            "commit": commit,
            "files": files,
        }));
    }
    let done = results.iter().filter(|item| item["state"] == "ok").count();
    let waiting = results.iter().filter(|item| item["state"] != "ok").count();
    let not_reached = items.len() - results.len();
    let mut notes = Vec::new();
    if let Some(why) = &stopped {
        notes.push(json!(["error", format!("It stopped: {why}.")]));
    }
    if baseline == Some(false) {
        notes.push(json!([
            "warn",
            "The project's check already failed before tonight, so items were not checked."
        ]));
    } else if check.is_none() {
        notes.push(json!([
            "warn",
            "No check was found for this project, so items were not checked."
        ]));
    }
    if !start_note.trim().is_empty() {
        notes.push(json!(["warn", start_note.trim()]));
    }
    notes.push(json!([
        "ok",
        "Your folder is untouched until Keep, which brings these changes in without committing them."
    ]));
    let summary = match (waiting, not_reached) {
        (0, 0) => format!("All {done} done, checked"),
        (w, 0) => format!("{done} of {} done · {w} need you", items.len()),
        (_, n) => format!("{done} of {} done · stopped with {n} to go", items.len()),
    };
    Ok(Outcome {
        ok: true,
        result: json!({
            "summary": summary,
            "needs": waiting > 0 || not_reached > 0,
            "items": results,
            "notes": notes,
            "branch": repo.as_ref().map(|_| branch),
            "place": if repo.is_some() { "its own branch" } else { "a copy of the project" },
            "source": if repo.is_some() { "git" } else { "copy" },
            "base": base,
            "repo": source.to_string_lossy(),
            "tree": tree.to_string_lossy(),
            "check": check,
            "not_reached": items.iter().skip(results.len()).collect::<Vec<_>>(),
            "cost_usd": cost,
        }),
    })
}

/// A worktree of `repo` on `branch`, with the person's uncommitted work
/// (changed and new files) carried over and committed there as the night's
/// starting point. Returns a note when some of it could not come along.
pub(crate) fn start_worktree(
    repo: &Path,
    tree: &Path,
    branch: &str,
    dir: &Path,
) -> Result<Option<String>> {
    git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            &tree.to_string_lossy(),
            "HEAD",
        ],
    )?;
    let mut note = None;
    let patch = dir.join("uncommitted.patch");
    git_to_file(repo, &["diff", "HEAD", "--binary"], &patch)?;
    if std::fs::metadata(&patch).map_or(0, |meta| meta.len()) > 0
        && git(
            tree,
            &[
                "apply",
                "--binary",
                "--whitespace=nowarn",
                &patch.to_string_lossy(),
            ],
        )
        .is_err()
    {
        note = Some(
            "Your uncommitted changes could not be carried over, so it started from your last \
             commit."
                .to_owned(),
        );
    }
    let new_files = git(repo, &["ls-files", "-o", "--exclude-standard", "-z"]).unwrap_or_default();
    let mut budget = MAX_COPY_BYTES;
    let mut left_out = 0;
    for name in new_files.split('\0').filter(|name| !name.is_empty()) {
        let from = repo.join(name);
        let size = std::fs::symlink_metadata(&from).map_or(u64::MAX, |meta| meta.len());
        if size > MAX_FILE_BYTES || size > budget || !from.is_file() {
            left_out += 1;
            continue;
        }
        budget -= size;
        let to = tree.join(name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&from, &to)?;
    }
    if left_out > 0 && note.is_none() {
        note = Some(format!(
            "{left_out} new files of yours were too large to carry over."
        ));
    }
    git(tree, &["add", "-A"])?;
    if git(tree, &["diff", "--cached", "--quiet"]).is_err() {
        git(
            tree,
            &[
                "commit",
                "-q",
                "-m",
                "Night shift: your uncommitted work, as it was",
            ],
        )?;
    }
    Ok(note)
}

/// A copy of `project` (a folder without git) in `tree`, kept in a git
/// repository of Freebyrd's own so each item can be committed and Keep can
/// bring back only what the night changed.
pub(crate) fn start_copy(project: &Path, tree: &Path) -> Result<Option<String>> {
    let mut budget = MAX_COPY_BYTES;
    let mut left_out = 0;
    copy_dir(project, tree, &mut budget, &mut left_out)?;
    git(tree, &["init", "-q"])?;
    git(tree, &["config", "user.name", "Freebyrd night shift"])?;
    git(
        tree,
        &["config", "user.email", "night-shift@freebyrd.invalid"],
    )?;
    git(tree, &["add", "-A"])?;
    git(
        tree,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Night shift: the project as it was",
        ],
    )?;
    Ok((left_out > 0).then(|| format!("{left_out} files over 20 MB were left out of the copy.")))
}

fn copy_dir(from: &Path, to: &Path, budget: &mut u64, left_out: &mut usize) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        let kind = entry.file_type()?;
        if NOT_COPIED.contains(&name_text.as_ref()) || kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if name_text == "worktrees" && from.ends_with(".claude") {
                continue;
            }
            copy_dir(&path, &to.join(&name), budget, left_out)?;
        } else if kind.is_file() {
            let size = entry.metadata()?.len();
            if size > MAX_FILE_BYTES {
                *left_out += 1;
                continue;
            }
            if size > *budget {
                bail!("the project is larger than 500 MB");
            }
            *budget -= size;
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn failure(summary: &str, detail: &str) -> Outcome {
    Outcome {
        ok: false,
        result: json!({ "summary": summary, "needs": true, "notes": [["error", detail]] }),
    }
}

fn ask(
    tree: &Path,
    repo: &Path,
    session: Option<&str>,
    prompt: &str,
) -> Result<claude_cli::AgentRun> {
    claude_cli::agent(
        &claude_cli::Agent {
            dir: tree,
            model: super::WORK_MODEL,
            effort: "high",
            budget_usd: ITEM_BUDGET_USD,
            resume: session,
            timeout: ITEM_TIMEOUT,
            rules: RULES,
            // The project's packages are linked from the person's checkout.
            read: &[repo],
        },
        prompt,
    )
}

/// The first sentence of what an answer says was done, without DONE.
fn summary_line(text: &str) -> String {
    let line = text
        .trim()
        .lines()
        .map(|line| line.trim().trim_start_matches(['-', '*', '#', ' ']))
        .find(|line| !line.is_empty() && *line != "DONE" && !line.starts_with("BLOCKED"))
        .unwrap_or_default();
    let sentence = line.split(". ").next().unwrap_or(line);
    super::no_dashes(&sentence.chars().take(160).collect::<String>())
}

fn tail_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// The worktree has no installed packages: link the project's, so checks
/// can run without the network. They are read through the link, not
/// changed (the sandbox allows no writes there).
fn link_modules(repo: &Path, tree: &Path) {
    for relative in module_dirs(repo) {
        let target = repo.join(&relative);
        let link = tree.join(&relative);
        if link.exists() || !link.parent().is_some_and(Path::is_dir) {
            continue;
        }
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&target, &link);
    }
}

/// `node_modules` folders of the project and its packages, two levels deep.
fn module_dirs(repo: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if repo.join("node_modules").is_dir() {
        dirs.push(PathBuf::from("node_modules"));
    }
    for group in ["packages", "apps", "services", "libs"] {
        for entry in std::fs::read_dir(repo.join(group))
            .into_iter()
            .flatten()
            .flatten()
        {
            let relative = PathBuf::from(group)
                .join(entry.file_name())
                .join("node_modules");
            if repo.join(&relative).is_dir() {
                dirs.push(relative);
            }
        }
    }
    dirs.truncate(50);
    dirs
}

/// Where `cargo check` builds for the night: its own folder, kept between
/// nights so only the first check is slow (about 40 s and 900 MB for
/// Freebyrd, 2026-10-14).
fn night_target() -> Option<PathBuf> {
    crate::home().map(|home| home.join("Library/Caches/Freebyrd/night-target"))
}

/// The project's type check: a package.json script named for it, tsc with
/// the project's tsconfig, or `cargo check` without the network for Rust.
pub(crate) fn check_command(tree: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(tree.join("package.json"))
        && let Ok(package) = serde_json::from_str::<Value>(&text)
    {
        for name in ["typecheck", "type-check", "check-types", "tsc", "types"] {
            if package["scripts"][name].is_string() {
                return Some(format!("npm run -s {name}"));
            }
        }
    }
    if tree.join("tsconfig.json").is_file() && tree.join("node_modules/.bin/tsc").exists() {
        return Some("./node_modules/.bin/tsc --noEmit -p .".to_owned());
    }
    if tree.join("Cargo.toml").is_file() {
        let cargo = crate::home()?.join(".cargo/bin/cargo");
        let target = night_target()?;
        if cargo.is_file() {
            return Some(format!(
                "CARGO_TARGET_DIR='{}' '{}' check --offline --workspace --quiet",
                target.display(),
                cargo.display()
            ));
        }
    }
    None
}

/// Runs a check in the sandbox: whether it passed, and what it printed.
fn run_check(command: &str, tree: &Path, dir: &Path) -> (bool, String) {
    let mut writable = vec![tree.to_path_buf()];
    if let Some(target) = night_target().filter(|_| command.contains("CARGO_TARGET_DIR")) {
        let _ = std::fs::create_dir_all(&target);
        writable.push(target);
    }
    match sandbox::run(
        command,
        tree,
        &writable,
        &[],
        &dir.join("check.log"),
        CHECK_TIMEOUT,
        || {},
    ) {
        Ok(ended) => (ended.ok, ended.tail),
        Err(e) => (false, format!("{e:#}")),
    }
}

/// Keep: applies what the night changed (from its starting point to its
/// last item) to the person's folder, as changes they have not committed,
/// all of it or nothing, then removes the worktree or the copy.
pub(crate) fn keep(job: &Job) -> Result<()> {
    keep_result(job.result.as_ref().context("nothing was made")?)
}

pub(crate) fn keep_result(result: &Value) -> Result<()> {
    let Some(base) = result["base"].as_str() else {
        return merge(result);
    };
    let folder = PathBuf::from(result["repo"].as_str().context("no project")?);
    let tree = PathBuf::from(result["tree"].as_str().context("no work folder")?);
    let patch = tree.with_file_name("keep.patch");
    git_to_file(&tree, &["diff", "--binary", base, "HEAD"], &patch)?;
    if std::fs::metadata(&patch).map_or(0, |meta| meta.len()) > 0 {
        if let Err(why) = apply(&folder, &patch, true) {
            bail!(
                "Nothing was changed: some of these files changed in {} since the night started \
                 ({why}). Its work is still in {}",
                folder.display(),
                tree.display()
            );
        }
        apply(&folder, &patch, false).map_err(|why| anyhow::anyhow!("Keep failed: {why}"))?;
    }
    remove(result)
}

/// `git apply` of `patch` in `folder` (a checkout or a plain folder), or
/// with `check` only whether it would apply.
fn apply(folder: &Path, patch: &Path, check: bool) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .current_dir(folder)
        .args(["apply", "--binary", "--whitespace=nowarn"]);
    if check {
        command.arg("--check");
    }
    let out = command
        .arg(patch)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&out.stderr);
    let first = error
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("it did not apply");
    Err(first.trim().trim_start_matches("error: ").to_owned())
}

/// Keep for a list run before 2026-10-14: merges its branch into the
/// person's branch, if they have no uncommitted changes.
fn merge(result: &Value) -> Result<()> {
    let repo = PathBuf::from(result["repo"].as_str().context("no repository")?);
    let branch = result["branch"].as_str().context("no branch")?;
    if !git(&repo, &["status", "--porcelain"])?.is_empty() {
        bail!("Commit or stash your changes in {} first", repo.display());
    }
    git(&repo, &["merge", "--no-ff", "--no-edit", branch])?;
    if let Some(tree) = result["tree"].as_str() {
        let _ = git(&repo, &["worktree", "remove", "--force", tree]);
    }
    git(&repo, &["branch", "-d", branch])?;
    Ok(())
}

/// Discard: removes the worktree and deletes its branch, or the copy.
pub(crate) fn discard(job: &Job) -> Result<()> {
    remove(job.result.as_ref().context("nothing was made")?)
}

fn remove(result: &Value) -> Result<()> {
    let tree = result["tree"].as_str();
    if result["source"] == "copy" {
        if let Some(tree) = tree {
            std::fs::remove_dir_all(tree)?;
        }
        return Ok(());
    }
    let repo = PathBuf::from(result["repo"].as_str().context("no repository")?);
    let branch = result["branch"].as_str().context("no branch")?;
    if let Some(tree) = tree {
        let _ = git(&repo, &["worktree", "remove", "--force", tree]);
    }
    git(&repo, &["branch", "-D", branch])?;
    Ok(())
}
