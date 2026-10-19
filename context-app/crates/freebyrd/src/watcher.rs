//! Watches the window in front for things that belong on a calendar: a
//! meeting time in a chat, a deadline in an email, a launch date in Notion.
//!
//! Every few seconds it notes the app in front. When that changes, and
//! every 15 s while it stays, it reads the window as the overlay does (the
//! browser extension for web pages, Accessibility otherwise, never password
//! managers or System Settings) and keeps the lines that are new and look
//! like an event (`memory::screen::candidates`), from the person's own
//! places only (`memory::screen::is_own_place`). At most every 30 s, those
//! lines go to Claude to be filled in as events; each new one becomes a
//! nudge card and the notch lights up. Windows read through Accessibility
//! are also kept in the memory, at most every 2 minutes each.
//!
//! The same reads tell `memory::moments` what the person just started: a
//! page or window that stays in front, their messages on an AI chat site or
//! in a chat in the Claude app (their own only, `claude_app`), or, while a
//! terminal, an editor or the Claude app's Code tab is in front, their last
//! request to Claude Code. Each new moment is weighed on a thread of its own
//! (Jev and Claude take seconds), and a card that comes of it lights the
//! notch.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use memory::Store;
use memory::moments::{self, Moment, Tracker};
use memory::screen::{self, Snippet};
use memory::store::NewActivity;

use crate::claude_app::{self, Front};
use crate::context::{self, Source};
use crate::whatsapp;

const TICK: Duration = Duration::from_secs(3);
const REREAD: Duration = Duration::from_secs(15);
const SEND_EVERY: Duration = Duration::from_secs(30);
const KEEP_EVERY: Duration = Duration::from_secs(120);
const REMEMBERED_WINDOWS: usize = 64;
/// A request to Claude Code this recent is the moment while a terminal, an
/// editor or the Claude app is in front.
const PROMPT_FRESH: Duration = Duration::from_secs(10 * 60);

/// Not watched: code and terminals are full of dates that are not plans,
/// Claude's sessions are read from their transcripts, and a locked screen
/// shows nothing of the person's.
const SKIPPED_APPS: &[&str] = &[
    "Claude",
    "Code",
    "Cursor",
    "Ghostty",
    "iTerm2",
    "loginwindow",
    "ScreenSaverEngine",
    "Terminal",
    "Visual Studio Code",
    "Warp",
    "Xcode",
];

struct Window {
    text: String,
    kept_at: Option<Instant>,
}

/// What the watcher keeps between reads.
struct Seen {
    last_read: Option<(i32, Instant)>,
    windows: HashMap<String, Window>,
    pending: Vec<Snippet>,
    tracker: Tracker,
    /// Where new moments go to be weighed; `None` when moments are off.
    moments: Option<Sender<Moment>>,
}

/// Starts the watcher. `is_open` says whether the overlay is open (then
/// nothing is read); `on_nudge` runs when new nudges or an Idea card were
/// added.
pub fn start(
    is_open: impl Fn() -> bool + Send + 'static,
    on_nudge: impl Fn() + Send + Sync + 'static,
) {
    if std::env::var_os("FREEBYRD_NO_WATCH").is_some() {
        eprintln!("freebyrd: watching the screen is off (FREEBYRD_NO_WATCH is set)");
        return;
    }
    let on_nudge = Arc::new(on_nudge);
    let moments = if std::env::var_os("FREEBYRD_NO_MOMENTS").is_some() {
        eprintln!("freebyrd: ideas at the moment are off (FREEBYRD_NO_MOMENTS is set)");
        None
    } else {
        let (sender, receiver) = mpsc::channel();
        let glow = Arc::clone(&on_nudge);
        let spawned = thread::Builder::new()
            .name("moments".to_owned())
            .spawn(move || weigh_moments(&receiver, &*glow));
        match spawned {
            Ok(_) => Some(sender),
            Err(e) => {
                eprintln!("freebyrd: moments: could not start: {e}");
                None
            }
        }
    };
    let spawned = thread::Builder::new()
        .name("watcher".to_owned())
        .spawn(move || {
            let mut seen = Seen {
                last_read: None,
                windows: HashMap::new(),
                pending: Vec::new(),
                tracker: Tracker::default(),
                moments,
            };
            let mut last_sent = Instant::now();
            loop {
                thread::sleep(TICK);
                // Paused from the iPad (`companion`): nothing is read.
                if !is_open() && crate::companion::paused_until().is_none() {
                    read_front(&mut seen);
                }
                let pending = &mut seen.pending;
                if !pending.is_empty() && last_sent.elapsed() >= SEND_EVERY {
                    last_sent = Instant::now();
                    let batch = std::mem::take(pending);
                    match Store::open_default()
                        .and_then(|store| screen::nudge(&store, &batch, Local::now()))
                    {
                        Ok(added) if !added.is_empty() => {
                            eprintln!(
                                "freebyrd: watcher: {} new nudges: {}",
                                added.len(),
                                added.join("; ")
                            );
                            on_nudge();
                        }
                        Ok(_) => {
                            eprintln!("freebyrd: watcher: {} snippets, no new events", batch.len())
                        }
                        Err(e) => eprintln!("freebyrd: watcher: {e:#}"),
                    }
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("freebyrd: watcher: could not start: {e}");
    }
}

/// Weighs each new moment the watcher sends, the newest first: one that
/// waited while another was weighed is stale once a newer one arrives.
fn weigh_moments(moments: &Receiver<Moment>, on_card: &dyn Fn()) {
    while let Ok(mut moment) = moments.recv() {
        while let Ok(newer) = moments.try_recv() {
            moment = newer;
        }
        let weighed = Store::open_default().and_then(|store| {
            let before = memory::cards::newest_waiting(&store)?;
            let helpers = moments::helpers(&store);
            let line = moments::weigh(&store, &moment, Local::now(), &helpers)?;
            let after = memory::cards::newest_waiting(&store)?;
            Ok((line, after > before))
        });
        match weighed {
            Ok((line, card)) => {
                if let Some(line) = line {
                    eprintln!("freebyrd: memory: {line}");
                }
                if card {
                    on_card();
                }
            }
            Err(e) => eprintln!("freebyrd: moments: {e:#}"),
        }
    }
}

/// Tells the tracker what is in front, and sends it on when it has become a
/// moment.
fn observe(seen: &mut Seen, moment: Option<Moment>) {
    let Some(sender) = &seen.moments else {
        return;
    };
    let now_ms = memory::time::now_ms();
    if let Some(moment) = seen.tracker.observe(moment, now_ms) {
        let _ = sender.send(moment);
    }
}

/// The person's last request to Claude Code, if they typed it lately, as a
/// moment in `app`.
fn code_moment(app: &str) -> Option<Moment> {
    let since = memory::time::now_ms() - i64::try_from(PROMPT_FRESH.as_millis()).ok()?;
    let latest = memory::ingest::claude_code::latest_prompt(since)?;
    let title: String = Store::open_default()
        .ok()
        .and_then(|store| {
            store
                .conn()
                .query_row(
                    "SELECT coalesce(title, '') FROM session WHERE id = ?1",
                    [&latest.session_id],
                    |row| row.get(0),
                )
                .ok()
        })
        .unwrap_or_default();
    Moment::from_prompt(app, &latest, &title)
}

fn read_front(seen: &mut Seen) {
    let Some(target) = context::frontmost_app() else {
        return;
    };
    let due = seen
        .last_read
        .is_none_or(|(pid, at)| pid != target.pid || at.elapsed() >= REREAD);
    if !due {
        return;
    }
    seen.last_read = Some((target.pid, Instant::now()));
    let name = target.name.clone().unwrap_or_default();
    // Terminals, editors and the Claude app's Code tab: the moment is the
    // person's last request to Claude Code; their windows are not read. A
    // chat in the Claude app is read for the person's own messages only.
    if moments::CODE_APPS.contains(&name.as_str()) {
        let moment = if name == claude_app::APP_NAME {
            match claude_app::read(target.pid) {
                Some(Front::Chat {
                    title,
                    url,
                    messages,
                }) => Moment::from_claude_app(&title, &url, &messages),
                Some(Front::Other) => None,
                Some(Front::Code) | None => code_moment(&name),
            }
        } else {
            code_moment(&name)
        };
        observe(seen, moment);
        return;
    }
    if SKIPPED_APPS.contains(&name.as_str()) {
        observe(seen, None);
        return;
    }
    let (windows, pending) = (&mut seen.windows, &mut seen.pending);
    if name == whatsapp::APP_NAME {
        read_whatsapp(target.pid, windows, pending);
        observe(seen, None);
        return;
    }
    let screen = context::capture(&target);
    if screen.withheld.is_some() || screen.text.trim().is_empty() {
        observe(seen, None);
        return;
    }
    let app = screen.app_name.clone().unwrap_or_default();
    let window = screen.window_title.clone().unwrap_or_default();
    let moment = match (&screen.source, screen.url.as_deref()) {
        (Source::BrowserExtension, Some(url)) => {
            Moment::from_page(&app, url, &window, screen.ask.as_deref())
        }
        _ => Moment::from_window(&app, &window),
    };
    observe(seen, moment);
    let (windows, pending) = (&mut seen.windows, &mut seen.pending);
    let key = format!("{app}\u{1f}{window}");
    let previous = windows.get(&key).map(|seen| seen.text.as_str());
    if previous == Some(screen.text.as_str()) {
        return;
    }
    if let Some(text) = screen::candidates(previous, &screen.text) {
        // A date on a product's site or in an article is someone else's.
        if screen::is_own_place(&app, screen.url.as_deref()) {
            eprintln!("freebyrd: watcher: {app}: {}", screen::describe(&text));
            pending.push(Snippet {
                app: app.clone(),
                window: window.clone(),
                text,
            });
        } else {
            eprintln!(
                "freebyrd: watcher: {app}: {} chars of event-like lines, not the person's own place, not sent",
                text.chars().count()
            );
        }
    }

    // Web pages are kept by the extension already.
    let kept_at = windows.get(&key).and_then(|seen| seen.kept_at);
    let keep = matches!(screen.source, Source::Accessibility)
        && kept_at.is_none_or(|at| at.elapsed() >= KEEP_EVERY);
    let kept_at = if keep && keep_window(&app, &window, screen.url.as_deref(), &screen.text) {
        Some(Instant::now())
    } else {
        kept_at
    };
    if windows.len() >= REMEMBERED_WINDOWS && !windows.contains_key(&key) {
        windows.clear();
    }
    windows.insert(
        key,
        Window {
            text: screen.text,
            kept_at,
        },
    );
}

/// WhatsApp: its chats go into the memory as conversations (so promises in
/// work chats reach the calendar), and the open chat's new lines are looked
/// at for events, like any window.
fn read_whatsapp(pid: i32, windows: &mut HashMap<String, Window>, pending: &mut Vec<Snippet>) {
    let shown = match whatsapp::read(pid, Local::now()) {
        Ok(shown) => shown,
        Err(e) => {
            eprintln!("freebyrd: watcher: WhatsApp: {e}");
            return;
        }
    };
    let stored = Store::open_default()
        .and_then(|store| whatsapp::store(&store, &shown, memory::time::now_ms()));
    match stored {
        Ok(added) if added > 0 => eprintln!("freebyrd: watcher: WhatsApp: {added} new messages"),
        Ok(_) => {}
        Err(e) => eprintln!("freebyrd: watcher: WhatsApp: could not store: {e:#}"),
    }
    let text = whatsapp::to_text(&shown);
    let window = shown
        .open
        .as_ref()
        .map(|chat| chat.name.clone())
        .unwrap_or_default();
    let key = format!("{}\u{1f}{window}", whatsapp::APP_NAME);
    let previous = windows.get(&key).map(|seen| seen.text.as_str());
    if previous != Some(text.as_str())
        && let Some(lines) = screen::candidates(previous, &text)
    {
        eprintln!("freebyrd: watcher: WhatsApp: {}", screen::describe(&lines));
        pending.push(Snippet {
            app: whatsapp::APP_NAME.to_owned(),
            window,
            text: lines,
        });
    }
    if windows.len() >= REMEMBERED_WINDOWS && !windows.contains_key(&key) {
        windows.clear();
    }
    windows.insert(
        key,
        Window {
            text,
            kept_at: None,
        },
    );
}

/// Stores a window's text in the memory, like a saved page. Returns whether
/// it was stored.
fn keep_window(app: &str, window: &str, url: Option<&str>, text: &str) -> bool {
    let kept = Store::open_default().and_then(|store| {
        let text = memory::redact::redact(text);
        let text_id = store.page_text_id(&text)?;
        let now = memory::time::now_ms();
        let ext_id = format!("{app}\u{1f}{window}\u{1f}{now}");
        store.add_activity(&NewActivity {
            source: "screen",
            kind: "window",
            ts: now,
            app: Some(app),
            title: Some(window).filter(|w| !w.is_empty()),
            url,
            text_id: Some(text_id),
            ext_id: &ext_id,
            ..Default::default()
        })
    });
    kept.unwrap_or_else(|e| {
        eprintln!("freebyrd: watcher: could not keep {app}: {e:#}");
        false
    })
}
