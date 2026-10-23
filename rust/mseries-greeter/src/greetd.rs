//! greetd IPC (`greetd-ipc(7)`): a 4-byte native-endian length, then a JSON body, over the unix socket in `$GREETD_SOCK`.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<String> },
    StartSession { cmd: Vec<String>, env: Vec<String> },
    CancelSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    Error,
    AuthError,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: String,
    },
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: String,
    },
}

pub fn write_request(w: &mut impl Write, req: &Request) -> std::io::Result<()> {
    let body = serde_json::to_vec(req).map_err(std::io::Error::other)?;
    w.write_all(&(body.len() as u32).to_ne_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

pub fn read_response(r: &mut impl Read) -> std::io::Result<Response> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("greetd reply too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}
