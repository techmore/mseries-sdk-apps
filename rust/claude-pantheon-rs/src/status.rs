//! The at-a-glance feed for the Wingpanel indicator (`wingpanel-indicator/`): running threads, pending
//! permission prompts and the plan windows nearest exhaustion, written to
//! `~/.cache/claude-pantheon-rs/status.json` whenever they change. T3 itself has no desktop tray or
//! menubar indicator, so this file is the only desktop at-a-glance view of the server.

use crate::t3::model::{Provider, ThreadShell};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    crate::index::cache_dir().join("status.json")
}

pub fn build(connected: bool, label: &str, providers: &[Provider], threads: &[ThreadShell]) -> Value {
    let running: Vec<Value> = threads
        .iter()
        .filter(|t| t.is_running())
        .map(|t| json!({"id": t.id, "title": t.title, "provider": t.provider_instance_id}))
        .collect();
    let pending: Vec<Value> = threads
        .iter()
        .filter(|t| t.pending_requests() > 0)
        .map(|t| json!({"id": t.id, "title": t.title, "count": t.pending_requests()}))
        .collect();
    let windows: Vec<Value> = providers
        .iter()
        .filter(|p| p.ready())
        .flat_map(|p| {
            let name = p.name();
            p.usage_limits
                .iter()
                .flat_map(|u| u.windows.iter())
                .map(move |w| {
                    json!({
                        "provider": name, "kind": w.kind, "label": w.label,
                        "usedPercent": w.used_percent, "resetsAt": w.resets_at,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let next =
        crate::t3::model::pick_provider(providers).map(|(p, reason)| json!({"provider": p.name(), "reason": reason}));
    json!({
        "schemaVersion": 1,
        "writtenAt": crate::workflow::now_iso(),
        "connected": connected,
        "server": label,
        "running": running,
        "pendingPrompts": pending,
        "windows": windows,
        "nextTask": next,
    })
}

pub fn write(v: &Value) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_lists_running_and_windows() {
        let threads = vec![ThreadShell {
            id: "t1".into(),
            title: "Build".into(),
            rest: json!({"activeRunId": "r1", "pendingRuntimeRequestCount": 1}),
            ..Default::default()
        }];
        let v = build(true, "eos", &[], &threads);
        assert_eq!(v["running"].as_array().unwrap().len(), 1);
        assert_eq!(v["pendingPrompts"][0]["count"], 1);
        assert!(v["windows"].as_array().unwrap().is_empty());
    }
}
