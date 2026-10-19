//! People: one page for each person the person deals with, built from
//! everywhere Freebyrd saw them. Ben is "Ben Merton" in a WhatsApp chat, a
//! sender with an address in a Gmail thread, and an attendee and owner of
//! action items in a Fireflies meeting; his page joins all three, so "what's
//! open with Ben?" gets what he owes, what is owed to him and what waits on
//! a reply, not only messages that happen to contain the word "Ben".
//! Littlebird keeps a people wiki that merges identities the same way
//! (`littlebird/FINDINGS.md` §5.0); we know its shape only, and this is
//! built from Freebyrd's own memory.
//!
//! Plain code on this Mac, no model:
//! - Each name or address seen is an alias, normalized ("ben merton",
//!   "ben.merton@unifize.com"), with the conversations it was seen in.
//! - The same full name in two apps is one person. An address joins the
//!   name it was seen with, or the name its first part spells
//!   ("ben.merton@" is Ben Merton); its domain is where they work.
//! - A first name alone ("Ben") joins the one person with that first name,
//!   and stays undecided while two have it: their pages offer it as "maybe".
//! - The person themselves, group chats and bots are never people.
//! - What the person joins or splits by hand is never moved by code.
//!
//! Nothing here leaves the Mac. A page goes to Claude only inside the
//! memory of a question about that person (`recall`), or with Brief me.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::conversations::ME;
use crate::redact::redact;
use crate::store::Store;
use crate::todos::{self, Item};
use crate::{one_line, person_name, time, truncate};

/// A chat named after them: a WhatsApp chat with one other side.
pub const ROLE_CHAT: &str = "chat";
/// They wrote a message in it.
pub const ROLE_SENDER: &str = "sender";
/// A meeting they own an action item from.
pub const ROLE_OWNER: &str = "owner";
/// A meeting they attended or spoke in.
pub const ROLE_ATTENDEE: &str = "attendee";
/// An email of the person's went to them (to or cc).
pub const ROLE_RECIPIENT: &str = "recipient";

/// Senders that are not people: a meeting's summary, an action item with no
/// owner.
const NOT_SENDERS: [&str; 3] = [ME, crate::connectors::SUMMARY, "action item"];
/// Words that mark a bot, a mailing list, a shared inbox, or a group named
/// as an action item's owner ("All managers and meeting owners").
const NOT_PEOPLE: [&str; 28] = [
    "notetaker",
    "fireflies",
    "otter",
    "bot",
    "noreply",
    "no-reply",
    "donotreply",
    "notifications",
    "notification",
    "calendar",
    "support",
    "team",
    "admin",
    "newsletter",
    "mailer-daemon",
    "unknown",
    "all",
    "everyone",
    "owners",
    "managers",
    "teams",
    "members",
    "leads",
    "participants",
    "attendees",
    "folks",
    "staff",
    "tbd",
];
/// Email boxes that are not one person.
const NOT_MAILBOXES: [&str; 8] = [
    "info", "hello", "contact", "sales", "billing", "updates", "news", "bounce",
];
/// Addresses at these domains say nothing about where someone works.
const PUBLIC_MAIL: [&str; 12] = [
    "gmail.com",
    "googlemail.com",
    "yahoo.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "proton.me",
    "protonmail.com",
    "aol.com",
];
/// First names that are also common words: a question saying "will" or
/// "may" is not about Will or May.
const WORD_NAMES: [&str; 14] = [
    "will", "may", "mark", "bill", "ray", "april", "june", "art", "hope", "joy", "grace", "rose",
    "sun", "summer",
];

/// How long a reply can wait on the person and still count as waiting.
const WAITING_DAYS: i64 = 7;
/// The memory sent with a question about someone, per person.
const MEMORY_CHARS: usize = 3_000;
/// At most this many people a question.
const MAX_FOR_QUESTION: usize = 2;
/// The People tab lists at most this many.
const MAX_LISTED: usize = 40;

// Names and aliases.

/// What kind of alias a written name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Email,
    Phone,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Email => "email",
            Self::Phone => "phone",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "email" => Self::Email,
            "phone" => Self::Phone,
            _ => Self::Name,
        }
    }
}

/// A name as written, normalized: lowercase words, with WhatsApp's "~",
/// emoji, periods and brackets taken out ("~ Ben M. (Unifize)" is "ben m
/// unifize").
pub fn normalize_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' || c == '-' {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(|word| word.trim_matches(|c| c == '\'' || c == '-'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A written name or address as an alias: its kind and normalized value.
fn alias_of(text: &str) -> Option<(Kind, String)> {
    let text = text.trim();
    if text.contains('@') {
        let email = text
            .trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace())
            .to_lowercase();
        let (local, domain) = email.split_once('@')?;
        let valid = !local.is_empty()
            && domain.contains('.')
            && !email.contains(char::is_whitespace)
            && email.matches('@').count() == 1;
        return valid.then_some((Kind::Email, email));
    }
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if digits.len() >= 7 && letters == 0 {
        return Some((Kind::Phone, format!("+{digits}")));
    }
    let name = normalize_name(text);
    (!name.is_empty() && letters > 0).then_some((Kind::Name, name))
}

/// The name an address spells: "ben.merton@unifize.com" is "ben merton".
fn name_from_email(email: &str) -> String {
    let local = email.split('@').next().unwrap_or_default();
    let local = local.split('+').next().unwrap_or_default();
    local
        .split(|c: char| !c.is_alphabetic())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The name an alias stands for: the name itself, the name an address was
/// seen with or spells, or a phone number (which only ever matches itself).
fn identity(kind: Kind, value: &str, pair: Option<&str>) -> Option<String> {
    match kind {
        Kind::Name => Some(value.to_owned()),
        Kind::Phone => Some(value.to_owned()),
        Kind::Email => {
            let name = pair
                .map(normalize_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| name_from_email(value));
            (!name.is_empty()).then_some(name)
        }
    }
}

/// A bot, a list, a shared inbox, or a sender that is not a person.
fn not_a_person(kind: Kind, value: &str) -> bool {
    if NOT_SENDERS.contains(&value) {
        return true;
    }
    let words: Vec<&str> = value
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|word| !word.is_empty())
        .collect();
    if words.iter().any(|word| NOT_PEOPLE.contains(word)) {
        return true;
    }
    if kind == Kind::Email {
        let local = value.split('@').next().unwrap_or_default();
        return NOT_MAILBOXES.contains(&local)
            || NOT_PEOPLE.iter().any(|word| local.contains(word));
    }
    false
}

/// The person themselves, by their Mac account's name: their full name, or
/// their first name alone or with their last name.
#[derive(Debug, Clone, Default)]
struct Me {
    full: String,
    first: String,
    last: String,
}

impl Me {
    fn new(name: Option<&str>) -> Self {
        let full = name.map(normalize_name).unwrap_or_default();
        let words: Vec<&str> = full.split(' ').collect();
        Self {
            first: words.first().copied().unwrap_or_default().to_owned(),
            last: if words.len() > 1 {
                words.last().copied().unwrap_or_default().to_owned()
            } else {
                String::new()
            },
            full,
        }
    }

    fn is(&self, name: &str) -> bool {
        if self.full.is_empty() || name.is_empty() {
            return false;
        }
        if name == self.full {
            return true;
        }
        let words: Vec<&str> = name.split(' ').collect();
        words.first() == Some(&self.first.as_str())
            && (words.len() == 1 || (!self.last.is_empty() && words.contains(&self.last.as_str())))
    }
}

/// "ben merton" as "Ben Merton", for a person known only by an address.
fn title_case(name: &str) -> String {
    name.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A name as it should show: without WhatsApp's "~" and surrounding space.
fn clean_shown(text: &str) -> String {
    one_line(text.trim().trim_start_matches('~').trim(), 80)
}

// Sightings: where each alias was seen.

struct Sighting {
    shown: String,
    pair: Option<String>,
    conversation_id: i64,
    role: &'static str,
    at: i64,
}

/// Brings the people up to date with what the memory holds: every sender,
/// chat, meeting attendee and action item owner becomes an alias with where
/// it was seen, and aliases are joined into people. Cheap enough to run on
/// every pass: it reads conversations grouped by sender.
pub fn sync(store: &Store, now_ms: i64) -> Result<usize> {
    sync_as(store, person_name().as_deref(), now_ms)
}

fn sync_as(store: &Store, me: Option<&str>, now_ms: i64) -> Result<usize> {
    let me = Me::new(me);
    let sightings = sightings(store)?;
    let tx = store.transaction()?;
    for sighting in &sightings {
        record(store, &me, sighting)?;
    }
    let changed = assign(store, now_ms)?;
    tx.commit()?;
    Ok(changed)
}

fn sightings(store: &Store) -> Result<Vec<Sighting>> {
    let mut found = Vec::new();
    // Who wrote in each conversation, and who owes each meeting's action
    // items.
    let mut statement = store.conn().prepare(
        "SELECT c.id, c.app, m.sender, max(coalesce(m.sent_at, m.first_seen_at))
         FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE m.sender NOT IN (?1, ?2, ?3)
         GROUP BY c.id, m.sender",
    )?;
    let rows = statement.query_map(
        params![NOT_SENDERS[0], NOT_SENDERS[1], NOT_SENDERS[2]],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    let mut senders: HashMap<i64, HashSet<String>> = HashMap::new();
    for row in rows {
        let (conversation_id, app, sender, at) = row?;
        senders
            .entry(conversation_id)
            .or_default()
            .insert(normalize_name(&sender));
        if app == "fireflies" {
            // An action item can name two owners ("Arun and Smriti", "Shilpa
            // V / Shilpashree V"): each is someone. A group is no one.
            if let Some((kind, value)) = alias_of(&sender)
                && not_a_person(kind, &value)
            {
                continue;
            }
            for owner in each_name(&sender) {
                found.push(Sighting {
                    shown: owner,
                    pair: None,
                    conversation_id,
                    role: ROLE_OWNER,
                    at,
                });
            }
        } else {
            found.push(Sighting {
                shown: sender,
                pair: None,
                conversation_id,
                role: ROLE_SENDER,
                at,
            });
        }
    }
    // A WhatsApp chat whose only other side is the chat's name is a chat
    // with that person; a group's name is not a person.
    let mut statement = store.conn().prepare(
        "SELECT c.id, c.title, coalesce(max(coalesce(m.sent_at, m.first_seen_at)), c.last_seen_at)
         FROM conversation c LEFT JOIN message m ON m.conversation_id = c.id
         WHERE c.app = 'whatsapp' GROUP BY c.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (conversation_id, title, at) = row?;
        let name = normalize_name(&title);
        let one_to_one = senders
            .get(&conversation_id)
            .is_none_or(|others| others.iter().all(|other| *other == name));
        if one_to_one {
            found.push(Sighting {
                shown: title,
                pair: None,
                conversation_id,
                role: ROLE_CHAT,
                at,
            });
        }
    }
    // A meeting's attendees, from its summary ("<title>, 45 min, with Ben
    // Merton, raj@unifize.com and 2 others."), and who spoke in it.
    let mut statement = store.conn().prepare(
        "SELECT c.id, m.text, coalesce(m.sent_at, m.first_seen_at)
         FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE c.app = 'fireflies' AND m.sender = ?1",
    )?;
    let rows = statement.query_map([crate::connectors::SUMMARY], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut met_at: HashMap<i64, i64> = HashMap::new();
    for row in rows {
        let (conversation_id, text, at) = row?;
        met_at.insert(conversation_id, at);
        for attendee in attendees(&text) {
            found.push(Sighting {
                shown: attendee,
                pair: None,
                conversation_id,
                role: ROLE_ATTENDEE,
                at,
            });
        }
    }
    let mut statement = store.conn().prepare(
        "SELECT DISTINCT l.conversation_id, l.speaker, c.last_seen_at
         FROM meeting_line l JOIN conversation c ON c.id = l.conversation_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (conversation_id, speaker, seen) = row?;
        found.push(Sighting {
            shown: speaker,
            pair: None,
            conversation_id,
            role: ROLE_ATTENDEE,
            at: met_at.get(&conversation_id).copied().unwrap_or(seen),
        });
    }
    Ok(found)
}

/// The names in a list of them: "Ben and Raj", "Ben & Raj", "Ben / Raj",
/// "Ben, Raj".
fn each_name(text: &str) -> Vec<String> {
    static AND: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\s+and\s+|[,&/+]").expect("valid pattern")
    });
    AND.split(text)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The attendees a meeting's summary lists on its first line, after
/// ", with ".
fn attendees(summary: &str) -> Vec<String> {
    let first = summary.lines().next().unwrap_or_default();
    let Some(at) = first.rfind(", with ") else {
        return Vec::new();
    };
    let mut list = first[at + ", with ".len()..].trim_end_matches('.').trim();
    if let Some(cut) = list.rfind(" and ")
        && list[cut + 5..].ends_with(" others")
    {
        list = &list[..cut];
    }
    list.split(", ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Keeps one sighting: its alias (unless it is the person, or not a
/// person) and where it was seen.
fn record(store: &Store, me: &Me, sighting: &Sighting) -> Result<()> {
    let Some((kind, value)) = alias_of(&sighting.shown) else {
        return Ok(());
    };
    let pair = sighting
        .pair
        .as_deref()
        .map(normalize_name)
        .filter(|pair| !pair.is_empty());
    let Some(identity) = identity(kind, &value, pair.as_deref()) else {
        return Ok(());
    };
    if me.is(&identity) || not_a_person(kind, &value) || not_a_person(Kind::Name, &identity) {
        return Ok(());
    }
    let shown = if kind == Kind::Name {
        clean_shown(&sighting.shown)
    } else {
        value.clone()
    };
    store.conn().execute(
        "INSERT INTO person_alias (value, kind, shown, pair, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT (value) DO UPDATE SET
             shown = CASE WHEN excluded.last_seen_at >= last_seen_at THEN excluded.shown ELSE shown END,
             pair = coalesce(excluded.pair, pair),
             first_seen_at = min(first_seen_at, excluded.first_seen_at),
             last_seen_at = max(last_seen_at, excluded.last_seen_at)",
        params![value, kind.as_str(), shown, pair, sighting.at],
    )?;
    store.conn().execute(
        "INSERT INTO person_seen (alias, conversation_id, role, last_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (alias, conversation_id, role) DO UPDATE SET last_at = max(last_at, excluded.last_at)",
        params![value, sighting.conversation_id, sighting.role, sighting.at],
    )?;
    Ok(())
}

/// An email thread's sender with their address, as the Gmail reader sees
/// them (`ingest::pages`): the address joins the name it was seen with.
pub fn note_email(
    store: &Store,
    conversation_id: i64,
    name: &str,
    email: &str,
    at: i64,
) -> Result<()> {
    let me = Me::new(person_name().as_deref());
    let name = name.trim();
    record(
        store,
        &me,
        &Sighting {
            shown: email.to_owned(),
            pair: (!name.is_empty() && !name.contains('@')).then(|| name.to_owned()),
            conversation_id,
            role: ROLE_SENDER,
            at,
        },
    )
}

/// Someone an email in `conversation_id` went to, by their address and the
/// name shown with it.
pub fn note_recipient(
    store: &Store,
    conversation_id: i64,
    name: &str,
    email: &str,
    at: i64,
) -> Result<()> {
    let me = Me::new(person_name().as_deref());
    let name = name.trim();
    record(
        store,
        &me,
        &Sighting {
            shown: email.to_owned(),
            pair: (!name.is_empty() && !name.contains('@')).then(|| name.to_owned()),
            conversation_id,
            role: ROLE_RECIPIENT,
            at,
        },
    )
}

// Joining aliases into people.

struct AliasRow {
    value: String,
    kind: Kind,
    shown: String,
    pair: Option<String>,
    person_id: Option<i64>,
    by_hand: bool,
    last_seen_at: i64,
}

fn aliases(store: &Store) -> Result<Vec<AliasRow>> {
    let mut statement = store.conn().prepare(
        "SELECT value, kind, shown, pair, person_id, by_hand, last_seen_at FROM person_alias",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AliasRow {
            value: row.get(0)?,
            kind: Kind::parse(&row.get::<_, String>(1)?),
            shown: row.get(2)?,
            pair: row.get(3)?,
            person_id: row.get(4)?,
            by_hand: row.get(5)?,
            last_seen_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl AliasRow {
    fn identity(&self) -> Option<String> {
        identity(self.kind, &self.value, self.pair.as_deref())
    }
}

/// A full name, or a phone number: something that is one person by itself.
fn is_anchor(identity: &str) -> bool {
    identity.starts_with('+') || identity.contains(' ')
}

fn first_word(identity: &str) -> &str {
    identity.split(' ').next().unwrap_or_default()
}

/// Joins aliases into people, keeping each person's ID while their names
/// stay the same, and names each person. Returns how many aliases moved.
fn assign(store: &Store, now_ms: i64) -> Result<usize> {
    let rows = aliases(store)?;
    let hidden: HashSet<i64> = store
        .conn()
        .prepare("SELECT id FROM person WHERE hidden")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;

    // What the person joined by hand: those names belong to that person.
    let mut by_hand: HashMap<String, i64> = HashMap::new();
    for row in rows.iter().filter(|row| row.by_hand) {
        if let (Some(identity), Some(person)) = (row.identity(), row.person_id) {
            by_hand.entry(identity).or_insert(person);
        }
    }
    // The rest, grouped by the name each stands for.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.by_hand
            && let Some(identity) = row.identity()
        {
            groups.entry(identity).or_default().push(index);
        }
    }

    let mut target: HashMap<usize, Option<i64>> = HashMap::new();
    let mut used: HashSet<i64> = HashSet::new();
    let mut created = 0;
    let mut create = |name: &str| -> Result<i64> {
        store.conn().execute(
            "INSERT INTO person (name, created_at) VALUES (?1, ?2)",
            params![title_case(name), now_ms],
        )?;
        created += 1;
        Ok(store.conn().last_insert_rowid())
    };
    // A person's ID stays with their name: the lowest one their aliases
    // already have that no other name took.
    let keep = |members: &[usize], used: &HashSet<i64>| -> Option<i64> {
        members
            .iter()
            .filter_map(|&index| rows[index].person_id)
            .filter(|id| !used.contains(id))
            .min()
    };

    // Full names and numbers first: each is one person.
    let mut first_names: HashMap<String, BTreeSet<i64>> = HashMap::new();
    for (identity, person) in &by_hand {
        if is_anchor(identity) && !hidden.contains(person) {
            first_names
                .entry(first_word(identity).to_owned())
                .or_default()
                .insert(*person);
        }
    }
    for (identity, members) in groups.iter().filter(|(identity, _)| is_anchor(identity)) {
        let person = match by_hand.get(identity) {
            Some(person) => *person,
            None => match keep(members, &used) {
                Some(person) => person,
                None => create(identity)?,
            },
        };
        used.insert(person);
        if !identity.starts_with('+') && !hidden.contains(&person) {
            first_names
                .entry(first_word(identity).to_owned())
                .or_default()
                .insert(person);
        }
        for &index in members {
            target.insert(index, Some(person));
        }
    }
    // A first name alone joins the one person who has it; with none, it is
    // a person of its own; with two or more, it waits for the person.
    for (identity, members) in groups.iter().filter(|(identity, _)| !is_anchor(identity)) {
        let person = if let Some(person) = by_hand.get(identity) {
            Some(*person)
        } else {
            match first_names.get(identity).map(BTreeSet::len) {
                Some(1) => first_names[identity].iter().next().copied(),
                Some(_) => None,
                None => Some(match keep(members, &used) {
                    Some(person) => person,
                    None => create(identity)?,
                }),
            }
        };
        if let Some(person) = person {
            used.insert(person);
        }
        for &index in members {
            target.insert(index, person);
        }
    }

    // Move what changed. A person left with no names hands their note on.
    let mut moved = 0;
    let mut emptied: HashMap<i64, Option<i64>> = HashMap::new();
    for (index, person) in &target {
        let row = &rows[*index];
        if row.person_id == *person {
            continue;
        }
        store.conn().execute(
            "UPDATE person_alias SET person_id = ?2 WHERE value = ?1",
            params![row.value, person],
        )?;
        if let Some(old) = row.person_id {
            emptied.insert(old, *person);
        }
        moved += 1;
    }
    for (old, new) in emptied {
        let left: i64 = store.conn().query_row(
            "SELECT count(*) FROM person_alias WHERE person_id = ?1",
            [old],
            |row| row.get(0),
        )?;
        if left > 0 {
            continue;
        }
        let note: Option<String> = store
            .conn()
            .query_row("SELECT note FROM person WHERE id = ?1", [old], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        match (note, new) {
            (Some(note), Some(new)) => {
                append_note(store, new, &note)?;
                store
                    .conn()
                    .execute("DELETE FROM person WHERE id = ?1", [old])?;
            }
            // Kept for its note, though nothing points at it now.
            (Some(_), None) => {}
            (None, _) => {
                store
                    .conn()
                    .execute("DELETE FROM person WHERE id = ?1", [old])?;
            }
        }
    }
    // People created and left without a name (none, normally).
    store.conn().execute(
        "DELETE FROM person WHERE note IS NULL
           AND NOT EXISTS (SELECT 1 FROM person_alias a WHERE a.person_id = person.id)",
        [],
    )?;
    name_people(store)?;
    Ok(moved + created)
}

/// Each person's name: their fullest name as written, the most recent
/// first; else what their address spells, or their number.
fn name_people(store: &Store) -> Result<()> {
    let mut best: HashMap<i64, (u8, usize, i64, String)> = HashMap::new();
    for row in aliases(store)? {
        let Some(person) = row.person_id else {
            continue;
        };
        let (rank, name) = match row.kind {
            Kind::Name => (2, row.shown.clone()),
            Kind::Email => (1, title_case(&row.identity().unwrap_or_default())),
            Kind::Phone => (0, row.shown.clone()),
        };
        let words = name.split_whitespace().count();
        let candidate = (rank, words, row.last_seen_at, name);
        let better = best.get(&person).is_none_or(|current| {
            (candidate.0, candidate.1, candidate.2) > (current.0, current.1, current.2)
        });
        if better {
            best.insert(person, candidate);
        }
    }
    for (person, (_, _, _, name)) in best {
        store.conn().execute(
            "UPDATE person SET name = ?2 WHERE id = ?1 AND name != ?2",
            params![person, name],
        )?;
    }
    Ok(())
}

fn append_note(store: &Store, person: i64, note: &str) -> Result<()> {
    store.conn().execute(
        "UPDATE person SET note = CASE WHEN note IS NULL OR note = '' THEN ?2
                                       ELSE note || char(10) || ?2 END
         WHERE id = ?1",
        params![person, note],
    )?;
    Ok(())
}

// The person's corrections.

/// "Same person": everything known as `from` is `into` from now on.
pub fn merge(store: &Store, from: i64, into: i64) -> Result<()> {
    if from == into {
        bail!("that is the same page");
    }
    exists(store, into)?;
    let note: Option<String> = store
        .conn()
        .query_row("SELECT note FROM person WHERE id = ?1", [from], |row| {
            row.get(0)
        })
        .optional()?
        .context("that person is not in the memory any more")?;
    let tx = store.transaction()?;
    store.conn().execute(
        "UPDATE person_alias SET person_id = ?2, by_hand = 1 WHERE person_id = ?1",
        params![from, into],
    )?;
    if let Some(note) = note.filter(|note| !note.trim().is_empty()) {
        append_note(store, into, &note)?;
    }
    store
        .conn()
        .execute("DELETE FROM person WHERE id = ?1", [from])?;
    name_people(store)?;
    tx.commit()?;
    Ok(())
}

/// "Not the same person": the alias becomes a person of its own. Returns
/// the new person's ID.
pub fn split(store: &Store, alias: &str, now_ms: i64) -> Result<i64> {
    let shown: String = store
        .conn()
        .query_row(
            "SELECT shown FROM person_alias WHERE value = ?1",
            [alias],
            |row| row.get(0),
        )
        .optional()?
        .context("that name is not in the memory any more")?;
    let tx = store.transaction()?;
    store.conn().execute(
        "INSERT INTO person (name, created_at) VALUES (?1, ?2)",
        params![shown, now_ms],
    )?;
    let id = store.conn().last_insert_rowid();
    store.conn().execute(
        "UPDATE person_alias SET person_id = ?2, by_hand = 1 WHERE value = ?1",
        params![alias, id],
    )?;
    tx.commit()?;
    Ok(id)
}

/// "Same person" on a name that was undecided: it is this person's.
pub fn claim(store: &Store, alias: &str, person: i64) -> Result<()> {
    exists(store, person)?;
    let changed = store.conn().execute(
        "UPDATE person_alias SET person_id = ?2, by_hand = 1 WHERE value = ?1",
        params![alias, person],
    )?;
    if changed == 0 {
        bail!("that name is not in the memory any more");
    }
    Ok(())
}

/// "Not a person" (a bot, a shared inbox): left out of lists, pages and
/// questions. `false` brings them back.
pub fn hide(store: &Store, person: i64, hidden: bool) -> Result<()> {
    exists(store, person)?;
    store.conn().execute(
        "UPDATE person SET hidden = ?2 WHERE id = ?1",
        params![person, hidden],
    )?;
    Ok(())
}

/// The person's own note about someone ("website client, reviews V1 and
/// V2"); empty clears it.
pub fn set_note(store: &Store, person: i64, note: &str) -> Result<()> {
    exists(store, person)?;
    let note = truncate(note.trim(), 500);
    store.conn().execute(
        "UPDATE person SET note = ?2 WHERE id = ?1",
        params![person, (!note.is_empty()).then_some(note)],
    )?;
    Ok(())
}

fn exists(store: &Store, person: i64) -> Result<()> {
    store
        .conn()
        .query_row("SELECT 1 FROM person WHERE id = ?1", [person], |_| Ok(()))
        .optional()?
        .context("that person is not in the memory any more")
}

// Looking people up.

/// A person on a to-do or in a list: what to show and what to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chip {
    pub id: i64,
    pub name: String,
}

/// The people and their names, loaded once to look up many names.
pub struct Directory {
    names: HashMap<i64, String>,
    alias_person: HashMap<String, Option<i64>>,
    /// A first name and the people who have it (of full names only).
    first_names: HashMap<String, BTreeSet<i64>>,
    /// Each person's full names, for finding them in text.
    full_names: Vec<(i64, String)>,
    emails: Vec<(i64, String)>,
    /// Conversations each person was seen in, and how.
    seen: HashMap<i64, Vec<(i64, String)>>,
}

impl Directory {
    pub fn load(store: &Store) -> Result<Self> {
        let names: HashMap<i64, String> = store
            .conn()
            .prepare("SELECT id, name FROM person WHERE NOT hidden")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut directory = Self {
            names,
            alias_person: HashMap::new(),
            first_names: HashMap::new(),
            full_names: Vec::new(),
            emails: Vec::new(),
            seen: HashMap::new(),
        };
        for row in aliases(store)? {
            let person = row.person_id.filter(|id| directory.names.contains_key(id));
            if row.person_id.is_some() && person.is_none() {
                // A hidden person's names find no one.
                directory.alias_person.insert(row.value, None);
                continue;
            }
            if let Some(person) = person {
                let full_name = row
                    .identity()
                    .filter(|identity| is_anchor(identity) && !identity.starts_with('+'));
                if let Some(identity) = full_name {
                    directory
                        .first_names
                        .entry(first_word(&identity).to_owned())
                        .or_default()
                        .insert(person);
                    directory.full_names.push((person, identity));
                }
                if row.kind == Kind::Email {
                    directory.emails.push((person, row.value.clone()));
                }
            }
            directory.alias_person.insert(row.value, person);
        }
        let mut statement = store.conn().prepare(
            "SELECT a.person_id, s.conversation_id, s.role FROM person_seen s
             JOIN person_alias a ON a.value = s.alias WHERE a.person_id IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (person, conversation, role) = row?;
            if directory.names.contains_key(&person) {
                directory
                    .seen
                    .entry(person)
                    .or_default()
                    .push((conversation, role));
            }
        }
        Ok(directory)
    }

    pub fn name(&self, person: i64) -> Option<&str> {
        self.names.get(&person).map(String::as_str)
    }

    pub fn chip(&self, person: i64) -> Option<Chip> {
        self.name(person).map(|name| Chip {
            id: person,
            name: name.to_owned(),
        })
    }

    /// The person a written name is: an alias of theirs, or a first name
    /// only one person has. A list ("Ben and Raj") is its first name that
    /// is someone.
    pub fn resolve(&self, written: &str) -> Option<i64> {
        std::iter::once(written.to_owned())
            .chain(each_name(written))
            .find_map(|part| self.resolve_one(&part))
    }

    fn resolve_one(&self, written: &str) -> Option<i64> {
        let (_, value) = alias_of(written)?;
        if let Some(found) = self.alias_person.get(&value) {
            return *found;
        }
        let first = first_word(&value);
        match self.first_names.get(first) {
            Some(people) if people.len() == 1 && !value.contains(' ') => {
                people.iter().next().copied()
            }
            _ => None,
        }
    }

    /// The people `text` names, in the order named: a full name, an
    /// address, or a first name only one person has (not one that is also
    /// a common word).
    pub fn mentioned(&self, text: &str) -> Vec<i64> {
        let normalized = format!(" {} ", normalize_name(text));
        let lower = text.to_lowercase();
        let mut found: Vec<(usize, i64)> = Vec::new();
        for (person, name) in &self.full_names {
            if let Some(at) = normalized.find(&format!(" {name} ")) {
                found.push((at, *person));
            }
        }
        for (person, email) in &self.emails {
            if let Some(at) = lower.find(email.as_str()) {
                found.push((at, *person));
            }
        }
        for (first, people) in &self.first_names {
            if people.len() != 1 || WORD_NAMES.contains(&first.as_str()) {
                continue;
            }
            if let Some(at) = normalized.find(&format!(" {first} ")) {
                found.push((at, *people.iter().next().unwrap_or(&0)));
            }
        }
        // A person known by one name only ("Ben", from a WhatsApp chat).
        for (value, person) in &self.alias_person {
            if let Some(person) = person
                && !value.contains(' ')
                && !value.contains('@')
                && !value.starts_with('+')
                && !WORD_NAMES.contains(&value.as_str())
                && !self.first_names.contains_key(value)
                && let Some(at) = normalized.find(&format!(" {value} "))
            {
                found.push((at, *person));
            }
        }
        found.sort_unstable();
        let mut people = Vec::new();
        for (_, person) in found {
            if !people.contains(&person) {
                people.push(person);
            }
        }
        people
    }

    /// The person a conversation is with: a WhatsApp chat named after them,
    /// or an email thread only they wrote in.
    pub fn with(&self, conversation: i64) -> Option<i64> {
        let mut chat = None;
        let mut senders = BTreeSet::new();
        for (person, places) in &self.seen {
            for (id, role) in places {
                if *id != conversation {
                    continue;
                }
                match role.as_str() {
                    ROLE_CHAT => chat = Some(*person),
                    ROLE_SENDER => {
                        senders.insert(*person);
                    }
                    _ => {}
                }
            }
        }
        chat.or_else(|| {
            (senders.len() == 1)
                .then(|| senders.into_iter().next())
                .flatten()
        })
    }

    /// The people a meeting had besides the person: its attendees and the
    /// owners of its action items.
    pub fn attendees(&self, conversation: i64) -> Vec<i64> {
        let mut people: Vec<i64> = self
            .seen
            .iter()
            .filter(|(_, places)| {
                places.iter().any(|(id, role)| {
                    *id == conversation && (role == ROLE_ATTENDEE || role == ROLE_OWNER)
                })
            })
            .map(|(person, _)| *person)
            .collect();
        people.sort_unstable();
        people
    }

    fn conversations(&self, person: i64) -> Vec<(i64, String)> {
        self.seen.get(&person).cloned().unwrap_or_default()
    }

    fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.names.keys().copied()
    }
}

// Pages.

/// A conversation waiting on the person: the other side wrote last.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub place: String,
    pub when: String,
    pub text: String,
    #[serde(skip)]
    pub at: i64,
}

/// Somewhere the person and they were together lately.
#[derive(Debug, Clone, Serialize)]
pub struct Recent {
    pub what: String,
    pub when: String,
    #[serde(skip)]
    at: i64,
}

/// A name that may be theirs: a first name two people have.
#[derive(Debug, Clone, Serialize)]
pub struct Maybe {
    pub alias: String,
    pub shown: String,
    pub place: String,
}

/// A name or address of theirs, which the person can split off.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alias {
    pub alias: String,
    pub shown: String,
    pub by_hand: bool,
}

/// One person's page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: i64,
    pub name: String,
    /// Where they work, from an address's domain ("unifize.com").
    pub org: Option<String>,
    /// From the conversations they were seen in, when the person said.
    pub work: Option<bool>,
    pub note: Option<String>,
    /// Marked "not a person" by the person.
    pub hidden: bool,
    pub aliases: Vec<Alias>,
    /// "WhatsApp chat", "2 email threads", "6 meetings".
    pub places: Vec<String>,
    /// "Wrote last on WhatsApp, Tue 13 Oct 14:02".
    pub last: Option<String>,
    pub waiting: Vec<Waiting>,
    /// What the person owes them, and what they owe the person.
    pub owe: Vec<Item>,
    pub owed: Vec<Item>,
    pub recent: Vec<Recent>,
    pub maybe: Vec<Maybe>,
    /// Others to say they are the same person as (for "Same person as…").
    pub others: Vec<Chip>,
    pub question: String,
}

/// What Brief me asks. It names no one: the page goes with it inside the
/// memory, as data (`focus`).
pub const BRIEF_QUESTION: &str = "Brief me on the person under \u{201c}Person\u{201d} at the top \
of the memory: who they are to me, where things stand between us, what is open on either side, \
and what I should raise with them next.";

/// Someone in the People tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: i64,
    pub name: String,
    /// "You owe 2 · waiting on you", or when they were last seen.
    pub detail: String,
    /// Something waits on the person: a reply, or something they owe.
    pub needs_you: bool,
}

/// The last message of a conversation the person has not answered.
struct LastWord {
    conversation: i64,
    place: String,
    sender: String,
    text: String,
    at: i64,
}

/// Conversations where the other side wrote last, lately: WhatsApp chats
/// and email threads, by who wrote.
fn waiting(store: &Store, since_ms: i64) -> Result<Vec<LastWord>> {
    let mut statement = store.conn().prepare(
        "SELECT c.id, c.app, c.title, m.sender, m.text, coalesce(m.sent_at, m.first_seen_at) AS at
         FROM conversation c
         JOIN message m ON m.id = (
             SELECT id FROM message WHERE conversation_id = c.id
             ORDER BY coalesce(sent_at, first_seen_at) DESC, id DESC LIMIT 1)
         WHERE c.app IN ('whatsapp', 'gmail') AND coalesce(m.sent_at, m.first_seen_at) >= ?1
           AND m.sender != ?2",
    )?;
    let rows = statement.query_map(params![since_ms, ME], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut found = Vec::new();
    for row in rows {
        let (id, app, title, sender, text, at) = row?;
        let place = match app.as_str() {
            "whatsapp" => format!("WhatsApp chat \u{201c}{}\u{201d}", one_line(&title, 60)),
            _ => format!("Email \u{201c}{}\u{201d}", one_line(&title, 60)),
        };
        found.push(LastWord {
            conversation: id,
            place,
            sender,
            text: one_line(&redact(&text), 160),
            at,
        });
    }
    Ok(found)
}

/// What waits on the person from `person`: their chat, or a thread only
/// they wrote in, where they wrote last.
fn waiting_for(directory: &Directory, person: i64, all: &[LastWord]) -> Vec<Waiting> {
    let mut found: Vec<Waiting> = all
        .iter()
        .filter(|last| {
            directory.with(last.conversation) == Some(person)
                && directory
                    .resolve(&last.sender)
                    .is_none_or(|who| who == person)
        })
        .map(|last| Waiting {
            place: last.place.clone(),
            when: time::moment(last.at),
            text: last.text.clone(),
            at: last.at,
        })
        .collect();
    found.sort_by_key(|waiting| -waiting.at);
    found
}

/// One person's page, as of `now`.
pub fn page(store: &Store, person: i64, now: DateTime<Local>) -> Result<Page> {
    let directory = Directory::load(store)?;
    let list = todos::items(store, &directory, now)?;
    page_with(store, &directory, &list, person, now)
}

fn page_with(
    store: &Store,
    directory: &Directory,
    list: &todos::Items,
    person: i64,
    now: DateTime<Local>,
) -> Result<Page> {
    let (name, note, hidden): (String, Option<String>, bool) = store
        .conn()
        .query_row(
            "SELECT name, note, hidden FROM person WHERE id = ?1",
            [person],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .context("that person is not in the memory any more")?;
    let rows: Vec<AliasRow> = aliases(store)?
        .into_iter()
        .filter(|row| row.person_id == Some(person))
        .collect();
    let org = rows
        .iter()
        .filter(|row| row.kind == Kind::Email)
        .filter_map(|row| row.value.split('@').nth(1))
        .find(|domain| !PUBLIC_MAIL.contains(domain))
        .map(str::to_owned);
    let mut aliases_shown: Vec<Alias> = rows
        .iter()
        .map(|row| Alias {
            alias: row.value.clone(),
            shown: row.shown.clone(),
            by_hand: row.by_hand,
        })
        .collect();
    aliases_shown.sort_by(|a, b| a.shown.cmp(&b.shown));

    // Where they were seen.
    let places = directory.conversations(person);
    let ids: BTreeSet<i64> = places.iter().map(|(id, _)| *id).collect();
    let mut recent = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut work = None;
    for id in &ids {
        let found: Option<(String, String, Option<bool>, i64)> = store
            .conn()
            .query_row(
                "SELECT c.app, c.title, c.work,
                        coalesce(max(coalesce(m.sent_at, m.first_seen_at)), c.last_seen_at)
                 FROM conversation c LEFT JOIN message m ON m.conversation_id = c.id
                 WHERE c.id = ?1 GROUP BY c.id",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((app, title, is_work, at)) = found else {
            continue;
        };
        work = match (work, is_work) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (_, Some(false)) => Some(false),
            (known, None) => known,
        };
        let title = one_line(&redact(&title), 80);
        let (kind, what) = match app.as_str() {
            "whatsapp" if directory.with(*id) == Some(person) => (
                "WhatsApp chat",
                format!("WhatsApp chat \u{201c}{title}\u{201d}"),
            ),
            "whatsapp" => (
                "WhatsApp group",
                format!("WhatsApp group \u{201c}{title}\u{201d}"),
            ),
            "gmail" => ("email thread", format!("Email \u{201c}{title}\u{201d}")),
            "fireflies" => ("meeting", format!("Meeting \u{201c}{title}\u{201d}")),
            _ => ("conversation", title.clone()),
        };
        *counts.entry(kind).or_default() += 1;
        recent.push(Recent {
            what,
            when: time::local(at)
                .map(|at| time::short_day_label(at.date_naive()))
                .unwrap_or_default(),
            at,
        });
    }
    recent.sort_by_key(|recent| -recent.at);
    recent.truncate(6);
    let places_shown: Vec<String> = counts
        .iter()
        .map(|(kind, count)| match count {
            1 => format!("1 {kind}"),
            n => format!("{n} {kind}s"),
        })
        .collect();

    // When they last wrote, anywhere.
    let last = last_heard(store, directory, person)?;

    let since = now.timestamp_millis() - WAITING_DAYS * 86_400_000;
    let waiting = waiting_for(directory, person, &waiting(store, since)?);

    // Undecided names that may be theirs: their first name, alone.
    let firsts: HashSet<String> = rows
        .iter()
        .filter_map(AliasRow::identity)
        .filter(|identity| is_anchor(identity) && !identity.starts_with('+'))
        .map(|identity| first_word(&identity).to_owned())
        .collect();
    let mut maybe = Vec::new();
    for row in aliases(store)?
        .into_iter()
        .filter(|row| row.person_id.is_none())
    {
        if !row
            .identity()
            .is_some_and(|identity| firsts.contains(&identity))
        {
            continue;
        }
        let place: Option<(String, String)> = store
            .conn()
            .query_row(
                "SELECT c.app, c.title FROM person_seen s JOIN conversation c ON c.id = s.conversation_id
                 WHERE s.alias = ?1 ORDER BY s.last_at DESC LIMIT 1",
                [&row.value],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        maybe.push(Maybe {
            alias: row.value.clone(),
            shown: row.shown.clone(),
            place: place
                .map(|(app, title)| match app.as_str() {
                    "whatsapp" => format!("WhatsApp \u{201c}{}\u{201d}", one_line(&title, 40)),
                    "gmail" => "email".to_owned(),
                    "fireflies" => format!("meeting \u{201c}{}\u{201d}", one_line(&title, 40)),
                    other => other.to_owned(),
                })
                .unwrap_or_default(),
        });
    }

    let mut others: Vec<Chip> = directory
        .ids()
        .filter(|id| *id != person)
        .filter_map(|id| directory.chip(id))
        .collect();
    others.sort_by_key(|chip| chip.name.to_lowercase());

    Ok(Page {
        id: person,
        name,
        org,
        work,
        note,
        hidden,
        aliases: aliases_shown,
        places: places_shown,
        last,
        waiting,
        owe: list.mine_for(person),
        owed: list.theirs_for(person),
        recent,
        maybe,
        others,
        question: BRIEF_QUESTION.to_owned(),
    })
}

/// "Wrote last on WhatsApp, Tue 13 Oct 14:02": their newest message
/// anywhere, by any of their names.
fn last_heard(store: &Store, directory: &Directory, person: i64) -> Result<Option<String>> {
    let mut statement = store.conn().prepare(
        "SELECT c.app, m.sender, max(coalesce(m.sent_at, m.first_seen_at)) AS at
         FROM message m JOIN conversation c ON c.id = m.conversation_id
         WHERE c.app IN ('whatsapp', 'gmail') AND m.sender != ?1
         GROUP BY c.app, m.sender ORDER BY at DESC",
    )?;
    let rows = statement.query_map([ME], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (app, sender, at) = row?;
        if directory.resolve(&sender) == Some(person) {
            let app = if app == "whatsapp" {
                "WhatsApp"
            } else {
                "email"
            };
            return Ok(Some(format!("Wrote last on {app}, {}", time::moment(at))));
        }
    }
    Ok(None)
}

/// The People tab: those with something open or waiting first, then the
/// most recently seen.
pub fn list(
    store: &Store,
    directory: &Directory,
    items: &todos::Items,
    now: DateTime<Local>,
) -> Result<Vec<Summary>> {
    let since = now.timestamp_millis() - WAITING_DAYS * 86_400_000;
    let waiting_all = waiting(store, since)?;
    let last_seen: HashMap<i64, i64> = store
        .conn()
        .prepare(
            "SELECT person_id, max(last_seen_at) FROM person_alias
             WHERE person_id IS NOT NULL GROUP BY person_id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut people: Vec<(bool, i64, Summary)> = Vec::new();
    for id in directory.ids() {
        let Some(name) = directory.name(id) else {
            continue;
        };
        let owe = items.mine_for(id).len();
        let owed = items.theirs_for(id).len();
        let waits = waiting_for(directory, id, &waiting_all).len();
        let at = last_seen.get(&id).copied().unwrap_or_default();
        let mut parts = Vec::new();
        if owe > 0 {
            parts.push(format!("You owe {owe}"));
        }
        if owed > 0 {
            parts.push(format!("Owes you {owed}"));
        }
        if waits > 0 {
            parts.push("Waiting on you".to_owned());
        }
        if parts.is_empty() && at > 0 {
            parts.push(format!(
                "Seen {}",
                time::local(at)
                    .map(|at| time::short_day_label(at.date_naive()))
                    .unwrap_or_default()
            ));
        }
        let needs_you = owe > 0 || waits > 0;
        people.push((
            needs_you || owed > 0,
            at,
            Summary {
                id,
                name: name.to_owned(),
                detail: parts.join(" \u{b7} "),
                needs_you,
            },
        ));
    }
    people.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    Ok(people
        .into_iter()
        .take(MAX_LISTED)
        .map(|(_, _, summary)| summary)
        .collect())
}

// For Claude.

/// The people a question is about: those it names, then those the window
/// in front names (a chat or an email with them). At most two.
pub fn for_question(store: &Store, question: &str, window_title: Option<&str>) -> Result<Vec<i64>> {
    let directory = Directory::load(store)?;
    let mut people = directory.mentioned(question);
    if let Some(title) = window_title {
        for person in directory.mentioned(title) {
            if !people.contains(&person) {
                people.push(person);
            }
        }
    }
    people.truncate(MAX_FOR_QUESTION);
    Ok(people)
}

/// A person's page as text for the memory: who they are, what is open
/// between them and the person, what waits on a reply, where they met,
/// and their latest messages. Redacted.
pub fn memory(store: &Store, person: i64, now: DateTime<Local>) -> Result<String> {
    let directory = Directory::load(store)?;
    let list = todos::items(store, &directory, now)?;
    let page = page_with(store, &directory, &list, person, now)?;
    let mut text = format!("### {}\n", page.name);
    let others: Vec<&str> = page
        .aliases
        .iter()
        .map(|alias| alias.shown.as_str())
        .filter(|shown| *shown != page.name)
        .collect();
    if !others.is_empty() {
        text.push_str(&format!("- Also seen as: {}\n", others.join("; ")));
    }
    if let Some(org) = &page.org {
        text.push_str(&format!("- Works at: {org} (from their email address)\n"));
    }
    match page.work {
        Some(true) => text.push_str("- A work contact\n"),
        Some(false) => text.push_str("- A personal contact\n"),
        None => {}
    }
    if let Some(note) = &page.note {
        text.push_str(&format!(
            "- The person's own note about them: {}\n",
            one_line(note, 300)
        ));
    }
    if !page.places.is_empty() {
        text.push_str(&format!("- Seen in: {}\n", page.places.join(", ")));
    }
    if let Some(last) = &page.last {
        text.push_str(&format!("- {last}\n"));
    }
    for waiting in &page.waiting {
        text.push_str(&format!(
            "- Waiting on the person: {} (they wrote last, {}): \u{201c}{}\u{201d}\n",
            waiting.place, waiting.when, waiting.text
        ));
    }
    for item in &page.owe {
        text.push_str(&format!("- The person owes them: {}\n", item.describe()));
    }
    for item in &page.owed {
        text.push_str(&format!("- They owe the person: {}\n", item.describe()));
    }
    if !page.recent.is_empty() {
        let recent: Vec<String> = page
            .recent
            .iter()
            .map(|recent| format!("{} ({})", recent.what, recent.when))
            .collect();
        text.push_str(&format!("- Lately: {}\n", recent.join("; ")));
    }
    let messages = latest_messages(store, &directory, person)?;
    if !messages.is_empty() {
        text.push_str("- Their latest messages with the person (oldest first):\n");
        for line in messages {
            text.push_str(&format!("  - {line}\n"));
        }
    }
    Ok(truncate(&redact(text.trim_end()), MEMORY_CHARS))
}

/// The last messages between the person and them: in their chat, theirs in
/// email threads, and the person's emails that went to them, oldest first.
fn latest_messages(store: &Store, directory: &Directory, person: i64) -> Result<Vec<String>> {
    let places: Vec<i64> = directory
        .conversations(person)
        .into_iter()
        .filter(|(_, role)| role == ROLE_CHAT || role == ROLE_SENDER || role == ROLE_RECIPIENT)
        .map(|(id, _)| id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut found: Vec<(i64, String)> = Vec::new();
    for id in places {
        let with_them = directory.with(id) == Some(person);
        let mut statement = store.conn().prepare(
            "SELECT m.sender, m.text, coalesce(m.sent_at, m.first_seen_at) AS at, c.app,
                    m.recipients
             FROM message m JOIN conversation c ON c.id = m.conversation_id
             WHERE m.conversation_id = ?1 ORDER BY at DESC, m.id DESC LIMIT 12",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        for row in rows {
            let (sender, text, at, app, recipients) = row?;
            let theirs = directory.resolve(&sender) == Some(person);
            let to_them = sender == ME
                && recipients.as_deref().is_some_and(|names| {
                    names
                        .split(", ")
                        .any(|name| directory.resolve(name) == Some(person))
                });
            if !(theirs || to_them || (with_them && sender == ME)) {
                continue;
            }
            let who = if sender == ME { "the person" } else { "them" };
            let place = if app == "whatsapp" {
                "WhatsApp"
            } else {
                "email"
            };
            found.push((
                at,
                format!(
                    "{who} on {place}, {}: {}",
                    time::moment(at),
                    one_line(&text, 200)
                ),
            ));
        }
    }
    found.sort_by_key(|(at, _)| -*at);
    found.truncate(8);
    found.reverse();
    Ok(found.into_iter().map(|(_, line)| line).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{self, Conversation, NewMessage};

    const ME_NAME: &str = "Asha Verma";

    fn chat(store: &Store, app: &str, key: &str, title: &str, lines: &[(&str, &str)]) -> i64 {
        let messages: Vec<NewMessage> = lines
            .iter()
            .enumerate()
            .map(|(i, (sender, text))| NewMessage {
                sender,
                text,
                sent_label: None,
                sent_at: Some(1_000 + i as i64),
            })
            .collect();
        conversations::record(
            store,
            &Conversation {
                app,
                key,
                title,
                work: Some(true),
            },
            &messages,
            2_000,
        )
        .unwrap()
        .0
    }

    fn person_of(store: &Store, alias: &str) -> Option<i64> {
        store
            .conn()
            .query_row(
                "SELECT person_id FROM person_alias WHERE value = ?1",
                [alias],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn name(store: &Store, id: i64) -> String {
        store
            .conn()
            .query_row("SELECT name FROM person WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn normalizes_names_and_reads_addresses() {
        assert_eq!(normalize_name("~ Ben M. (Unifize) 🚀"), "ben m unifize");
        assert_eq!(normalize_name("O'Brien-Smith"), "o'brien-smith");
        assert_eq!(
            alias_of("Ben.Merton@Unifize.com"),
            Some((Kind::Email, "ben.merton@unifize.com".to_owned()))
        );
        assert_eq!(
            alias_of("+91 98765 43210"),
            Some((Kind::Phone, "+919876543210".to_owned()))
        );
        assert_eq!(alias_of("  "), None);
        assert_eq!(name_from_email("ben.merton+news@unifize.com"), "ben merton");
        assert_eq!(
            attendees(
                "Website review, 45 min, with Ben Merton, raj@unifize.com and 2 others.\nSummary"
            ),
            ["Ben Merton", "raj@unifize.com"]
        );
        assert_eq!(attendees("Standup, 10 min.\nNo one"), Vec::<String>::new());
        assert!(not_a_person(Kind::Name, "fireflies ai notetaker"));
        assert!(not_a_person(Kind::Name, "all managers and meeting owners"));
        assert!(not_a_person(
            Kind::Name,
            "design system and prototype teams"
        ));
        assert_eq!(
            each_name("Arun Govil and Smriti Sharma"),
            ["Arun Govil", "Smriti Sharma"]
        );
        assert_eq!(
            each_name("Shilpa V / Shilpashree V"),
            ["Shilpa V", "Shilpashree V"]
        );
        assert_eq!(each_name("Ben"), ["Ben"]);
        assert!(not_a_person(Kind::Email, "noreply@github.com"));
        assert!(not_a_person(Kind::Email, "hello@vercel.com"));
        assert!(!not_a_person(Kind::Name, "ben merton"));
        let me = Me::new(Some(ME_NAME));
        assert!(me.is("asha verma") && me.is("asha") && !me.is("asha patel") && !me.is("ben"));
    }

    #[test]
    fn joins_one_person_across_apps() {
        let store = Store::open_in_memory().unwrap();
        let whatsapp = chat(
            &store,
            "whatsapp",
            "Ben Merton",
            "Ben Merton",
            &[("Ben Merton", "can you send it?"), (ME, "by Thursday")],
        );
        let thread = chat(
            &store,
            "gmail",
            "t1",
            "Pricing",
            &[("Ben Merton", "See attached")],
        );
        let meeting = chat(
            &store,
            "fireflies",
            "m1",
            "Website review",
            &[
                (
                    "summary",
                    "Website review, 45 min, with Ben Merton, raj.kumar@unifize.com, Asha Verma.\nWe talked.",
                ),
                ("Raj", "Publish the blog"),
                (ME, "Share the guide"),
            ],
        );
        let group = chat(
            &store,
            "whatsapp",
            "Launch crew",
            "Launch crew",
            &[("Ben Merton", "morning"), ("Raj Kumar", "hi")],
        );
        sync_as(&store, Some(ME_NAME), 3_000).unwrap();
        note_email_as(
            &store,
            thread,
            "Ben Merton",
            "ben.merton@unifize.com",
            1_500,
        );
        sync_as(&store, Some(ME_NAME), 3_000).unwrap();

        let ben = person_of(&store, "ben merton").unwrap();
        assert_eq!(person_of(&store, "ben.merton@unifize.com"), Some(ben));
        let raj = person_of(&store, "raj kumar").unwrap();
        // The address spells his name; "Raj" alone is the one Raj.
        assert_eq!(person_of(&store, "raj.kumar@unifize.com"), Some(raj));
        assert_eq!(person_of(&store, "raj"), Some(raj));
        assert_ne!(ben, raj);
        assert_eq!(name(&store, ben), "Ben Merton");
        // Never the person, never a group, never a summary.
        for missing in ["asha verma", "launch crew", "summary", "me"] {
            let found: Option<String> = store
                .conn()
                .query_row(
                    "SELECT value FROM person_alias WHERE value = ?1",
                    [missing],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            assert_eq!(found, None, "{missing}");
        }

        let directory = Directory::load(&store).unwrap();
        assert_eq!(directory.with(whatsapp), Some(ben));
        assert_eq!(directory.with(thread), Some(ben));
        assert_eq!(directory.with(group), None);
        assert_eq!(directory.attendees(meeting), {
            let mut both = vec![ben, raj];
            both.sort_unstable();
            both
        });
        assert_eq!(directory.resolve("Ben"), Some(ben));
        assert_eq!(directory.resolve("Ben Merton and Raj"), Some(ben));
        assert_eq!(
            directory.mentioned("what's open with raj and ben?"),
            [raj, ben]
        );
        assert_eq!(directory.mentioned("will it ship?"), Vec::<i64>::new());

        // Nothing moves on a second pass.
        assert_eq!(sync_as(&store, Some(ME_NAME), 4_000).unwrap(), 0);
        assert_eq!(person_of(&store, "ben merton"), Some(ben));
    }

    fn note_email_as(store: &Store, conversation: i64, name: &str, email: &str, at: i64) {
        record(
            store,
            &Me::new(Some(ME_NAME)),
            &Sighting {
                shown: email.to_owned(),
                pair: Some(name.to_owned()),
                conversation_id: conversation,
                role: ROLE_SENDER,
                at,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_shared_first_name_waits_and_corrections_stay() {
        let store = Store::open_in_memory().unwrap();
        chat(&store, "whatsapp", "Ben", "Ben", &[("Ben", "hey")]);
        sync_as(&store, Some(ME_NAME), 1_000).unwrap();
        // Alone, "Ben" is a person of their own.
        let lone = person_of(&store, "ben").unwrap();
        assert_eq!(name(&store, lone), "Ben");
        set_note(&store, lone, "my cousin").unwrap();

        // Ben Merton arrives: the one Ben, so "Ben" joins him, note and all.
        chat(&store, "gmail", "t1", "Pricing", &[("Ben Merton", "hi")]);
        sync_as(&store, Some(ME_NAME), 2_000).unwrap();
        let merton = person_of(&store, "ben merton").unwrap();
        assert_eq!(person_of(&store, "ben"), Some(merton));
        let note: Option<String> = store
            .conn()
            .query_row("SELECT note FROM person WHERE id = ?1", [merton], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(note.as_deref(), Some("my cousin"));

        // A second Ben: "Ben" is undecided, and shows as maybe on both pages.
        chat(&store, "gmail", "t2", "Hello", &[("Ben Carter", "hi")]);
        sync_as(&store, Some(ME_NAME), 3_000).unwrap();
        let carter = person_of(&store, "ben carter").unwrap();
        assert_eq!(person_of(&store, "ben"), None);
        let now = Local::now();
        let page = page(&store, carter, now).unwrap();
        assert_eq!(page.maybe.len(), 1);
        assert_eq!(page.maybe[0].shown, "Ben");

        // The person says it is Carter: it stays Carter's.
        claim(&store, "ben", carter).unwrap();
        sync_as(&store, Some(ME_NAME), 4_000).unwrap();
        assert_eq!(person_of(&store, "ben"), Some(carter));

        // Not the same: split off, and kept apart.
        let apart = split(&store, "ben", 5_000).unwrap();
        sync_as(&store, Some(ME_NAME), 6_000).unwrap();
        assert_eq!(person_of(&store, "ben"), Some(apart));

        // Same person: Carter's names become Merton's for good.
        merge(&store, carter, merton).unwrap();
        sync_as(&store, Some(ME_NAME), 7_000).unwrap();
        assert_eq!(person_of(&store, "ben carter"), Some(merton));

        // Not a person: out of lookups.
        hide(&store, merton, true).unwrap();
        let directory = Directory::load(&store).unwrap();
        assert_eq!(directory.resolve("Ben Merton"), None);
    }

    #[test]
    fn a_page_says_what_is_open_and_what_waits() {
        let store = Store::open_in_memory().unwrap();
        let now = Local::now();
        let at = now.timestamp_millis() - 3_600_000;
        conversations::record(
            &store,
            &Conversation {
                app: "whatsapp",
                key: "Ben Merton",
                title: "Ben Merton",
                work: Some(true),
            },
            &[
                NewMessage {
                    sender: ME,
                    text: "sending the scope soon",
                    sent_label: Some("10:00"),
                    sent_at: Some(at - 60_000),
                },
                NewMessage {
                    sender: "Ben Merton",
                    text: "can you send it?",
                    sent_label: Some("11:00"),
                    sent_at: Some(at),
                },
            ],
            at,
        )
        .unwrap();
        sync_as(&store, Some(ME_NAME), at).unwrap();
        let ben = person_of(&store, "ben merton").unwrap();
        crate::todos::add(&store, "Send Ben the V2 deck", now).unwrap();

        let page = page(&store, ben, now).unwrap();
        assert_eq!(page.name, "Ben Merton");
        assert_eq!(page.work, Some(true));
        assert_eq!(page.waiting.len(), 1);
        assert_eq!(page.waiting[0].text, "can you send it?");
        assert_eq!(page.owe.len(), 1);
        assert_eq!(page.owe[0].title, "Send Ben the V2 deck");
        assert!(
            page.last
                .as_deref()
                .unwrap_or_default()
                .starts_with("Wrote last on WhatsApp")
        );

        let memory = memory(&store, ben, now).unwrap();
        assert!(memory.starts_with("### Ben Merton"));
        assert!(memory.contains("Waiting on the person"));
        assert!(memory.contains("The person owes them: Send Ben the V2 deck"));
        assert!(memory.contains("them on WhatsApp"));

        let directory = Directory::load(&store).unwrap();
        let items = crate::todos::items(&store, &directory, now).unwrap();
        let people = list(&store, &directory, &items, now).unwrap();
        assert_eq!(people[0].name, "Ben Merton");
        assert!(people[0].needs_you);
        assert_eq!(people[0].detail, "You owe 1 \u{b7} Waiting on you");

        assert_eq!(
            for_question(&store, "what's open with ben?", None).unwrap(),
            [ben]
        );
    }
}
