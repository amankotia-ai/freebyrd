//! Notifications on the iPad's Lock Screen, through Web Push. They work
//! once the iPad app is opened over HTTPS (Tailscale's `serve`) and the
//! person allowed notifications in it.
//!
//! A push carries nothing: no payload is sent, so Apple's push service sees
//! only that something is waiting. The iPad's service worker then asks this
//! Mac what to say (`summary`), in full or as a count, as the person chose.
//! The key that signs pushes (VAPID) is made once with macOS's `openssl`,
//! and pushes go out with `curl`, both from `/usr/bin`.
//!
//! A loop looks for new cards every minute. It sends at most one push in
//! ten minutes, none in the person's quiet hours (they sleep from about
//! 03:00 to 14:30), and one when the quiet hours end if anything came in.

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::state::{self, PushSubscription};

const OPENSSL: &str = "/usr/bin/openssl";
const CURL: &str = "/usr/bin/curl";
const EVERY: Duration = Duration::from_secs(60);
const AT_MOST_EVERY_MS: i64 = 10 * 60_000;
/// Push services an iPad (or a browser) may subscribe with. Pushes go
/// nowhere else, whatever a subscription says.
const SERVICES: &[&str] = &[
    "web.push.apple.com",
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
];

fn key_path() -> Result<PathBuf> {
    Ok(state::dir()?.join("vapid.pem"))
}

/// The signing key, made the first time.
fn key() -> Result<PathBuf> {
    let path = key_path()?;
    if !path.exists() {
        let made = Command::new(OPENSSL)
            .args([
                "ecparam",
                "-name",
                "prime256v1",
                "-genkey",
                "-noout",
                "-out",
            ])
            .arg(&path)
            .status()
            .context("running openssl")?;
        if !made.success() {
            bail!("openssl could not make the notification key");
        }
    }
    Ok(path)
}

/// The public half, as the browser's `applicationServerKey` (base64url of
/// the 65-byte point).
pub fn public_key() -> Result<String> {
    let der = Command::new(OPENSSL)
        .args(["ec", "-pubout", "-outform", "DER", "-in"])
        .arg(key()?)
        .output()
        .context("running openssl")?;
    if !der.status.success() || der.stdout.len() < 65 {
        bail!("openssl could not read the notification key");
    }
    let point = &der.stdout[der.stdout.len() - 65..];
    Ok(base64url(point))
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A VAPID token for `endpoint`'s push service, good for 12 hours.
fn token(endpoint: &str) -> Result<String> {
    let audience = origin(endpoint).context("not a push address")?;
    let header = base64url(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = base64url(
        json!({
            "aud": audience,
            "exp": memory::time::now_ms() / 1000 + 12 * 3600,
            "sub": "mailto:push@freebyrd.invalid",
        })
        .to_string()
        .as_bytes(),
    );
    let signed = format!("{header}.{claims}");
    let mut child = Command::new(OPENSSL)
        .args(["dgst", "-sha256", "-sign"])
        .arg(key()?)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("running openssl")?;
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .context("no stdin for openssl")?
            .write_all(signed.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("openssl could not sign the notification");
    }
    let raw = raw_signature(&output.stdout).context("openssl's signature was not understood")?;
    Ok(format!("{signed}.{}", base64url(&raw)))
}

/// An ECDSA signature from DER (`SEQUENCE { INTEGER r, INTEGER s }`) to the
/// 64 bytes JWTs use.
fn raw_signature(der: &[u8]) -> Option<[u8; 64]> {
    let mut at = 0;
    let take = |at: &mut usize| -> Option<u8> {
        let byte = *der.get(*at)?;
        *at += 1;
        Some(byte)
    };
    if take(&mut at)? != 0x30 {
        return None;
    }
    let length = take(&mut at)?;
    if length & 0x80 != 0 {
        // Long form; a P-256 signature never needs it, but skip its bytes.
        at += usize::from(length & 0x7f);
    }
    let mut out = [0u8; 64];
    for half in 0..2 {
        if take(&mut at)? != 0x02 {
            return None;
        }
        let length = usize::from(take(&mut at)?);
        let mut number = der.get(at..at + length)?;
        at += length;
        while number.len() > 32 && number[0] == 0 {
            number = &number[1..];
        }
        if number.len() > 32 {
            return None;
        }
        let start = half * 32 + (32 - number.len());
        out[start..half * 32 + 32].copy_from_slice(number);
    }
    Some(out)
}

/// `https://web.push.apple.com` from a subscription's endpoint, for a push
/// service this Mac sends to; `None` otherwise.
fn origin(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let name = host.split(':').next()?;
    SERVICES.contains(&name).then(|| format!("https://{host}"))
}

/// Remembers where to push this iPad.
pub fn subscribe(device: &str, endpoint: &str) -> Result<()> {
    if origin(endpoint).is_none() {
        bail!("that is not a push service Freebyrd sends to");
    }
    let mut devices = state::devices();
    let found = devices
        .iter_mut()
        .find(|d| d.id == device)
        .context("this iPad is not paired")?;
    found.push = Some(PushSubscription {
        endpoint: endpoint.to_owned(),
    });
    state::save_devices(&devices)
}

pub fn unsubscribe(device: &str) -> Result<()> {
    let mut devices = state::devices();
    if let Some(found) = devices.iter_mut().find(|d| d.id == device) {
        found.push = None;
    }
    state::save_devices(&devices)
}

/// One push to this iPad now, quiet hours or not.
pub fn test(device: &str) -> Result<()> {
    let devices = state::devices();
    let endpoint = devices
        .iter()
        .find(|d| d.id == device)
        .and_then(|d| d.push.as_ref())
        .context("notifications are not on for this iPad")?
        .endpoint
        .clone();
    send(&endpoint)?;
    Ok(())
}

enum Sent {
    Delivered,
    /// The iPad dropped the subscription: forget it.
    Gone,
}

fn send(endpoint: &str) -> Result<Sent> {
    let authorization = format!("vapid t={}, k={}", token(endpoint)?, public_key()?);
    let output = Command::new(CURL)
        .args([
            "-sS",
            "--max-time",
            "20",
            "--proto",
            "=https",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
        ])
        .args([
            "-H",
            "TTL: 21600",
            "-H",
            "Urgency: normal",
            "-H",
            "Content-Length: 0",
        ])
        // The token reaches curl in its config on stdin, not in arguments.
        .args(["--config", "-"])
        .arg(endpoint)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running curl")
        .and_then(|mut child| {
            use std::io::Write;
            let line = format!("header = \"Authorization: {authorization}\"\n");
            child
                .stdin
                .take()
                .context("no stdin for curl")?
                .write_all(line.as_bytes())?;
            Ok(child.wait_with_output()?)
        })?;
    let code = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    match code.as_str() {
        "200" | "201" | "202" => Ok(Sent::Delivered),
        "404" | "410" => Ok(Sent::Gone),
        _ => bail!(
            "the push service answered {code}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

/// What a notification says, for the service worker. Card titles only in
/// full mode; otherwise a count.
pub fn summary(store: &memory::Store, _now_ms: i64) -> Result<Value> {
    let mut statement = store.conn().prepare(
        "SELECT kind, title FROM card WHERE seen_at IS NULL AND dismissed_at IS NULL AND kind != 'block'
         ORDER BY created_at DESC LIMIT 20",
    )?;
    let waiting: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let count = waiting.len();
    let full = state::settings().notify == "full";
    let (title, body) = if count == 0 {
        ("Freebyrd".to_owned(), "Nothing new is waiting.".to_owned())
    } else if full {
        let lines: Vec<String> = waiting
            .iter()
            .take(3)
            .map(|(kind, title)| format!("{}: {title}", kind_label(kind)))
            .collect();
        let more = count.saturating_sub(3);
        let tail = if more > 0 {
            format!("\n{more} more")
        } else {
            String::new()
        };
        ("Freebyrd".to_owned(), format!("{}{tail}", lines.join("\n")))
    } else {
        (
            "Freebyrd".to_owned(),
            if count == 1 {
                "1 thing is waiting.".to_owned()
            } else {
                format!("{count} things are waiting.")
            },
        )
    };
    Ok(json!({ "title": title, "body": body, "count": count, "url": "/#cards" }))
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "recap" => "Recap",
        "night" => "Night shift",
        "nightplan" => "Tonight",
        "stop" => "Stop for tonight",
        "job" => "Job",
        "due" => "Due",
        "draft" => "Draft",
        "suggestion" => "Suggestion",
        "event" => "Event",
        "question" => "Question",
        "idea" => "Idea",
        other => other,
    }
}

/// What the loop remembers between runs.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Memo {
    /// The newest card already told about.
    newest: i64,
    last_push_at: i64,
}

fn sent_path() -> Result<PathBuf> {
    Ok(state::dir()?.join("push.json"))
}

fn load_sent() -> Memo {
    sent_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_sent(sent: &Memo) {
    if let Ok(path) = sent_path() {
        let _ = std::fs::write(path, serde_json::to_vec(sent).unwrap_or_default());
    }
}

fn quiet(hour: u32, from: u32, until: u32) -> bool {
    if from == until {
        false
    } else if from < until {
        hour >= from && hour < until
    } else {
        hour >= from || hour < until
    }
}

/// Starts the loop that pushes new cards to the paired iPads.
pub fn start() {
    thread::spawn(|| {
        loop {
            thread::sleep(EVERY);
            if let Err(e) = tick() {
                eprintln!("freebyrd: companion: notifications: {e:#}");
            }
        }
    });
}

fn tick() -> Result<()> {
    let devices = state::devices();
    let endpoints: Vec<(String, String)> = devices
        .iter()
        .filter_map(|d| d.push.as_ref().map(|p| (d.id.clone(), p.endpoint.clone())))
        .collect();
    if endpoints.is_empty() {
        return Ok(());
    }
    let store = super::memory()?;
    let newest: i64 = store.conn().query_row(
        "SELECT coalesce(max(created_at), 0) FROM card WHERE seen_at IS NULL AND dismissed_at IS NULL AND kind != 'block'",
        [],
        |row| row.get(0),
    )?;
    let mut sent = load_sent();
    if newest <= sent.newest {
        return Ok(());
    }
    let settings = state::settings();
    let now = Local::now();
    if quiet(now.hour(), settings.quiet_from, settings.quiet_until) {
        return Ok(());
    }
    let now_ms = now.timestamp_millis();
    if now_ms - sent.last_push_at < AT_MOST_EVERY_MS {
        return Ok(());
    }
    for (device, endpoint) in endpoints {
        match send(&endpoint) {
            Ok(Sent::Delivered) => {}
            Ok(Sent::Gone) => {
                eprintln!(
                    "freebyrd: companion: an iPad turned notifications off; forgetting its subscription"
                );
                let _ = unsubscribe(&device);
            }
            Err(e) => eprintln!("freebyrd: companion: push: {e:#}"),
        }
    }
    sent.newest = newest;
    sent.last_push_at = now_ms;
    save_sent(&sent);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_go_only_to_push_services() {
        assert_eq!(
            origin("https://web.push.apple.com/QK1x?a=b").as_deref(),
            Some("https://web.push.apple.com")
        );
        assert!(origin("https://evil.example/x").is_none());
        assert!(origin("http://web.push.apple.com/x").is_none());
        assert!(origin("https://web.push.apple.com.evil.example/x").is_none());
    }

    #[test]
    fn quiet_hours_cross_midnight() {
        assert!(quiet(3, 3, 14));
        assert!(quiet(13, 3, 14));
        assert!(!quiet(14, 3, 14));
        assert!(!quiet(2, 3, 14));
        assert!(quiet(1, 23, 7));
        assert!(!quiet(12, 23, 7));
        assert!(!quiet(5, 5, 5));
    }

    #[test]
    fn signatures_become_64_bytes() {
        // r with a leading zero (33 bytes), s short (31 bytes).
        let mut der = vec![0x30, 0x44, 0x02, 0x21, 0x00];
        der.extend([0xAA; 32]);
        der.extend([0x02, 0x1F]);
        der.extend([0xBB; 31]);
        let raw = raw_signature(&der).unwrap();
        assert!(raw[..32].iter().all(|b| *b == 0xAA));
        assert_eq!(raw[32], 0);
        assert!(raw[33..].iter().all(|b| *b == 0xBB));
        assert!(raw_signature(&[0x31, 0x00]).is_none());
    }
}
