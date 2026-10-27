//! Small loopback-only client for the already-running M-Series OpenAI-compatible text API.
//! It never starts, stops, or configures the model worker.

use crate::{DESKTOP_RESERVE_BYTES, host::HostCapabilities};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const INTENTS: &[&str] = &[
    "Portrait",
    "Streamer scene",
    "Product photo",
    "Illustration",
    "Graphic or poster",
];

pub fn endpoint() -> String {
    std::env::var("M_SERIES_LLM_API").unwrap_or_else(|_| "http://127.0.0.1:8766/v1".into())
}

pub fn suggest_prompt(intent: &str, prompt: &str) -> Result<String, String> {
    check_memory_reserve()?;
    complete(
        "Help shape a concise image-generation prompt. Preserve the subject and intent. Treat college-age people as adults. Do not add nudity or sexualization. Return a suggested prompt and one short reason. Do not change model settings or submit a generation.",
        &format!("Prompt type: {intent}\nOriginal prompt: {prompt}"),
    )
}

pub fn suggest_intent(prompt: &str) -> Result<String, String> {
    check_memory_reserve()?;
    let options = INTENTS.join(", ");
    let suggestion = complete(
        "Choose the best single image prompt type from the user's list. Return only the exact label, with no explanation or punctuation. Do not rewrite the prompt. If it describes a livestream creator, choose Streamer scene. Treat college-age people as adults.",
        &format!("Available prompt types: {options}\nPrompt: {prompt}"),
    )?;
    INTENTS
        .iter()
        .find(|choice| suggestion.trim().eq_ignore_ascii_case(choice))
        .map(|s| (*s).to_string())
        .ok_or_else(|| {
            "The local assistant did not return one of the available prompt types.".into()
        })
}

fn check_memory_reserve() -> Result<(), String> {
    let host = HostCapabilities::capture();
    if host.available_bytes < DESKTOP_RESERVE_BYTES {
        return Err(
            "Prompt help is paused until the 6 GiB desktop memory reserve is available.".into(),
        );
    }
    Ok(())
}

fn complete(system: &str, user: &str) -> Result<String, String> {
    let base = endpoint();
    let models: Value = request(&base, "GET", "/models", None)?;
    let model = models
        .get("data")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "No model is available from the M-Series local assistant.".to_string())?;
    let body = json!({
        "model": model,
        "messages": [
            {"role":"system", "content":system},
            {"role":"user", "content":user}
        ],
        "stream": false,
        "temperature": 0.25
    });
    let response: Value = request(&base, "POST", "/chat/completions", Some(body))?;
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "The local assistant returned no prompt suggestion.".to_string())
}

fn request(base: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let (address, prefix) = parse_loopback_endpoint(base)?;
    let address = address
        .to_socket_addrs()
        .map_err(|e| format!("Invalid local assistant address: {e}"))?
        .next()
        .ok_or_else(|| "The local assistant address did not resolve.".to_string())?;
    if !address.ip().is_loopback() {
        return Err("Prompt help only connects to a loopback local assistant.".into());
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|e| format!("Local prompt service is unavailable: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let payload = body.map(|value| serde_json::to_vec(&value).expect("JSON body"));
    let target = format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        address
    );
    if let Some(bytes) = &payload {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            bytes.len()
        ));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Could not contact local prompt service: {e}"))?;
    if let Some(bytes) = payload {
        stream
            .write_all(&bytes)
            .map_err(|e| format!("Could not send prompt request: {e}"))?;
    }
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("Could not read local prompt service: {e}"))?;
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "Local prompt service returned an invalid HTTP response.".to_string())?;
    let headers = String::from_utf8_lossy(&response[..split]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("Local prompt service returned HTTP {status}."));
    }
    let mut bytes = response[split + 4..].to_vec();
    if headers.lines().any(|line| {
        line.to_ascii_lowercase().starts_with("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked")
    }) {
        bytes = decode_chunked(&bytes)?;
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Local prompt service returned invalid JSON: {e}"))
}

fn parse_loopback_endpoint(endpoint: &str) -> Result<(String, String), String> {
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| "The local assistant URL must use HTTP on loopback.".to_string())?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, "v1"));
    let host = authority
        .rsplit_once(':')
        .map(|(_, port)| port.parse::<u16>().ok())
        .flatten()
        .map(|port| format!("{}:{port}", authority.rsplit_once(':').unwrap().0))
        .unwrap_or_else(|| format!("{authority}:80"));
    let hostname = authority
        .split(':')
        .next()
        .unwrap_or("")
        .trim_matches(['[', ']']);
    if !matches!(hostname, "localhost" | "127.0.0.1" | "::1") {
        return Err("Prompt help only connects to a loopback local assistant.".into());
    }
    Ok((host, format!("/{path}")))
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| "Invalid chunked HTTP response.".to_string())?;
        let size_text = std::str::from_utf8(&input[..line_end])
            .map_err(|_| "Invalid chunk size.".to_string())?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or(""), 16)
            .map_err(|_| "Invalid chunk size.".to_string())?;
        input = &input[line_end + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size + 2 || &input[size..size + 2] != b"\r\n" {
            return Err("Truncated HTTP response chunk.".into());
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_endpoints_are_accepted() {
        assert_eq!(
            parse_loopback_endpoint("http://127.0.0.1:8766/v1").unwrap(),
            ("127.0.0.1:8766".into(), "/v1".into())
        );
        assert!(parse_loopback_endpoint("https://127.0.0.1:8766/v1").is_err());
        assert!(parse_loopback_endpoint("http://example.com:8766/v1").is_err());
    }

    #[test]
    fn chunked_responses_are_decoded() {
        assert_eq!(
            decode_chunked(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(),
            b"Wikipedia"
        );
    }
}
