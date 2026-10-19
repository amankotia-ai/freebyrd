//! The Claude app's chats, for moments: the person's own messages in the
//! chat in front, read through Accessibility, never Claude's replies (the
//! person's decision, 2026-10-13, so a new chat there is weighed at its
//! start like one in the browser).
//!
//! The app shows claude.ai. A chat's address is `claude.ai/chat/<id>`
//! (`/new` before the first message is sent); Claude Code's tab is
//! `claude.ai/epitaxy/...`, whose moments come from Claude Code's
//! transcripts instead. Elements carry their web classes (`AXDOMClassList`),
//! and a bubble of the person's has one naming `user-message`
//! (`bg-[var(--cds-bg-user-message)]`, 2026-10), which Claude's replies do
//! not. Checked on the Code tab's bubbles on 2026-10-13 in counts and
//! lengths only: three bubbles, the person's three messages.

use capture::snapshot::{self, AppSnapshot, Limits, Node, WindowScope};

pub const APP_NAME: &str = "Claude";
const SITE: &str = "https://claude.ai/";
/// The class word that marks the person's own message.
const MINE: &str = "user-message";
/// The address is near the top of the window: a shallow read finds it, and
/// only a chat is read in full.
const FIND_ADDRESS: Limits = Limits {
    max_depth: 12,
    max_nodes: 600,
    max_value_chars: 200,
    dom_classes: false,
};
const READ_CHAT: Limits = Limits {
    max_depth: 64,
    max_nodes: 8_000,
    max_value_chars: 20_000,
    dom_classes: true,
};

/// What the Claude app shows in front.
#[derive(Debug, PartialEq, Eq)]
pub enum Front {
    /// A chat: the window's title, its address, and the person's messages
    /// in it, oldest first.
    Chat {
        title: String,
        url: String,
        messages: Vec<String>,
    },
    /// Claude Code's tab.
    Code,
    /// Anything else (settings, the chat list).
    Other,
}

/// Reads the app in front; `None` when its window could not be read.
pub fn read(pid: i32) -> Option<Front> {
    let _ = snapshot::enable_web_accessibility(pid);
    let shallow = snapshot::snapshot_app(pid, WindowScope::Focused, FIND_ADDRESS).ok()?;
    let url = address(&shallow)?;
    if !is_chat(&url) {
        return Some(kind(&url));
    }
    let full = snapshot::snapshot_app(pid, WindowScope::Focused, READ_CHAT).ok()?;
    Some(parse(&full))
}

/// For `freebyrd --read-claude`: what the reader understood of the window
/// in front, in counts and lengths only.
pub fn print_counts() -> i32 {
    // `-a`: run from a Claude Code session, the app is pgrep's ancestor,
    // which pgrep otherwise leaves out.
    let pid = std::process::Command::new("/usr/bin/pgrep")
        .args(["-a", "-x", APP_NAME])
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()?
                .trim()
                .parse::<i32>()
                .ok()
        });
    let Some(pid) = pid else {
        eprintln!("the Claude app is not running");
        return 1;
    };
    match read(pid) {
        None => {
            eprintln!("could not read the Claude app's window");
            1
        }
        Some(Front::Code) => {
            println!("Claude Code's tab: moments come from its transcripts");
            0
        }
        Some(Front::Other) => {
            println!("neither a chat nor Claude Code's tab: no moment");
            0
        }
        Some(Front::Chat {
            title, messages, ..
        }) => {
            let lengths: Vec<usize> = messages.iter().map(|m| m.chars().count()).collect();
            println!(
                "a chat: title {} characters, {} of the person's messages, lengths {lengths:?}",
                title.chars().count(),
                messages.len()
            );
            0
        }
    }
}

/// What a full read of the window shows.
pub fn parse(snapshot: &AppSnapshot) -> Front {
    let Some(window) = snapshot.windows.first() else {
        return Front::Other;
    };
    let Some(url) = address(snapshot) else {
        return Front::Other;
    };
    if !is_chat(&url) {
        return kind(&url);
    }
    let mut messages = Vec::new();
    collect_mine(&window.root, &mut messages);
    Front::Chat {
        title: window.title.clone().unwrap_or_default(),
        url,
        messages,
    }
}

/// The address of the claude.ai page in the window.
fn address(snapshot: &AppSnapshot) -> Option<String> {
    fn find(node: &Node) -> Option<String> {
        if let Some(url) = node.url.as_deref().filter(|url| url.starts_with(SITE)) {
            return Some(url.to_owned());
        }
        node.children.iter().find_map(find)
    }
    snapshot
        .windows
        .iter()
        .find_map(|window| find(&window.root))
}

fn path(url: &str) -> &str {
    url.strip_prefix(SITE).unwrap_or_default()
}

fn is_chat(url: &str) -> bool {
    let path = path(url);
    path.starts_with("chat/") || path == "new" || path.starts_with("new?")
}

fn kind(url: &str) -> Front {
    if path(url).starts_with("epitaxy") {
        Front::Code
    } else {
        Front::Other
    }
}

fn is_mine(node: &Node) -> bool {
    node.classes.iter().any(|class| class.contains(MINE))
}

/// The person's messages under `node`, in order. A bubble inside another
/// marked one is the message; the outer is its row.
fn collect_mine(node: &Node, messages: &mut Vec<String>) {
    if is_mine(node) && !has_mine_inside(node) {
        let mut lines = Vec::new();
        text(node, &mut lines);
        let message = lines.join("\n").trim().to_owned();
        if !message.is_empty() {
            messages.push(message);
        }
        return;
    }
    for child in &node.children {
        collect_mine(child, messages);
    }
}

fn has_mine_inside(node: &Node) -> bool {
    node.children
        .iter()
        .any(|child| is_mine(child) || has_mine_inside(child))
}

fn text(node: &Node, lines: &mut Vec<String>) {
    if node.role == "AXStaticText"
        && let Some(value) = node.value.as_deref().map(str::trim)
        && !value.is_empty()
    {
        lines.push(value.to_owned());
    }
    for child in &node.children {
        text(child, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture::snapshot::{Stats, WindowSnapshot};

    fn node(role: &str, classes: &[&str], value: Option<&str>, children: Vec<Node>) -> Node {
        Node {
            role: role.to_owned(),
            subrole: None,
            title: None,
            value: value.map(str::to_owned),
            description: None,
            identifier: None,
            classes: classes.iter().map(|class| (*class).to_owned()).collect(),
            url: None,
            frame: None,
            shown: None,
            children,
        }
    }

    fn text(value: &str) -> Node {
        node("AXStaticText", &[], Some(value), vec![])
    }

    fn window(url: &str, body: Vec<Node>) -> AppSnapshot {
        let mut page = node("AXWebArea", &[], None, body);
        page.url = Some(url.to_owned());
        AppSnapshot {
            pid: 1,
            app_name: Some("Claude".into()),
            windows: vec![WindowSnapshot {
                title: Some("Freebyrd logo".into()),
                root: node(
                    "AXWindow",
                    &[],
                    None,
                    vec![node("AXGroup", &[], None, vec![page])],
                ),
            }],
            stats: Stats::default(),
        }
    }

    fn bubble(lines: &[&str]) -> Node {
        let row = node(
            "AXGroup",
            &["group/message-row"],
            None,
            vec![node(
                "AXGroup",
                &[
                    "rounded-[var(--cds-message-bubble-radius)]",
                    "bg-[var(--cds-bg-user-message)]",
                ],
                None,
                lines.iter().map(|line| text(line)).collect(),
            )],
        );
        node("AXGroup", &["user-message-row"], None, vec![row])
    }

    fn reply(line: &str) -> Node {
        node("AXGroup", &["group/message-row"], None, vec![text(line)])
    }

    #[test]
    fn a_chat_gives_the_persons_messages_only_in_order() {
        let snap = window(
            "https://claude.ai/chat/0f1e",
            vec![
                bubble(&["design a minimal logo", "for freebyrd"]),
                reply("Here are three directions."),
                bubble(&["make it face right"]),
                reply("Done."),
            ],
        );
        assert_eq!(
            parse(&snap),
            Front::Chat {
                title: "Freebyrd logo".into(),
                url: "https://claude.ai/chat/0f1e".into(),
                messages: vec![
                    "design a minimal logo\nfor freebyrd".into(),
                    "make it face right".into()
                ],
            }
        );
    }

    #[test]
    fn the_code_tab_and_other_pages_are_no_chat() {
        let code = window(
            "https://claude.ai/epitaxy/abc",
            vec![bubble(&["fix the glow"])],
        );
        assert_eq!(parse(&code), Front::Code);
        let settings = window("https://claude.ai/settings", vec![]);
        assert_eq!(parse(&settings), Front::Other);
        let new = window("https://claude.ai/new", vec![]);
        assert!(matches!(parse(&new), Front::Chat { messages, .. } if messages.is_empty()));
    }
}

// Sending a prompt that Freebyrd's link filled in.

/// Sends the prompt a `claude://code/new?folder=&q=` link put in the Code
/// tab's input. The desktop app only fills it in, by design, so a link
/// alone never runs anything; the person pressed Run in Claude Code, so
/// Freebyrd presses Return for them (their ask, 2026-10-13). Only when the
/// Claude app is in front and its focused input holds exactly this prompt
/// (compared by its words); otherwise it stays filled in for them to send.
///
/// The app also asks to trust the folder each time a link brings one, even
/// a folder it trusts (it tags the session "external"). For a folder the
/// person already trusts in Claude Code and has had sessions in, Freebyrd
/// presses that dialog's trust button too (the person's choice, 2026-10-13);
/// any other folder is left for them. Returns what it did, for the log.
pub fn send_filled_prompt(prompt: &str, folder: Option<&str>) -> String {
    use std::time::{Duration, Instant};

    let wanted = words(prompt);
    let may_trust = folder.is_some_and(already_trusted);
    let mut trusted_with: Option<String> = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(350));
        let Some(front) = crate::context::frontmost_app() else {
            continue;
        };
        if front.name.as_deref() != Some(APP_NAME) {
            continue;
        }
        if focused_value(front.pid).is_some_and(|value| words(&value) == wanted) {
            // The editor settles a moment after the text arrives.
            std::thread::sleep(Duration::from_millis(250));
            press_return(front.pid);
            return match trusted_with {
                Some(label) => format!("started (confirmed \"{label}\")"),
                None => "started".to_owned(),
            };
        }
        if may_trust
            && trusted_with.is_none()
            && let Some(label) = press_trust(front.pid)
        {
            trusted_with = Some(label);
        }
    }
    match (trusted_with, may_trust) {
        (Some(label), _) => format!("opened, confirmed \"{label}\"; its prompt waits to be sent"),
        (None, true) => "opened; no trust button found, its prompt waits to be sent".to_owned(),
        (None, false) => "opened; its prompt waits to be sent".to_owned(),
    }
}

/// Whether the person already trusts `folder`: Claude Code has it as
/// trusted (`~/.claude.json`) and the desktop app has had sessions there.
fn already_trusted(folder: &str) -> bool {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return false;
    };
    let trusted = std::fs::read_to_string(home.join(".claude.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|config| config["projects"][folder]["hasTrustDialogAccepted"] == true);
    trusted
        && has_sessions(
            &home.join("Library/Application Support/Claude/claude-code-sessions"),
            folder,
            0,
        )
}

/// Whether a desktop session's record names `folder` as its folder.
fn has_sessions(dir: &std::path::Path, folder: &str, depth: u32) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                return depth < 3 && has_sessions(&path, folder, depth + 1);
            }
            path.extension().is_some_and(|ext| ext == "json")
                && std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .is_some_and(|session| {
                        session["cwd"] == folder || session["originCwd"] == folder
                    })
        })
}

/// Presses the trust dialog's button in the window in front, if it shows:
/// a button whose name says trust (and not "don't" or "cancel"). Returns
/// its name.
fn press_trust(pid: i32) -> Option<String> {
    use capture::ax::Element;
    let window = Element::application(pid)?.element("AXFocusedWindow")?;
    let mut seen = 0;
    let button = find_trust_button(&window, 0, &mut seen)?;
    let name = button_name(&button);
    button.press().ok()?;
    Some(name)
}

fn find_trust_button(
    node: &capture::ax::Element,
    depth: u32,
    seen: &mut u32,
) -> Option<capture::ax::Element> {
    *seen += 1;
    if depth > 40 || *seen > 4000 {
        return None;
    }
    if node.string("AXRole").as_deref() == Some("AXButton") && is_trust(&button_name(node)) {
        return Some(node.clone());
    }
    node.elements("AXChildren")
        .ok()?
        .into_iter()
        .find_map(|child| find_trust_button(&child, depth + 1, seen))
}

/// A button's name: its title, description, or the text inside it.
fn button_name(node: &capture::ax::Element) -> String {
    for attribute in ["AXTitle", "AXDescription"] {
        if let Some(name) = node
            .string(attribute)
            .filter(|name| !name.trim().is_empty())
        {
            return name.trim().to_owned();
        }
    }
    node.elements("AXChildren")
        .unwrap_or_default()
        .iter()
        .filter_map(|child| child.string("AXValue").or_else(|| child.string("AXTitle")))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

/// "Trust", "Trust folder", "Yes, trust": not "Don't trust" or "Cancel".
fn is_trust(name: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("trust")
        && name.len() < 60
        && ![
            "don't", "dont", "do not", "not now", "cancel", "untrust", "why",
        ]
        .iter()
        .any(|no| name.contains(no))
}

/// The value of the element with keyboard focus in an app.
fn focused_value(pid: i32) -> Option<String> {
    use capture::ax::{Element, Value};
    let app = Element::application(pid)?;
    let Ok(Some(Value::Element(focused))) = app.attribute("AXFocusedUIElement") else {
        return None;
    };
    match focused.attribute("AXValue") {
        Ok(Some(Value::String(value))) => Some(value),
        _ => None,
    }
}

/// A prompt's words, without punctuation or list numbers: the input may
/// show "1." as a list and quotes as curly ones.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !word.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .collect()
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: *const std::ffi::c_void,
        keycode: u16,
        down: bool,
    ) -> *mut std::ffi::c_void;
    fn CGEventPostToPid(pid: i32, event: *mut std::ffi::c_void);
}

/// Return, sent to that app alone.
fn press_return(pid: i32) {
    const RETURN: u16 = 36;
    for down in [true, false] {
        unsafe {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), RETURN, down);
            if event.is_null() {
                return;
            }
            CGEventPostToPid(pid, event);
            core_foundation_sys::base::CFRelease(event.cast_const());
        }
    }
}

#[cfg(test)]
mod send_tests {
    use super::words;

    #[test]
    fn only_a_button_that_says_trust_is_pressed() {
        use super::is_trust;
        assert!(is_trust("Trust"));
        assert!(is_trust("Trust folder"));
        assert!(is_trust("Yes, I trust this folder"));
        assert!(!is_trust("Don't trust"));
        assert!(!is_trust("Cancel"));
        assert!(!is_trust("Not now"));
        assert!(!is_trust("Open"));
    }

    #[test]
    fn the_input_s_list_and_quotes_still_match_the_prompt() {
        let prompt = "Add a farewell to the test app\nFor: A test task\n\n1. Add farewell(name) that returns \"Goodbye, name!\"\n2. Run npm run -s typecheck";
        let shown = "Add a farewell to the test app\nFor: A test task\nAdd farewell(name) that returns \u{201c}Goodbye, name!\u{201d}\nRun npm run -s typecheck";
        assert_eq!(words(prompt), words(shown));
        assert_ne!(words(prompt), words("Add a farewell to the test app"));
    }
}
