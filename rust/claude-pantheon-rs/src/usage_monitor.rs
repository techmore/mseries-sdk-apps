//! Read-only projections of T3 server observations. Plan percentages are not token history,
//! and provider-level history cannot identify the account that produced a record.
use crate::t3::model::{Provider, UsageSummary, UsageTotals};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const FRESH_SECONDS: i64 = 15 * 60;

fn timestamp(raw: &str) -> Option<i64> {
    gtk::glib::DateTime::from_iso8601(raw, None).ok().map(|d| d.to_unix()).filter(|t| *t >= 0)
}

fn failed(value: Option<&Value>) -> bool {
    value.is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
}

/// Shared eligibility boundary for the read-only monitor and runner selection.
/// This computes no scores and allocates no collections.
pub fn plan_observation(p: &Provider, now: i64) -> (&'static str, Option<i64>) {
    let limits = p.usage_limits.as_ref();
    let observed = limits.and_then(|l| timestamp(&l.checked_at)).filter(|t| *t <= now);
    let state = if !p.installed || p.availability.as_deref() == Some("unavailable")
        || failed(limits.and_then(|l| l.unavailable.as_ref()))
        || limits.is_none_or(|l| l.windows.is_empty()) { "unavailable" }
        else if observed.is_none() || limits.is_some_and(|l| l.windows.iter().any(|w|
            !w.used_percent.is_finite() || !(0.0..=100.0).contains(&w.used_percent))) { "unknown" }
        else if now.saturating_sub(observed.unwrap()) > FRESH_SECONDS { "stale" }
        else { "fresh" };
    (state, observed)
}

/// Every enabled account remains visible, even without a usable plan observation.
/// `observed_at` is server checkedAt, never the time this projection was requested.
/// Only a fresh, valid window has `live: true`; unknown amounts stay null.
pub fn account_rows(providers: &[Provider], now: i64) -> Vec<Value> {
    providers.iter().filter(|p| p.enabled).map(|p| {
        let limits = p.usage_limits.as_ref();
        let checked_at = limits.map(|l| l.checked_at.as_str()).unwrap_or("");
        let (state, observed) = plan_observation(p, now);
        let explanation = match state {
            "unavailable" => "Server plan observation unavailable; quota is unknown",
            "unknown" if observed.is_none() => "Missing, invalid or future server observation time",
            "unknown" => "Invalid or missing plan percentage; quota is unknown",
            "stale" => "Last-known server plan windows; not live quota",
            _ => "Server plan windows checked within 15 minutes; not token history",
        };
        let windows: Vec<Value> = limits.into_iter().flat_map(|l| &l.windows).enumerate().map(|(i, w)| {
            let valid = w.used_percent.is_finite() && (0.0..=100.0).contains(&w.used_percent);
            let window_state = if !valid && state != "unavailable" { "unknown" } else { state };
            let key = if !w.id.is_empty() { w.id.clone() } else if !w.kind.is_empty() { w.kind.clone() } else { format!("window-{i}") };
            let title = if !w.label.is_empty() { w.label.as_str() } else if !w.kind.is_empty() { w.kind.as_str() } else { "Plan window" };
            json!({"id": format!("{}:{key}", p.instance_id), "title": title,
                "kind": w.kind, "state": window_state, "used_percent": if valid { Some(w.used_percent) } else { None },
                "live": window_state == "fresh", "resets_at": w.resets_at.as_deref().filter(|s| timestamp(s).is_some()),
                "source": "t3-server-plan", "observed_at": observed})
        }).collect();
        json!({"id": p.instance_id, "title": p.name(), "detail": format!("{} · {} · {}", p.driver, p.instance_id, explanation),
            "state": state, "source": "t3-server-plan", "observed_at": observed, "checked_at": checked_at,
            "semantics": "server-account-plan-windows-percent-used", "instance_id": p.instance_id,
            "driver": p.driver, "windows": windows})
    }).collect()
}

#[derive(Default)]
struct History {
    totals: UsageTotals,
    cost: f64,
    savings: f64,
    buckets: usize,
    sources: usize,
    failed_sources: usize,
    unpriced: bool,
}

/// Historical token totals aggregate provider/model/day buckets, NOT account quotas.
/// Without a caller-supplied clock freshness cannot be established. Use history_rows_at
/// for a validated readAt observation; this clock-free form never claims freshness.
pub fn history_rows(summary: &UsageSummary) -> Vec<Value> {
    project_history(summary, None)
}

pub fn history_rows_at(summary: &UsageSummary, now: i64) -> Vec<Value> {
    project_history(summary, Some(now))
}

fn project_history(summary: &UsageSummary, now: Option<i64>) -> Vec<Value> {
    let observed = now.and_then(|now| timestamp(&summary.read_at).filter(|t| *t <= now));
    let scan_state = match (now, observed) {
        (Some(now), Some(at)) if now.saturating_sub(at) > FRESH_SECONDS => "stale",
        (Some(_), Some(_)) => "fresh",
        _ => "unknown",
    };
    let mut by: BTreeMap<&str, History> = BTreeMap::new();
    for b in &summary.buckets {
        let h = by.entry(&b.provider).or_default();
        h.totals.add(&b.totals);
        h.cost += b.cost_usd;
        h.savings += b.cache_savings_usd;
        h.buckets += 1;
        h.unpriced |= b.cost_source == "unpriced" || b.cost_source.is_empty() || !b.cost_usd.is_finite() || !b.cache_savings_usd.is_finite();
    }
    for s in &summary.sources {
        let provider = s.fingerprint.get("provider").and_then(Value::as_str).unwrap_or("unknown");
        let h = by.entry(provider).or_default();
        h.sources += 1;
        h.failed_sources += usize::from(s.status != "ok");
    }
    by.into_iter().map(|(provider, h)| {
        let has_data = h.buckets > 0 || (h.sources > 0 && h.failed_sources == 0);
        let state = if !has_data { "unavailable" } else if h.failed_sources > 0 { "stale" } else { scan_state };
        json!({"id": format!("history:{provider}"), "title": provider,
            "detail": format!("{} to {} · provider aggregate, not account attribution or remaining quota · {state} historical scan{}{}", summary.since_day,
                summary.until_day, if h.failed_sources > 0 { " · incomplete source scan" } else if observed.is_none() { " · observation time unverified" } else { "" },
                if h.unpriced { " · API-equivalent pricing incomplete" } else { "" }),
            "state": state, "source": "t3-server-token-history", "observed_at": observed, "read_at": summary.read_at,
            "semantics": "provider-aggregate-historical-tokens-not-account-quota", "provider": provider,
            "since_day": summary.since_day, "until_day": summary.until_day,
            "tokens": if has_data { Some(h.totals.total()) } else { None },
            "input_tokens": if has_data { Some(h.totals.input()) } else { None },
            "output_tokens": if has_data { Some(h.totals.output) } else { None },
            "reasoning_tokens": if has_data { Some(h.totals.reasoning) } else { None },
            "api_equivalent_cost_usd": if has_data && !h.unpriced { Some(h.cost) } else { None },
            "cache_savings_usd": if has_data && !h.unpriced { Some(h.savings) } else { None },
            "buckets": h.buckets, "failed_sources": h.failed_sources})
    }).collect()
}

/// The wire models intentionally default unknown fields for forwards compatibility.
/// A monitoring response must additionally prove that it actually contains history.
pub fn decode_history(mut raw: Value) -> Result<UsageSummary, String> {
    let valid_day = |v: &Value| v.as_str().is_some_and(|s| s.len() == 10 && timestamp(&format!("{s}T00:00:00Z")).is_some());
    if !valid_day(&raw["sinceDay"]) || !valid_day(&raw["untilDay"])
        || raw["sinceDay"].as_str() > raw["untilDay"].as_str()
        || !raw["buckets"].is_array() || !raw["sources"].is_array() {
        return Err("Missing or invalid history range, buckets or sources".into());
    }
    let since = raw["sinceDay"].as_str().unwrap().to_string();
    let until = raw["untilDay"].as_str().unwrap().to_string();
    for b in raw["buckets"].as_array_mut().unwrap() {
        if !valid_day(&b["day"]) || b["provider"].as_str().is_none_or(str::is_empty)
            || b["day"].as_str().is_some_and(|d| d < since.as_str() || d > until.as_str())
            || b["model"].as_str().is_none_or(str::is_empty) || !b["totals"].is_object()
            || ["uncachedInputTokens", "cachedInputTokens", "cacheCreationTokens", "outputTokens"]
                .iter().any(|key| b["totals"][*key].as_u64().is_none()) {
            return Err("Missing or invalid token bucket values".into());
        }
        if ["costUsd", "cacheSavingsUsd"].iter().any(|key| b[*key].as_f64().is_none_or(|n| !n.is_finite() || n < 0.0)) {
            b["costSource"] = json!("unpriced");
        }
    }
    if raw["sources"].as_array().unwrap().iter().any(|s|
        s["fingerprint"]["provider"].as_str().is_none_or(str::is_empty) || s["status"].as_str().is_none_or(str::is_empty)) {
        return Err("Missing or invalid history source identity/status".into());
    }
    serde_json::from_value(raw).map_err(|_| "Invalid history response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(id: &str, checked: &str) -> Provider {
        serde_json::from_value(json!({"instanceId": id, "driver":"codex", "displayName":"Codex", "enabled":true,
            "installed":true,"usageLimits":{"checkedAt":checked,"windows":[{"id":"session","usedPercent":25.0}]}})).unwrap()
    }
    #[test]
    fn freshness_boundary_and_future_are_not_live() {
        let p = account("work", "2026-10-25T12:00:00Z");
        let observed = timestamp("2026-10-25T12:00:00Z").unwrap();
        assert_eq!(account_rows(&[p.clone()], observed + FRESH_SECONDS)[0]["state"], "fresh");
        let stale = account_rows(&[p.clone()], observed + FRESH_SECONDS + 1);
        assert_eq!(stale[0]["state"], "stale");
        assert_eq!(stale[0]["windows"][0]["live"], false);
        let future = account_rows(&[p], observed - 1);
        assert_eq!(future[0]["state"], "unknown");
        assert!(future[0]["observed_at"].is_null());
    }
    #[test]
    fn failed_missing_and_invalid_observations_keep_account_identity() {
        let now = timestamp("2026-10-25T12:00:00Z").unwrap();
        let mut p = account("work", "2026-10-25T12:00:00Z");
        p.usage_limits.as_mut().unwrap().unavailable = Some(json!({"message":"fetch failed"}));
        let mut second = account("personal", "invalid");
        second.usage_limits = None;
        let rows = account_rows(&[p, second], now);
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0]["id"], rows[1]["id"]);
        assert!(rows.iter().all(|r| r["state"] == "unavailable"));
        assert_eq!(rows[0]["windows"][0]["live"], false);
        assert!(rows[1]["windows"].as_array().unwrap().is_empty());
        let mut p = account("work", "2026-10-25T12:00:00Z");
        p.usage_limits.as_mut().unwrap().windows[0].used_percent = f64::NAN;
        let rows = account_rows(&[p], now);
        assert_eq!(rows[0]["state"], "unknown");
        assert!(rows[0]["windows"][0]["used_percent"].is_null());
    }
    #[test]
    fn history_aggregates_providers_not_account_quota_or_reasoning_twice() {
        let summary: UsageSummary = serde_json::from_value(json!({"sinceDay":"2026-10-24","untilDay":"2026-10-25",
            "buckets":[{"provider":"codex","totals":{"outputTokens":20,"reasoningTokens":7},"costSource":"unpriced"},
                {"provider":"codex","totals":{"uncachedInputTokens":10,"outputTokens":30}}],
            "sources":[{"fingerprint":{"provider":"claude"},"status":"failed"}]})).unwrap();
        let rows = history_rows(&summary);
        let codex = rows.iter().find(|r| r["provider"] == "codex").unwrap();
        assert_eq!(codex["tokens"], 60);
        assert_eq!(codex["reasoning_tokens"], 7);
        assert!(codex["api_equivalent_cost_usd"].is_null());
        assert!(codex["observed_at"].is_null());
        assert!(codex["detail"].as_str().unwrap().contains("not account attribution"));
        let failed = rows.iter().find(|r| r["provider"] == "claude").unwrap();
        assert_eq!(failed["state"], "unavailable");
        assert!(failed["tokens"].is_null());
    }
    #[test]
    fn history_scan_freshness_is_not_the_receipt_time() {
        let mut summary: UsageSummary = serde_json::from_value(json!({"readAt":"2026-10-25T12:00:00Z",
            "sources":[{"fingerprint":{"provider":"codex"},"status":"ok"}]})).unwrap();
        let observed = timestamp(&summary.read_at).unwrap();
        assert_eq!(history_rows_at(&summary, observed + FRESH_SECONDS)[0]["state"], "fresh");
        assert_eq!(history_rows_at(&summary, observed + FRESH_SECONDS + 1)[0]["state"], "stale");
        assert_eq!(history_rows_at(&summary, observed - 1)[0]["state"], "unknown");
        assert!(history_rows_at(&summary, observed - 1)[0]["observed_at"].is_null());
        assert_eq!(history_rows(&summary)[0]["state"], "unknown");
        summary.read_at = "invalid".into();
        assert_eq!(history_rows_at(&summary, observed)[0]["state"], "unknown");
    }
    #[test]
    fn empty_object_and_missing_amount_are_not_zero_token_history() {
        assert!(decode_history(json!({})).is_err());
        assert!(decode_history(json!({"sinceDay":"2026-10-24","untilDay":"2026-10-25","sources":[],
            "buckets":[{"day":"2026-10-24","provider":"codex","model":"model","totals":{}}]})).is_err());
        assert!(decode_history(json!({"sinceDay":"2026-10-24","untilDay":"2026-10-25","buckets":[],"sources":[]})).is_ok());
    }
}
