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

use crate::host::{Event, Services};

pub struct FixtureServices {
    feeds: BTreeMap<String, Value>,
    calls: BTreeMap<String, Value>,
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
        }
    }

    pub fn load(path: &std::path::Path) -> Result<FixtureServices, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text)
            .map(|v| FixtureServices::from_json(&v))
            .map_err(|e| e.to_string())
    }
}

impl Services for FixtureServices {
    fn call(
        &self,
        service: &str,
        args: &BTreeMap<String, Value>,
        then: Option<String>,
        _error: Option<String>,
        tx: UnboundedSender<Event>,
    ) -> bool {
        eprintln!(
            "fixture: {service} {}",
            serde_json::to_string(args).unwrap_or_default()
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
