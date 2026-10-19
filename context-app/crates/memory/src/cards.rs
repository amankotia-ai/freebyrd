//! Cards are what the notch has to show: recaps, promises put on the
//! calendar, events seen on screen, questions. A card is new until the
//! overlay has shown it, and while any card is new, the notch glows.
//!
//! A card can carry buttons (`actions`) and data for its kind, such as the
//! event a nudge proposes, which the overlay shows as fields to edit.

use anyhow::Result;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::Store;

/// How long a card stays in the overlay after it was first shown.
pub const SHOWN_FOR_MS: i64 = 8 * 3_600_000;
const MAX_SHOWN: i64 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
}

impl Action {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub kind: String,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub created_at: i64,
    pub seen_at: Option<i64>,
    pub actions: Vec<Action>,
    pub data: Option<Value>,
}

const COLUMNS: &str = "id, kind, title, body, created_at, seen_at, actions, data";

fn from_row(row: &Row) -> rusqlite::Result<Card> {
    let actions: String = row.get(6)?;
    let data: Option<String> = row.get(7)?;
    Ok(Card {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
        seen_at: row.get(5)?,
        actions: serde_json::from_str(&actions).unwrap_or_default(),
        data: data.and_then(|data| serde_json::from_str(&data).ok()),
    })
}

/// Adds the card for `reference`, or replaces it as new if it exists.
pub fn upsert(
    store: &Store,
    kind: &str,
    reference: &str,
    title: &str,
    body: &str,
    now_ms: i64,
) -> Result<()> {
    upsert_with(store, kind, reference, title, body, &[], None, now_ms)
}

/// `upsert`, with buttons and data.
#[allow(clippy::too_many_arguments)]
pub fn upsert_with(
    store: &Store,
    kind: &str,
    reference: &str,
    title: &str,
    body: &str,
    actions: &[Action],
    data: Option<&Value>,
    now_ms: i64,
) -> Result<()> {
    store.conn().execute(
        "INSERT INTO card (kind, ref, title, body, actions, data, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (ref) DO UPDATE SET
             kind = excluded.kind, title = excluded.title, body = excluded.body,
             actions = excluded.actions, data = excluded.data,
             created_at = excluded.created_at, seen_at = NULL, dismissed_at = NULL",
        params![
            kind,
            reference,
            title,
            body,
            serde_json::to_string(actions)?,
            data.map(Value::to_string),
            now_ms
        ],
    )?;
    Ok(())
}

/// Adds the card for `reference` unless there ever was one: a question is
/// asked once. Returns whether it was added.
#[allow(clippy::too_many_arguments)]
pub fn add_once(
    store: &Store,
    kind: &str,
    reference: &str,
    title: &str,
    body: &str,
    actions: &[Action],
    data: Option<&Value>,
    now_ms: i64,
) -> Result<bool> {
    let added = store.conn().execute(
        "INSERT OR IGNORE INTO card (kind, ref, title, body, actions, data, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            kind,
            reference,
            title,
            body,
            serde_json::to_string(actions)?,
            data.map(Value::to_string),
            now_ms
        ],
    )?;
    Ok(added == 1)
}

/// After a button: the card now says `body`, with `actions` (often none).
/// It stays seen, so the glow does not come back for it.
pub fn settle(
    store: &Store,
    id: i64,
    title: &str,
    body: &str,
    actions: &[Action],
    data: Option<&Value>,
) -> Result<()> {
    store.conn().execute(
        "UPDATE card SET title = ?2, body = ?3, actions = ?4, data = ?5 WHERE id = ?1",
        params![
            id,
            title,
            body,
            serde_json::to_string(actions)?,
            data.map(Value::to_string)
        ],
    )?;
    Ok(())
}

pub fn get(store: &Store, id: i64) -> Result<Option<Card>> {
    Ok(store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM card WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

/// The card's reference: what it is about, such as `plan:12`.
pub fn reference(store: &Store, id: i64) -> Result<Option<String>> {
    Ok(store
        .conn()
        .query_row("SELECT ref FROM card WHERE id = ?1", [id], |row| row.get(0))
        .optional()?)
}

/// What the overlay shows when it opens: cards not dismissed, and either
/// new or first shown recently. Newest first. A day plan's blocks show with
/// their plan only (`day`).
pub fn for_overlay(store: &Store, now_ms: i64) -> Result<Vec<Card>> {
    let mut statement = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM card
         WHERE dismissed_at IS NULL AND (seen_at IS NULL OR seen_at > ?1) AND kind != 'block'
         ORDER BY created_at DESC, id DESC LIMIT ?2"
    ))?;
    let cards = statement
        .query_map(params![now_ms - SHOWN_FOR_MS, MAX_SHOWN], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cards)
}

pub fn has_new(store: &Store) -> Result<bool> {
    Ok(store
        .conn()
        .query_row(
            "SELECT 1 FROM card WHERE seen_at IS NULL AND dismissed_at IS NULL LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// When the newest card waiting to be seen was added. It changes when a
/// card arrives from outside the app, such as a scheduled job's result.
pub fn newest_waiting(store: &Store) -> Result<Option<i64>> {
    Ok(store.conn().query_row(
        "SELECT max(created_at) FROM card WHERE seen_at IS NULL AND dismissed_at IS NULL",
        [],
        |row| row.get(0),
    )?)
}

/// The kinds of the cards waiting to be seen, each once, the kind with the
/// newest card first. The notch shows them beside it while it glows.
pub fn waiting_kinds(store: &Store) -> Result<Vec<String>> {
    let mut statement = store.conn().prepare(
        "SELECT kind FROM card WHERE seen_at IS NULL AND dismissed_at IS NULL
         GROUP BY kind ORDER BY max(created_at) DESC",
    )?;
    let kinds = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(kinds)
}

pub fn mark_seen(store: &Store, ids: &[i64], now_ms: i64) -> Result<()> {
    let mut statement = store
        .conn()
        .prepare("UPDATE card SET seen_at = ?2 WHERE id = ?1 AND seen_at IS NULL")?;
    for id in ids {
        statement.execute(params![id, now_ms])?;
    }
    Ok(())
}

pub fn dismiss(store: &Store, id: i64, now_ms: i64) -> Result<()> {
    store.conn().execute(
        "UPDATE card SET dismissed_at = ?2, seen_at = coalesce(seen_at, ?2) WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cards_glow_until_shown_and_stay_a_while() {
        let store = Store::open_in_memory().unwrap();
        assert!(!has_new(&store).unwrap());
        upsert(&store, "recap", "recap:a", "Mon", "body", 100).unwrap();
        assert!(has_new(&store).unwrap());

        let shown = for_overlay(&store, 200).unwrap();
        assert_eq!(shown.len(), 1);
        mark_seen(&store, &[shown[0].id], 200).unwrap();
        assert!(!has_new(&store).unwrap());
        assert_eq!(
            for_overlay(&store, 200 + SHOWN_FOR_MS - 1).unwrap().len(),
            1
        );
        assert!(for_overlay(&store, 200 + SHOWN_FOR_MS).unwrap().is_empty());

        // Replacing a card makes it new again.
        upsert(&store, "recap", "recap:a", "Mon", "better body", 300).unwrap();
        assert!(has_new(&store).unwrap());
        let id = for_overlay(&store, 300).unwrap()[0].id;
        dismiss(&store, id, 400).unwrap();
        assert!(!has_new(&store).unwrap());
        assert!(for_overlay(&store, 400).unwrap().is_empty());
    }

    #[test]
    fn waiting_kinds_come_newest_first_once_each() {
        let store = Store::open_in_memory().unwrap();
        upsert(&store, "recap", "recap:a", "Mon", "x", 100).unwrap();
        upsert(&store, "due", "due:a", "Ben", "x", 200).unwrap();
        upsert(&store, "recap", "recap:b", "Tue", "x", 300).unwrap();
        assert_eq!(waiting_kinds(&store).unwrap(), ["recap", "due"]);

        let shown: Vec<i64> = for_overlay(&store, 400)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        mark_seen(&store, &shown, 400).unwrap();
        assert!(waiting_kinds(&store).unwrap().is_empty());
    }

    #[test]
    fn buttons_and_data_round_trip_and_questions_are_asked_once() {
        let store = Store::open_in_memory().unwrap();
        let data = serde_json::json!({ "title": "Demo", "date": "2026-10-15" });
        let actions = [Action::new("add", "Add to calendar")];
        assert!(
            add_once(
                &store,
                "event",
                "plan:1",
                "Thu",
                "body",
                &actions,
                Some(&data),
                1
            )
            .unwrap()
        );
        assert!(!add_once(&store, "event", "plan:1", "Thu", "again", &actions, None, 2).unwrap());

        let card = &for_overlay(&store, 3).unwrap()[0];
        assert_eq!(card.actions, actions);
        assert_eq!(card.data.as_ref(), Some(&data));
        assert_eq!(
            reference(&store, card.id).unwrap().as_deref(),
            Some("plan:1")
        );

        settle(
            &store,
            card.id,
            "Thu",
            "Added",
            &[Action::new("undo", "Undo")],
            None,
        )
        .unwrap();
        let settled = get(&store, card.id).unwrap().unwrap();
        assert_eq!(
            (settled.body.as_str(), settled.actions.len(), settled.data),
            ("Added", 1, None)
        );
    }
}
