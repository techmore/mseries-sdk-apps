use gtk::{gio, glib, prelude::*};
use serde_json::Value;

const BUS_NAME: &str = "io.weirdware.BlueFerry";
const OBJECT_PATH: &str = "/io/weirdware/BlueFerry";
const MESSAGES_IFACE: &str = "io.weirdware.BlueFerry.Messages1";
const EVENTS_IFACE: &str = "io.weirdware.BlueFerry.Events1";
const API_VERSION: u64 = 2;
const STATUS_TIMEOUT_MS: i32 = 10_000;
const SNAPSHOT_TIMEOUT_MS: i32 = 20_000;
const SEND_TIMEOUT_MS: i32 = 240_000;

#[derive(Clone)]
pub struct Backend {
    bus: gio::DBusConnection,
}

impl Backend {
    pub fn connect() -> Result<Self, String> {
        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .map(|bus| Self { bus })
            .map_err(|error| format!("Could not connect to the session bus: {error}"))
    }

    async fn call(
        &self,
        method: &str,
        args: Option<&glib::Variant>,
        timeout_ms: i32,
    ) -> Result<glib::Variant, String> {
        self.bus
            .call_future(
                Some(BUS_NAME),
                OBJECT_PATH,
                MESSAGES_IFACE,
                method,
                args,
                None,
                gio::DBusCallFlags::NONE,
                timeout_ms,
            )
            .await
            .map_err(|error| error.message().to_string())
    }

    pub async fn status(&self) -> Result<Value, String> {
        let result = self.call("GetStatus", None, STATUS_TIMEOUT_MS).await?;
        let json: String = result
            .child_value(0)
            .get()
            .ok_or_else(|| "BlueFerry returned an invalid status response".to_string())?;
        let status: Value = serde_json::from_str(&json)
            .map_err(|error| format!("BlueFerry returned invalid status JSON: {error}"))?;
        if status.get("api_version").and_then(Value::as_u64) != Some(API_VERSION) {
            return Err(
                "This Messages app needs a current BlueFerry backend (Messages API v2).".into(),
            );
        }
        Ok(status)
    }

    pub async fn threads(&self) -> Result<Vec<crate::model::Thread>, String> {
        let args = (1000_u32,).to_variant();
        let result = self
            .call("ListThreads", Some(&args), SNAPSHOT_TIMEOUT_MS)
            .await?;
        let json: String = result
            .child_value(0)
            .get()
            .ok_or_else(|| "BlueFerry returned an invalid conversation list".to_string())?;
        serde_json::from_str(&json)
            .map_err(|error| format!("BlueFerry returned invalid conversation JSON: {error}"))
    }

    pub async fn mark_read(&self, key: &str) -> Result<(), String> {
        let args = (key,).to_variant();
        self.call("MarkThreadRead", Some(&args), SNAPSHOT_TIMEOUT_MS)
            .await
            .map(|_| ())
    }

    pub async fn set_starred(&self, key: &str, starred: bool) -> Result<(), String> {
        let args = (key, starred).to_variant();
        self.call("SetThreadStarred", Some(&args), SNAPSHOT_TIMEOUT_MS)
            .await
            .map(|_| ())
    }

    pub async fn send(
        &self,
        key: &str,
        body: &str,
        confirm_group: bool,
        expected_group_token: &str,
    ) -> Result<(), String> {
        let args = (key, body, confirm_group, expected_group_token).to_variant();
        self.call("SendToThreadChecked", Some(&args), SEND_TIMEOUT_MS)
            .await
            .map(|_| ())
    }

    pub fn watch_history(&self, changed: impl Fn() + 'static) -> gio::SignalSubscription {
        self.bus.subscribe_to_signal(
            Some(BUS_NAME),
            Some(EVENTS_IFACE),
            Some("HistoryChanged"),
            Some(OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_| changed(),
        )
    }
}
