//! Read-only Omarchy subscription records. These describe this computer, not a paired T3 server.
//! Never feed these supplemental records into provider selection or credential management.

use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_BYTES: u64 = 1_048_576;
pub const MAX_FILES: usize = 64;
pub const FRESH_SECONDS: i64 = 15 * 60;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub title: String,
    /// Fraction, unlike T3's 0–100 usedPercent.
    pub percent: f64,
    #[serde(default)]
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Balance {
    pub remaining: f64,
    pub currency: String,
    #[serde(default)]
    pub estimated: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub schema_version: u64,
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub ready: bool,
    #[serde(default)]
    pub tier_label: String,
    #[serde(default)]
    pub usage_status_text: String,
    #[serde(default)]
    pub auth_help_text: String,
    #[serde(default)]
    pub limits: Vec<Window>,
    #[serde(default)]
    pub balance: Option<Balance>,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub record: Record,
    pub observed: i64,
    pub stale: bool,
}

#[derive(Default, Debug)]
pub struct Snapshot {
    pub reports: Vec<Report>,
    pub problems: Vec<String>,
}

/// Timestamp validation is supplied by GLib in the app, so model tests need no GTK runtime.
pub fn parse(raw: &str, now: i64, timestamp: impl Fn(&str) -> Option<i64>) -> Result<Report, String> {
    let record: Record = serde_json::from_str(raw).map_err(|_| "Invalid subscription JSON".to_string())?;
    if record.schema_version != 1 || record.id.is_empty() || record.name.trim().is_empty() {
        return Err("Unsupported subscription record".into());
    }
    if record.limits.len() > 64
        || record.limits.iter().any(|w| !w.percent.is_finite() || !(0.0..=1.0).contains(&w.percent))
        || record.balance.as_ref().is_some_and(|b| !b.remaining.is_finite() || b.currency.is_empty())
    {
        return Err("Invalid subscription amounts".into());
    }
    if record
        .limits
        .iter()
        .any(|w| w.resets_at.as_deref().filter(|s| !s.is_empty()).is_some_and(|s| timestamp(s).is_none()))
    {
        return Err("Invalid subscription reset time".into());
    }
    let observed = timestamp(&record.updated_at).ok_or("Invalid subscription observation time")?;
    if observed > now + 60 || observed < 0 {
        return Err("Subscription observation time is in the future or invalid".into());
    }
    // History-based records are ready upstream, but are never fresh observations.
    let stale = now.saturating_sub(observed) > FRESH_SECONDS
        || record.usage_status_text.to_ascii_lowercase().contains("last known");
    Ok(Report { record, observed, stale })
}

pub fn directory() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| local_home().join(".local/state"));
    state.join("omarchy/agents/usage")
}

fn local_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// No collector execution, config/credential reads or writes. Work runs off the GTK main loop.
pub fn read(dir: &Path, now: i64, timestamp: impl Fn(&str) -> Option<i64>) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return snapshot,
        Err(_) => {
            snapshot.problems.push("Could not read local subscription reports".into());
            return snapshot;
        }
    };
    // Cap discovery as well as file reads. Never follow symlinks or read devices/pipes.
    for (i, entry) in entries.enumerate() {
        if i >= MAX_FILES {
            snapshot.problems.push("Local report directory exceeds the 64-entry limit; results are partial".into());
            break;
        }
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let valid_file = entry.file_type().is_ok_and(|t| t.is_file());
        if !valid_file {
            continue;
        }
        let result = (|| {
            let file = std::fs::File::open(&path).map_err(|_| "Unreadable report".to_string())?;
            if file.metadata().map_err(|_| "Unreadable report".to_string())?.len() > MAX_BYTES {
                return Err("Report exceeds 1 MiB".into());
            }
            let mut raw = String::new();
            file.take(MAX_BYTES + 1).read_to_string(&mut raw).map_err(|_| "Unreadable report".to_string())?;
            if raw.len() as u64 > MAX_BYTES {
                return Err("Report exceeds 1 MiB".into());
            }
            parse(&raw, now, &timestamp)
        })();
        match result {
            Ok(report) => snapshot.reports.push(report),
            Err(error) => snapshot.problems.push(error),
        }
    }
    snapshot.reports.sort_by(|a, b| (&a.record.name, &a.record.id).cmp(&(&b.record.name, &b.record.id)));
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        json!({"schemaVersion":1,"id":"codex-plan","name":"Codex","updatedAt":"test-time","ready":true,
            "limits":[{"title":"Session · A","percent":0.25},{"title":"Session · B","percent":1.0}]})
    }
    fn decode(v: serde_json::Value, now: i64) -> Result<Report, String> {
        parse(&v.to_string(), now, |s| (s == "test-time").then_some(1000))
    }
    #[test]
    fn preserves_accounts_and_fraction_units() {
        let r = decode(fixture(), 1000).unwrap();
        assert_eq!(r.record.limits.len(), 2);
        assert_eq!(r.record.limits[0].percent * 100.0, 25.0);
        assert_eq!(r.record.limits[1].percent, 1.0);
        assert!(!r.stale);
    }
    #[test]
    fn unknown_failed_and_historical_states_are_not_fresh_quota() {
        let mut v = fixture();
        v["ready"] = json!(false);
        assert!(!decode(v, 1000).unwrap().record.ready);
        assert!(decode(fixture(), 2000).unwrap().stale);
        let mut v = fixture();
        v["usageStatusText"] = json!("Last known usage (broker probe is down)");
        assert!(decode(v, 1000).unwrap().stale);
    }
    #[test]
    fn rejects_missing_invalid_or_future_data() {
        let mut v = fixture();
        v["limits"][0]["percent"] = json!(25);
        assert!(decode(v, 1000).is_err());
        let mut v = fixture();
        v.as_object_mut().unwrap().remove("ready");
        assert!(decode(v, 1000).is_err());
        let mut v = fixture();
        v["schemaVersion"] = json!(2);
        assert!(decode(v, 1000).is_err());
        let mut v = fixture();
        v["updatedAt"] = json!("bad");
        assert!(decode(v, 1000).is_err());
        assert!(decode(fixture(), 900).is_err());
    }
    #[test]
    fn distinguishes_absent_directory_from_invalid_records() {
        let dir = std::env::temp_dir().join(format!("t3-quota-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.json"), "{}").unwrap();
        std::fs::write(dir.join("good.json"), fixture().to_string()).unwrap();
        let s = read(&dir, 1000, |_| Some(1000));
        assert_eq!(s.reports.len(), 1);
        assert_eq!(s.problems.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(read(&dir, 1000, |_| Some(1000)).problems.is_empty());
    }
}
