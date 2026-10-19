//! Reads the app that was in front when the overlay opened.
//!
//! Browsers with the extension are asked for the page directly; everything
//! else is read through the Accessibility API.

use std::thread;
use std::time::{Duration, Instant};

use capture::bridge::{self, ReadError};
use capture::snapshot::{self, AppSnapshot, Limits, Node, SnapshotError, WindowScope};
use capture::text;
use capture::{ax, browser};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};

use crate::picture::{self, Picture};
use crate::whatsapp;
use serde::Serialize;

/// Roughly 10,000 tokens; enough for a page, small enough to answer quickly.
const MAX_CONTEXT_CHARS: usize = 40_000;
/// What a window shows at once; more is a sign the in-view reading failed.
const MAX_IN_VIEW_CHARS: usize = 12_000;
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(2);
/// Why a browser window was not read when the extension cannot see it.
const HIDDEN_WINDOW: &str = "the browser extension cannot see this window (an incognito window, or a browser profile \
     without the extension)";

/// A window with fewer elements than this, right after asking for the web
/// tree, is probably still being built.
const SETTLED_NODES: usize = 50;
const TREE_RETRIES: usize = 4;
const TREE_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Never read, whatever is on screen. Matched against the app's name.
const EXCLUDED_APPS: &[&str] = &[
    "1Password",
    "Bitwarden",
    "Dashlane",
    "Keychain Access",
    "LastPass",
    "Passwords",
    "System Settings",
];

#[derive(Debug, Clone)]
pub struct Target {
    pub pid: i32,
    pub name: Option<String>,
    /// When the app was seen in front (Unix milliseconds), before the
    /// overlay took focus. A browser's extension reads the window that had
    /// focus then. `None` for an app read from the terminal.
    pub seen_at_ms: Option<u64>,
}

/// The active app, unless it is this overlay. Read from `NSWorkspace`, which
/// tracks app activation; the Accessibility "focused application" can lag
/// behind it or fail to answer.
pub fn frontmost_app() -> Option<Target> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    if u32::try_from(pid).ok()? == std::process::id() {
        return None;
    }
    let name = app.localizedName().map(|name| app_name(&name.to_string()));
    Some(Target {
        pid,
        name,
        seen_at_ms: u64::try_from(memory::time::now_ms()).ok(),
    })
}

/// The running app with this process ID, for testing from the terminal.
pub fn target(pid: i32) -> Option<Target> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let name = app.localizedName().map(|name| app_name(&name.to_string()));
    Some(Target {
        pid,
        name,
        seen_at_ms: None,
    })
}

/// An app's name without the invisible direction marks some apps put in
/// it: WhatsApp calls itself "\u{200e}WhatsApp", which matched no list.
fn app_name(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'))
        .collect::<String>()
        .trim()
        .to_owned()
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    BrowserExtension,
    Accessibility,
    Nothing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenContext {
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub source: Source,
    pub text: String,
    pub truncated: bool,
    /// The part of `text` the window showed: what "this" and "here" mean.
    /// `None` when it is not known.
    pub in_view: Option<String>,
    /// The window as the person saw it.
    pub picture: Option<Picture>,
    /// Why there is no picture, when the screen was read.
    pub no_picture: Option<String>,
    pub elapsed_ms: u128,
    /// Why nothing was read. Shown to the person; the screen is not sent.
    pub withheld: Option<String>,
    /// On an AI chat site, the person's own last message, read by the
    /// extension, for `memory::moments`. Not shown or sent with a question.
    #[serde(skip)]
    pub ask: Option<String>,
}

impl ScreenContext {
    fn withheld(target: Option<&Target>, reason: String, started: Instant) -> Self {
        Self {
            app_name: target.and_then(|t| t.name.clone()),
            window_title: None,
            url: None,
            source: Source::Nothing,
            text: String::new(),
            truncated: false,
            in_view: None,
            picture: None,
            no_picture: None,
            elapsed_ms: started.elapsed().as_millis(),
            withheld: Some(reason),
            ask: None,
        }
    }

    pub fn nothing_in_front() -> Self {
        Self::withheld(None, "no app was in front".to_owned(), Instant::now())
    }
}

/// Reads the app's window, then pictures it. Only what could be read is
/// pictured: never an excluded app, a page the extension declined, or an
/// incognito window.
pub fn capture(target: &Target) -> ScreenContext {
    let mut screen = read(target);
    if screen.withheld.is_none() {
        let started = Instant::now();
        match picture::take(target.pid) {
            Ok(picture) => {
                eprintln!(
                    "freebyrd: pictured the window, {}x{}, {} KB, in {} ms",
                    picture.width,
                    picture.height,
                    picture.jpeg.len() / 1024,
                    started.elapsed().as_millis()
                );
                screen.picture = Some(picture);
            }
            Err(reason) => {
                eprintln!("freebyrd: no picture: {reason}");
                screen.no_picture = Some(reason);
            }
        }
    }
    screen
}

fn read(target: &Target) -> ScreenContext {
    let started = Instant::now();
    if let Some(name) = &target.name
        && EXCLUDED_APPS.contains(&name.as_str())
    {
        return ScreenContext::withheld(Some(target), format!("{name} is never read"), started);
    }

    // WhatsApp keeps its text where the generic reader does not look.
    if target.name.as_deref() == Some(whatsapp::APP_NAME) {
        match whatsapp::read(target.pid, chrono::Local::now()) {
            Ok(shown) if shown.open.is_some() || !shown.list.is_empty() => {
                let (text, truncated) = limit(whatsapp::to_text(&shown));
                return ScreenContext {
                    app_name: target.name.clone(),
                    window_title: shown.open.map(|chat| chat.name),
                    url: None,
                    source: Source::Accessibility,
                    text,
                    truncated,
                    in_view: None,
                    picture: None,
                    no_picture: None,
                    elapsed_ms: started.elapsed().as_millis(),
                    withheld: None,
                    ask: None,
                };
            }
            Ok(_) => {}
            Err(e) => return ScreenContext::withheld(Some(target), e.to_string(), started),
        }
    }

    // The extension owns the rules for web pages, so its answer is final for
    // a browser it is connected to: a refusal or silence means nothing is
    // read, rather than reading the same page through Accessibility.
    let browser_pid = u32::try_from(target.pid).unwrap_or_default();
    // The extension cannot see an incognito window it is not allowed in, or
    // a window of a profile without it. Asked for "the last focused window",
    // it would answer with another window's page, and the text would be one
    // page and the picture another. So it is asked for the window that had
    // focus when the browser was seen in front, and declines if that was one
    // it cannot see. A loose title check backs that up, for an extension
    // that cannot tell.
    match bridge::read_active_page(browser_pid, target.seen_at_ms, BRIDGE_TIMEOUT) {
        Err(ReadError::Refused(reason)) if reason == "hidden-window" => {
            return ScreenContext::withheld(Some(target), HIDDEN_WINDOW.to_owned(), started);
        }
        Ok(page)
            if front_window_title(target.pid)
                .is_some_and(|title| !browser::window_shows(&title, &page)) =>
        {
            eprintln!("freebyrd: the window in front is not the page the extension read");
            return ScreenContext::withheld(Some(target), HIDDEN_WINDOW.to_owned(), started);
        }
        Ok(page) => {
            let (text, truncated) = limit(page.text);
            return ScreenContext {
                app_name: target.name.clone(),
                window_title: Some(page.title),
                url: Some(page.url),
                source: Source::BrowserExtension,
                text,
                truncated: truncated || page.truncated,
                in_view: in_view(page.in_view),
                picture: None,
                no_picture: None,
                elapsed_ms: started.elapsed().as_millis(),
                withheld: None,
                ask: page.ask,
            };
        }
        Err(ReadError::NoBridge) => {}
        Err(ReadError::Refused(reason)) => {
            return ScreenContext::withheld(
                Some(target),
                format!("the browser extension declined ({reason})"),
                started,
            );
        }
        Err(e) => {
            return ScreenContext::withheld(
                Some(target),
                format!("the browser did not answer ({e})"),
                started,
            );
        }
    }

    match read_app(target.pid) {
        Ok(snap) => {
            let window = snap.windows.first();
            let (text, truncated) = limit(text::to_text(&snap));
            ScreenContext {
                app_name: snap.app_name.clone().or_else(|| target.name.clone()),
                window_title: window.and_then(|w| w.title.clone()),
                url: window.and_then(|w| web_area_url(&w.root)),
                source: Source::Accessibility,
                text,
                truncated: truncated || snap.stats.truncated,
                in_view: in_view(text::in_view_text(&snap)),
                picture: None,
                no_picture: None,
                elapsed_ms: started.elapsed().as_millis(),
                withheld: None,
                ask: None,
            }
        }
        Err(e) => ScreenContext::withheld(Some(target), e.to_string(), started),
    }
}

/// The title of the app's window in front: the one the picture shows, or,
/// without Screen Recording access, the one Accessibility calls main.
fn front_window_title(pid: i32) -> Option<String> {
    picture::front_window_title(pid).or_else(|| {
        let app = ax::Element::application(pid)?;
        app.set_messaging_timeout(0.5);
        let window = app.element("AXMainWindow")?;
        window.set_messaging_timeout(0.5);
        window.string("AXTitle")
    })
}

/// Reads the app's main window. Electron apps and Arc build their page tree
/// only after being asked, and in the background, so a first read can be
/// nearly empty; it is retried while the tree keeps growing.
fn read_app(pid: i32) -> Result<AppSnapshot, SnapshotError> {
    let asked_for_web_tree = snapshot::enable_web_accessibility(pid)
        .is_ok_and(|request| request.manual_accessibility.is_ok());
    let mut snap = snapshot::snapshot_app(pid, WindowScope::Focused, Limits::default())?;
    if !asked_for_web_tree || snap.stats.nodes >= SETTLED_NODES {
        return Ok(snap);
    }
    for _ in 0..TREE_RETRIES {
        thread::sleep(TREE_RETRY_DELAY);
        let next = snapshot::snapshot_app(pid, WindowScope::Focused, Limits::default())?;
        let grew = next.stats.nodes > snap.stats.nodes;
        snap = next;
        if !grew && snap.stats.nodes > 0 {
            break;
        }
    }
    Ok(snap)
}

fn web_area_url(node: &Node) -> Option<String> {
    if node.role == "AXWebArea" && node.url.is_some() {
        return node.url.clone();
    }
    node.children.iter().find_map(web_area_url)
}

fn limit(text: String) -> (String, bool) {
    cut(text, MAX_CONTEXT_CHARS)
}

/// The text in view, if any was found.
fn in_view(text: String) -> Option<String> {
    let text = cut(text, MAX_IN_VIEW_CHARS).0;
    (!text.trim().is_empty()).then_some(text)
}

fn cut(text: String, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => (text[..cut].to_owned(), true),
        None => (text, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_lose_direction_marks() {
        assert_eq!(app_name("\u{200e}WhatsApp"), "WhatsApp");
        assert_eq!(app_name("TextEdit"), "TextEdit");
        assert_eq!(app_name("\u{202b}Arc\u{202c} "), "Arc");
    }
}
