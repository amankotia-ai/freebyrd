//! WhatsApp's Mac app, read through Accessibility.
//!
//! Its elements carry identifiers (`ChatListView_TableView`,
//! `ChatMessagesTableView`, `WAMessageBubbleTableViewCell`), and each
//! message's label says who sent it, what and when: "Your message, Sure, by
//! Thursday, 13Octoberat3:34 PM, Sent to Ben, Delivered", or "message, Can
//! you send it?, 13Octoberat3:30 PM, Received from Ben". The chat list's
//! rows say the same about each chat's last message.
//!
//! This reads the open chat and the list's last messages into conversations
//! (`store`), so promises in work chats go on the calendar and recall knows
//! who wrote last, and writes the open chat as text (`to_text`) for the
//! overlay and the screen watcher: the generic reader takes values, and
//! WhatsApp puts its text in descriptions. Littlebird reads the open chat
//! only, not the chat list (`littlebird/FINDINGS.md` §5.6). WhatsApp shows
//! no unread state per chat to Accessibility, so "they wrote last" stands in
//! for "waiting on you".

use std::sync::LazyLock;

use anyhow::Result;
use capture::snapshot::{self, AppSnapshot, Limits, Node, WindowScope};
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeDelta, TimeZone, Weekday};
use memory::Store;
use memory::conversations::{self, Conversation, ME, NewMessage};
use regex::Regex;

pub const APP_NAME: &str = "WhatsApp";

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub mine: bool,
    /// Who sent it, when it is not the person: a group member, or the
    /// other side of a one-to-one chat.
    pub sender: String,
    pub text: String,
    /// WhatsApp's own label, such as "13Octoberat3:34 PM".
    pub when: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub sent_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chat {
    pub name: String,
    pub messages: Vec<Message>,
}

/// What the WhatsApp window shows: the open chat, and each chat in the
/// list with its last message.
#[derive(Debug, Default, PartialEq)]
pub struct Shown {
    pub open: Option<Chat>,
    pub list: Vec<Chat>,
}

/// "13Octoberat3:34 PM", "Yesterdayat9:05 AM", "Mondayat18:20".
static WHEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^
        (?: (?P<day>\d{1,2})\s?(?P<month>[A-Za-z]+)\s?(?P<year>\d{4})?
          | (?P<relative>Today|Yesterday|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday) )
        \s?at\s?(?P<hour>\d{1,2}):(?P<minute>\d{2})\s?(?P<half>AM|PM)?$",
    )
    .expect("valid pattern")
});

/// "Message from Ben, ...", "Photo from Ben, ...".
static FROM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<kind>Message|Photo|Video|Voice message|Audio|Document|Sticker|GIF|Link|Poll|Location|Contact) from (?P<sender>.+?), ",
    )
    .expect("valid pattern")
});

/// Reads WhatsApp's main window.
pub fn read(pid: i32, now: DateTime<Local>) -> Result<Shown, snapshot::SnapshotError> {
    let limits = Limits {
        max_nodes: 4_000,
        ..Limits::default()
    };
    let snapshot = snapshot::snapshot_app(pid, WindowScope::Focused, limits)?;
    Ok(parse(&snapshot, now))
}

pub fn parse(snapshot: &AppSnapshot, now: DateTime<Local>) -> Shown {
    let mut shown = Shown::default();
    let Some(window) = snapshot.windows.first() else {
        return shown;
    };
    let root = &window.root;

    if let Some(messages) = find(root, "ChatMessagesTableView") {
        let name = find(root, "NavigationBar_HeaderViewButton")
            .and_then(|header| header.description.as_deref())
            .map(clean)
            .or_else(|| {
                messages
                    .description
                    .as_deref()
                    .map(clean)
                    .and_then(|d| d.strip_prefix("Messages in chat with ").map(str::to_owned))
            })
            .unwrap_or_default();
        let mut cells = Vec::new();
        collect(messages, "WAMessageBubbleTableViewCell", &mut cells);
        let messages = cells
            .iter()
            .filter_map(|cell| cell.description.as_deref().or(cell.value.as_deref()))
            .filter_map(|label| parse_label(label, &name, now))
            .collect();
        if !name.is_empty() {
            shown.open = Some(Chat { name, messages });
        }
    }

    if let Some(list) = find(root, "ChatListView_TableView") {
        for row in &list.children {
            let (Some(name), Some(label)) = (row.description.as_deref(), row.value.as_deref())
            else {
                continue;
            };
            let name = clean(name);
            if let Some(last) = parse_label(label, &name, now) {
                shown.list.push(Chat {
                    name,
                    messages: vec![last],
                });
            }
        }
    }
    shown
}

fn find<'a>(node: &'a Node, identifier: &str) -> Option<&'a Node> {
    if node.identifier.as_deref() == Some(identifier) {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find(child, identifier))
}

fn collect<'a>(node: &'a Node, identifier: &str, found: &mut Vec<&'a Node>) {
    if node.identifier.as_deref() == Some(identifier) {
        found.push(node);
    }
    for child in &node.children {
        collect(child, identifier, found);
    }
}

/// Without direction marks, and with narrow spaces as plain ones.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}'))
        .map(|c| {
            if matches!(c, '\u{202f}' | '\u{a0}') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// One message's label, from a bubble or a chat list row in chat `chat`.
pub fn parse_label(label: &str, chat: &str, now: DateTime<Local>) -> Option<Message> {
    let label = clean(label);
    let mut body = label.as_str();
    // A reply quotes what it answers on a line of its own first.
    if body.starts_with("Replying to ") {
        body = body.split_once('\n')?.1;
    }
    // The end says where it went or came from, then how far it got.
    let (at, marker) = [", Sent to ", ", Received from ", ", Received in "]
        .iter()
        .filter_map(|marker| body.rfind(marker).map(|at| (at, *marker)))
        .max_by_key(|(at, _)| *at)?;
    let mine = marker == ", Sent to ";
    let rest = &body[..at];
    let (rest, when) = match rest.rsplit_once(", ") {
        Some((head, last)) if WHEN.is_match(last.trim()) => (head, Some(last.trim().to_owned())),
        _ => (rest, None),
    };
    let (sender, text) = if let Some(text) = rest.strip_prefix("Your message, ") {
        (None, text.to_owned())
    } else if let Some(text) = rest.strip_prefix("message, ") {
        (None, text.to_owned())
    } else if let Some(found) = FROM.captures(rest) {
        let text = &rest[found.get(0).map_or(0, |m| m.end())..];
        let text = match &found["kind"] {
            "Message" => text.to_owned(),
            kind => format!("[{kind}] {text}").trim().to_owned(),
        };
        (Some(found["sender"].to_owned()), text)
    } else {
        (None, rest.to_owned())
    };
    let text = text.trim().to_owned();
    if text.is_empty() {
        return None;
    }
    Some(Message {
        mine,
        sender: if mine {
            ME.to_owned()
        } else {
            sender.unwrap_or_else(|| chat.to_owned())
        },
        sent_at: when.as_deref().and_then(|when| parse_when(when, now)),
        when,
        text,
    })
}

/// WhatsApp's time label as local milliseconds. Labels have no year: a date
/// that would be in the future is from last year.
pub fn parse_when(label: &str, now: DateTime<Local>) -> Option<i64> {
    let found = WHEN.captures(label.trim())?;
    let today = now.date_naive();
    let date = if let Some(relative) = found.name("relative") {
        match relative.as_str() {
            "Today" => today,
            "Yesterday" => today.pred_opt()?,
            weekday => {
                let weekday: Weekday = weekday.parse().ok()?;
                let back = (7 + today.weekday().num_days_from_monday()
                    - weekday.num_days_from_monday())
                    % 7;
                today - TimeDelta::days(i64::from(if back == 0 { 7 } else { back }))
            }
        }
    } else {
        let day: u32 = found.name("day")?.as_str().parse().ok()?;
        let month = month_number(found.name("month")?.as_str())?;
        let year = match found.name("year") {
            Some(year) => year.as_str().parse().ok()?,
            None => today.year(),
        };
        let date = NaiveDate::from_ymd_opt(year, month, day)?;
        if found.name("year").is_none() && date > today + TimeDelta::days(1) {
            NaiveDate::from_ymd_opt(year - 1, month, day)?
        } else {
            date
        }
    };
    let mut hour: u32 = found.name("hour")?.as_str().parse().ok()?;
    let minute: u32 = found.name("minute")?.as_str().parse().ok()?;
    match found.name("half").map(|half| half.as_str()) {
        Some("PM") if hour < 12 => hour += 12,
        Some("AM") if hour == 12 => hour = 0,
        _ => {}
    }
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    Local
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|at| at.timestamp_millis())
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let name = name.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|month| name.starts_with(month))
        .and_then(|at| u32::try_from(at + 1).ok())
}

/// The window as text for the overlay and the watcher: the open chat, one
/// message a line, then the list's last messages. Times are written as
/// "[sent 2026-10-13 15:34]", which the screen filter does not take for a
/// plan and which tells Claude when "Thursday" was said.
pub fn to_text(shown: &Shown) -> String {
    let mut lines = Vec::new();
    let line = |message: &Message| {
        let who = if message.mine {
            "me"
        } else {
            message.sender.as_str()
        };
        let when = message
            .sent_at
            .and_then(memory::time::local)
            .map(|at| format!(" [sent {}]", at.format("%Y-%m-%d %H:%M")))
            .unwrap_or_default();
        format!("{who}{when}: {}", message.text.replace('\n', " / "))
    };
    if let Some(chat) = &shown.open {
        lines.push(format!("WhatsApp chat with {}", chat.name));
        lines.extend(chat.messages.iter().map(line));
    }
    if !shown.list.is_empty() {
        lines.push(String::new());
        lines.push("Chats, each with its last message:".to_owned());
        for chat in &shown.list {
            if let Some(last) = chat.messages.last() {
                lines.push(format!("{}: {}", chat.name, line(last)));
            }
        }
    }
    lines.join("\n")
}

/// Stores the open chat and the list's last messages as conversations.
/// Returns how many messages were new. Chats start neither work nor
/// personal; the first promise-like message asks which.
pub fn store(store: &Store, shown: &Shown, now_ms: i64) -> Result<usize> {
    let tx = store.transaction()?;
    let mut added = 0;
    let chats = shown.open.iter().chain(shown.list.iter());
    for chat in chats {
        let messages: Vec<NewMessage> = chat
            .messages
            .iter()
            .map(|message| NewMessage {
                sender: &message.sender,
                text: &message.text,
                sent_label: message.when.as_deref(),
                sent_at: message.sent_at,
            })
            .collect();
        let conversation = Conversation {
            app: "whatsapp",
            key: &chat.name,
            title: &chat.name,
            work: None,
        };
        added += conversations::record(store, &conversation, &messages, now_ms)?.1;
    }
    tx.commit()?;
    Ok(added)
}

/// For `freebyrd --read-whatsapp`: how much of the open window the reader
/// understood, in counts, with no names or text.
pub fn print_counts() -> i32 {
    let Some(pid) = std::process::Command::new("/usr/bin/pgrep")
        .args(["-x", APP_NAME])
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()?
                .trim()
                .parse::<i32>()
                .ok()
        })
    else {
        eprintln!("WhatsApp is not running");
        return 1;
    };
    let limits = Limits {
        max_nodes: 4_000,
        ..Limits::default()
    };
    let snapshot = match snapshot::snapshot_app(pid, WindowScope::Focused, limits) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let now = Local::now();
    let shown = parse(&snapshot, now);
    let mut cells = Vec::new();
    if let Some(window) = snapshot.windows.first() {
        collect(&window.root, "WAMessageBubbleTableViewCell", &mut cells);
    }
    let rows = snapshot
        .windows
        .first()
        .and_then(|window| find(&window.root, "ChatListView_TableView"))
        .map_or(0, |list| {
            list.children
                .iter()
                .filter(|row| row.value.is_some())
                .count()
        });
    let messages = shown
        .open
        .as_ref()
        .map(|chat| chat.messages.as_slice())
        .unwrap_or_default();
    println!(
        "open chat: {} | bubbles {} read {} (mine {}, theirs {}, with a time {}, kinds {}) | list rows {} read {} | text {} chars in {} ms",
        if shown.open.is_some() { "yes" } else { "no" },
        cells.len(),
        messages.len(),
        messages.iter().filter(|m| m.mine).count(),
        messages.iter().filter(|m| !m.mine).count(),
        messages.iter().filter(|m| m.sent_at.is_some()).count(),
        messages.iter().filter(|m| m.text.starts_with('[')).count(),
        rows,
        shown.list.len(),
        to_text(&shown).chars().count(),
        snapshot.stats.elapsed_ms,
    );
    0
}

#[cfg(test)]
mod tests {
    use capture::snapshot::{Stats, WindowSnapshot};

    use super::*;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn reads_whatsapp_times() {
        let now = now();
        assert_eq!(
            parse_when("11Octoberat10:41\u{202f}PM", now),
            Some(at(2026, 10, 11, 22, 41))
        );
        assert_eq!(
            parse_when("11Octoberat10:41 PM", now),
            Some(at(2026, 10, 11, 22, 41))
        );
        assert_eq!(
            parse_when("8Octoberat3:34 PM", now),
            Some(at(2026, 10, 8, 15, 34))
        );
        assert_eq!(
            parse_when("1Januaryat12:05 AM", now),
            Some(at(2026, 1, 1, 0, 5))
        );
        // No year, and later than today: last year.
        assert_eq!(
            parse_when("24Decemberat9:00 AM", now),
            Some(at(2025, 12, 24, 9, 0))
        );
        assert_eq!(
            parse_when("Todayat9:05 AM", now),
            Some(at(2026, 10, 13, 9, 5))
        );
        assert_eq!(
            parse_when("Yesterdayat18:20", now),
            Some(at(2026, 10, 12, 18, 20))
        );
        // 13 Oct 2026 is a Tuesday: "Thursday" is 8 Oct, "Tuesday" a week back.
        assert_eq!(
            parse_when("Thursdayat3:00 PM", now),
            Some(at(2026, 10, 8, 15, 0))
        );
        assert_eq!(
            parse_when("Tuesdayat3:00 PM", now),
            Some(at(2026, 10, 6, 15, 0))
        );
        assert_eq!(parse_when("soon", now), None);
    }

    #[test]
    fn reads_message_labels() {
        let now = now();
        let mine = parse_label(
            "\u{200e}Your message, Sure, by Thursday, 13Octoberat3:34\u{202f}PM, \u{200e}Sent to Ben Ashton, \u{200e}Delivered",
            "Ben Ashton",
            now,
        )
        .unwrap();
        assert!(mine.mine);
        assert_eq!(mine.sender, ME);
        assert_eq!(mine.text, "Sure, by Thursday");
        assert_eq!(mine.when.as_deref(), Some("13Octoberat3:34 PM"));
        assert_eq!(mine.sent_at, Some(at(2026, 10, 13, 15, 34)));

        let theirs = parse_label(
            "\u{200e}Replying to \u{200e}You.\n\u{200e}message, Can you send it?, 13Octoberat3:30\u{202f}PM, \u{200e}Received from Ben Ashton",
            "Ben Ashton",
            now,
        )
        .unwrap();
        assert!(!theirs.mine);
        assert_eq!(theirs.sender, "Ben Ashton");
        assert_eq!(theirs.text, "Can you send it?");

        let group = parse_label(
            "\u{200e}Photo from Priya, the new deck, 11Octoberat9:18\u{202f}AM, \u{200e}Received in Acme team",
            "Acme team",
            now,
        )
        .unwrap();
        assert_eq!(group.sender, "Priya");
        assert_eq!(group.text, "[Photo] the new deck");

        let call = parse_label(
            "\u{200e}Missed video call, 11Octoberat2:53\u{202f}PM, \u{200e}Received from Ben",
            "Ben",
            now,
        )
        .unwrap();
        assert_eq!(call.text, "Missed video call");
        assert!(parse_label("Search", "Ben", now).is_none());
    }

    fn node(
        role: &str,
        identifier: Option<&str>,
        description: Option<&str>,
        value: Option<&str>,
        children: Vec<Node>,
    ) -> Node {
        Node {
            role: role.to_owned(),
            subrole: None,
            title: None,
            value: value.map(str::to_owned),
            description: description.map(str::to_owned),
            identifier: identifier.map(str::to_owned),
            classes: Vec::new(),
            url: None,
            frame: None,
            shown: None,
            children,
        }
    }

    #[test]
    fn reads_the_open_chat_and_the_list() {
        let cell = |label: &str| {
            node(
                "AXStaticText",
                Some("WAMessageBubbleTableViewCell"),
                Some(label),
                None,
                vec![],
            )
        };
        let root = node(
            "AXWindow",
            None,
            None,
            None,
            vec![
                node(
                    "AXGroup",
                    Some("ChatListView_TableView"),
                    Some("\u{200e}List of chats"),
                    None,
                    vec![node(
                        "AXButton",
                        None,
                        Some("Priya"),
                        Some(
                            "\u{200e}message, demo Thursday at 3pm?, 12Octoberat6:00\u{202f}PM, \u{200e}Received from Priya",
                        ),
                        vec![],
                    )],
                ),
                node(
                    "AXButton",
                    Some("NavigationBar_HeaderViewButton"),
                    Some("Ben Ashton"),
                    None,
                    vec![],
                ),
                node(
                    "AXGroup",
                    Some("ChatMessagesTableView"),
                    Some("\u{200e}Messages in chat with Ben Ashton"),
                    None,
                    vec![
                        node("AXHeading", None, None, None, vec![]),
                        cell(
                            "\u{200e}message, Can you send the proposal?, 13Octoberat11:00\u{202f}AM, \u{200e}Received from Ben Ashton",
                        ),
                        cell(
                            "\u{200e}Your message, Kal bhej dunga, 13Octoberat11:02\u{202f}AM, \u{200e}Sent to Ben Ashton, \u{200e}Read",
                        ),
                    ],
                ),
            ],
        );
        let snapshot = AppSnapshot {
            pid: 1,
            app_name: Some("WhatsApp".into()),
            windows: vec![WindowSnapshot {
                title: Some("WhatsApp".into()),
                root,
            }],
            stats: Stats::default(),
        };
        let shown = parse(&snapshot, now());
        let open = shown.open.as_ref().unwrap();
        assert_eq!(open.name, "Ben Ashton");
        assert_eq!(open.messages.len(), 2);
        assert!(open.messages[1].mine);
        assert_eq!(shown.list[0].name, "Priya");

        let text = to_text(&shown);
        assert!(text.starts_with("WhatsApp chat with Ben Ashton\nBen Ashton [sent 2026-10-13 11:00]: Can you send the proposal?\nme [sent 2026-10-13 11:02]: Kal bhej dunga"));
        assert!(text.contains("Priya: Priya [sent 2026-10-12 18:00]: demo Thursday at 3pm?"));
        // Timestamps alone do not look like plans; the promise does.
        let candidates = memory::screen::candidates(None, &text).unwrap();
        assert!(candidates.contains("demo Thursday at 3pm"));

        let store = Store::open_in_memory().unwrap();
        assert_eq!(super::store(&store, &shown, 1).unwrap(), 3);
        assert_eq!(super::store(&store, &shown, 2).unwrap(), 0);
    }
}
