//! Walks an app's Accessibility tree into a plain, serializable snapshot.

use std::fmt;
use std::time::Instant;

use serde::Serialize;

use crate::ax::{self, AxError, Element, Value};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_depth: usize,
    pub max_nodes: usize,
    /// Longer values (whole documents in a text view) are cut to this many characters.
    pub max_value_chars: usize,
    /// Read each web element's classes (`AXDOMClassList`), one more round
    /// trip per element: how a web app (claude.ai in the Claude app) marks
    /// whose a message is, which its roles and text do not say.
    pub dom_classes: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 10_000,
            max_value_chars: 20_000,
            dom_classes: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowScope {
    Focused,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The app's own name for the element (`AXIdentifier`), when it sets
    /// one: steadier than its text for parsing a particular app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// A web element's classes, read only when `Limits::dom_classes` is set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    /// The page address, read only on `AXWebArea` nodes to save round trips.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Rect>,
    /// The part of `value` a text view that scrolls shows right now, when
    /// it shows less than all of it (`AXVisibleCharacterRange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Serialize)]
pub struct WindowSnapshot {
    pub title: Option<String>,
    pub root: Node,
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    pub nodes: usize,
    /// True when a depth or node limit cut the walk short.
    pub truncated: bool,
    /// Child lists the target app failed to return.
    pub failed_reads: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct AppSnapshot {
    pub pid: i32,
    pub app_name: Option<String>,
    pub windows: Vec<WindowSnapshot>,
    pub stats: Stats,
}

#[derive(Debug)]
pub enum SnapshotError {
    NotTrusted,
    NoSuchApp(i32),
    Ax(AxError),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTrusted => write!(
                f,
                "Accessibility access is not granted. Allow the app that launched this tool in \
                 System Settings > Privacy & Security > Accessibility."
            ),
            Self::NoSuchApp(pid) => write!(f, "no app with pid {pid}"),
            Self::Ax(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<AxError> for SnapshotError {
    fn from(e: AxError) -> Self {
        match e {
            AxError::ApiDisabled => Self::NotTrusted,
            other => Self::Ax(other),
        }
    }
}

/// The app that currently has keyboard focus.
pub fn focused_app_pid() -> Result<i32, SnapshotError> {
    let system = Element::system_wide().ok_or(SnapshotError::Ax(AxError::Other(0)))?;
    match system.attribute("AXFocusedApplication")? {
        Some(Value::Element(app)) => app.pid().ok_or(SnapshotError::Ax(AxError::InvalidElement)),
        _ => Err(SnapshotError::Ax(AxError::CannotComplete)),
    }
}

/// Asks Chromium-based browsers and Electron apps to build their
/// Accessibility tree, which they skip unless an assistive app requests it.
/// Electron answers `AXManualAccessibility`, Chromium `AXEnhancedUserInterface`;
/// an app usually rejects the one it does not know.
pub fn enable_web_accessibility(pid: i32) -> Result<WebAxRequest, SnapshotError> {
    let app = Element::application(pid).ok_or(SnapshotError::NoSuchApp(pid))?;
    Ok(WebAxRequest {
        manual_accessibility: app.set_bool("AXManualAccessibility", true),
        enhanced_user_interface: app.set_bool("AXEnhancedUserInterface", true),
    })
}

#[derive(Debug)]
pub struct WebAxRequest {
    pub manual_accessibility: Result<(), AxError>,
    pub enhanced_user_interface: Result<(), AxError>,
}

pub fn snapshot_app(
    pid: i32,
    scope: WindowScope,
    limits: Limits,
) -> Result<AppSnapshot, SnapshotError> {
    if !ax::is_trusted(false) {
        return Err(SnapshotError::NotTrusted);
    }
    let started = Instant::now();
    let app = Element::application(pid).ok_or(SnapshotError::NoSuchApp(pid))?;
    // `AXTitle` on the app element doubles as a liveness check.
    let app_name = match app.attribute("AXTitle")? {
        Some(Value::String(name)) => Some(name),
        _ => None,
    };

    let windows = match scope {
        // The main window is the document the person works in. The focused
        // window can be a panel instead: Arc reports an untitled one while
        // another app is in front.
        WindowScope::Focused => app
            .element("AXMainWindow")
            .or_else(|| app.element("AXFocusedWindow"))
            .or_else(|| app.elements("AXWindows").ok()?.into_iter().next())
            .into_iter()
            .collect(),
        WindowScope::All => app.elements("AXWindows")?,
    };

    let mut walker = Walker {
        limits,
        stats: Stats::default(),
    };
    let windows = windows
        .iter()
        .map(|window| WindowSnapshot {
            title: clean(window.string("AXTitle")),
            root: walker.walk(window, 0),
        })
        .collect();

    let mut stats = walker.stats;
    stats.elapsed_ms = started.elapsed().as_millis();
    Ok(AppSnapshot {
        pid,
        app_name,
        windows,
        stats,
    })
}

struct Walker {
    limits: Limits,
    stats: Stats,
}

impl Walker {
    fn walk(&mut self, element: &Element, depth: usize) -> Node {
        self.stats.nodes += 1;
        let role = element
            .string("AXRole")
            .unwrap_or_else(|| "AXUnknown".to_owned());
        let value = clean(element.string("AXValue"));
        let shown = (role == "AXTextArea")
            .then(|| shown(element, value.as_deref()?))
            .flatten();
        let mut node = Node {
            url: (role == "AXWebArea")
                .then(|| element.url("AXURL"))
                .flatten(),
            role,
            subrole: element.string("AXSubrole"),
            title: clean(element.string("AXTitle")),
            value: value.map(|v| truncate_chars(v, self.limits.max_value_chars)),
            shown,
            description: clean(element.string("AXDescription")),
            identifier: clean(element.string("AXIdentifier")),
            classes: if self.limits.dom_classes {
                classes(element)
            } else {
                Vec::new()
            },
            frame: frame(element),
            children: Vec::new(),
        };

        let children = match element.elements("AXChildren") {
            Ok(children) => children,
            Err(_) => {
                self.stats.failed_reads += 1;
                return node;
            }
        };
        if !children.is_empty() && depth >= self.limits.max_depth {
            self.stats.truncated = true;
            return node;
        }
        for child in &children {
            if self.stats.nodes >= self.limits.max_nodes {
                self.stats.truncated = true;
                break;
            }
            node.children.push(self.walk(child, depth + 1));
        }
        node
    }
}

fn classes(element: &Element) -> Vec<String> {
    let Ok(Some(Value::Array(values))) = element.attribute("AXDOMClassList") else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|value| match value {
            Value::String(class) => Some(class),
            _ => None,
        })
        .collect()
}

fn frame(element: &Element) -> Option<Rect> {
    let Ok(Some(Value::Point { x, y })) = element.attribute("AXPosition") else {
        return None;
    };
    let Ok(Some(Value::Size { width, height })) = element.attribute("AXSize") else {
        return None;
    };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// The part of a text view's `value` it shows, if that is less than all of
/// it: TextEdit and Notes keep a whole document in one text view.
fn shown(element: &Element, value: &str) -> Option<String> {
    let Ok(Some(Value::Range { location, length })) = element.attribute("AXVisibleCharacterRange")
    else {
        return None;
    };
    utf16_slice(value, location, length)
}

/// `length` UTF-16 code units of `text` from `location` (Accessibility
/// counts text that way), or `None` if that is all of the text.
fn utf16_slice(text: &str, location: usize, length: usize) -> Option<String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let start = location.min(units.len());
    let end = location.saturating_add(length).min(units.len());
    if start == 0 && end == units.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&units[start..end]))
}

fn clean(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

fn truncate_chars(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_by_utf16_units() {
        // "é" is one unit, the emoji two.
        let text = "café 🙂 end";
        assert_eq!(utf16_slice(text, 5, 2).as_deref(), Some("🙂"));
        assert_eq!(utf16_slice(text, 8, 99).as_deref(), Some("end"));
        assert_eq!(utf16_slice(text, 0, 11), None);
        assert_eq!(utf16_slice(text, 50, 3).as_deref(), Some(""));
    }
}
