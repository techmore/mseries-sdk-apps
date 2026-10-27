//! signal-cli's JSON-RPC over a unix socket: the same protocol and the same
//! `send` / result semantics as the Signal Pantheon client
//! (`mseries/signal-pantheon/src/signal-backend.vala`, `message-commands.vala`).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// Why a request did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcError {
    /// Nothing was sent: the socket could not be used.
    Unavailable(String),
    /// signal-cli answered with an error.
    Remote(String),
    /// The request left this process but no answer arrived, so a send may have
    /// reached Signal. Never retried automatically.
    OutcomeUnknown(String),
}

impl RpcError {
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(message) | Self::Remote(message) | Self::OutcomeUnknown(message) => {
                message
            }
        }
    }
}

pub struct RpcConnection {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    /// Notifications (incoming messages and so on) seen while waiting for an answer.
    pub events: Vec<Value>,
}

impl RpcConnection {
    pub fn connect(socket: &Path) -> Result<Self, String> {
        let writer = UnixStream::connect(socket)
            .map_err(|error| format!("could not reach the Signal service: {error}"))?;
        let reader = writer
            .try_clone()
            .map_err(|error| format!("could not use the Signal service socket: {error}"))?;
        Ok(Self {
            writer,
            reader: BufReader::new(reader),
            next_id: 1,
            events: Vec::new(),
        })
    }

    /// One request and its answer. Lines without our id are notifications and
    /// are kept in `events`.
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = format!("messages-{}", self.next_id);
        self.next_id += 1;
        let mut line =
            json!({"jsonrpc": "2.0", "method": method, "id": id, "params": params}).to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(|error| {
            RpcError::Unavailable(format!("could not write to Signal: {error}"))
        })?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timed_out());
            }
            self.reader
                .get_ref()
                .set_read_timeout(Some(remaining))
                .map_err(|error| RpcError::OutcomeUnknown(error.to_string()))?;
            let mut text = String::new();
            match self.reader.read_line(&mut text) {
                Ok(0) => {
                    return Err(RpcError::OutcomeUnknown(
                        "The Signal service closed the connection. A send may already have reached Signal."
                            .into(),
                    ));
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(timed_out());
                }
                Err(error) => return Err(RpcError::OutcomeUnknown(error.to_string())),
            }
            let Ok(message) = serde_json::from_str::<Value>(text.trim()) else {
                continue;
            };
            if message["id"].as_str() != Some(id.as_str()) {
                if message.get("method").is_some() {
                    self.events.push(message);
                }
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(RpcError::Remote(
                    error["message"]
                        .as_str()
                        .unwrap_or("Signal reported an error")
                        .to_string(),
                ));
            }
            return Ok(message["result"].clone());
        }
    }
}

fn timed_out() -> RpcError {
    RpcError::OutcomeUnknown(
        "Signal did not answer in time. A send may already have reached Signal.".into(),
    )
}

/// `send` parameters: a direct recipient or a group id.
pub fn send_params(account: &str, conversation: &str, is_group: bool, text: &str) -> Value {
    let mut params = json!({"account": account, "message": text});
    if is_group {
        params["groupId"] = json!(conversation);
    } else {
        params["recipient"] = json!([conversation]);
    }
    params
}

/// Delivery confirmation, as the client's `MessageCommands.confirm`: a positive
/// timestamp and at least one SUCCESS result. Returns the message timestamp.
pub fn confirm_send(result: &Value) -> Result<i64, String> {
    let timestamp = result["timestamp"].as_i64().unwrap_or(0);
    let success = result["results"]
        .as_array()
        .map(|results| {
            results
                .iter()
                .filter(|entry| entry["type"].as_str() == Some("SUCCESS"))
                .count()
        })
        .unwrap_or(0);
    if timestamp <= 0 || success == 0 {
        return Err(
            "Signal did not confirm the message. Check the conversation before trying again."
                .into(),
        );
    }
    Ok(timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_parameters_match_the_pantheon_client() {
        assert_eq!(
            send_params("+1", "+2", false, "hi"),
            json!({"account": "+1", "recipient": ["+2"], "message": "hi"})
        );
        assert_eq!(
            send_params("+1", "grp==", true, "hi"),
            json!({"account": "+1", "groupId": "grp==", "message": "hi"})
        );
    }

    #[test]
    fn a_send_needs_a_timestamp_and_one_success() {
        let ok =
            json!({"timestamp": 5, "results": [{"type": "SUCCESS"}, {"type": "NETWORK_FAILURE"}]});
        assert_eq!(confirm_send(&ok), Ok(5));
        assert!(confirm_send(&json!({"timestamp": 5, "results": []})).is_err());
        assert!(confirm_send(&json!({"timestamp": 0, "results": [{"type": "SUCCESS"}]})).is_err());
        assert!(confirm_send(&json!(null)).is_err());
    }

    #[test]
    fn notifications_are_kept_and_the_matching_answer_is_returned() {
        let directory = std::env::temp_dir().join(format!("signal-rpc-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let id = serde_json::from_str::<Value>(&request).unwrap()["id"].clone();
            let mut stream = stream;
            writeln!(
                stream,
                r#"{{"jsonrpc":"2.0","method":"receive","params":{{}}}}"#
            )
            .unwrap();
            writeln!(stream, r#"{{"jsonrpc":"2.0","id":"other","result":1}}"#).unwrap();
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"result":{"timestamp":9}})
            )
            .unwrap();
        });
        let mut connection = RpcConnection::connect(&socket).unwrap();
        let result = connection
            .call("send", json!({}), Duration::from_secs(5))
            .unwrap();
        assert_eq!(result["timestamp"], 9);
        assert_eq!(connection.events.len(), 1);
        server.join().unwrap();
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn silence_is_an_unknown_outcome_not_a_failure() {
        let directory =
            std::env::temp_dir().join(format!("signal-rpc-quiet-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("s.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let mut connection = RpcConnection::connect(&socket).unwrap();
        let error = connection
            .call("send", json!({}), Duration::from_millis(150))
            .unwrap_err();
        assert!(matches!(error, RpcError::OutcomeUnknown(_)));
        std::fs::remove_dir_all(&directory).ok();
    }
}
