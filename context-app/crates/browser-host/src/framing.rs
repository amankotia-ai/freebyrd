//! Chrome native messaging framing: each message is a 32-bit length in
//! native byte order followed by that many bytes of UTF-8 JSON.

use std::io::{self, Read, Write};

use serde::Serialize;

/// Chrome's limit for messages sent to a host.
const MAX_INCOMING_BYTES: usize = 64 * 1024 * 1024;
/// Chrome's limit for messages sent from a host.
const MAX_OUTGOING_BYTES: usize = 1024 * 1024;

/// Reads one message. Returns `None` when Chrome closes the pipe.
pub fn read_message(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_INCOMING_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message of {len} bytes exceeds the {MAX_INCOMING_BYTES}-byte limit"),
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_message(writer: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    if body.len() > MAX_OUTGOING_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("reply of {} bytes exceeds Chrome's 1 MB limit", body.len()),
        ));
    }
    writer.write_all(&(body.len() as u32).to_ne_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_message() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &serde_json::json!({"type": "ack", "stored": true}),
        )
        .unwrap();
        assert_eq!(&buf[..4], &(buf.len() as u32 - 4).to_ne_bytes());

        let mut reader = buf.as_slice();
        let body = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(body, br#"{"stored":true,"type":"ack"}"#);
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_length() {
        let len = (MAX_INCOMING_BYTES as u32 + 1).to_ne_bytes();
        let err = read_message(&mut len.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
