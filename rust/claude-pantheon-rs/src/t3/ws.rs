//! A small RFC 6455 WebSocket client on gio's async sockets, so the whole protocol stays on the GLib
//! main loop: no tokio, no extra threads. Only what the T3 server needs: text frames, ping/pong,
//! close, and the 426 "update your client" answer on the upgrade.

use gtk::glib;
use gtk::{gio, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug)]
pub enum WsError {
    Io(String),
    /// The server refused the upgrade with HTTP `status` and this body (426 carries the update message).
    Refused {
        status: u16,
        body: String,
    },
    Closed,
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsError::Io(e) => write!(f, "{e}"),
            WsError::Refused { status, body } => write!(f, "HTTP {status}: {body}"),
            WsError::Closed => write!(f, "connection closed"),
        }
    }
}

impl From<glib::Error> for WsError {
    fn from(e: glib::Error) -> Self {
        WsError::Io(e.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<u16>),
}

/// Parsed `ws://host:port/path?query` (the only form the T3 server speaks on loopback).
pub struct WsUrl {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl WsUrl {
    pub fn parse(url: &str) -> Option<WsUrl> {
        let rest = url.strip_prefix("ws://").or_else(|| url.strip_prefix("http://"))?;
        let (hostport, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (hostport.to_string(), 80),
        };
        Some(WsUrl { host, port, path: path.to_string() })
    }
}

pub struct WebSocket {
    input: gio::InputStream,
    output: gio::OutputStream,
    _conn: gio::SocketConnection,
    buf: RefCell<Vec<u8>>,
    partial: RefCell<Option<(u8, Vec<u8>)>>,
}

fn accept_key(key: &str) -> String {
    let mut sum = glib::Checksum::new(glib::ChecksumType::Sha1).expect("sha1");
    sum.update(key.as_bytes());
    sum.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    glib::base64_encode(&sum.digest()).to_string()
}

fn random_bytes(n: usize) -> Vec<u8> {
    (0..n).map(|_| (glib::random_int() & 0xff) as u8).collect()
}

/// Encodes one client frame (always masked, as the RFC requires of clients).
pub fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | (opcode & 0x0f));
    let len = payload.len();
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len < 65_536 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// (fin, opcode, payload, bytes consumed).
pub type DecodedFrame = (bool, u8, Vec<u8>, usize);

/// One server frame from the front of `buf`, or None if more bytes are needed.
pub fn decode_frame(buf: &[u8]) -> Result<Option<DecodedFrame>, WsError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let fin = buf[0] & 0x80 != 0;
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    let mut len = (buf[1] & 0x7f) as usize;
    let mut at = 2;
    if len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        at = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize;
        at = 10;
    }
    let mut mask = [0u8; 4];
    if masked {
        if buf.len() < at + 4 {
            return Ok(None);
        }
        mask.copy_from_slice(&buf[at..at + 4]);
        at += 4;
    }
    if buf.len() < at + len {
        return Ok(None);
    }
    let mut payload = buf[at..at + len].to_vec();
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Some((fin, opcode, payload, at + len)))
}

impl WebSocket {
    /// Opens the socket and performs the HTTP upgrade. Any non-101 answer comes back as `Refused`.
    pub async fn connect(url: &str) -> Result<Rc<WebSocket>, WsError> {
        let parsed = WsUrl::parse(url).ok_or_else(|| WsError::Io(format!("bad WebSocket URL {url}")))?;
        let client = gio::SocketClient::new();
        client.set_timeout(15);
        let conn = client.connect_to_host_future(&format!("{}:{}", parsed.host, parsed.port), parsed.port).await?;
        // The 15 s applies to the connect; a stream then waits as long as the server stays quiet.
        conn.socket().set_timeout(0);
        let input = conn.input_stream();
        let output = conn.output_stream();
        let key = glib::base64_encode(&random_bytes(16)).to_string();
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: claude-pantheon-rs\r\n\r\n",
            parsed.path, parsed.host, parsed.port, key
        );
        output.write_all_future(request.into_bytes(), glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;

        // Read the response head.
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let head_end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i + 4;
            }
            if buf.len() > 64 * 1024 {
                return Err(WsError::Io("upgrade response too large".into()));
            }
            let (chunk, n) = input.read_future(vec![0u8; 8192], glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
            if n == 0 {
                return Err(WsError::Closed);
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        if status != 101 {
            // Pull the body so the update message can be shown.
            let content_length = head
                .lines()
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = buf[head_end..].to_vec();
            while body.len() < content_length {
                let (chunk, n) =
                    input.read_future(vec![0u8; 8192], glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            return Err(WsError::Refused { status, body: String::from_utf8_lossy(&body).into_owned() });
        }
        let accept = head
            .lines()
            .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-accept")))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        if accept != accept_key(&key) {
            return Err(WsError::Io("bad Sec-WebSocket-Accept".into()));
        }
        let leftover = buf[head_end..].to_vec();
        Ok(Rc::new(WebSocket { input, output, _conn: conn, buf: RefCell::new(leftover), partial: RefCell::new(None) }))
    }

    pub async fn send_text(&self, text: &str) -> Result<(), WsError> {
        self.send_raw(1, text.as_bytes()).await
    }

    pub async fn send_pong(&self, payload: &[u8]) -> Result<(), WsError> {
        self.send_raw(10, payload).await
    }

    pub async fn send_ping(&self) -> Result<(), WsError> {
        self.send_raw(9, b"").await
    }

    pub async fn send_close(&self) -> Result<(), WsError> {
        self.send_raw(8, &1000u16.to_be_bytes()).await
    }

    async fn send_raw(&self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mask: [u8; 4] = random_bytes(4).try_into().unwrap();
        let frame = encode_frame(opcode, payload, mask);
        self.output.write_all_future(frame, glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
        Ok(())
    }

    /// The next complete message. Fragmented messages are joined; control frames come through as is.
    pub async fn next(&self) -> Result<Frame, WsError> {
        loop {
            let decoded = decode_frame(&self.buf.borrow())?;
            if let Some((fin, opcode, payload, used)) = decoded {
                self.buf.borrow_mut().drain(..used);
                match opcode {
                    0 => {
                        let mut partial = self.partial.borrow_mut();
                        let Some((first, mut acc)) = partial.take() else { continue };
                        acc.extend_from_slice(&payload);
                        if fin {
                            return Ok(finish(first, acc));
                        }
                        *partial = Some((first, acc));
                    }
                    1 | 2 => {
                        if fin {
                            return Ok(finish(opcode, payload));
                        }
                        *self.partial.borrow_mut() = Some((opcode, payload));
                    }
                    8 => {
                        let code = (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                        return Ok(Frame::Close(code));
                    }
                    9 => return Ok(Frame::Ping(payload)),
                    10 => return Ok(Frame::Pong(payload)),
                    _ => continue,
                }
                continue;
            }
            let (chunk, n) =
                self.input.read_future(vec![0u8; 64 * 1024], glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
            if n == 0 {
                return Err(WsError::Closed);
            }
            self.buf.borrow_mut().extend_from_slice(&chunk[..n]);
        }
    }
}

fn finish(opcode: u8, payload: Vec<u8>) -> Frame {
    if opcode == 1 { Frame::Text(String::from_utf8_lossy(&payload).into_owned()) } else { Frame::Binary(payload) }
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = encode_frame(1, b"hello", [1, 2, 3, 4]);
        let (fin, op, payload, used) = decode_frame(&frame).unwrap().unwrap();
        assert!(fin);
        assert_eq!(op, 1);
        assert_eq!(payload, b"hello");
        assert_eq!(used, frame.len());
        let big = vec![7u8; 70_000];
        let frame = encode_frame(2, &big, [0, 0, 0, 0]);
        let (_, _, payload, _) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(payload.len(), 70_000);
        assert!(decode_frame(&frame[..100]).unwrap().is_none());
    }

    #[test]
    fn accept_key_matches_rfc_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn urls_parse() {
        let u = WsUrl::parse("ws://127.0.0.1:3773/ws?wsTicket=x").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("127.0.0.1", 3773, "/ws?wsTicket=x"));
    }
}
