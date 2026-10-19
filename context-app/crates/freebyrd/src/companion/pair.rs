//! Pairing: an iPad gets its token by typing a code that this Mac shows on
//! its pairing page (`/pair`, from Freebyrd's menu-bar item). The page and
//! its settings answer this Mac only: not the Wi-Fi, not Tailscale, and not
//! a web page that tries to reach them under another name or from another
//! origin.

use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::http::{Request, Response};
use super::state::{self, Device};
use super::{Companion, LAN_PORT, PORT, lock};

/// A code lasts this long, and allows this many wrong tries.
const CODE_MS: i64 = 10 * 60_000;
const TRIES: u32 = 5;
const TAILSCALE: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";

pub struct Code {
    code: String,
    expires_at: i64,
    tries: u32,
}

/// The code to show now, made afresh when there is none or it ran out.
fn current_code(c: &Companion) -> (String, i64) {
    let now = memory::time::now_ms();
    let mut pairing = lock(&c.pairing);
    if pairing.as_ref().is_none_or(|code| code.expires_at <= now) {
        *pairing = Some(new_code(now));
    }
    let code = pairing.as_ref().expect("just set");
    (code.code.clone(), code.expires_at)
}

fn new_code(now: i64) -> Code {
    let n = u32::from_str_radix(&state::random_hex(4), 16).unwrap_or(0) % 1_000_000;
    Code {
        code: format!("{n:06}"),
        expires_at: now + CODE_MS,
        tries: 0,
    }
}

/// The pairing page itself; its data comes from `/pair/state`.
pub fn page(_c: &Companion, request: &Request) -> Response {
    if !request.is_from_this_mac() {
        return Response::error(403, "Pairing works on the Mac itself.");
    }
    super::assets::file("/pair.html")
}

/// What the pairing page shows: the code, where the iPad can reach this Mac,
/// the settings only this Mac may change, and the paired iPads.
pub fn page_state(c: &Companion, request: &Request) -> Response {
    if !request.is_from_this_mac() || !same_origin(request) {
        return Response::error(403, "Pairing works on the Mac itself.");
    }
    let (code, expires_at) = current_code(c);
    let settings = state::settings();
    let devices: Vec<_> = state::devices()
        .into_iter()
        .map(|device| {
            json!({
                "id": device.id,
                "name": device.name,
                "pairedAt": device.paired_at,
                "lastSeenAt": device.last_seen_at,
                "notifications": device.push.is_some(),
            })
        })
        .collect();
    Response::json(&json!({
        "code": code,
        "expiresAt": expires_at,
        "port": super::port(),
        "lanPort": LAN_PORT,
        "lan": lan_addresses(),
        "tailscale": tailscale(),
        "settings": settings,
        "devices": devices,
    }))
}

#[derive(Deserialize)]
struct Change {
    action: String,
    #[serde(default)]
    value: serde_json::Value,
}

/// A change made on the pairing page: a new code, the Wi-Fi on or off,
/// what the iPad may see, or an iPad unpaired.
pub fn change(c: &Companion, request: &Request) -> Response {
    // A custom header, which a form on another site cannot send, and this
    // page's own origin.
    if !request.is_from_this_mac()
        || request.header("x-freebyrd") != Some("pair")
        || !same_origin(request)
    {
        return Response::error(403, "Pairing works on the Mac itself.");
    }
    let change: Change = match request.json() {
        Ok(change) => change,
        Err(e) => return Response::error(400, &e),
    };
    let mut settings = state::settings();
    match change.action.as_str() {
        "code" => {
            *lock(&c.pairing) = Some(new_code(memory::time::now_ms()));
        }
        "lan" => settings.lan = change.value.as_bool().unwrap_or(false),
        "claude_code" => settings.claude_code = change.value.as_bool().unwrap_or(false),
        "shown" => {
            if let Ok(shown) = serde_json::from_value(change.value) {
                settings.shown = shown;
            }
        }
        "remove" => {
            let id = change.value.as_str().unwrap_or_default();
            let devices: Vec<Device> = state::devices()
                .into_iter()
                .filter(|d| d.id != id)
                .collect();
            if let Err(e) = state::save_devices(&devices) {
                return Response::error(500, &format!("{e:#}"));
            }
        }
        _ => return Response::error(400, "Not a change the pairing page makes."),
    }
    if let Err(e) = state::save_settings(&settings) {
        return Response::error(500, &format!("{e:#}"));
    }
    page_state(c, request)
}

fn same_origin(request: &Request) -> bool {
    match request.header("origin") {
        None => true,
        Some(origin) => {
            origin == format!("http://127.0.0.1:{}", super::port())
                || origin == format!("http://localhost:{}", super::port())
        }
    }
}

#[derive(Deserialize)]
struct Claim {
    code: String,
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Paired {
    token: String,
    id: String,
    name: String,
    mac: String,
}

/// The iPad's half of pairing: the code shown on the Mac, for a token.
pub fn claim(c: &Companion, request: &Request) -> Response {
    let claim: Claim = match request.json() {
        Ok(claim) => claim,
        Err(e) => return Response::error(400, &e),
    };
    let now = memory::time::now_ms();
    {
        let mut pairing = lock(&c.pairing);
        let Some(code) = pairing.as_mut().filter(|code| code.expires_at > now) else {
            return Response::error(
                403,
                "There is no code to pair with. On the Mac, choose Pair an iPad from Freebyrd's menu-bar icon.",
            );
        };
        let typed: String = claim.code.chars().filter(char::is_ascii_digit).collect();
        if !state::same_secret(&typed, &code.code) {
            code.tries += 1;
            let left = TRIES.saturating_sub(code.tries);
            if left == 0 {
                *pairing = None;
            }
            drop(pairing);
            std::thread::sleep(Duration::from_millis(400));
            return Response::error(
                403,
                &if left == 0 {
                    "That code was wrong too often. Make a new one on the Mac.".to_owned()
                } else {
                    format!("That code is not the one on the Mac. {left} tries left.")
                },
            );
        }
        *pairing = None;
    }
    let name = claim.name.trim();
    let device = Device {
        id: state::random_hex(6),
        name: if name.is_empty() {
            "iPad".to_owned()
        } else {
            name.chars().take(40).collect()
        },
        token: state::random_hex(32),
        paired_at: now,
        last_seen_at: now,
        push: None,
    };
    let mut devices = state::devices();
    devices.push(device.clone());
    if let Err(e) = state::save_devices(&devices) {
        return Response::error(500, &format!("Could not save the pairing: {e:#}"));
    }
    eprintln!("freebyrd: companion: paired {}", device.name);
    Response::json(&Paired {
        token: device.token,
        id: device.id,
        name: device.name,
        mac: mac_name(),
    })
}

/// The paired iPad a request comes from, by its token. Noting when it was
/// last seen writes at most once a minute.
pub fn device(request: &Request) -> Option<Device> {
    let token = request.token()?;
    let mut devices = state::devices();
    let index = devices
        .iter()
        .position(|d| state::same_secret(&d.token, token))?;
    let now = memory::time::now_ms();
    if now - devices[index].last_seen_at > 60_000 {
        devices[index].last_seen_at = now;
        let _ = state::save_devices(&devices);
    }
    Some(devices.swap_remove(index))
}

/// This Mac's name, as the iPad shows it ("Abhishek's MacBook Pro").
pub fn mac_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        run("/usr/sbin/scutil", &["--get", "ComputerName"]).unwrap_or_else(|| "Your Mac".to_owned())
    })
    .clone()
}

/// Where an iPad on the same Wi-Fi reaches this Mac.
fn lan_addresses() -> Vec<String> {
    let mut found = Vec::new();
    for interface in ["en0", "en1"] {
        if let Some(ip) = run("/usr/sbin/ipconfig", &["getifaddr", interface]) {
            found.push(format!("http://{ip}:{LAN_PORT}"));
        }
    }
    if let Some(name) = run("/usr/sbin/scutil", &["--get", "LocalHostName"]) {
        found.push(format!("http://{name}.local:{LAN_PORT}"));
    }
    found
}

/// Tailscale, if it is installed: this Mac's name on the person's tailnet,
/// and whether `tailscale serve` passes HTTPS on to the companion.
fn tailscale() -> serde_json::Value {
    if !std::path::Path::new(TAILSCALE).exists() {
        return json!({ "installed": false });
    }
    let status = run(TAILSCALE, &["status", "--json"])
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let name = status
        .as_ref()
        .and_then(|status| status["Self"]["DNSName"].as_str())
        .map(|name| name.trim_end_matches('.').to_owned());
    let serving = run(TAILSCALE, &["serve", "status", "--json"]).is_some_and(|text| {
        text.contains(&format!("127.0.0.1:{PORT}")) || text.contains(&format!("localhost:{PORT}"))
    });
    json!({
        "installed": true,
        "name": name,
        "url": name.as_ref().map(|name| format!("https://{name}")),
        "serving": serving,
        "command": format!("{TAILSCALE} serve --bg {PORT}"),
    })
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}
