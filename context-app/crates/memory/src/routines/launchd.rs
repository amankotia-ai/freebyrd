//! launchd agents for scheduled jobs: one property list per job in
//! `~/Library/LaunchAgents`, loaded into the person's login session
//! (`gui/<uid>`). Jobs run while the person is logged in; a time missed
//! while the Mac slept runs once when it wakes.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::Schedule;

pub fn label(name: &str) -> String {
    format!("com.freebyrd.job.{name}")
}

/// The property list of a job that runs `program` at `schedule`'s times,
/// from `dir`, writing what it prints to `log`.
pub fn plist(label: &str, program: &[&str], schedule: &Schedule, dir: &Path, log: &Path) -> String {
    let (hour, minute) = schedule.hour_minute();
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    out.push_str(&format!(
        "  <key>Label</key>\n  <string>{}</string>\n",
        escape(label)
    ));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for arg in program {
        out.push_str(&format!("    <string>{}</string>\n", escape(arg)));
    }
    out.push_str("  </array>\n  <key>StartCalendarInterval</key>\n  <array>\n");
    for day in &schedule.weekdays {
        out.push_str(&format!(
            "    <dict>\n      <key>Weekday</key>\n      <integer>{day}</integer>\n      \
             <key>Hour</key>\n      <integer>{hour}</integer>\n      \
             <key>Minute</key>\n      <integer>{minute}</integer>\n    </dict>\n"
        ));
    }
    out.push_str("  </array>\n");
    out.push_str(&format!(
        "  <key>WorkingDirectory</key>\n  <string>{}</string>\n",
        escape(&dir.to_string_lossy())
    ));
    for key in ["StandardOutPath", "StandardErrorPath"] {
        out.push_str(&format!(
            "  <key>{key}</key>\n  <string>{}</string>\n",
            escape(&log.to_string_lossy())
        ));
    }
    out.push_str(
        "  <key>ProcessType</key>\n  <string>Background</string>\n  \
         <key>LowPriorityIO</key>\n  <true/>\n</dict>\n</plist>\n",
    );
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Loads the agent at `path`, replacing one already loaded with its label.
pub fn load(path: &Path, label: &str) -> Result<()> {
    unload(label)?;
    launchctl(&["bootstrap", &domain()?, &path.to_string_lossy()])
}

/// Unloads the agent, if it is loaded.
pub fn unload(label: &str) -> Result<()> {
    if !is_loaded(label) {
        return Ok(());
    }
    launchctl(&["bootout", &format!("{}/{label}", domain()?)])
}

/// Starts a run now, as if its time had come.
pub fn start(label: &str) -> Result<()> {
    launchctl(&["kickstart", &format!("{}/{label}", domain()?)])
}

pub fn is_loaded(label: &str) -> bool {
    domain().is_ok_and(|domain| {
        Command::new("/bin/launchctl")
            .args(["print", &format!("{domain}/{label}")])
            .output()
            .is_ok_and(|out| out.status.success())
    })
}

/// The person's login session, `gui/<uid>`.
fn domain() -> Result<String> {
    let out = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || uid.is_empty() {
        bail!("could not tell this user's ID");
    }
    Ok(format!("gui/{uid}"))
}

fn launchctl(args: &[&str]) -> Result<()> {
    let out = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .context("running launchctl")?;
    if !out.status.success() {
        bail!(
            "launchctl {} failed: {}",
            args[0],
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_runs_on_its_days_at_its_time() {
        let schedule = Schedule::new(&[1, 3], "09:05").unwrap();
        let dir = std::env::temp_dir().join(format!("freebyrd-plist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = plist(
            &label("brief"),
            &[
                "/x/Application Support/bin/freebyrd-memory",
                "job",
                "run",
                "brief",
            ],
            &schedule,
            Path::new("/x/runs/a&b"),
            Path::new("/x/runs/a&b/launchd.log"),
        );
        assert!(text.contains("<string>com.freebyrd.job.brief</string>"));
        assert!(text.contains("<string>/x/Application Support/bin/freebyrd-memory</string>"));
        assert!(text.contains("<string>/x/runs/a&amp;b</string>"));
        assert_eq!(text.matches("<key>Weekday</key>").count(), 2);
        assert!(
            text.contains(
                "<integer>3</integer>\n      <key>Hour</key>\n      <integer>9</integer>"
            )
        );
        assert!(text.contains("<key>Minute</key>\n      <integer>5</integer>"));
        // macOS's own checker accepts it.
        let path = dir.join("job.plist");
        std::fs::write(&path, &text).unwrap();
        if let Ok(out) = Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&path)
            .output()
        {
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
