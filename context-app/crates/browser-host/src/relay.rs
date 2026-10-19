//! Relays on-demand page reads from desktop clients to the extension.
//!
//! Clients connect to this host's Unix socket and send one `BridgeRequest`
//! line. The host forwards it to the extension with a request ID, waits for
//! the matching `PageResponse`, and writes one `BridgeResponse` line back.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use capture::bridge::{BridgeRequest, BridgeResponse};
use capture::browser::HostMessage;

use crate::Log;
use crate::framing;

/// How long a client has to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// How long to wait for the extension to read the page. Clients should wait
/// longer than this so they receive `extension-timeout` instead of timing out.
const EXTENSION_TIMEOUT: Duration = Duration::from_secs(3);

/// State shared by the stdin loop and the socket threads.
pub struct Relay {
    /// The pipe to the extension: stdout when Chrome runs the host.
    to_extension: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<String, Sender<BridgeResponse>>>,
    next_request: AtomicU64,
    pub log: Log,
}

impl Relay {
    pub fn new(to_extension: Box<dyn Write + Send>, log: Log) -> Arc<Self> {
        Arc::new(Self {
            to_extension: Mutex::new(to_extension),
            pending: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            log,
        })
    }

    /// Writes one message to the extension. Socket threads and the stdin
    /// loop both send, so writes are serialized.
    pub fn send(&self, message: &HostMessage) -> io::Result<()> {
        let mut out = self
            .to_extension
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        framing::write_message(&mut *out, message)
    }

    /// Hands the extension's answer to the client waiting for `request_id`.
    pub fn resolve(&self, request_id: &str, response: BridgeResponse) {
        let waiting = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(request_id);
        match waiting {
            Some(client) => {
                let _ = client.send(response);
            }
            None => self.log.line(&format!(
                "response {request_id} arrived after its client gave up"
            )),
        }
    }

    fn forget(&self, request_id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(request_id);
    }

    fn read_active_page(&self, front_at_ms: Option<u64>) -> io::Result<BridgeResponse> {
        self.ask_extension(
            |request_id| HostMessage::ReadActivePage {
                request_id,
                front_at_ms,
            },
            EXTENSION_TIMEOUT,
        )
    }

    /// Sends the message `make` builds around a new request ID, and waits up
    /// to `timeout` for the extension's answer.
    fn ask_extension(
        &self,
        make: impl FnOnce(String) -> HostMessage,
        timeout: Duration,
    ) -> io::Result<BridgeResponse> {
        let request_id = self
            .next_request
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(request_id.clone(), tx);

        if let Err(e) = self.send(&make(request_id.clone())) {
            self.forget(&request_id);
            return Err(e);
        }
        Ok(rx.recv_timeout(timeout).unwrap_or_else(|_| {
            self.forget(&request_id);
            BridgeResponse::Refused {
                reason: "extension-timeout".to_owned(),
            }
        }))
    }
}

/// The listening socket. Dropping it removes the socket file, unless another
/// host has since replaced it.
pub struct Socket {
    path: PathBuf,
    inode: u64,
}

impl Socket {
    pub fn bind(path: PathBuf, relay: &Arc<Relay>) -> io::Result<Self> {
        // A host that crashed leaves its file behind; binding needs it gone.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let inode = fs::metadata(&path)?.ino();

        let relay = Arc::clone(relay);
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let relay = Arc::clone(&relay);
                        thread::spawn(move || serve_client(&stream, &relay));
                    }
                    Err(e) => relay.log.line(&format!("accept failed: {e}")),
                }
            }
        });
        Ok(Self { path, inode })
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        if fs::metadata(&self.path).is_ok_and(|m| m.ino() == self.inode) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn serve_client(stream: &UnixStream, relay: &Relay) {
    let response = answer(stream, relay).unwrap_or_else(|e| BridgeResponse::Refused {
        reason: format!("bad-request: {e}"),
    });
    let Ok(mut line) = serde_json::to_vec(&response) else {
        return;
    };
    line.push(b'\n');
    let mut stream = stream;
    let _ = stream.write_all(&line);
}

fn answer(stream: &UnixStream, relay: &Relay) -> io::Result<BridgeResponse> {
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    match serde_json::from_str(&line)? {
        BridgeRequest::ReadActivePage { front_at_ms } => relay.read_active_page(front_at_ms),
        BridgeRequest::ReloadExtension => {
            relay.log.line("reloading the extension on request");
            relay.send(&HostMessage::ReloadExtension)?;
            Ok(BridgeResponse::Done)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use capture::browser::PageSnapshot;

    use super::*;

    /// Collects each framed message the relay sends to the extension.
    struct Outbox {
        buf: Vec<u8>,
        sent: Sender<Vec<u8>>,
    }

    impl Write for Outbox {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            let _ = self.sent.send(std::mem::take(&mut self.buf));
            Ok(())
        }
    }

    fn relay() -> (Arc<Relay>, Receiver<Vec<u8>>) {
        let (sent, outbox) = mpsc::channel();
        let relay = Relay::new(
            Box::new(Outbox {
                buf: Vec::new(),
                sent,
            }),
            Log(None),
        );
        (relay, outbox)
    }

    fn page() -> PageSnapshot {
        PageSnapshot {
            tab_id: 1,
            url: "https://example.com/".to_owned(),
            title: "Example".to_owned(),
            text: "Hello".to_owned(),
            truncated: false,
            headings: vec![],
            selection: String::new(),
            lang: None,
            reason: "requested".to_owned(),
            captured_at_ms: 1,
            conversation: None,
            in_view: String::new(),
            ask: None,
        }
    }

    fn ask(client: &mut UnixStream, request: &str) -> serde_json::Value {
        client.write_all(request.as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(&*client).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn routes_the_extension_answer_back_to_the_client() {
        let (relay, outbox) = relay();
        let (mut client, server) = UnixStream::pair().unwrap();
        let worker = Arc::clone(&relay);
        thread::spawn(move || serve_client(&server, &worker));

        // Play the extension: read the forwarded request, answer it by ID.
        let extension = thread::spawn(move || {
            let frame = outbox.recv_timeout(Duration::from_secs(2)).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&frame[4..]).unwrap();
            assert_eq!(request["type"], "readActivePage");
            assert_eq!(request["frontAtMs"], 1_700_000_000_000_u64);
            let id = request["requestId"].as_str().unwrap().to_owned();
            relay.resolve(
                &id,
                BridgeResponse::Page {
                    page: Box::new(page()),
                },
            );
        });

        let response = ask(
            &mut client,
            "{\"type\":\"readActivePage\",\"frontAtMs\":1700000000000}\n",
        );
        extension.join().unwrap();
        assert_eq!(response["type"], "page");
        assert_eq!(response["page"]["text"], "Hello");
    }

    #[test]
    fn refuses_a_malformed_request() {
        let (relay, _outbox) = relay();
        let (mut client, server) = UnixStream::pair().unwrap();
        thread::spawn(move || serve_client(&server, &relay));

        let response = ask(&mut client, "{\"type\":\"deleteEverything\"}\n");
        assert_eq!(response["type"], "refused");
        assert!(
            response["reason"]
                .as_str()
                .unwrap()
                .starts_with("bad-request")
        );
    }
}
