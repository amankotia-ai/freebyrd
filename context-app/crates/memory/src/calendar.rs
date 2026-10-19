//! Puts promises on Apple Calendar through `Freebyrd Calendar.app`
//! (`crates/calendar`). The app is started with `open`, so macOS treats it
//! as its own app, with its own calendar access; it answers with one JSON
//! object, which `open` writes to a file.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::home;

/// The first call can wait on macOS asking the person for access.
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub title: String,
    /// Seconds since the Unix epoch; for an all-day event, the start of its day.
    pub start: i64,
    /// Seconds since the Unix epoch; `None` for an all-day event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
    pub all_day: bool,
    pub notes: String,
    /// An alert this many seconds after the start (negative: before).
    pub alert_after: Option<i64>,
}

pub fn app_path() -> Option<PathBuf> {
    Some(home()?.join("Library/Application Support/Freebyrd/bin/Freebyrd Calendar.app"))
}

/// Adds `event` to the Freebyrd calendar and returns its identifier.
pub fn add(event: &Event) -> Result<String> {
    let answer = run(&["add", &serde_json::to_string(event)?])?;
    answer["id"]
        .as_str()
        .map(str::to_owned)
        .context("Freebyrd Calendar gave no event identifier")
}

/// Removes an event from the Freebyrd calendar. `false` if it was gone.
pub fn remove(id: &str) -> Result<bool> {
    Ok(run(&["remove", id])?["removed"].as_bool().unwrap_or(false))
}

fn run(args: &[&str]) -> Result<Value> {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let app = app_path().filter(|app| app.exists()).context(
        "Freebyrd Calendar is not installed; run `freebyrd-calendar install` from the build folder",
    )?;
    let dir = home()
        .context("HOME is not set")?
        .join("Library/Application Support/Freebyrd/run");
    fs::create_dir_all(&dir)?;
    let out = dir.join(format!(
        "calendar-{}-{}.json",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let mut child = Command::new("/usr/bin/open")
        .args(["-W", "-g", "-n", "-a"])
        .arg(&app)
        .arg("--stdout")
        .arg(&out)
        .args(["--stderr", "/dev/null", "--args"])
        .args(args)
        .spawn()
        .context("starting Freebyrd Calendar")?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            bail!(
                "Freebyrd Calendar did not answer within {} s",
                TIMEOUT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(50));
    };
    let text = fs::read_to_string(&out).unwrap_or_default();
    let _ = fs::remove_file(&out);
    let Ok(answer) = serde_json::from_str::<Value>(text.trim()) else {
        bail!("Freebyrd Calendar gave no answer ({status})");
    };
    if let Some(error) = answer["error"].as_str() {
        bail!("{error}");
    }
    Ok(answer)
}
