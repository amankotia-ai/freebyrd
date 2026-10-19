//! Native messaging host for the Freebyrd browser extension.
//!
//! Chrome starts this binary with the calling extension's origin as the first
//! argument and talks to it over stdin/stdout. While connected, it also
//! listens on a Unix socket so the desktop app can ask for the current page
//! (see `relay`). Run it yourself with `install` or `uninstall` to register it
//! with Chrome.

mod framing;
mod relay;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use capture::bridge::{self, BridgeResponse};
use capture::browser::{ExtensionMessage, HostMessage, PageSnapshot, PageStore};
use relay::{Relay, Socket};

const HOST_NAME: &str = "com.freebyrd.browser_host";
/// Fixed by the public key in `extension/manifest.json`.
const DEFAULT_EXTENSION_ID: &str = "aoleafdlidoeidiadgoofcojpnenlpff";

const USAGE: &str = "\
usage:
  browser-host install [--extension-id ID] [--chrome-dir DIR]
  browser-host uninstall [--chrome-dir DIR]
  browser-host reload

DIR is Chrome's user data directory. It defaults to
~/Library/Application Support/Google/Chrome, which Arc also reads; pass the
--user-data-dir of a separate Chrome profile to register only there.

reload asks every connected browser to reload the unpacked extension.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some(origin) if origin.starts_with("chrome-extension://") => serve(origin),
        Some("install") => install(&args[1..]),
        Some("uninstall") => uninstall(&args[1..]),
        Some("reload") => reload(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn serve(origin: &str) -> Result<()> {
    let dir = PageStore::default_dir().context("HOME is not set")?;
    let store = PageStore::open(&dir).with_context(|| format!("opening {}", dir.display()))?;
    // Chrome launches the host directly, so the parent is the browser process
    // a desktop client sees as the frontmost app.
    let browser_pid = std::os::unix::process::parent_id();
    let relay = Relay::new(Box::new(io::stdout()), Log::open(store.dir()));
    relay
        .log
        .line(&format!("started for {origin}, browser pid {browser_pid}"));

    // On-demand reads are optional; snapshots still flow without the socket.
    let _socket = match bridge::socket_path(browser_pid).and_then(|path| Socket::bind(path, &relay))
    {
        Ok(socket) => Some(socket),
        Err(e) => {
            relay.log.line(&format!("on-demand reads unavailable: {e}"));
            None
        }
    };

    let mut stdin = io::stdin().lock();
    while let Some(body) = framing::read_message(&mut stdin)? {
        let reply = match serde_json::from_slice::<ExtensionMessage>(&body) {
            Ok(ExtensionMessage::Hello { version }) => {
                relay.log.line(&format!("extension {version} connected"));
                Some(HostMessage::Welcome {
                    host_version: env!("CARGO_PKG_VERSION").to_owned(),
                })
            }
            Ok(ExtensionMessage::Page(page)) => Some(record(&store, &relay.log, &page)),
            Ok(ExtensionMessage::PageResponse {
                request_id,
                page,
                error,
            }) => {
                let response = match page {
                    Some(page) => {
                        record(&store, &relay.log, &page);
                        BridgeResponse::Page {
                            page: Box::new(page),
                        }
                    }
                    None => {
                        let reason = error.unwrap_or_else(|| "unknown".to_owned());
                        relay
                            .log
                            .line(&format!("request {request_id} declined: {reason}"));
                        BridgeResponse::Refused { reason }
                    }
                };
                relay.resolve(&request_id, response);
                None
            }
            Err(e) => {
                relay.log.line(&format!("invalid message: {e}"));
                Some(HostMessage::Error {
                    message: format!("invalid message: {e}"),
                })
            }
        };
        if let Some(reply) = reply {
            relay.send(&reply)?;
        }
    }
    relay.log.line("extension disconnected");
    Ok(())
}

fn record(store: &PageStore, log: &Log, page: &PageSnapshot) -> HostMessage {
    match store.record(page) {
        Ok(stored) => {
            log.line(&format!(
                "{} {} chars={} stored={stored}",
                page.reason,
                page.url,
                page.text.chars().count()
            ));
            HostMessage::Ack { stored }
        }
        Err(e) => {
            log.line(&format!("store failed: {e}"));
            HostMessage::Error {
                message: format!("could not store page: {e}"),
            }
        }
    }
}

/// Appends to `host.log` next to the store. Chrome discards a host's stderr,
/// so this is the only place errors show up.
pub struct Log(Option<fs::File>);

impl Log {
    fn open(dir: &Path) -> Self {
        Self(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("host.log"))
                .ok(),
        )
    }

    pub fn line(&self, text: &str) {
        let Some(mut file) = self.0.as_ref() else {
            return;
        };
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let _ = writeln!(file, "{ms} {text}");
    }
}

fn install(args: &[String]) -> Result<()> {
    let extension_id = flag(args, "--extension-id")?.unwrap_or(DEFAULT_EXTENSION_ID.to_owned());
    if extension_id.len() != 32 || !extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        bail!("{extension_id:?} is not a Chrome extension ID (32 letters a-p)");
    }
    let host_path = std::env::current_exe()?.canonicalize()?;
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "Freebyrd browser bridge",
        "path": host_path,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    });

    let path = manifest_path(args)?;
    fs::create_dir_all(path.parent().expect("manifest path has a parent"))?;
    fs::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
    println!("registered {HOST_NAME} for extension {extension_id}");
    println!("manifest: {}", path.display());
    println!("host:     {}", host_path.display());
    Ok(())
}

fn uninstall(args: &[String]) -> Result<()> {
    let path = manifest_path(args)?;
    match fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            println!("not registered at {}", path.display())
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn reload() -> Result<()> {
    let browsers = bridge::connected_browsers()?;
    if browsers.is_empty() {
        bail!("no browser has the extension connected");
    }
    for pid in browsers {
        match bridge::reload_extension(pid, Duration::from_secs(2)) {
            Ok(()) => println!("browser {pid}: reloading the extension"),
            Err(e) => println!("browser {pid}: {e}"),
        }
    }
    Ok(())
}

fn manifest_path(args: &[String]) -> Result<PathBuf> {
    let chrome_dir = match flag(args, "--chrome-dir")? {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").context("HOME is not set")?;
            Path::new(&home).join("Library/Application Support/Google/Chrome")
        }
    };
    Ok(chrome_dir
        .join("NativeMessagingHosts")
        .join(format!("{HOST_NAME}.json")))
}

fn flag(args: &[String], name: &str) -> Result<Option<String>> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .cloned()
            .map(Some)
            .with_context(|| format!("{name} needs a value")),
        None => Ok(None),
    }
}
