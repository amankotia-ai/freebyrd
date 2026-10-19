//! Freebyrd for iPad: a web app the person adds to their iPad's Home
//! Screen, served by this Mac. It shows what the notch shows (cards, the
//! to-do list, people, night shift, the day plan) and more that needs a
//! bigger screen (meeting briefs, a board of the day's Claude Code
//! sessions, the week, links shared to read, a sketch pad), and its buttons act
//! here, through the same memory functions the notch's buttons call.
//!
//! The server listens on 127.0.0.1 (`PORT`) only, which Tailscale's `serve`
//! passes HTTPS requests from the person's own devices on to, and, when the
//! person turns it on, on the Wi-Fi as well (`LAN_PORT`, plain HTTP).
//! Everything under `/api/` needs a paired iPad's token, which an iPad gets
//! once, with a code shown on this Mac (`/pair`, which answers this Mac
//! only). Nothing is kept anywhere but this Mac: the iPad caches what it was
//! last shown, so it still reads while the Mac sleeps.

mod api;
mod assets;
mod board;
mod code;
pub mod http;
mod pair;
mod push;
mod state;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;

use http::{Door, Request, Response};

/// The port on this Mac, and the one on the Wi-Fi.
pub const PORT: u16 = 4747;
pub const LAN_PORT: u16 = 4748;

/// The port on this Mac: `PORT`, or `FREEBYRD_COMPANION_PORT` for trying a
/// build with `--companion` while Freebyrd itself holds `PORT`.
pub fn port() -> u16 {
    std::env::var("FREEBYRD_COMPANION_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(PORT)
}

/// Shared by every request.
pub(crate) struct Companion {
    pub pairing: Mutex<Option<pair::Code>>,
    pub asks: api::Asks,
    /// The day plan made last from the iPad, with its summary line.
    pub plan: Mutex<Option<memory::day::DayPlan>>,
    /// Tasks sent to Claude Code from the iPad, newest last.
    pub handoffs: Mutex<Vec<code::Handoff>>,
    lan_bound: AtomicBool,
}

static COMPANION: OnceLock<Arc<Companion>> = OnceLock::new();

fn companion() -> &'static Arc<Companion> {
    COMPANION.get_or_init(|| {
        Arc::new(Companion {
            pairing: Mutex::new(None),
            asks: api::Asks::default(),
            plan: Mutex::new(None),
            handoffs: Mutex::new(Vec::new()),
            lan_bound: AtomicBool::new(false),
        })
    })
}

/// Starts the server and the notification loop, in the background. Logs and
/// carries on if the port is taken (another Freebyrd, or `--companion`).
pub fn start() {
    let handler: Arc<http::Handler> = Arc::new(|request: &Request| route(companion(), request));
    match TcpListener::bind(("127.0.0.1", port())) {
        Ok(listener) => {
            let handler = Arc::clone(&handler);
            thread::spawn(move || http::serve(listener, Door::Local, Arc::new(|| true), handler));
            eprintln!("freebyrd: companion: listening on http://127.0.0.1:{}", port());
        }
        Err(e) => eprintln!("freebyrd: companion: port {} is taken ({e}); the iPad app is off", port()),
    }
    if state::settings().lan {
        open_lan();
    }
    push::start();
}

/// `freebyrd --companion`: the server alone, without the notch, for trying
/// the iPad app in a browser. Runs until stopped.
pub fn run_foreground() -> i32 {
    start();
    eprintln!(
        "freebyrd: companion: open http://127.0.0.1:{}/pair on this Mac to pair; Ctrl-C stops it",
        port()
    );
    loop {
        thread::park();
    }
}

/// Starts answering on the Wi-Fi, once. Turning it off is checked per
/// connection (`lan_allowed`), so the socket can stay bound.
fn open_lan() {
    let c = companion();
    if c.lan_bound.swap(true, Ordering::SeqCst) {
        return;
    }
    match TcpListener::bind(("0.0.0.0", LAN_PORT)) {
        Ok(listener) => {
            let handler: Arc<http::Handler> =
                Arc::new(|request: &Request| route(companion(), request));
            thread::spawn(move || {
                http::serve(
                    listener,
                    Door::Lan,
                    Arc::new(|| state::settings().lan),
                    handler,
                );
            });
            eprintln!("freebyrd: companion: also on the Wi-Fi, port {LAN_PORT}");
        }
        Err(e) => {
            c.lan_bound.store(false, Ordering::SeqCst);
            eprintln!("freebyrd: companion: port {LAN_PORT} is taken ({e}); not on the Wi-Fi");
        }
    }
}

/// The address an iPad may use: this Mac's own name for it, the Wi-Fi
/// address on `LAN_PORT`, or a Tailscale name. Anything else is turned away,
/// so a web page cannot reach the server under a name of its own (DNS
/// rebinding).
fn host_allowed(request: &Request) -> bool {
    let Some(host) = request.header("host") else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) => (name.to_owned(), port.parse::<u16>().ok()),
        None => (host.clone(), None),
    };
    match request.door {
        Door::Local => {
            let local = (name == "127.0.0.1" || name == "localhost") && port == Some(self::port());
            let tailscale = name.ends_with(".ts.net") && port.is_none_or(|port| port == 443);
            local || tailscale
        }
        Door::Lan => {
            port == Some(LAN_PORT)
                && (name
                    .parse::<std::net::Ipv4Addr>()
                    .is_ok_and(|ip| ip.is_private())
                    || name.ends_with(".local"))
        }
    }
}

fn route(c: &Companion, request: &Request) -> Response {
    if !host_allowed(request) {
        return Response::error(403, "This address is not one Freebyrd answers on.");
    }
    let method = request.method.as_str();
    let path = request.path.as_str();
    match (method, path) {
        ("GET", "/pair") => pair::page(c, request),
        ("GET", "/pair/state") => pair::page_state(c, request),
        ("POST", "/pair/settings") => {
            let response = pair::change(c, request);
            if state::settings().lan {
                open_lan();
            }
            response
        }
        ("POST", "/api/pair") => pair::claim(c, request),
        // A Shortcut on the iPad: what was shared, or said to Siri.
        ("POST", "/api/inbox") => match pair::device(request) {
            Some(device) => api::inbox(&device, request),
            None => unauthorized(),
        },
        (_, path) if path.starts_with("/api/") => match pair::device(request) {
            Some(device) => api::handle(c, &device, request),
            None => unauthorized(),
        },
        ("GET" | "HEAD", path) => assets::get(path),
        _ => Response::error(405, "Not something this server does."),
    }
}

fn unauthorized() -> Response {
    Response::error(
        401,
        "This iPad is not paired with the Mac. Pair it again from Settings.",
    )
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The memory: `FREEBYRD_COMPANION_DB` (a copy, for trying buttons out), or
/// the person's.
pub(crate) fn memory() -> anyhow::Result<memory::Store> {
    match std::env::var_os("FREEBYRD_COMPANION_DB") {
        Some(path) => memory::Store::open(&PathBuf::from(path)),
        None => memory::Store::open_default(),
    }
}

/// While watching is paused from the iPad: until when, in milliseconds.
/// The watcher reads nothing until then (`watcher.rs`); a double tap still
/// reads the screen, since the person asked.
pub fn paused_until() -> Option<i64> {
    let path = state::freebyrd_dir().ok()?.join("paused-until");
    let until: i64 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    (until > memory::time::now_ms()).then_some(until)
}

pub(crate) fn set_paused_until(until: Option<i64>) -> anyhow::Result<()> {
    let path = state::freebyrd_dir()?.join("paused-until");
    match until {
        Some(until) => std::fs::write(path, until.to_string())?,
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn request(door: Door, host: &str) -> Request {
        Request {
            method: "GET".to_owned(),
            path: "/".to_owned(),
            headers: HashMap::from([("host".to_owned(), host.to_owned())]),
            body: Vec::new(),
            peer: "127.0.0.1:5000".parse().unwrap(),
            door,
        }
    }

    #[test]
    fn answers_only_its_own_names() {
        assert!(host_allowed(&request(Door::Local, "127.0.0.1:4747")));
        assert!(host_allowed(&request(Door::Local, "localhost:4747")));
        assert!(host_allowed(&request(Door::Local, "mac.tail1234.ts.net")));
        assert!(host_allowed(&request(Door::Lan, "192.168.1.20:4748")));
        assert!(host_allowed(&request(Door::Lan, "my-mac.local:4748")));
        // A web page's own name, pointed at this Mac.
        assert!(!host_allowed(&request(Door::Local, "evil.example:4747")));
        assert!(!host_allowed(&request(Door::Lan, "evil.example:4748")));
        // A public address on the Wi-Fi port, or the Mac's port on the Wi-Fi.
        assert!(!host_allowed(&request(Door::Lan, "8.8.8.8:4748")));
        assert!(!host_allowed(&request(Door::Lan, "192.168.1.20:4747")));
        assert!(!host_allowed(&request(Door::Local, "127.0.0.1:80")));
    }
}
