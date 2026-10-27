//! Cursor-based older-thread history from the T3 orchestration HTTP API.

use super::{auth, http, model::TurnItem};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedTurnItem {
    pub position: u64,
    #[serde(default)]
    pub source_thread_id: String,
    #[serde(default)]
    pub source_item_id: String,
    pub item: TurnItem,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryPage {
    pub snapshot_sequence: u64,
    pub items: Vec<ProjectedTurnItem>,
    pub next_cursor: Option<String>,
    pub has_more_history: bool,
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| value.get("reason").and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "The T3 server could not load earlier history.".to_string())
}

pub async fn fetch_page(cred: &auth::Credential, thread_id: &str, cursor: &str) -> Result<ThreadHistoryPage, String> {
    let query = http::form(&[("cursor", cursor)]);
    let url = format!(
        "{}/api/orchestration/threads/{}/history?{}",
        cred.server.trim_end_matches('/'),
        http::path_segment(thread_id),
        query
    );
    let authorization = format!("Bearer {}", cred.access_token);
    let protocol = auth::PROTOCOL_VERSION.to_string();
    let response = http::request(
        "GET",
        &url,
        &[
            ("Authorization", &authorization),
            ("Accept", "application/json"),
            ("x-t3-orchestration-protocol", &protocol),
        ],
        None,
    )
    .await
    .map_err(|error| error.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(format!("{} (HTTP {})", error_message(&response.body), response.status));
    }
    serde_json::from_str(&response.body).map_err(|error| format!("Invalid T3 history response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn fetches_history_with_encoded_thread_and_opaque_cursor() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            let body = serde_json::json!({
                "snapshotSequence": 91,
                "items": [{
                    "position": 4,
                    "sourceThreadId": "thread/with space",
                    "sourceItemId": "older-4",
                    "item": {"id": "older-4", "type": "assistant_message", "text": "Earlier answer"}
                }],
                "nextCursor": "next +/opaque",
                "hasMoreHistory": true
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            String::from_utf8(request).unwrap()
        });

        let credential = auth::Credential {
            server: format!("http://{address}"),
            access_token: "fixture-token".into(),
            label: "fixture".into(),
            environment_id: "fixture-env".into(),
        };
        let context = gtk::glib::MainContext::new();
        let page = context.block_on(fetch_page(&credential, "thread/with space", "opaque+/=cursor")).unwrap();
        let request = server.join().unwrap();

        assert!(request.starts_with(
            "GET /api/orchestration/threads/thread%2Fwith%20space/history?cursor=opaque%2B%2F%3Dcursor HTTP/1.1\r\n"
        ));
        assert!(request.lines().any(|line| line == "Authorization: Bearer fixture-token"));
        assert!(request.lines().any(|line| line == "x-t3-orchestration-protocol: 2"));
        assert_eq!(page.snapshot_sequence, 91);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].source_thread_id, "thread/with space");
        assert_eq!(page.items[0].source_item_id, "older-4");
        assert_eq!(page.next_cursor.as_deref(), Some("next +/opaque"));
        assert!(page.has_more_history);
    }

    #[test]
    fn slow_history_response_does_not_block_the_gtk_main_context() {
        use std::cell::Cell;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent_tx, sent_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            // A delayed response stands in for a high-latency T3 Connect or relay hop.
            thread::sleep(Duration::from_millis(350));
            sent_tx.send(Instant::now()).unwrap();
            let body = serde_json::json!({
                "snapshotSequence": 92,
                "items": [],
                "nextCursor": null,
                "hasMoreHistory": false
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let credential = auth::Credential {
            server: format!("http://{address}"),
            access_token: "fixture-token".into(),
            label: "fixture".into(),
            environment_id: "fixture-env".into(),
        };
        let context = gtk::glib::MainContext::new();
        let timer_at = Cell::new(None);
        let page = context.block_on(async {
            let timer = async {
                gtk::glib::timeout_future(Duration::from_millis(50)).await;
                timer_at.set(Some(Instant::now()));
            };
            let fetch = fetch_page(&credential, "slow-thread", "slow-cursor");
            let (_, result) = futures_util::future::join(timer, fetch).await;
            result.unwrap()
        });
        let server_sent_at = sent_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();

        assert!(page.items.is_empty());
        assert!(!page.has_more_history);
        assert!(
            timer_at.get().unwrap() < server_sent_at,
            "the GTK main context must run while the delayed HTTP response is pending"
        );
    }
}
