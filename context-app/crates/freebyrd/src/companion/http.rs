//! A small HTTP/1.1 server for the iPad companion: one thread a connection,
//! one request a connection (`Connection: close`), bodies by length only.
//! Enough for one person's iPad, with nothing to download or configure.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;

/// The largest body taken: a photo or a sketch, as base64 in JSON.
const MAX_BODY: usize = 24 * 1024 * 1024;
const MAX_HEAD: usize = 32 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Which socket a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    /// 127.0.0.1: this Mac, or Tailscale's `serve` passing a request on.
    Local,
    /// The Wi-Fi, when the person turned it on.
    Lan,
}

pub struct Request {
    pub method: String,
    pub path: String,
    /// Names in lower case.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub peer: SocketAddr,
    pub door: Door,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Typed on this Mac: the loopback socket, from the loopback address, not
    /// passed on by Tailscale (which adds these headers).
    pub fn is_from_this_mac(&self) -> bool {
        self.door == Door::Local
            && self.peer.ip().is_loopback()
            && self.header("x-forwarded-for").is_none()
            && self.header("tailscale-user-login").is_none()
    }

    /// The bearer token, if any.
    pub fn token(&self) -> Option<&str> {
        self.header("authorization")?
            .strip_prefix("Bearer ")
            .map(str::trim)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        let body = if self.body.is_empty() {
            b"{}".as_slice()
        } else {
            &self.body
        };
        serde_json::from_slice(body).map_err(|e| format!("That request was not understood: {e}"))
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_owned(), content_type.to_owned())],
            body: body.into(),
        }
    }

    pub fn json(value: &impl Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self::new(200, "application/json", body).no_store(),
            Err(e) => Self::error(500, &format!("Could not send that: {e}")),
        }
    }

    /// `{"error": message}`, which the page shows as text.
    pub fn error(status: u16, message: &str) -> Self {
        let body = serde_json::to_vec(&serde_json::json!({ "error": message })).unwrap_or_default();
        Self::new(status, "application/json", body).no_store()
    }

    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn no_store(self) -> Self {
        self.with("Cache-Control", "no-store")
    }
}

pub type Handler = dyn Fn(&Request) -> Response + Send + Sync;

/// Accepts connections on `listener` until the process ends. `admit` is
/// asked for each connection; a refused one is closed unanswered.
pub fn serve(
    listener: TcpListener,
    door: Door,
    admit: Arc<dyn Fn() -> bool + Send + Sync>,
    handler: Arc<Handler>,
) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if !admit() {
            continue;
        }
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            if let Err(e) = handle(stream, door, &*handler)
                && !matches!(
                    e.kind(),
                    io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                )
            {
                eprintln!("freebyrd: companion: {e}");
            }
        });
    }
}

fn handle(stream: TcpStream, door: Door, handler: &Handler) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let peer = stream.peer_addr()?;
    let mut writer = stream.try_clone()?;
    let response = match read_request(stream, peer, door) {
        Ok(Some(request)) => {
            let head = request.method == "HEAD";
            let mut response = handler(&request);
            if head {
                response.body.clear();
            }
            response
        }
        Ok(None) => return Ok(()),
        Err(Bad(status, message)) => Response::error(status, message),
    };
    write_response(&mut writer, &response)
}

struct Bad(u16, &'static str);

fn read_request(stream: TcpStream, peer: SocketAddr, door: Door) -> Result<Option<Request>, Bad> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut read = 0;
    let mut next_line =
        |reader: &mut BufReader<TcpStream>, line: &mut String| -> Result<usize, Bad> {
            line.clear();
            let n = reader
                .read_line(line)
                .map_err(|_| Bad(400, "The request could not be read."))?;
            read += n;
            if read > MAX_HEAD {
                return Err(Bad(431, "The request's headers are too long."));
            }
            Ok(n)
        };
    if next_line(&mut reader, &mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(Bad(400, "That is not an HTTP request."));
    };
    let method = method.to_owned();
    let path = percent_decode(target.split_once('?').map_or(target, |(path, _)| path));

    let mut headers = HashMap::new();
    loop {
        if next_line(&mut reader, &mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        return Err(Bad(411, "Send the body with a length."));
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(Bad(413, "That is too large to send."));
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| Bad(400, "The request's body was cut short."))?;
    Ok(Some(Request {
        method,
        path,
        headers,
        body,
        peer,
        door,
    }))
}

fn write_response(writer: &mut TcpStream, response: &Response) -> io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", response.status);
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\r\n",
        response.body.len()
    ));
    writer.write_all(head.as_bytes())?;
    writer.write_all(&response.body)?;
    writer.flush()
}

/// `%20` and the like decoded. A path's `+` stays a `+`.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if let Some(hex) = bytes.get(i + 1..i + 3)
                    && hex.iter().all(u8::is_ascii_hexdigit)
                    && let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(hex), 16)
                {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b+c"), "a b+c");
        assert_eq!(percent_decode("%E2%9C%93"), "\u{2713}");
        // A broken escape stays as it was.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
