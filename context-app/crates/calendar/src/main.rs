//! Freebyrd Calendar: adds and removes Freebyrd's events in Apple Calendar,
//! in a calendar of their own named "Freebyrd".
//!
//! macOS gives calendar access to an app, by name, and only to an app whose
//! Info.plist says why it wants it. So this runs as its own small app,
//! `Freebyrd Calendar.app`, which `freebyrd-calendar install` builds; the
//! main app starts it through `open`, so it answers for itself rather than
//! for whatever launched Freebyrd. It only ever removes events from the
//! Freebyrd calendar. Littlebird also writes Apple Calendar through EventKit
//! on the Mac (`littlebird/FINDINGS.md` §5.5).
//!
//! Each command prints one JSON object: the result, or `{"error": ...}`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc;

use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAlarm, EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStore, EKSourceType,
    EKSpan,
};
use objc2_foundation::{NSDate, NSError, NSString};
use serde::Deserialize;
use serde_json::{Value, json};

const CALENDAR_TITLE: &str = "Freebyrd";
const APP_NAME: &str = "Freebyrd Calendar.app";
const BUNDLE_ID: &str = "com.freebyrd.calendar";
const EXECUTABLE: &str = "freebyrd-calendar";

const USAGE: &str = "\
usage:
  freebyrd-calendar install          build Freebyrd Calendar.app
  freebyrd-calendar access           ask for calendar access
  freebyrd-calendar add JSON         {\"title\", \"start\" (Unix seconds), \"end\"?, \"allDay\"?, \"notes\"?, \"alertAfter\"? (seconds)}
  freebyrd-calendar remove ID        remove an event from the Freebyrd calendar";

const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>com.freebyrd.calendar</string>
  <key>CFBundleName</key><string>Freebyrd Calendar</string>
  <key>CFBundleDisplayName</key><string>Freebyrd Calendar</string>
  <key>CFBundleExecutable</key><string>freebyrd-calendar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSCalendarsFullAccessUsageDescription</key><string>Freebyrd puts what you promised people on a Freebyrd calendar, and takes it off when you press Undo.</string>
  <key>NSCalendarsUsageDescription</key><string>Freebyrd puts what you promised people on a Freebyrd calendar, and takes it off when you press Undo.</string>
</dict>
</plist>
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewEvent {
    title: String,
    /// Seconds since the Unix epoch; for an all-day event, the start of its day.
    start: f64,
    end: Option<f64>,
    #[serde(default)]
    all_day: bool,
    notes: Option<String>,
    /// An alert this many seconds after the start (for an all-day event,
    /// after the start of the day).
    alert_after: Option<f64>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["install"] => install(),
        ["access"] => request_access().map(|granted| json!({ "granted": granted })),
        ["add", event] => add(event),
        ["remove", id] => remove(id),
        _ => Err(anyhow!(USAGE)),
    };
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", json!({ "error": format!("{e:#}") }));
            ExitCode::FAILURE
        }
    }
}

/// Asks for full access the first time; macOS shows its prompt and this
/// waits for the answer.
fn request_access() -> Result<bool> {
    let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
    if status == EKAuthorizationStatus::FullAccess {
        return Ok(true);
    }
    if status == EKAuthorizationStatus::Denied || status == EKAuthorizationStatus::Restricted {
        return Ok(false);
    }
    let store = unsafe { EKEventStore::new() };
    let (sender, receiver) = mpsc::channel();
    let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = sender.send(granted.as_bool());
    });
    unsafe { store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion)) };
    Ok(receiver.recv()?)
}

/// An event store that can read and write, or an error saying how to allow it.
fn store() -> Result<Retained<EKEventStore>> {
    if !request_access()? {
        bail!(
            "Freebyrd Calendar has no calendar access. Allow it in System Settings > Privacy & Security > Calendars."
        );
    }
    // A store made after access was granted sees the calendars.
    Ok(unsafe { EKEventStore::new() })
}

/// The Freebyrd calendar, made the first time where the person's default
/// calendar lives (iCloud, usually, so it shows on their phone too), or on
/// this Mac if they have none.
fn freebyrd_calendar(store: &EKEventStore) -> Result<Retained<EKCalendar>> {
    let calendars = unsafe { store.calendarsForEntityType(EKEntityType::Event) };
    if let Some(found) = calendars
        .iter()
        .find(|calendar| unsafe { calendar.title() }.to_string() == CALENDAR_TITLE)
    {
        return Ok(found);
    }
    let calendar =
        unsafe { EKCalendar::calendarForEntityType_eventStore(EKEntityType::Event, store) };
    unsafe { calendar.setTitle(&NSString::from_str(CALENDAR_TITLE)) };
    let source = unsafe { store.defaultCalendarForNewEvents() }
        .and_then(|default| unsafe { default.source() })
        .or_else(|| {
            unsafe { store.sources() }
                .iter()
                .find(|source| unsafe { source.sourceType() } == EKSourceType::Local)
        })
        .context("no calendar account to put the Freebyrd calendar in")?;
    unsafe { calendar.setSource(Some(&source)) };
    unsafe { store.saveCalendar_commit_error(&calendar, true) }.map_err(|e| {
        anyhow!(
            "could not make the Freebyrd calendar: {}",
            e.localizedDescription()
        )
    })?;
    Ok(calendar)
}

fn add(json: &str) -> Result<Value> {
    let new: NewEvent = serde_json::from_str(json).context("reading the event")?;
    let store = store()?;
    let calendar = freebyrd_calendar(&store)?;
    let event = unsafe { EKEvent::eventWithEventStore(&store) };
    let end = new.end.unwrap_or(if new.all_day {
        new.start
    } else {
        new.start + 1_800.0
    });
    unsafe {
        event.setTitle(Some(&NSString::from_str(&new.title)));
        event.setCalendar(Some(&calendar));
        event.setAllDay(new.all_day);
        event.setStartDate(Some(&NSDate::dateWithTimeIntervalSince1970(new.start)));
        event.setEndDate(Some(&NSDate::dateWithTimeIntervalSince1970(end)));
        if let Some(notes) = &new.notes {
            event.setNotes(Some(&NSString::from_str(notes)));
        }
        if let Some(after) = new.alert_after {
            event.addAlarm(&EKAlarm::alarmWithRelativeOffset(after));
        }
        store.saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
    }
    .map_err(|e| anyhow!("could not save the event: {}", e.localizedDescription()))?;
    let id = unsafe { event.eventIdentifier() }
        .map(|id| id.to_string())
        .context("the calendar gave the event no identifier")?;
    Ok(json!({ "id": id, "calendar": CALENDAR_TITLE }))
}

/// Removes an event, but only one in the Freebyrd calendar.
fn remove(id: &str) -> Result<Value> {
    let store = store()?;
    let Some(event) = (unsafe { store.eventWithIdentifier(&NSString::from_str(id)) }) else {
        return Ok(json!({ "removed": false, "reason": "not found" }));
    };
    let in_freebyrd = unsafe { event.calendar() }
        .is_some_and(|calendar| unsafe { calendar.title() }.to_string() == CALENDAR_TITLE);
    if !in_freebyrd {
        bail!("event {id} is not in the Freebyrd calendar, so it is left alone");
    }
    unsafe { store.removeEvent_span_commit_error(&event, EKSpan::ThisEvent, true) }
        .map_err(|e| anyhow!("could not remove the event: {}", e.localizedDescription()))?;
    Ok(json!({ "removed": true }))
}

/// `~/Library/Application Support/Freebyrd/bin/Freebyrd Calendar.app`.
fn app_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(Path::new(&home)
        .join("Library/Application Support/Freebyrd/bin")
        .join(APP_NAME))
}

/// Builds the app around this executable and signs it (ad hoc). Signing
/// anew can make macOS ask for calendar access again.
fn install() -> Result<Value> {
    let app = app_path()?;
    let macos = app.join("Contents/MacOS");
    fs::create_dir_all(&macos)?;
    fs::write(app.join("Contents/Info.plist"), INFO_PLIST)?;
    // Copy, then rename: overwriting a signed binary in place gets its next
    // launch killed by macOS.
    let staged = macos.join(format!("{EXECUTABLE}.new"));
    fs::copy(std::env::current_exe()?, &staged)?;
    fs::rename(&staged, macos.join(EXECUTABLE))?;
    let status = Command::new("codesign")
        .args(["--force", "--sign", "-", "--identifier", BUNDLE_ID])
        .arg(&app)
        .status()
        .context("running codesign")?;
    if !status.success() {
        bail!("codesign failed ({status})");
    }
    Ok(json!({ "installed": app.display().to_string() }))
}
