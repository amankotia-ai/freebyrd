//! Page snapshots sent by the browser extension, and their on-disk store.
//!
//! The store keeps `latest.json` (the page most recently read) and appends
//! every new snapshot to `pages.jsonl`. A snapshot whose URL and text match
//! the latest one is not stored again.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSnapshot {
    pub tab_id: i64,
    pub url: String,
    pub title: String,
    pub text: String,
    pub truncated: bool,
    #[serde(default)]
    pub headings: Vec<Heading>,
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub lang: Option<String>,
    /// Why the extension read the page: `activated`, `loaded`, `focused`,
    /// `changed`, or `requested` when the desktop side asked for it.
    pub reason: String,
    pub captured_at_ms: u64,
    /// For an open email thread (Gmail), who said what and when, as the
    /// extension read it. Passed through to the memory as is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<serde_json::Value>,
    /// The lines of `text` in the browser's viewport, on reads the desktop
    /// side asked for. They matter only to the question being asked, so the
    /// store leaves them out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub in_view: String,
    /// On an AI chat site, the person's own last message, on reads the
    /// desktop side asked for (`memory::moments`). The store leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// Messages the extension sends to the native host.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtensionMessage {
    Hello {
        version: String,
    },
    Page(PageSnapshot),
    /// The answer to [`HostMessage::ReadActivePage`]: a page, or the reason
    /// the extension declined (`paused`, `excluded:...`, `incognito`, ...).
    #[serde(rename_all = "camelCase")]
    PageResponse {
        request_id: String,
        page: Option<PageSnapshot>,
        error: Option<String>,
    },
}

/// Messages the native host sends back.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostMessage {
    #[serde(rename_all = "camelCase")]
    Welcome {
        host_version: String,
    },
    Ack {
        stored: bool,
    },
    Error {
        message: String,
    },
    /// Asks the extension to read the active tab of the focused window now,
    /// or of the window focused at `front_at_ms`.
    #[serde(rename_all = "camelCase")]
    ReadActivePage {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        front_at_ms: Option<u64>,
    },
    /// Asks the extension to reload itself, picking up a new unpacked build.
    ReloadExtension,
}

pub struct PageStore {
    dir: PathBuf,
}

impl PageStore {
    /// `~/Library/Application Support/Freebyrd/browser`.
    pub fn default_dir() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(Path::new(&home).join("Library/Application Support/Freebyrd/browser"))
    }

    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stores `page` unless it repeats the latest snapshot. Returns whether it
    /// was stored.
    pub fn record(&self, page: &PageSnapshot) -> io::Result<bool> {
        let without_view;
        let page = if page.in_view.is_empty() && page.ask.is_none() {
            page
        } else {
            without_view = PageSnapshot {
                in_view: String::new(),
                ask: None,
                ..page.clone()
            };
            &without_view
        };
        if let Some(latest) = self.latest()?
            && latest.url == page.url
            && latest.text == page.text
        {
            return Ok(false);
        }

        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("pages.jsonl"))?;
        let mut line = serde_json::to_vec(page)?;
        line.push(b'\n');
        log.write_all(&line)?;

        // Write then rename so readers never see a half-written file.
        let tmp = self.dir.join("latest.json.tmp");
        let mut writer = BufWriter::new(File::create(&tmp)?);
        serde_json::to_writer_pretty(&mut writer, page)?;
        writer
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?;
        fs::rename(&tmp, self.dir.join("latest.json"))?;
        Ok(true)
    }

    /// The latest page, if one of `window_titles` is showing it.
    pub fn latest_shown_in<'a>(
        &self,
        window_titles: impl IntoIterator<Item = &'a str>,
    ) -> io::Result<Option<PageSnapshot>> {
        let Some(page) = self.latest()? else {
            return Ok(None);
        };
        let shown = !page.title.is_empty()
            && window_titles
                .into_iter()
                .any(|title| !title.trim().is_empty() && window_shows(title, &page));
        Ok(shown.then_some(page))
    }

    pub fn latest(&self) -> io::Result<Option<PageSnapshot>> {
        match fs::read(self.dir.join("latest.json")) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Whether a browser window titled `window_title` could be showing `page`.
/// Chromium names a window after its active tab: the page's title, in some
/// browsers with a suffix ("Page title - Google Chrome", a profile's name),
/// or the address when the page has no title. Arc uses its own name for the
/// tab when it has one (a tab the person renamed, or a title Arc tidied:
/// "LinkedIn" for "Feed | LinkedIn"), so a title inside the page's, or one
/// sharing a word with it, counts too. A loose check, meant to catch a page
/// from another window entirely.
pub fn window_shows(window_title: &str, page: &PageSnapshot) -> bool {
    let window = squash(window_title).to_lowercase();
    let title = squash(&page.title).to_lowercase();
    if window.is_empty() {
        return true;
    }
    if !title.is_empty() {
        return window.starts_with(&title)
            || title.contains(&window)
            || share_a_word(&window, &title);
    }
    host(&page.url).is_none_or(|host| window.contains(host.trim_start_matches("www.")))
}

/// Runs of whitespace as one space, trimmed.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether two titles share a word of four or more letters or digits.
fn share_a_word(a: &str, b: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .map(str::to_owned)
            .collect()
    };
    let b = words(b);
    words(a).iter().any(|word| b.contains(word))
}

fn host(url: &str) -> Option<&str> {
    let authority = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = match authority.strip_prefix('[') {
        Some(v6) => &authority[..v6.find(']')? + 2],
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_window_to_its_page() {
        let docs = PageSnapshot {
            url: "https://docs.example.com/models".to_owned(),
            title: "Models  |  Example".to_owned(),
            ..page("")
        };
        // Arc: the title alone, or its own shorter name. Chrome: a suffix.
        assert!(window_shows("Models | Example", &docs));
        assert!(window_shows("Example", &docs));
        assert!(window_shows("Example models", &docs));
        assert!(window_shows("Models | Example - Work", &docs));
        // Another window in front, such as an incognito one the extension
        // cannot see, shows another page.
        assert!(!window_shows("History / X", &docs));
        // No title to go on.
        assert!(window_shows("", &docs));

        // A page without a title is named by its address.
        let untitled = PageSnapshot {
            url: "http://www.example.org:8080/a?b".to_owned(),
            title: String::new(),
            ..page("")
        };
        assert!(window_shows("example.org:8080/a?b", &untitled));
        assert!(!window_shows("History / X", &untitled));
        assert_eq!(host("http://[::1]:3000/x"), Some("[::1]"));
        assert_eq!(host("https://me@example.com"), Some("example.com"));
        assert_eq!(host("about:blank"), None);
    }

    fn page(text: &str) -> PageSnapshot {
        PageSnapshot {
            tab_id: 7,
            url: "https://example.com/".to_owned(),
            title: "Example".to_owned(),
            text: text.to_owned(),
            truncated: false,
            headings: vec![],
            selection: String::new(),
            lang: None,
            reason: "loaded".to_owned(),
            captured_at_ms: 1,
            conversation: None,
            in_view: String::new(),
            ask: None,
        }
    }

    #[test]
    fn parses_the_extension_page_message() {
        let json = r#"{"type":"page","tabId":3,"reason":"activated","capturedAtMs":1700000000000,
            "url":"https://example.com/a","title":"A","lang":"en","text":"Hello","truncated":false,
            "headings":[{"level":1,"text":"Hello"}],"selection":""}"#;
        let ExtensionMessage::Page(page) = serde_json::from_str(json).unwrap() else {
            panic!("expected a page message");
        };
        assert_eq!(page.tab_id, 3);
        assert_eq!(
            page.headings,
            vec![Heading {
                level: 1,
                text: "Hello".to_owned()
            }]
        );
    }

    #[test]
    fn skips_a_repeat_of_the_latest_page() {
        let dir = std::env::temp_dir().join(format!("freebyrd-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = PageStore::open(&dir).unwrap();

        assert!(store.record(&page("one")).unwrap());
        assert!(!store.record(&page("one")).unwrap());
        assert!(store.record(&page("two")).unwrap());
        assert_eq!(store.latest().unwrap().unwrap().text, "two");
        let lines = fs::read_to_string(dir.join("pages.jsonl")).unwrap();
        assert_eq!(lines.lines().count(), 2);

        // What was in view, and the person's last message in a chat, go to
        // the desktop side, not into the store.
        let viewed = PageSnapshot {
            in_view: "three".to_owned(),
            ask: Some("make the logo face right".to_owned()),
            ..page("three")
        };
        assert!(store.record(&viewed).unwrap());
        assert_eq!(store.latest().unwrap().unwrap().in_view, "");
        assert_eq!(store.latest().unwrap().unwrap().ask, None);
        let lines = fs::read_to_string(dir.join("pages.jsonl")).unwrap();
        assert!(!lines.contains("inView") && !lines.contains("\"ask\""));

        // An extension from before still sends the posts that scrolled past
        // on X; they are no longer kept (2026-10-16).
        let json = r#"{"tabId":1,"url":"https://x.com/home","title":"Home / X","text":"four",
            "truncated":false,"reason":"changed","capturedAtMs":2,"x":{"posts":[{"id":"1"}]}}"#;
        let posts: PageSnapshot = serde_json::from_str(json).unwrap();
        assert!(store.record(&posts).unwrap());
        let lines = fs::read_to_string(dir.join("pages.jsonl")).unwrap();
        assert!(!lines.contains("posts"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
