//! Just enough HTTP/1.1 for the auth endpoints: one request per connection, `Connection: close`,
//! bodies read to EOF or Content-Length. Loopback only, no TLS.

use super::ws::{WsError, WsUrl, find};
use gtk::glib;
use gtk::{gio, prelude::*};

pub struct Response {
    pub status: u16,
    pub body: String,
}

pub async fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Result<Response, WsError> {
    let parsed = WsUrl::parse(url).ok_or_else(|| WsError::Io(format!("bad URL {url}")))?;
    let client = gio::SocketClient::new();
    client.set_timeout(20);
    let conn = client.connect_to_host_future(&format!("{}:{}", parsed.host, parsed.port), parsed.port).await?;
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nUser-Agent: claude-pantheon-rs\r\n",
        method, parsed.path, parsed.host, parsed.port
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    let mut bytes = Vec::new();
    match body {
        Some(b) => {
            head.push_str(&format!("Content-Length: {}\r\n\r\n", b.len()));
            bytes.extend_from_slice(head.as_bytes());
            bytes.extend_from_slice(b);
        }
        None => {
            head.push_str("\r\n");
            bytes.extend_from_slice(head.as_bytes());
        }
    }
    let output = conn.output_stream();
    output.write_all_future(bytes, glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
    let input = conn.input_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(16 * 1024);
    loop {
        let (chunk, n) = input.read_future(vec![0u8; 64 * 1024], glib::Priority::DEFAULT).await.map_err(|(_, e)| e)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 * 1024 {
            return Err(WsError::Io("response too large".into()));
        }
    }
    let head_end = find(&buf, b"\r\n\r\n").ok_or_else(|| WsError::Io("no HTTP response head".into()))? + 4;
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let chunked = head.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    });
    let raw = &buf[head_end..];
    let body = if chunked { dechunk(raw) } else { raw.to_vec() };
    Ok(Response { status, body: String::from_utf8_lossy(&body).into_owned() })
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    while let Some(nl) = find(raw, b"\r\n") {
        let size_line = String::from_utf8_lossy(&raw[..nl]);
        let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = nl + 2;
        if raw.len() < start + size {
            out.extend_from_slice(&raw[start..]);
            break;
        }
        out.extend_from_slice(&raw[start..start + size]);
        raw = &raw[(start + size + 2).min(raw.len())..];
    }
    out
}

/// `application/x-www-form-urlencoded` body.
pub fn form(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_encode() {
        assert_eq!(form(&[("a", "x y"), ("b", "urn:ietf:x")]), "a=x+y&b=urn%3Aietf%3Ax");
    }

    #[test]
    fn chunked_bodies_join() {
        assert_eq!(dechunk(b"5\r\nhello\r\n1\r\n!\r\n0\r\n\r\n"), b"hello!");
    }
}
