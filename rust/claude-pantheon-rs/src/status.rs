//! Privacy-safe selected-account cache consumed by the existing Dock dropdown and indicator.
//! Compact surfaces observe this file, never a second T3 connection. Token history is explicitly
//! provider-wide and must never be presented as the selected account's usage or remaining quota.

use crate::t3::model::{Provider, ThreadShell, UsageSummary};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    crate::index::cache_dir().join("status.json")
}

struct HistoryState<'a> {
    summary: Option<&'a UsageSummary>,
    failed: bool,
}

pub fn build(
    connected: bool,
    label: &str,
    selected_provider_id: &str,
    providers: &[Provider],
    threads: &[ThreadShell],
    summary: Option<&UsageSummary>,
    history_failed: bool,
) -> Value {
    build_at(
        connected,
        label,
        selected_provider_id,
        providers,
        threads,
        HistoryState { summary, failed: history_failed },
        crate::look::now_unix(),
    )
}

fn build_at(
    connected: bool,
    label: &str,
    selected_provider_id: &str,
    providers: &[Provider],
    threads: &[ThreadShell],
    history: HistoryState<'_>,
    now: i64,
) -> Value {
    let written = crate::workflow::now_iso();
    let running: Vec<Value> = threads
        .iter()
        .filter(|t| t.is_running() && t.pending_requests() == 0)
        .map(|t| compact_thread(t, false, selected_provider_id, providers))
        .collect();
    let pending: Vec<Value> = threads
        .iter()
        .filter(|t| t.pending_requests() > 0)
        .map(|t| compact_thread(t, true, selected_provider_id, providers))
        .collect();
    let selected = providers.iter().find(|p| p.instance_id == selected_provider_id);
    let mut windows = Vec::new();
    if let Some(p) = selected {
        // Reuse the monitor's timestamp/invalid-percentage boundary without dropping disabled
        // selected accounts: every reported plan window remains visible, never an implied zero.
        let (state, _) = crate::usage_monitor::plan_observation(p, now);
        if let Some(limits) = &p.usage_limits {
            for w in &limits.windows {
                let valid = w.used_percent.is_finite() && (0.0..=100.0).contains(&w.used_percent);
                let state = if !valid && state != "unavailable" {
                    "unknown"
                } else if !connected && state == "fresh" {
                    "stale"
                } else {
                    state
                };
                let title = if !w.label.is_empty() { &w.label } else { &w.kind };
                let reset = w.resets_at.as_deref().filter(|s| gtk::glib::DateTime::from_iso8601(s, None).is_ok());
                windows.push(json!({"instanceId": p.instance_id, "provider": p.name(),
                    "kind": w.kind, "label": title, "usedPercent": if valid { Some(w.used_percent) } else { None },
                    "resetsAt": reset, "state": state, "live": connected && state == "fresh",
                    "checkedAt": limits.checked_at}));
            }
        }
    }
    json!({
        "schemaVersion": 1, "writtenAt": written, "updatedAt": written,
        "connected": connected, "stale": !connected, "server": label,
        "selectedProviderId": selected_provider_id,
        "selectedProvider": selected.map(Provider::name).unwrap_or_default(),
        "running": running, "pendingPrompts": pending, "windows": windows,
        "tokenUsage": token_usage(selected, history.summary, history.failed || !connected, now),
    })
}

fn provider_kind(driver: &str) -> &str {
    match driver {
        "claudeAgent" => "claude",
        "openCode" => "opencode",
        other => other,
    }
}

fn token_usage(selected: Option<&Provider>, summary: Option<&UsageSummary>, refresh_failed: bool, now: i64) -> Value {
    let kind = selected.map(|p| provider_kind(&p.driver)).unwrap_or("");
    let mut token = json!({"state": "unavailable", "source": "T3 server token history",
        "scope": "provider (all accounts)", "providerKind": kind,
        "sinceDay": "", "untilDay": "", "readAt": "", "totalTokens": null,
        "inputTokens": null, "outputTokens": null, "cachedInputTokens": null,
        "reasoningTokens": null, "estimatedCostUsd": null});
    let Some(summary) = summary else { return token };
    token["sinceDay"] = json!(summary.since_day);
    token["untilDay"] = json!(summary.until_day);
    token["readAt"] = json!(summary.read_at);
    let Some(row) = crate::usage_monitor::history_rows_at(summary, now)
        .into_iter()
        .find(|row| row["provider"].as_str() == Some(kind))
    else {
        return token;
    };
    let state = row["state"].as_str().unwrap_or("unknown");
    token["state"] = json!(if refresh_failed && state != "unavailable" { "stale" } else { state });
    for (dest, source) in [
        ("totalTokens", "tokens"),
        ("inputTokens", "input_tokens"),
        ("outputTokens", "output_tokens"),
        ("reasoningTokens", "reasoning_tokens"),
        ("estimatedCostUsd", "api_equivalent_cost_usd"),
    ] {
        token[dest] = row[source].clone();
    }
    if !row["tokens"].is_null() {
        let cached = summary
            .buckets
            .iter()
            .filter(|b| b.provider == kind)
            .fold(0u64, |n, b| n.saturating_add(b.totals.cached_input));
        token["cachedInputTokens"] = json!(cached);
    }
    token
}

fn compact_thread(thread: &ThreadShell, pending: bool, selected_id: &str, providers: &[Provider]) -> Value {
    let mut summary = if pending {
        json!({"id": thread.id, "title": thread.title, "count": thread.pending_requests()})
    } else {
        json!({"id": thread.id, "title": thread.title})
    };
    if !selected_id.is_empty()
        && thread.provider_instance_id == selected_id
        && let Some(key) = thread.model_selection.get("model").and_then(Value::as_str).filter(|k| !k.is_empty())
        && let Some(p) = providers.iter().find(|p| p.instance_id == selected_id)
    {
        let model = p
            .models
            .iter()
            .find(|m| m.key() == key || m.aliases.as_ref().is_some_and(|a| a.iter().any(|a| a == key)))
            .map(|m| m.display())
            .filter(|s| !s.is_empty())
            .unwrap_or(key);
        summary["model"] = json!(model);
    }
    summary
}

pub fn write(v: &Value) {
    let p = path();
    let payload = if v["connected"] == false {
        std::fs::read(&p)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|previous| previous["schemaVersion"] == 1)
            .map(|previous| preserve_last_known_at(&previous, v, &crate::workflow::now_iso()))
            .unwrap_or_else(|| v.clone())
    } else {
        v.clone()
    };
    write_payload(&p, &payload);
}

/// The application's normal shutdown hook preserves identity without claiming a live connection.
pub fn mark_stale() {
    let p = path();
    let Ok(bytes) = std::fs::read(&p) else { return };
    let Ok(previous) = serde_json::from_slice::<Value>(&bytes) else { return };
    if previous["schemaVersion"] != 1 {
        return;
    }
    let now = crate::workflow::now_iso();
    let disconnected = json!({"schemaVersion": 1, "writtenAt": now});
    write_payload(&p, &preserve_last_known_at(&previous, &disconnected, &now));
}

fn preserve_last_known_at(previous: &Value, disconnected: &Value, now: &str) -> Value {
    let mut next = disconnected.clone();
    for key in [
        "server",
        "selectedProviderId",
        "selectedProvider",
        "running",
        "pendingPrompts",
        "windows",
        "writtenAt",
        "tokenUsage",
    ] {
        if let Some(value) = previous.get(key) {
            next[key] = value.clone();
        }
    }
    let id = next["selectedProviderId"].as_str().unwrap_or("").to_string();
    if let Some(windows) = next["windows"].as_array_mut() {
        windows.retain(|w| w["instanceId"].as_str() == Some(id.as_str()));
        for w in windows {
            w["live"] = json!(false);
            if w["state"] == "fresh" {
                w["state"] = json!("stale");
            }
        }
    }
    for key in ["running", "pendingPrompts"] {
        if let Some(threads) = next[key].as_array_mut() {
            for thread in threads {
                if let Some(fields) = thread.as_object_mut() {
                    fields.remove("provider");
                }
            }
        }
    }
    if next["tokenUsage"].is_object() && next["tokenUsage"]["state"] != "unavailable" {
        next["tokenUsage"]["state"] = json!("stale");
    }
    next["connected"] = json!(false);
    next["stale"] = json!(true);
    next["updatedAt"] = json!(now);
    next
}

fn write_payload(p: &PathBuf, payload: &Value) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    let Ok(bytes) = serde_json::to_vec_pretty(payload) else { return };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&tmp)
        && std::io::Write::write_all(&mut file, &bytes).is_ok()
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, p);
    }
}

/// Explicit unpair removes both the cached identity and any partially written feed.
pub fn clear() {
    let p = path();
    let _ = std::fs::remove_file(&p);
    let _ = std::fs::remove_file(p.with_extension("json.tmp"));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider(id: &str, driver: &str, name: &str) -> Provider {
        serde_json::from_value(json!({"instanceId": id, "driver": driver, "displayName": name,
            "enabled": true, "installed": true, "auth": "ready", "usageLimits": {
                "checkedAt": "2026-10-25T12:00:00Z", "windows": [
                    {"kind": "session", "label": "Session", "usedPercent": 24.0},
                    {"kind": "weekly", "label": "Weekly", "usedPercent": null}]}}))
        .unwrap()
    }
    fn now() -> i64 {
        crate::look::iso_to_unix("2026-10-25T12:00:00Z")
    }
    fn history() -> UsageSummary {
        serde_json::from_value(json!({"sinceDay": "2026-10-24", "untilDay": "2026-10-25",
            "readAt": "2026-10-25T12:00:00Z", "buckets": [
                {"provider": "claude", "totals": {"uncachedInputTokens": 10, "cachedInputTokens": 5, "outputTokens": 20, "reasoningTokens": 7}, "costSource": "priced", "costUsd": 0.2},
                {"provider": "claude", "totals": {"outputTokens": 30}, "costSource": "priced", "costUsd": 0.3},
                {"provider": "codex", "totals": {"outputTokens": 999}}]})).unwrap()
    }
    #[test]
    fn selected_account_privacy_preserves_activity_without_other_account_labels() {
        let providers = [provider("work", "claudeAgent", "Work"), provider("private", "codex", "PRIVATE NAME")];
        let threads = [
            ThreadShell {
                id: "running".into(),
                title: "Build".into(),
                provider_instance_id: "private".into(),
                rest: json!({"activeRunId": "r1"}),
                ..Default::default()
            },
            ThreadShell {
                id: "approval".into(),
                rest: json!({"activeRunId": "r2", "pendingRuntimeRequestCount": 1}),
                ..Default::default()
            },
        ];
        let feed = build_at(
            true,
            "server",
            "work",
            &providers,
            &threads,
            HistoryState { summary: Some(&history()), failed: false },
            now(),
        );
        assert_eq!(feed["selectedProviderId"], "work");
        assert_eq!(feed["selectedProvider"], "Work");
        assert_eq!(feed["running"].as_array().unwrap().len(), 1);
        assert_eq!(feed["pendingPrompts"][0]["count"], 1);
        assert_eq!(feed["windows"].as_array().unwrap().len(), 2);
        assert!(feed["windows"][1]["usedPercent"].is_null());
        assert_eq!(feed["windows"][1]["live"], false);
        assert!(!feed.to_string().contains("PRIVATE NAME"));
        assert!(feed.get("nextTask").is_none());
    }
    #[test]
    fn tokens_are_provider_aggregate_not_selected_account_and_reasoning_is_not_double_counted() {
        let p = provider("work", "claudeAgent", "Work");
        let tokens = token_usage(Some(&p), Some(&history()), false, now());
        assert_eq!(tokens["providerKind"], "claude");
        assert_eq!(tokens["scope"], "provider (all accounts)");
        assert_eq!(tokens["state"], "fresh");
        assert_eq!(tokens["totalTokens"], 65);
        assert_eq!(tokens["inputTokens"], 15);
        assert_eq!(tokens["outputTokens"], 50);
        assert_eq!(tokens["cachedInputTokens"], 5);
        assert_eq!(tokens["reasoningTokens"], 7);
        assert_eq!(tokens["estimatedCostUsd"], 0.5);
    }
    #[test]
    fn missing_empty_failed_and_stale_history_never_become_confident_zero() {
        let p = provider("work", "claudeAgent", "Work");
        for summary in [None, Some(UsageSummary::default())] {
            let token = token_usage(Some(&p), summary.as_ref(), false, now());
            assert_eq!(token["state"], "unavailable");
            assert!(token["totalTokens"].is_null());
        }
        let mut summary = history();
        assert_eq!(token_usage(Some(&p), Some(&summary), true, now())["state"], "stale");
        assert_eq!(token_usage(Some(&p), Some(&summary), false, now() + 901)["state"], "stale");
        summary.read_at.clear();
        assert_eq!(token_usage(Some(&p), Some(&summary), false, now())["state"], "unknown");
        summary.buckets.clear();
        summary.sources =
            serde_json::from_value(json!([{"fingerprint": {"provider": "claude"}, "status": "failed"}])).unwrap();
        let token = token_usage(Some(&p), Some(&summary), true, now());
        assert_eq!(token["state"], "unavailable");
        assert!(token["totalTokens"].is_null());
    }
    #[test]
    fn disconnect_retains_selected_identity_and_stales_tokens_and_windows() {
        let previous = build_at(
            true,
            "server",
            "work",
            &[provider("work", "claudeAgent", "Work")],
            &[],
            HistoryState { summary: Some(&history()), failed: false },
            now(),
        );
        let stale = preserve_last_known_at(&previous, &json!({"schemaVersion": 1}), "later");
        assert_eq!(stale["selectedProviderId"], "work");
        assert_eq!(stale["writtenAt"], previous["writtenAt"]);
        assert_eq!(stale["updatedAt"], "later");
        assert_eq!(stale["stale"], true);
        assert_eq!(stale["connected"], false);
        assert_eq!(stale["tokenUsage"]["state"], "stale");
        assert_eq!(stale["tokenUsage"]["totalTokens"], 65);
        assert_eq!(stale["windows"][0]["live"], false);
    }
}
