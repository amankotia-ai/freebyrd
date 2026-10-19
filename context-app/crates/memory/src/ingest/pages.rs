//! Pages the browser extension saved, from
//! `~/Library/Application Support/Freebyrd/browser/pages.jsonl` (written by
//! `browser-host`). Each distinct page text is stored once, in `page_text`.

use serde::Deserialize;

use super::{SourceReport, read_new_lines};
use crate::conversations::{self, Conversation, ME, NewMessage};
use crate::redact::redact;
use crate::store::{NewActivity, Store, fingerprint};
use crate::{exclusions, home, one_line, truncate};

const SOURCE: &str = "page";
const MAX_TEXT_CHARS: usize = 100_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    tab_id: i64,
    url: String,
    title: String,
    text: String,
    reason: String,
    captured_at_ms: i64,
    #[serde(default)]
    conversation: Option<Thread>,
}

/// An email thread as `extension/content.js` reads it from Gmail.
#[derive(Deserialize)]
struct Thread {
    app: String,
    key: String,
    title: String,
    #[serde(default)]
    messages: Vec<ThreadMessage>,
}

#[derive(Deserialize)]
struct ThreadMessage {
    sender: String,
    /// The sender's address, which joins their name on their page
    /// (`people`). Older reads carry none.
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    me: bool,
    /// Who it went to, the person left out. Older reads carry none.
    #[serde(default)]
    to: Vec<Recipient>,
    time: Option<String>,
    text: String,
}

#[derive(Deserialize)]
struct Recipient {
    name: Option<String>,
    email: Option<String>,
}

pub fn ingest(store: &Store) -> SourceReport {
    let mut report = SourceReport::new("Browser pages");
    let Some(path) =
        home().map(|home| home.join("Library/Application Support/Freebyrd/browser/pages.jsonl"))
    else {
        return report;
    };
    if !path.is_file() {
        return report;
    }
    report.files = 1;
    let result = read_new_lines(store, SOURCE, &path, |line| {
        match serde_json::from_str::<Snapshot>(line) {
            Ok(snapshot) => record(store, &snapshot),
            Err(_) => Ok(0),
        }
    });
    match result {
        Ok(added) => report.added = added,
        Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
    }
    report
}

fn record(store: &Store, page: &Snapshot) -> anyhow::Result<usize> {
    // The extension never sends excluded pages; checked again in case its
    // rules and these drift apart.
    if exclusions::reason(&page.url).is_some() {
        return Ok(0);
    }
    let text = truncate(&redact(&page.text), MAX_TEXT_CHARS);
    let text_id = if text.trim().is_empty() {
        None
    } else {
        Some(store.page_text_id(&text)?)
    };
    let url = redact(&page.url);
    let title = redact(page.title.trim());
    if let Some(thread) = &page.conversation {
        record_thread(store, thread, page.captured_at_ms)?;
    }
    let ext_id = format!(
        "{}:{}:{}",
        page.captured_at_ms,
        page.tab_id,
        fingerprint(&page.url)
    );
    let added = store.add_activity(&NewActivity {
        source: SOURCE,
        kind: "page",
        ts: page.captured_at_ms,
        title: (!title.is_empty()).then_some(title.as_str()),
        url: Some(&url),
        detail: Some(&page.reason),
        text_id,
        ext_id: &ext_id,
        ..Default::default()
    })?;
    Ok(usize::from(added))
}

/// An email thread: each message is stored once, and email counts as work,
/// so promises in it go on the calendar.
fn record_thread(store: &Store, thread: &Thread, seen_at: i64) -> anyhow::Result<()> {
    if thread.app != "gmail" || thread.key.is_empty() {
        return Ok(());
    }
    let title = one_line(&redact(&thread.title), 160);
    let messages: Vec<NewMessage> = thread
        .messages
        .iter()
        .map(|message| NewMessage {
            sender: if message.me {
                ME
            } else {
                message.sender.trim()
            },
            text: &message.text,
            sent_label: message.time.as_deref(),
            sent_at: message
                .time
                .as_deref()
                .and_then(conversations::parse_full_time),
        })
        .collect();
    let (conversation, _) = conversations::record(
        store,
        &Conversation {
            app: "gmail",
            key: &thread.key,
            title: &title,
            work: Some(true),
        },
        &messages,
        seen_at,
    )?;
    for (message, stored) in thread.messages.iter().zip(&messages) {
        let at = stored.sent_at.unwrap_or(seen_at);
        if let Some(email) = message.email.as_deref().filter(|_| !message.me) {
            crate::people::note_email(store, conversation, &message.sender, email, at)?;
        }
        // Who it went to: on the message, and on their pages.
        let names: Vec<&str> = message
            .to
            .iter()
            .filter_map(|to| to.name.as_deref().or(to.email.as_deref()))
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        if !names.is_empty() {
            conversations::set_recipients(store, conversation, stored, &names.join(", "))?;
        }
        for to in &message.to {
            if let Some(email) = to.email.as_deref() {
                let name = to.name.as_deref().unwrap_or_default();
                crate::people::note_recipient(store, conversation, name, email, at)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_pages_and_their_text_once() {
        let store = Store::open_in_memory().unwrap();
        let page = |ts: i64, url: &str, text: &str| Snapshot {
            tab_id: 1,
            url: url.into(),
            title: "Sprint board".into(),
            text: text.into(),
            reason: "activated".into(),
            captured_at_ms: ts,
            conversation: None,
        };
        assert_eq!(
            record(&store, &page(1, "https://notion.so/a", "Tasks: ship it")).unwrap(),
            1
        );
        assert_eq!(
            record(&store, &page(2, "https://notion.so/a", "Tasks: ship it")).unwrap(),
            1
        );
        assert_eq!(
            record(&store, &page(2, "https://notion.so/a", "Tasks: ship it")).unwrap(),
            0
        );
        assert_eq!(
            record(&store, &page(3, "https://login.example.com", "x")).unwrap(),
            0
        );
        let texts: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM page_text", [], |row| row.get(0))
            .unwrap();
        assert_eq!(texts, 1);
    }

    #[test]
    fn stores_an_email_thread_as_a_work_conversation() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tabId":1,"url":"https://mail.google.com/mail/u/0/#inbox/FMfc123","title":"Pilot - me@x.com - Gmail","text":"...","reason":"changed","capturedAtMs":5,
            "conversation":{"app":"gmail","key":"FMfc123","title":"Pilot next steps","messages":[
              {"sender":"Priya Shah","email":"priya.shah@acme.com","me":false,"time":"Tue, 13 Oct 2026, 11:00","text":"Could you send the proposal?"},
              {"sender":"Abhishek","me":true,"time":"Tue, 13 Oct 2026, 11:30","text":"Sure, by Thursday."}]}}"#;
        let page: Snapshot = serde_json::from_str(json).unwrap();
        record(&store, &page).unwrap();
        record(&store, &page).unwrap();
        let rows: Vec<(String, bool, Option<i64>)> = store
            .conn()
            .prepare("SELECT m.sender, c.work, m.sent_at FROM message m JOIN conversation c ON c.id = m.conversation_id ORDER BY m.id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0, ME);
        assert!(rows[0].1);
        assert!(rows[1].2.is_some());
        // The sender's address is kept with their name, for their page.
        let pair: Option<String> = store
            .conn()
            .query_row(
                "SELECT pair FROM person_alias WHERE value = 'priya.shah@acme.com'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pair.as_deref(), Some("priya shah"));
    }

    #[test]
    fn a_thread_read_again_learns_who_a_forward_went_to() {
        let store = Store::open_in_memory().unwrap();
        let before = r#"{"tabId":1,"url":"https://mail.google.com/mail/u/0/#inbox/FMfc9","title":"Factors - me@x.com - Gmail","text":"...","reason":"changed","capturedAtMs":5,
            "conversation":{"app":"gmail","key":"FMfc9","title":"Unifize <> Factors","messages":[
              {"sender":"Abhishek","me":true,"time":"Wed, 14 Oct 2026, 19:11","text":"---------- Forwarded message ---------"}]}}"#;
        record(&store, &serde_json::from_str(before).unwrap()).unwrap();
        // The same message, read by the new reader: it gains who it went to.
        let after = before.replace(
            r#""me":true,"#,
            r#""me":true,"to":[{"name":"Ben Merton","email":"ben.merton@unifize.com"}],"#,
        );
        record(&store, &serde_json::from_str(&after).unwrap()).unwrap();
        let rows: Vec<Option<String>> = store
            .conn()
            .prepare("SELECT recipients FROM message")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [Some("Ben Merton".to_owned())]);
        // Ben's address joins his name, seen as a recipient there.
        let seen: (Option<String>, String) = store
            .conn()
            .query_row(
                "SELECT a.pair, s.role FROM person_alias a JOIN person_seen s ON s.alias = a.value
                 WHERE a.value = 'ben.merton@unifize.com'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            seen,
            (Some("ben merton".to_owned()), "recipient".to_owned())
        );
    }

    #[test]
    fn posts_from_x_in_an_older_read_are_no_longer_kept() {
        // Reads saved before the X suggestions were removed (2026-10-16)
        // carry the posts that scrolled past; the page is kept, the posts
        // are not.
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tabId":1,"url":"https://x.com/i/bookmarks","title":"Bookmarks / X","text":"...","reason":"changed","capturedAtMs":5,
            "x":{"posts":[{"id":"2108302539498414208","author":"AnthropicAI","name":"Anthropic","text":"We are launching",
              "postedAt":1760000000000,"links":[],"media":0,"quoted":null,"bookmarked":true,"liked":null,"seenAt":4}]}}"#;
        let page: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(record(&store, &page).unwrap(), 1);
        let posts: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM x_post", [], |row| row.get(0))
            .unwrap();
        assert_eq!(posts, 0);
    }
}
