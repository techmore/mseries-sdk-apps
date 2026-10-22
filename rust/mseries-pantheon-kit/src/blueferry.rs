use crate::messages::{ContactMatch, Thread};
use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

const BUS_NAME: &str = "io.weirdware.BlueFerry";
const OBJECT_PATH: &str = "/io/weirdware/BlueFerry";
const MESSAGES_IFACE: &str = "io.weirdware.BlueFerry.Messages1";
const EVENTS_IFACE: &str = "io.weirdware.BlueFerry.Events1";
const STATUS_TIMEOUT_MS: i32 = 10_000;
const CONTACT_TIMEOUT_MS: i32 = 8_000;
const SNAPSHOT_TIMEOUT_MS: i32 = 20_000;
const SEND_TIMEOUT_MS: i32 = 240_000;

/// Version of the public BlueFerry Messages D-Bus contract consumed by this SDK.
pub const BLUEFERRY_MESSAGES_API_VERSION: u64 = 2;

/// Typed subset of BlueFerry status used by first-party app clients.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct BlueFerryStatus {
    pub api_version: u64,
    pub map: bool,
    pub connectivity_detail: Option<String>,
    pub contact_photos: bool,
    pub contact_photo_revision: Option<u64>,
}

/// App-facing client for BlueFerry's versioned session D-Bus contract.
///
/// BlueFerry remains the owner of iPhone MAP access, message history, recipient
/// validation, and contact-photo policy. This adapter deliberately exposes only
/// its supported API and never reads its private database or service files.
#[derive(Clone)]
pub struct BlueFerryMessagesClient {
    bus: gio::DBusConnection,
}

impl BlueFerryMessagesClient {
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

    pub async fn status(&self) -> Result<BlueFerryStatus, String> {
        let result = self.call("GetStatus", None, STATUS_TIMEOUT_MS).await?;
        let json: String = result
            .child_value(0)
            .get()
            .ok_or_else(|| "BlueFerry returned an invalid status response".to_string())?;
        let status: BlueFerryStatus = serde_json::from_str(&json)
            .map_err(|error| format!("BlueFerry returned invalid status JSON: {error}"))?;
        if status.api_version != BLUEFERRY_MESSAGES_API_VERSION {
            return Err(format!(
                "This app needs BlueFerry Messages API v{BLUEFERRY_MESSAGES_API_VERSION}."
            ));
        }
        Ok(status)
    }

    pub async fn threads(&self) -> Result<Vec<Thread>, String> {
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

    pub async fn find_contacts(&self, query: &str) -> Result<Vec<ContactMatch>, String> {
        let args = (query,).to_variant();
        let result = self
            .call("FindContacts", Some(&args), CONTACT_TIMEOUT_MS)
            .await?;
        let json: String = result
            .child_value(0)
            .get()
            .ok_or_else(|| "BlueFerry returned an invalid contact search response".to_string())?;
        serde_json::from_str(&json)
            .map_err(|error| format!("BlueFerry returned invalid contact JSON: {error}"))
    }

    pub async fn contact_photo(&self, address: &str) -> Result<Vec<u8>, String> {
        let args = (address,).to_variant();
        let result = self
            .call("GetContactPhoto", Some(&args), CONTACT_TIMEOUT_MS)
            .await?;
        result
            .child_value(0)
            .get::<Vec<u8>>()
            .ok_or_else(|| "BlueFerry returned an invalid contact photo response".to_string())
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

    pub async fn send_to_thread(
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

    pub async fn send_to_recipient(&self, recipient: &str, body: &str) -> Result<(), String> {
        let args = (recipient, body).to_variant();
        self.call("Send", Some(&args), SEND_TIMEOUT_MS)
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
