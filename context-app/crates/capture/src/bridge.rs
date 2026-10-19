//! On-demand page reads through the browser extension.
//!
//! Each running browser starts its own native host, and each host listens on
//! a Unix socket named after that browser's process ID. A desktop client that
//! knows which browser is in front connects to that socket, sends one
//! request line, and reads one response line.

use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::browser::PageSnapshot;

/// `sun_path` holds 104 bytes on macOS, including the trailing NUL.
const MAX_SOCKET_PATH_BYTES: usize = 103;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BridgeRequest {
    /// `front_at_ms` is when the client saw the browser in front (Unix
    /// milliseconds). The extension then reads the window that had focus at
    /// that moment, and declines with `hidden-window` if that window was one
    /// it cannot see (incognito, another profile).
    #[serde(rename_all = "camelCase")]
    ReadActivePage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        front_at_ms: Option<u64>,
    },
    /// Reloads the unpacked extension so it picks up a new build.
    ReloadExtension,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BridgeResponse {
    Page {
        page: Box<PageSnapshot>,
    },
    /// The extension or host declined: `paused`, `excluded:...`, `incognito`,
    /// `hidden-window`, `no-active-tab`, `unreadable`, or `extension-timeout`.
    Refused {
        reason: String,
    },
    /// A request without a result was passed on.
    Done,
}

/// `~/Library/Application Support/Freebyrd/run`, created owner-only.
pub fn socket_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let dir = Path::new(&home).join("Library/Application Support/Freebyrd/run");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    Ok(dir)
}

pub fn socket_path(browser_pid: u32) -> io::Result<PathBuf> {
    let path = socket_dir()?.join(format!("{browser_pid}.sock"));
    if path.as_os_str().len() > MAX_SOCKET_PATH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("socket path is too long for macOS: {}", path.display()),
        ));
    }
    Ok(path)
}

#[derive(Debug)]
pub enum ReadError {
    /// No extension is connected for that browser.
    NoBridge,
    Refused(String),
    Timeout,
    Protocol(String),
    Io(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBridge => write!(f, "the browser extension is not connected for this browser"),
            Self::Refused(reason) => write!(f, "the extension declined: {reason}"),
            Self::Timeout => write!(f, "the browser did not answer in time"),
            Self::Protocol(detail) => write!(f, "unexpected reply from the bridge: {detail}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Self::Timeout,
            _ => Self::Io(e),
        }
    }
}

/// Asks the browser with process ID `browser_pid` for the page in its
/// focused window's active tab: the window focused at `front_at_ms`, when
/// given (see [`BridgeRequest::ReadActivePage`]).
pub fn read_active_page(
    browser_pid: u32,
    front_at_ms: Option<u64>,
    timeout: Duration,
) -> Result<PageSnapshot, ReadError> {
    let request = BridgeRequest::ReadActivePage { front_at_ms };
    match send(browser_pid, &request, timeout)? {
        BridgeResponse::Page { page } => Ok(*page),
        BridgeResponse::Refused { reason } => Err(ReadError::Refused(reason)),
        BridgeResponse::Done => Err(ReadError::Protocol("expected a page".to_owned())),
    }
}

/// Reloads the extension in the browser with process ID `browser_pid`. Its
/// host exits and a new one starts once the extension reconnects.
pub fn reload_extension(browser_pid: u32, timeout: Duration) -> Result<(), ReadError> {
    match send(browser_pid, &BridgeRequest::ReloadExtension, timeout)? {
        BridgeResponse::Done => Ok(()),
        BridgeResponse::Refused { reason } => Err(ReadError::Refused(reason)),
        BridgeResponse::Page { .. } => Err(ReadError::Protocol("unexpected page".to_owned())),
    }
}

/// Browser process IDs with a connected extension, from the socket names.
/// A listed socket can still be stale; requests to it return `NoBridge`.
pub fn connected_browsers() -> io::Result<Vec<u32>> {
    let mut pids: Vec<u32> = fs::read_dir(socket_dir()?)?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            name.to_str()?.strip_suffix(".sock")?.parse().ok()
        })
        .collect();
    pids.sort_unstable();
    Ok(pids)
}

fn send(
    browser_pid: u32,
    request: &BridgeRequest,
    timeout: Duration,
) -> Result<BridgeResponse, ReadError> {
    let path = socket_path(browser_pid)?;
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        // A missing file means no host; a refused connection means a host
        // that exited without removing its socket.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(ReadError::NoBridge);
        }
        Err(e) => return Err(e.into()),
    };
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut line = serde_json::to_vec(request).map_err(|e| ReadError::Protocol(e.to_string()))?;
    line.push(b'\n');
    stream.write_all(&line)?;

    let mut response = String::new();
    if BufReader::new(stream).read_line(&mut response)? == 0 {
        return Err(ReadError::Protocol(
            "the bridge closed without answering".to_owned(),
        ));
    }
    serde_json::from_str(&response).map_err(|e| ReadError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_carries_when_the_browser_was_in_front() {
        let request = BridgeRequest::ReadActivePage {
            front_at_ms: Some(42),
        };
        let line = serde_json::to_string(&request).unwrap();
        assert_eq!(line, r#"{"type":"readActivePage","frontAtMs":42}"#);
        // Older clients send no time.
        let old: BridgeRequest = serde_json::from_str(r#"{"type":"readActivePage"}"#).unwrap();
        assert!(matches!(
            old,
            BridgeRequest::ReadActivePage { front_at_ms: None }
        ));
    }
}
