//! What the companion keeps of its own, in JSON files under
//! `Application Support/Freebyrd/companion/`: the paired iPads and their
//! tokens, the person's settings, links shared from the iPad, sketches and
//! the push key. None of it is in the memory database, so it needs no
//! migration and can be deleted to start over (every iPad then pairs again).

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The companion's folder: `FREEBYRD_COMPANION_DIR` (for tests), or the one
/// beside the memory.
pub fn dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("FREEBYRD_COMPANION_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => freebyrd_dir()?.join("companion"),
    };
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// `~/Library/Application Support/Freebyrd`.
pub fn freebyrd_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Freebyrd"))
}

/// A paired iPad.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub token: String,
    pub paired_at: i64,
    pub last_seen_at: i64,
    /// Its Web Push subscription, once it allowed notifications.
    #[serde(default)]
    pub push: Option<PushSubscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscription {
    pub endpoint: String,
}

/// What the iPad may see, set on the Mac only (the pairing page).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shown {
    pub ask: bool,
    pub people: bool,
    pub meetings: bool,
    pub work: bool,
    pub reading: bool,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            ask: true,
            people: true,
            meetings: true,
            work: true,
            reading: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Answer on the Wi-Fi too (port `LAN_PORT`), not only through Tailscale.
    pub lan: bool,
    pub shown: Shown,
    /// Lock Screen notifications: `private` ("3 things waiting") or `full`.
    pub notify: String,
    /// No notifications between these hours (the person sleeps from about
    /// 03:00 to 14:30); what arrives meanwhile is one notification after.
    pub quiet_from: u32,
    pub quiet_until: u32,
    /// Reading shelf items the person finished with, by id.
    pub read: Vec<String>,
    /// The iPad may start Claude Code sessions on this Mac (set on the Mac
    /// only; off until the person turns it on).
    pub claude_code: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lan: false,
            shown: Shown::default(),
            notify: "private".to_owned(),
            quiet_from: 3,
            quiet_until: 14,
            read: Vec::new(),
            claude_code: false,
        }
    }
}

/// Something shared from the iPad: a link or text from the share sheet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    pub id: String,
    pub at: i64,
    pub url: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
}

fn load<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    let Ok(path) = dir().map(|dir| dir.join(name)) else {
        return T::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            eprintln!(
                "freebyrd: companion: {} is unreadable ({e}); starting it afresh",
                path.display()
            );
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Written whole to a new file, then moved over the old: a crash never
/// leaves half a file.
fn save<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let path = dir()?.join(name);
    let next = path.with_extension("json.new");
    fs::write(&next, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&next, &path)?;
    Ok(())
}

pub fn devices() -> Vec<Device> {
    load("devices.json")
}

pub fn save_devices(devices: &[Device]) -> Result<()> {
    save("devices.json", &devices)
}

pub fn settings() -> Settings {
    load("settings.json")
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    save("settings.json", settings)
}

pub fn shared() -> Vec<Shared> {
    load("shared.json")
}

/// Keeps the newest 300.
pub fn save_shared(items: &[Shared]) -> Result<()> {
    let start = items.len().saturating_sub(300);
    save("shared.json", &items[start..].to_vec())
}

/// `bytes` random bytes from the system, as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    let filled = fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut buffer));
    if filled.is_err() {
        // Never expected on macOS; the clock alone would be guessable, so
        // refuse rather than mint a weak token.
        panic!("/dev/urandom could not be read");
    }
    buffer.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares secrets in time that does not depend on where they differ.
pub fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}
