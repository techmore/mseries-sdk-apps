//! Pairing and credentials. A pairing token (from `t3 pair`) is exchanged once at `/oauth/token` for a
//! bearer access token; that token is what we keep. Each connection then mints a short-lived WebSocket
//! ticket at `/api/auth/websocket-ticket` and opens `/ws?wsTicket=…&orchestrationProtocol=2`.
//!
//! The token lives in the keyring through `secret-tool` (libsecret's CLI, so no keyring crate is
//! needed offline); without a keyring it goes to `~/.config/claude-pantheon-rs/credentials.json`, 0600.

use super::http;
use super::ws::WsError;
use gtk::gio;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Credential {
    pub server: String,
    pub access_token: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub environment_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Descriptor {
    #[serde(rename = "environmentId")]
    pub environment_id: String,
    pub label: String,
    #[serde(rename = "serverVersion")]
    pub server_version: String,
    #[serde(rename = "orchestrationProtocolVersion")]
    pub orchestration_protocol_version: u32,
}

fn api_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let reason = v.get("reason").and_then(|r| r.as_str()).map(str::to_string);
            let message = v.get("message").and_then(|r| r.as_str()).map(str::to_string);
            message.or(reason)
        })
        .unwrap_or_else(|| body.chars().take(200).collect())
}

/// Normalizes what the user pastes: a token, or the pairing URL `http://host:port/pair#token=XXXX`.
pub fn parse_pairing(input: &str, default_server: &str) -> (String, String) {
    let input = input.trim();
    if let Some((base, frag)) = input.split_once("#token=") {
        let server = base.trim_end_matches("/pair").trim_end_matches('/').to_string();
        return (server, frag.trim().to_string());
    }
    (default_server.trim_end_matches('/').to_string(), input.to_string())
}

pub async fn descriptor(server: &str) -> Result<Descriptor, WsError> {
    let r = http::request("GET", &format!("{server}/.well-known/t3/environment"), &[], None).await?;
    if r.status != 200 {
        return Err(WsError::Refused { status: r.status, body: api_error(&r.body) });
    }
    serde_json::from_str(&r.body).map_err(|e| WsError::Io(format!("bad descriptor: {e}")))
}

/// Exchanges a pairing token for a bearer access token with the default scope set.
pub async fn pair(server: &str, token: &str) -> Result<Credential, WsError> {
    let body = http::form(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:token-exchange"),
        ("subject_token", token),
        ("subject_token_type", "urn:t3:params:oauth:token-type:environment-bootstrap"),
        ("requested_token_type", "urn:ietf:params:oauth:token-type:access_token"),
        ("client_label", "Claude for elementary OS"),
        ("client_device_type", "desktop"),
        ("client_os", "linux"),
    ]);
    let r = http::request(
        "POST",
        &format!("{server}/oauth/token"),
        &[("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")],
        Some(body.as_bytes()),
    )
    .await?;
    if r.status != 200 {
        return Err(WsError::Refused { status: r.status, body: api_error(&r.body) });
    }
    let t: TokenResponse = serde_json::from_str(&r.body).map_err(|e| WsError::Io(format!("bad token answer: {e}")))?;
    let d = descriptor(server).await.unwrap_or_default();
    Ok(Credential {
        server: server.to_string(),
        access_token: t.access_token,
        label: d.label,
        environment_id: d.environment_id,
    })
}

/// The socket URL for one connection: a fresh ticket plus the protocol version.
pub async fn socket_url(cred: &Credential) -> Result<String, WsError> {
    let auth = format!("Bearer {}", cred.access_token);
    let r = http::request(
        "POST",
        &format!("{}/api/auth/websocket-ticket", cred.server),
        &[("Authorization", &auth), ("Content-Type", "application/json"), ("Accept", "application/json")],
        Some(b"{}"),
    )
    .await?;
    if r.status != 200 {
        return Err(WsError::Refused { status: r.status, body: api_error(&r.body) });
    }
    let t: TicketResponse =
        serde_json::from_str(&r.body).map_err(|e| WsError::Io(format!("bad ticket answer: {e}")))?;
    let ws = cred.server.replacen("http://", "ws://", 1);
    Ok(format!("{ws}/ws?wsTicket={}&orchestrationProtocol={PROTOCOL_VERSION}", t.ticket))
}

// ---- storage ---------------------------------------------------------------------------------------

fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs")
}

fn credentials_file() -> PathBuf {
    config_dir().join("credentials.json")
}

const SECRET_ATTRS: [&str; 2] = ["application", "claude-pantheon-rs"];

async fn run(argv: &[&str], stdin: Option<&str>) -> Option<(i32, String)> {
    let flags = gio::SubprocessFlags::STDOUT_PIPE
        | gio::SubprocessFlags::STDERR_SILENCE
        | if stdin.is_some() { gio::SubprocessFlags::STDIN_PIPE } else { gio::SubprocessFlags::NONE };
    let launcher = gio::SubprocessLauncher::new(flags);
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let proc = launcher.spawn(&argv).ok()?;
    let (out, _) = proc.communicate_utf8_future(stdin.map(|s| s.to_string())).await.ok()?;
    let status = proc.exit_status();
    Some((status, out.map(|s| s.to_string()).unwrap_or_default()))
}

/// Loads the stored credential: keyring first, then the config file.
pub async fn load() -> Option<Credential> {
    if let Some((0, out)) = run(&["secret-tool", "lookup", SECRET_ATTRS[0], SECRET_ATTRS[1]], None).await
        && let Ok(c) = serde_json::from_str::<Credential>(out.trim())
    {
        return Some(c);
    }
    let bytes = std::fs::read(credentials_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Stores the credential, preferring the keyring. Returns where it went.
pub async fn store(cred: &Credential) -> &'static str {
    let json = serde_json::to_string(cred).unwrap_or_default();
    if let Some((0, _)) = run(
        &["secret-tool", "store", "--label=Claude for elementary OS (T3 session)", SECRET_ATTRS[0], SECRET_ATTRS[1]],
        Some(&json),
    )
    .await
    {
        let _ = std::fs::remove_file(credentials_file());
        return "keyring";
    }
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let path = credentials_file();
    if std::fs::write(&path, json).is_ok() {
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    "config file"
}

pub async fn forget() {
    let _ = run(&["secret-tool", "clear", SECRET_ATTRS[0], SECRET_ATTRS[1]], None).await;
    let _ = std::fs::remove_file(credentials_file());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_input_forms() {
        assert_eq!(
            parse_pairing("http://127.0.0.1:3773/pair#token=ABC", "http://x"),
            ("http://127.0.0.1:3773".to_string(), "ABC".to_string())
        );
        assert_eq!(parse_pairing("  ABC  ", "http://127.0.0.1:3773/"), ("http://127.0.0.1:3773".into(), "ABC".into()));
    }
}
