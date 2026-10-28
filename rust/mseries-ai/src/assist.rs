//! Client for `mseries-assistd`, the one local LFM3-Audio model behind the assistant (see `mseries/assistant`).
//!
//! Loopback only: the URL may be changed with `MSERIES_ASSIST_URL`, but a host other than 127.0.0.1, ::1 or
//! localhost is refused, so typed text never leaves the machine.

use serde_json::{Value, json};
use std::time::Duration;
use url::Url;

pub const DEFAULT_ASSIST_URL: &str = "http://127.0.0.1:8772";
/// The model may need a few seconds to load on the first request after an idle unload.
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SUGGESTION_CHARS: usize = 400;

/// The assistant daemon's base URL, checked to be loopback.
pub fn assist_url() -> Result<Url, String> {
    let raw = std::env::var("MSERIES_ASSIST_URL").unwrap_or_else(|_| DEFAULT_ASSIST_URL.into());
    let url = Url::parse(&raw).map_err(|_| format!("Invalid assistant URL: {raw}"))?;
    match url.host_str() {
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1") if url.scheme() == "http" => Ok(url),
        _ => Err("The assistant URL must be a loopback http address".into()),
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())
}

fn error_text(status: reqwest::StatusCode, body: &Value) -> String {
    body.get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("The assistant returned HTTP {status}"))
}

fn transport_error(error: &reqwest::Error) -> String {
    if error.is_connect() {
        "The assistant service is not running. Start it with: systemctl --user start mseries-assistd".into()
    } else if error.is_timeout() {
        "The assistant took too long to answer.".into()
    } else {
        format!("The assistant request failed: {error}")
    }
}

/// A short continuation for `prefix` (the text before the caret). Empty when the model had nothing to add.
pub fn complete_text(prefix: &str) -> Result<String, String> {
    let url = assist_url()?
        .join("v1/complete")
        .map_err(|e| e.to_string())?;
    let response = client()?
        .post(url)
        .json(&json!({ "prefix": prefix }))
        .send()
        .map_err(|error| transport_error(&error))?;
    let status = response.status();
    let body: Value = response
        .json()
        .map_err(|error| format!("The assistant returned an invalid answer: {error}"))?;
    if !status.is_success() {
        return Err(error_text(status, &body));
    }
    let text = body.get("text").and_then(Value::as_str).unwrap_or_default();
    Ok(clean_suggestion(text))
}

/// Keeps a suggestion to one bounded line of printable text.
pub fn clean_suggestion(text: &str) -> String {
    let line = text.split('\n').next().unwrap_or_default();
    line.chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .take(MAX_SUGGESTION_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_one_bounded_printable_line() {
        assert_eq!(clean_suggestion(" 2:00 PM.\nPlease confirm"), " 2:00 PM.");
        assert_eq!(clean_suggestion(" a\u{7}b  "), " ab");
        assert_eq!(
            clean_suggestion(&"x".repeat(1000)).len(),
            MAX_SUGGESTION_CHARS
        );
    }

    #[test]
    fn only_loopback_urls_are_accepted() {
        // SAFETY: tests in this module are the only readers of this variable.
        unsafe { std::env::set_var("MSERIES_ASSIST_URL", "http://example.com:8772") };
        assert!(assist_url().is_err());
        unsafe { std::env::set_var("MSERIES_ASSIST_URL", "http://127.0.0.1:9000") };
        assert!(assist_url().is_ok());
        unsafe { std::env::remove_var("MSERIES_ASSIST_URL") };
        assert_eq!(assist_url().unwrap().as_str(), "http://127.0.0.1:8772/");
    }
}
