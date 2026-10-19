//! Running the queue. The app checks every minute (`start_if_due`): once the
//! person has been away `AWAY_MIN` minutes after `FROM_HOUR`, on power, with
//! jobs queued, it starts `freebyrd-memory night run` as a process of its
//! own under `caffeinate`, which keeps the Mac awake (the lid must stay
//! open). Start now starts it at once. The runner takes the jobs in order,
//! one at a time; when the queue is empty it leaves one card that makes the
//! notch glow, finished work on the left.

use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde_json::json;

use super::{Job, Kind, base_dir, blender, list};
use crate::{Store, cards, time};

/// The queue starts by itself once the person has been away this long...
pub const AWAY_MIN: u64 = 20;
/// ...between these hours (the person stops around 03:00).
pub const FROM_HOUR: u32 = 2;
pub const UNTIL_HOUR: u32 = 12;
/// A night stops starting jobs once it has cost this much.
const NIGHT_BUDGET_USD: f64 = 40.0;

fn lock_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("run.lock"))
}

/// The runner's process ID, if one is working now.
fn running_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(lock_path().ok()?)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let alive = Command::new("/bin/ps")
        .args(["-p", &pid.to_string()])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    alive.then_some(pid)
}

pub fn is_running() -> bool {
    running_pid().is_some()
}

/// Whether the Mac runs on power, not its battery.
fn on_power() -> bool {
    Command::new("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("AC Power"))
}

/// Whether a night shift should start now, by itself.
pub fn due(
    store: &Store,
    now: DateTime<Local>,
    idle_seconds: Option<u64>,
    power: bool,
) -> Result<bool> {
    if !super::in_night_window(now) || !power {
        return Ok(false);
    }
    if idle_seconds.is_none_or(|idle| idle < AWAY_MIN * 60) {
        return Ok(false);
    }
    let queued: i64 = store.conn().query_row(
        "SELECT count(*) FROM night_job WHERE state = 'queued'",
        [],
        |row| row.get(0),
    )?;
    Ok(queued > 0 && !is_running())
}

/// The app's check, every minute. Returns a line for the log when it
/// started the runner.
pub fn start_if_due(store: &Store, now: DateTime<Local>) -> Option<String> {
    let idle = super::idle_seconds();
    match due(store, now, idle, on_power()) {
        Ok(true) => Some(match spawn(None) {
            Ok(()) => "night shift: started (away, on power)".to_owned(),
            Err(e) => format!("night shift: could not start: {e:#}"),
        }),
        Ok(false) => None,
        Err(e) => Some(format!("night shift: {e:#}")),
    }
}

/// `freebyrd-memory`, beside the running program or in Freebyrd's bin.
fn memory_program() -> Result<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("freebyrd-memory")))
        .filter(|path| path.is_file());
    let installed = crate::home()
        .map(|home| home.join("Library/Application Support/Freebyrd/bin/freebyrd-memory"))
        .filter(|path| path.is_file());
    beside
        .or(installed)
        .context("freebyrd-memory was not found beside Freebyrd")
}

/// Starts the runner as a process of its own, kept awake by `caffeinate`
/// (idle and system sleep, on power), unless one is working already: for
/// the whole queue, or `only` one job (Run now).
pub fn spawn(only: Option<i64>) -> Result<()> {
    if is_running() {
        return Ok(());
    }
    let base = base_dir()?;
    std::fs::create_dir_all(&base)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(base.join("run.log"))?;
    let mut command = Command::new("/usr/bin/caffeinate");
    command
        .args(["-i", "-s"])
        .arg(memory_program()?)
        .args(["night", "run"])
        .args(only.map(|id| id.to_string()))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    command.process_group(0);
    command.spawn().context("starting the night shift")?;
    Ok(())
}

/// Holds the runner's lock while it works.
struct Lock(PathBuf);

impl Lock {
    fn take() -> Result<Option<Self>> {
        if is_running() {
            return Ok(None);
        }
        let path = lock_path()?;
        std::fs::create_dir_all(path.parent().context("no folder")?)?;
        let mut file = std::fs::File::create(&path)?;
        write!(file, "{}", std::process::id())?;
        Ok(Some(Self(path)))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What a night's run did, for the log.
#[derive(Debug, Default)]
pub struct Summary {
    pub ran: usize,
    pub needs: usize,
    pub left: usize,
}

/// Runs every queued job in order, then leaves the morning card; or `only`
/// one job (Run now), then a card for it. Returns `None` when another
/// runner is working.
pub fn run_all(store: &Store, only: Option<i64>) -> Result<Option<Summary>> {
    let Some(_lock) = Lock::take()? else {
        return Ok(None);
    };
    let mut summary = Summary::default();
    let mut spent = 0.0;
    loop {
        if spent >= NIGHT_BUDGET_USD {
            break;
        }
        let Some(job) = next(store, only)? else {
            break;
        };
        let id = job.id;
        let dir = job_dir(&job)?;
        std::fs::create_dir_all(&dir)?;
        super::set_running(store, job.id, &dir, time::now_ms())?;
        eprintln!("night shift: {} ({})", job.title, job.kind.as_str());
        let outcome = match job.kind {
            Kind::Preview => blender::preview(store, &job, &dir),
            Kind::Final => blender::final_(store, &job, &dir),
            Kind::List => list::run(store, &job, &dir),
        };
        let (state, result) = match outcome {
            Ok(outcome) => (if outcome.ok { "done" } else { "failed" }, outcome.result),
            Err(e) => (
                "failed",
                json!({ "summary": "It stopped with an error", "needs": true, "notes": [["error", format!("{e:#}")]] }),
            ),
        };
        spent += result["cost_usd"].as_f64().unwrap_or(0.0);
        if usage_limit(&result) {
            // Claude's usage ran out: put it back and stop for now; the next
            // check starts again once the person is still away.
            super::requeue(store, job.id)?;
            break;
        }
        super::finish(store, job.id, state, &result, time::now_ms())?;
        summary.ran += 1;
        if result["needs"].as_bool().unwrap_or(false) || state == "failed" {
            summary.needs += 1;
        }
        if only.is_some() {
            done_card(store, id, time::now_ms())?;
            break;
        }
    }
    summary.left = super::tonight(store)?.len();
    if summary.ran > 0 && only.is_none() {
        morning_card(store, time::now_ms())?;
    }
    Ok(Some(summary))
}

fn usage_limit(result: &serde_json::Value) -> bool {
    let text = result.to_string().to_lowercase();
    text.contains("usage limit") || text.contains("rate limit") || text.contains("limit reached")
}

fn next(store: &Store, only: Option<i64>) -> Result<Option<Job>> {
    let jobs = match only {
        Some(id) => super::select(store, "state = 'queued' AND id = ?1", [id])?,
        None => super::select(
            store,
            "state = 'queued' ORDER BY kind = 'final', position LIMIT 1",
            [],
        )?,
    };
    Ok(jobs.into_iter().next())
}

/// A card for a job run now, when it is done.
pub fn done_card(store: &Store, id: i64, now_ms: i64) -> Result<()> {
    let Some(job) = super::get(store, id)? else {
        return Ok(());
    };
    let summary = job
        .result
        .as_ref()
        .and_then(|result| result["summary"].as_str())
        .unwrap_or_default();
    let title = if super::needs_person(&job) {
        format!("Needs you: {}", job.title)
    } else {
        format!("Done: {}", job.title)
    };
    cards::upsert_with(
        store,
        "night",
        &format!("night:job:{id}"),
        &title,
        &format!("{summary}\n\nOpen the moon at the top right to see it."),
        &[cards::Action::new("night", "Open")],
        Some(&json!({ "jobs": [id] })),
        now_ms,
    )
}

/// A job's folder: `night/<date>/<id>`, kept for a preview's final.
fn job_dir(job: &Job) -> Result<PathBuf> {
    if let Some(dir) = &job.dir {
        return Ok(PathBuf::from(dir));
    }
    let day = super::local(job.created_at).format("%Y-%m-%d");
    Ok(base_dir()?.join(day.to_string()).join(job.id.to_string()))
}

/// One card for the night: what needs the person and what is ready.
pub fn morning_card(store: &Store, now_ms: i64) -> Result<()> {
    let jobs = super::waiting(store, now_ms)?;
    if jobs.is_empty() {
        return Ok(());
    }
    let needs: Vec<&Job> = jobs.iter().filter(|job| super::needs_person(job)).collect();
    let title = match needs.len() {
        0 => format!("Night shift: {} ready", jobs.len()),
        n => format!("Night shift: {} ready, {n} need you", jobs.len()),
    };
    let mut lines = Vec::new();
    for job in needs
        .iter()
        .copied()
        .chain(jobs.iter().filter(|job| !super::needs_person(job)))
    {
        let summary = job
            .result
            .as_ref()
            .and_then(|result| result["summary"].as_str())
            .unwrap_or_default();
        lines.push(format!("- **{}**: {summary}", job.title));
    }
    lines.push(String::new());
    lines.push("Open the moon at the top right to see them.".to_owned());
    let day = time::working_day(super::local(now_ms));
    cards::upsert_with(
        store,
        "night",
        &format!("night:{day}"),
        &title,
        &lines.join("\n"),
        &[cards::Action::new("night", "Open")],
        Some(&json!({ "jobs": jobs.iter().map(|job| job.id).collect::<Vec<_>>() })),
        now_ms,
    )
}

/// Where the runner writes, for the person to read.
pub fn log_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("run.log"))
}
