//! macOS's sandbox for what night shift runs itself: the render scripts
//! Claude Code wrote and the project's checks. Like Claude Code's own
//! sandbox they may write only in the folders given and the temporary
//! folders, and reach no other machine. Checked live on 2026-10-13: Blender
//! 5.1 rendered on the GPU (Metal) inside it, while a write to the home
//! folder and a request to the internet were refused.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// The profile: everything allowed except writing outside `writable` (and
/// the temporary folders) and connecting to other machines.
pub fn profile(writable: &[PathBuf]) -> String {
    let mut allowed = String::new();
    for dir in writable {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        allowed.push_str(&format!(
            " (subpath \"{}\")",
            quote(&real.to_string_lossy())
        ));
    }
    format!(
        "(version 1)(allow default)(deny network-outbound (remote ip))(deny file-write*)\
         (allow file-write*{allowed} (subpath \"/private/tmp\") (subpath \"/private/var/folders\") \
         (literal \"/dev/null\") (literal \"/dev/zero\") (regex #\"^/dev/tty\") (regex #\"^/dev/fd/\"))"
    )
}

fn quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// How a sandboxed command ended.
#[derive(Debug, Clone)]
pub struct Ended {
    pub ok: bool,
    pub timed_out: bool,
    /// The end of what it printed.
    pub tail: String,
}

/// Runs `script` with bash in the sandbox, from `cwd`, printing to `log`.
/// `tick` runs every few seconds while it works (to report progress); after
/// `timeout` the whole process group is stopped.
pub fn run(
    script: &str,
    cwd: &Path,
    writable: &[PathBuf],
    env: &[(&str, String)],
    log: &Path,
    timeout: Duration,
    mut tick: impl FnMut(),
) -> Result<Ended> {
    let out = File::create(log).with_context(|| format!("creating {}", log.display()))?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-p")
        .arg(profile(writable))
        .args(["/bin/bash", "-c", script])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("starting the sandbox")?;
    let started = Instant::now();
    let mut last_tick = Instant::now();
    let (ok, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status.success(), false);
        }
        if started.elapsed() >= timeout {
            stop_group(child.id());
            let _ = child.wait();
            break (false, true);
        }
        if last_tick.elapsed() >= Duration::from_secs(5) {
            tick();
            last_tick = Instant::now();
        }
        thread::sleep(Duration::from_millis(500));
    };
    Ok(Ended {
        ok,
        timed_out,
        tail: tail(log, 4000),
    })
}

/// Stops a process group started with `process_group(0)`.
fn stop_group(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    thread::sleep(Duration::from_secs(2));
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
}

/// The last `max` bytes of a file, as text.
pub fn tail(path: &Path, max: u64) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(max)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_allows_writes_only_where_given() {
        let profile = profile(&[PathBuf::from("/nonexistent/job \"one\"")]);
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("(deny network-outbound (remote ip))"));
        assert!(profile.contains("(subpath \"/nonexistent/job \\\"one\\\"\")"));
        assert!(profile.contains("(subpath \"/private/tmp\")"));
    }

    #[test]
    fn a_sandboxed_script_writes_in_its_folder_only() {
        let dir = std::env::temp_dir().join(format!("freebyrd-sbx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let outside = crate::home()
            .unwrap()
            .join(format!(".freebyrd-sbx-{}", std::process::id()));
        let script = format!(
            "echo in > inside.txt; echo out > '{}'; echo $GREETING",
            outside.display()
        );
        let ended = run(
            &script,
            &dir,
            std::slice::from_ref(&dir),
            &[("GREETING", "hello".to_owned())],
            &dir.join("log.txt"),
            Duration::from_secs(30),
            || {},
        )
        .unwrap();
        assert!(dir.join("inside.txt").is_file());
        assert!(!outside.exists(), "wrote outside its folder");
        assert!(ended.tail.contains("hello"));
        assert!(!ended.timed_out);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
