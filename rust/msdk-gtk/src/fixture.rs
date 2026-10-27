//! A services provider that answers from a JSON file, so an app can run, be reviewed and screenshotted before its real
//! backend exists (the "fixture backend" of the app-building guide).
//!
//! `{"feeds": {"t3.threads": [ {"id": "t1", ...} ]}, "calls": {"t3.new_thread": "t7", "router.route": "claude, lane:build"}}`
//! Feeds push their list once. A call logs to stderr; when the file has a reply for it and the call has a `then` message,
//! the reply is sent back as that message's payload.

use std::collections::BTreeMap;
use std::rc::Rc;

use futures_channel::mpsc::UnboundedSender;
use serde_json::Value;

use crate::host::{Event, ServiceCall, Services};

pub struct FixtureServices {
    feeds: BTreeMap<String, Value>,
    calls: BTreeMap<String, Value>,
    api_version: u32,
}

impl FixtureServices {
    pub fn from_json(v: &Value) -> FixtureServices {
        let map = |k: &str| {
            v.get(k)
                .and_then(Value::as_object)
                .map(|o| o.iter().map(|(a, b)| (a.clone(), b.clone())).collect())
                .unwrap_or_default()
        };
        FixtureServices {
            feeds: map("feeds"),
            calls: map("calls"),
            api_version: msdk::runtime::SERVICES_API_VERSION,
        }
    }

    /// Parse an optional fixture extension version for compatibility testing.
    pub fn try_from_json(v: &Value) -> Result<FixtureServices, String> {
        let mut fixture = Self::from_json(v);
        if let Some(version) = v.get("api_version") {
            fixture.api_version = version
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    "fixture api_version must be an unsigned 32-bit integer".to_string()
                })?;
        }
        Ok(fixture)
    }

    pub fn load(path: &std::path::Path) -> Result<FixtureServices, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        FixtureServices::try_from_json(&value)
    }
}

impl Services for FixtureServices {
    fn api_version(&self) -> u32 {
        self.api_version
    }
    fn invoke(&self, call: ServiceCall) -> Result<bool, msdk::runtime::ServiceFailure> {
        let Some(value) = self.calls.get(&call.service) else {
            return Err(msdk::runtime::ServiceFailure::new(
                msdk::runtime::FailureKind::Unavailable,
                &call.service,
                "/fixture/calls",
                0,
            ));
        };
        eprintln!(
            "{}",
            serde_json::json!({"schema":"msdk.fixture.call.v1", "service":call.service})
        );
        call.reply.success(if value.is_null() {
            None
        } else {
            Some(value.clone())
        });
        Ok(true)
    }
    fn call(
        &self,
        service: &str,
        _args: &BTreeMap<String, Value>,
        then: Option<String>,
        _error: Option<String>,
        tx: UnboundedSender<Event>,
    ) -> bool {
        eprintln!(
            "{}",
            serde_json::json!({"schema":"msdk.fixture.call.v1", "service":service})
        );
        if let (Some(m), Some(reply)) = (then, self.calls.get(service)) {
            let _ = tx.unbounded_send((m, Some(reply.clone())));
        }
        true
    }

    fn start_feed(&self, feed: &str, push: Rc<dyn Fn(Value)>) -> bool {
        match self.feeds.get(feed) {
            Some(list) => {
                let list = list.clone();
                gtk::glib::idle_add_local_once(move || push(list));
                true
            }
            None => false,
        }
    }
}
