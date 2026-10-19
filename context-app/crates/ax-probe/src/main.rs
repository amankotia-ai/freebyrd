use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use capture::ax::{self, Element};
use capture::bridge::{self, ReadError};
use capture::browser::{PageSnapshot, PageStore};
use capture::snapshot::{self, Limits, WindowScope};
use capture::text;
use clap::Parser;

/// Longer than the host's own wait for the extension, so a slow extension
/// comes back as a refusal rather than a client timeout.
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Read another app's on-screen content through the macOS Accessibility API.
///
/// With no target, reads the app that has keyboard focus.
#[derive(Parser)]
struct Args {
    /// Process ID of the app to read.
    #[arg(long, conflicts_with = "app")]
    pid: Option<i32>,

    /// Exact process name of the app to read, as `pgrep -x` matches it (for example "TextEdit").
    #[arg(long)]
    app: Option<String>,

    /// Read every window instead of only the focused one.
    #[arg(long)]
    all_windows: bool,

    /// Ask Chromium and Electron apps to build their Accessibility tree before reading.
    #[arg(long)]
    enable_web_ax: bool,

    /// Print the element tree as JSON instead of extracted text.
    #[arg(long)]
    json: bool,

    /// Also read each web element's classes (`AXDOMClassList`), for `--json`.
    #[arg(long)]
    dom_classes: bool,

    #[arg(long, default_value_t = Limits::default().max_depth)]
    max_depth: usize,

    #[arg(long, default_value_t = Limits::default().max_nodes)]
    max_nodes: usize,

    /// Seconds to wait for each reply from the target app.
    #[arg(long, default_value_t = 1.0)]
    timeout: f32,

    /// Only report whether this process may use the Accessibility API.
    #[arg(long)]
    check: bool,

    /// With --check, show the macOS permission prompt when access is missing.
    #[arg(long, requires = "check")]
    prompt: bool,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<()> {
    if args.check {
        let trusted = ax::is_trusted(args.prompt);
        println!(
            "accessibility access: {}",
            if trusted { "granted" } else { "not granted" }
        );
        if !trusted {
            bail!("grant access in System Settings > Privacy & Security > Accessibility");
        }
        return Ok(());
    }

    if let Some(system) = Element::system_wide() {
        system.set_messaging_timeout(args.timeout);
    }

    let pid = match (args.pid, &args.app) {
        (Some(pid), _) => pid,
        (None, Some(name)) => pid_for_process_name(name)?,
        (None, None) => snapshot::focused_app_pid()?,
    };

    if args.enable_web_ax {
        let request = snapshot::enable_web_accessibility(pid)?;
        eprintln!(
            "AXManualAccessibility: {:?}, AXEnhancedUserInterface: {:?}",
            request.manual_accessibility, request.enhanced_user_interface
        );
        // Chromium builds the tree asynchronously after the first request.
        thread::sleep(Duration::from_millis(500));
    }

    let scope = if args.all_windows {
        WindowScope::All
    } else {
        WindowScope::Focused
    };
    let limits = Limits {
        max_depth: args.max_depth,
        max_nodes: args.max_nodes,
        dom_classes: args.dom_classes,
        ..Limits::default()
    };
    let snap = snapshot::snapshot_app(pid, scope, limits)?;

    let stats = &snap.stats;
    eprintln!(
        "app={} pid={} windows={} nodes={} failed_reads={} truncated={} elapsed={}ms",
        snap.app_name.as_deref().unwrap_or("?"),
        snap.pid,
        snap.windows.len(),
        stats.nodes,
        stats.failed_reads,
        stats.truncated,
        stats.elapsed_ms,
    );

    if args.json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
    } else {
        println!("{}", text::to_text(&snap));
        // Browsers hide page content from Accessibility; the extension fills the gap.
        if let Some((page, source)) = browser_page_for(&snap)? {
            println!(
                "\n## Page text from the browser extension ({source})\n{}",
                page.text
            );
        }
    }
    Ok(())
}

/// Asks the browser's extension for the current page, falling back to the
/// last stored snapshot when no extension is connected or it does not answer.
/// A refusal (paused, excluded page) is final: nothing stale is shown instead.
fn browser_page_for(snap: &snapshot::AppSnapshot) -> Result<Option<(PageSnapshot, String)>> {
    let started = Instant::now();
    match bridge::read_active_page(u32::try_from(snap.pid)?, None, BRIDGE_TIMEOUT) {
        Ok(page) => {
            let source = format!("live, {} ms", started.elapsed().as_millis());
            return Ok(Some((page, source)));
        }
        Err(ReadError::NoBridge) => {}
        Err(ReadError::Refused(reason)) => {
            eprintln!("browser extension declined: {reason}");
            return Ok(None);
        }
        Err(e) => eprintln!("live page read failed: {e}"),
    }

    let Some(dir) = PageStore::default_dir().filter(|dir| dir.exists()) else {
        return Ok(None);
    };
    let titles = snap.windows.iter().filter_map(|w| w.title.as_deref());
    let page = PageStore::open(dir)?.latest_shown_in(titles)?;
    Ok(page.map(|page| (page, "last stored snapshot".to_owned())))
}

fn pid_for_process_name(name: &str) -> Result<i32> {
    let output = Command::new("/usr/bin/pgrep")
        .args(["-x", name])
        .output()
        .context("running pgrep")?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
        .with_context(|| format!("no running process named {name:?}"))
}
