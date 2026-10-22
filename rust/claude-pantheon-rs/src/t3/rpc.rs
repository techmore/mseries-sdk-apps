//! Effect RPC over the WebSocket, JSON serialization (`RpcSerialization.layerJson`): one JSON envelope
//! per text frame. Client → server: `Request`, `Ack`, `Interrupt`, `Ping`. Server → client: `Chunk`,
//! `Exit`, `Defect`, `Pong`, `ClientProtocolError`. A unary call is a Request answered by one Exit; a
//! subscription is a Request answered by Chunks (each acknowledged) and a final Exit.

use super::ws::{Frame, WebSocket};
use futures_channel::{mpsc, oneshot};
use gtk::glib;
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RpcError {
    /// The server failed the request with this error value (`_tag`, `message`, …).
    Failed(Value),
    /// The connection died or the server reported a protocol defect.
    Transport(String),
    Interrupted,
}

impl RpcError {
    pub fn message(&self) -> String {
        match self {
            RpcError::Failed(v) => v
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| v.get("_tag").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| v.to_string()),
            RpcError::Transport(s) => s.clone(),
            RpcError::Interrupted => "interrupted".into(),
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            RpcError::Failed(v) => v.get("_tag").and_then(Value::as_str),
            _ => None,
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Server → client envelope, decoded loosely so unknown tags are skipped rather than fatal.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Incoming {
    #[serde(rename = "_tag")]
    pub tag: String,
    #[serde(rename = "requestId")]
    pub request_id: Option<Value>,
    pub values: Option<Vec<Value>>,
    pub exit: Option<Exit>,
    pub defect: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Exit {
    #[serde(rename = "_tag")]
    pub tag: String,
    pub value: Option<Value>,
    pub cause: Option<Vec<Value>>,
}

impl Exit {
    /// The request's outcome as the client sees it.
    pub fn into_result(self) -> Result<Value, RpcError> {
        if self.tag == "Success" {
            return Ok(self.value.unwrap_or(Value::Null));
        }
        let cause = self.cause.unwrap_or_default();
        for c in &cause {
            match c.get("_tag").and_then(Value::as_str) {
                Some("Fail") => return Err(RpcError::Failed(c.get("error").cloned().unwrap_or(Value::Null))),
                Some("Die") => {
                    return Err(RpcError::Transport(format!(
                        "server defect: {}",
                        c.get("defect").unwrap_or(&Value::Null)
                    )));
                }
                Some("Interrupt") => return Err(RpcError::Interrupted),
                _ => {}
            }
        }
        Err(RpcError::Transport("request failed without a cause".into()))
    }
}

enum Pending {
    Unary(oneshot::Sender<Result<Value, RpcError>>),
    Stream(mpsc::UnboundedSender<Result<Value, RpcError>>),
}

struct Inner {
    ws: Rc<WebSocket>,
    next_id: Cell<u64>,
    pending: RefCell<HashMap<String, Pending>>,
    outgoing: mpsc::UnboundedSender<String>,
    closed: Cell<bool>,
    on_close: RefCell<Vec<CloseHook>>,
}

type CloseHook = Box<dyn Fn(String)>;

/// A connected RPC client. Clone freely; all clones share one socket.
#[derive(Clone)]
pub struct Rpc {
    inner: Rc<Inner>,
}

impl Rpc {
    /// Wraps an upgraded socket and starts the reader, writer and keep-alive tasks on the main loop.
    pub fn start(ws: Rc<WebSocket>) -> Rpc {
        let (out_tx, mut out_rx) = mpsc::unbounded::<String>();
        let inner = Rc::new(Inner {
            ws: ws.clone(),
            next_id: Cell::new(1),
            pending: RefCell::new(HashMap::new()),
            outgoing: out_tx,
            closed: Cell::new(false),
            on_close: RefCell::new(Vec::new()),
        });
        let rpc = Rpc { inner };

        // Writer: a single task serializes frames, so concurrent requests never interleave bytes.
        let w = rpc.clone();
        glib::spawn_future_local(async move {
            use futures_util::StreamExt;
            while let Some(text) = out_rx.next().await {
                if let Err(e) = w.inner.ws.send_text(&text).await {
                    w.fail_all(format!("send failed: {e}"));
                    break;
                }
            }
        });

        // Reader.
        let r = rpc.clone();
        glib::spawn_future_local(async move {
            loop {
                match r.inner.ws.next().await {
                    Ok(Frame::Text(text)) => r.dispatch(&text),
                    Ok(Frame::Ping(p)) => {
                        let _ = r.inner.ws.send_pong(&p).await;
                    }
                    Ok(Frame::Pong(_)) | Ok(Frame::Binary(_)) => {}
                    Ok(Frame::Close(code)) => {
                        r.fail_all(format!("server closed the connection ({})", code.unwrap_or(0)));
                        break;
                    }
                    Err(e) => {
                        r.fail_all(format!("connection lost: {e}"));
                        break;
                    }
                }
            }
        });

        // Keep-alive: one tiny frame every 30 s, nothing else runs while idle.
        let k = rpc.clone();
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_secs(30)).await;
                if k.inner.closed.get() {
                    break;
                }
                k.send_json(&json!({"_tag": "Ping"}));
            }
        });
        rpc
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.get()
    }

    /// Runs `f` once with the reason when the connection ends.
    pub fn on_close(&self, f: impl Fn(String) + 'static) {
        self.inner.on_close.borrow_mut().push(Box::new(f));
    }

    fn send_json(&self, v: &Value) {
        if std::env::var_os("CLAUDE_PANTHEON_DEBUG").is_some() {
            let t = v.to_string();
            eprintln!("rpc -> {}", &t[..t.len().min(160)]);
        }
        let _ = self.inner.outgoing.unbounded_send(v.to_string());
    }

    fn new_id(&self) -> String {
        let id = self.inner.next_id.get();
        self.inner.next_id.set(id + 1);
        id.to_string()
    }

    fn request_envelope(&self, id: &str, tag: &str, payload: Value) -> Value {
        json!({"_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": []})
    }

    /// One request, one answer.
    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, RpcError> {
        debug_assert!(super::contracts::is_known(tag), "RPC tag {tag} is not in the vendored contracts");
        if self.inner.closed.get() {
            return Err(RpcError::Transport("not connected".into()));
        }
        let id = self.new_id();
        let (tx, rx) = oneshot::channel();
        self.inner.pending.borrow_mut().insert(id.clone(), Pending::Unary(tx));
        self.send_json(&self.request_envelope(&id, tag, payload));
        rx.await.unwrap_or_else(|_| Err(RpcError::Transport("connection closed".into())))
    }

    /// A streaming request: values arrive on the receiver until the server's Exit (Ok end) or an error.
    pub fn subscribe(&self, tag: &str, payload: Value) -> (String, mpsc::UnboundedReceiver<Result<Value, RpcError>>) {
        debug_assert!(super::contracts::is_known(tag), "RPC tag {tag} is not in the vendored contracts");
        let id = self.new_id();
        let (tx, rx) = mpsc::unbounded();
        if self.inner.closed.get() {
            let _ = tx.unbounded_send(Err(RpcError::Transport("not connected".into())));
            return (id, rx);
        }
        self.inner.pending.borrow_mut().insert(id.clone(), Pending::Stream(tx));
        self.send_json(&self.request_envelope(&id, tag, payload));
        (id, rx)
    }

    /// Ends a subscription (or an in-flight call) on the server.
    pub fn interrupt(&self, id: &str) {
        if self.inner.pending.borrow_mut().remove(id).is_some() {
            self.send_json(&json!({"_tag": "Interrupt", "requestId": id}));
        }
    }

    pub async fn close(&self) {
        self.inner.closed.set(true);
        let _ = self.inner.ws.send_close().await;
    }

    fn dispatch(&self, text: &str) {
        if std::env::var_os("CLAUDE_PANTHEON_DEBUG").is_some() {
            eprintln!("rpc <- {}", &text[..text.len().min(160)]);
        }
        let Ok(msg) = serde_json::from_str::<Incoming>(text) else { return };
        match msg.tag.as_str() {
            "Chunk" => {
                let Some(id) = id_text(&msg.request_id) else { return };
                let values = msg.values.unwrap_or_default();
                let pending = self.inner.pending.borrow();
                if let Some(Pending::Stream(tx)) = pending.get(&id) {
                    for v in values {
                        let _ = tx.unbounded_send(Ok(v));
                    }
                }
                drop(pending);
                self.send_json(&json!({"_tag": "Ack", "requestId": id}));
            }
            "Exit" => {
                let Some(id) = id_text(&msg.request_id) else { return };
                let result = msg.exit.map(Exit::into_result).unwrap_or(Ok(Value::Null));
                let entry = self.inner.pending.borrow_mut().remove(&id);
                match entry {
                    Some(Pending::Unary(tx)) => {
                        let _ = tx.send(result);
                    }
                    Some(Pending::Stream(tx)) => {
                        if let Err(e) = result {
                            let _ = tx.unbounded_send(Err(e));
                        }
                        // dropping tx ends the stream
                    }
                    None => {}
                }
            }
            "Defect" => self.fail_all(format!("server defect: {}", msg.defect.unwrap_or(Value::Null))),
            "ClientProtocolError" => self.fail_all(format!("protocol error: {}", msg.error.unwrap_or(Value::Null))),
            _ => {} // Pong, server-originated Request
        }
    }

    fn fail_all(&self, reason: String) {
        if self.inner.closed.replace(true) {
            return;
        }
        let pending: Vec<Pending> = self.inner.pending.borrow_mut().drain().map(|(_, p)| p).collect();
        for p in pending {
            match p {
                Pending::Unary(tx) => {
                    let _ = tx.send(Err(RpcError::Transport(reason.clone())));
                }
                Pending::Stream(tx) => {
                    let _ = tx.unbounded_send(Err(RpcError::Transport(reason.clone())));
                }
            }
        }
        for f in self.inner.on_close.borrow().iter() {
            f(reason.clone());
        }
    }
}

fn id_text(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exits_decode() {
        let ok: Incoming =
            serde_json::from_str(r#"{"_tag":"Exit","requestId":"3","exit":{"_tag":"Success","value":{"x":1}}}"#)
                .unwrap();
        assert_eq!(ok.exit.unwrap().into_result().unwrap()["x"], 1);
        let fail: Incoming = serde_json::from_str(
            r#"{"_tag":"Exit","requestId":"4","exit":{"_tag":"Failure","cause":[{"_tag":"Fail","error":{"_tag":"EnvironmentAuthorizationError","message":"This connection requires orchestration:operate."}}]}}"#,
        )
        .unwrap();
        let err = fail.exit.unwrap().into_result().unwrap_err();
        assert_eq!(err.tag(), Some("EnvironmentAuthorizationError"));
        assert!(err.message().contains("orchestration:operate"));
        let chunk: Incoming =
            serde_json::from_str(r#"{"_tag":"Chunk","requestId":7,"values":[{"kind":"synchronized"}]}"#).unwrap();
        assert_eq!(id_text(&chunk.request_id).as_deref(), Some("7"));
        assert_eq!(chunk.values.unwrap().len(), 1);
    }
}
