//! `mseries-update status --json` (schema 1) and the state a UI shows for it.
//!
//! Parsing is tolerant of fields it does not know (the contract may grow within schema 1) and
//! strict about the schema number: a newer schema is reported, never guessed at.

use serde::Deserialize;
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Status {
    pub schema: u32,
    pub channel: String,
    #[serde(default)]
    pub auto_stage: bool,
    /// `idle`, `staged`, `awaiting-confirmation` or `recovering`.
    pub state: String,
    #[serde(default)]
    pub staged: Option<Staged>,
    #[serde(default)]
    pub last_applied: Option<Applied>,
    #[serde(default)]
    pub last_check: Option<Check>,
    /// package -> versions that were rolled back and are not offered again.
    #[serde(default)]
    pub skipped: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub last_result: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Staged {
    pub target: BTreeMap<String, String>,
    #[serde(default)]
    pub from: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Applied {
    pub from: BTreeMap<String, String>,
    pub to: BTreeMap<String, String>,
    #[serde(default)]
    pub rolled_back: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Check {
    pub time: String,
    pub updates: BTreeMap<String, Version>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Version {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Outcome {
    pub time: String,
    pub outcome: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Written by a newer updater than this UI understands.
    UnsupportedSchema(u32),
    Invalid(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnsupportedSchema(n) => write!(f, "status schema {n} is newer than this app understands"),
            ParseError::Invalid(why) => write!(f, "unreadable status: {why}"),
        }
    }
}

impl Status {
    pub fn parse(json: &str) -> Result<Status, ParseError> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| ParseError::Invalid(e.to_string()))?;
        match value.get("schema").and_then(|s| s.as_u64()) {
            Some(n) if n == u64::from(SCHEMA) => {}
            Some(n) => return Err(ParseError::UnsupportedSchema(n as u32)),
            None => return Err(ParseError::Invalid("no schema field".into())),
        }
        serde_json::from_value(value).map_err(|e| ParseError::Invalid(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub from: Option<String>,
    pub to: String,
}

/// What the main area should say. One value, derived only from the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// An interrupted transaction is being restored (next boot or now).
    Recovering,
    /// A new set is installed and waiting for a healthy login; the user may roll it back.
    AwaitingConfirmation(Vec<Change>),
    /// Downloaded and verified; installs at the next restart.
    Staged(Vec<Change>),
    Available(Vec<Change>),
    UpToDate,
    NeverChecked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The last transaction was rolled back, and why.
    RolledBack { requested: bool, detail: String },
    /// Rolling back failed: the package set needs attention.
    RollbackFailed { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub phase: Phase,
    pub channel: String,
    pub can_roll_back: bool,
    pub can_switch_channel: bool,
    /// Unix seconds of the last check, if any.
    pub last_checked: Option<i64>,
    pub notice: Option<Notice>,
}

impl Summary {
    pub fn from_status(status: &Status) -> Summary {
        let phase = Self::phase(status);
        let rollback_pending = status.state == "awaiting-confirmation";
        let applied_ok = status.last_applied.as_ref().is_some_and(|a| a.rolled_back.is_none());
        let can_roll_back = status.state != "recovering" && applied_ok;
        // The updater refuses a switch during recovery or while a new set awaits confirmation.
        let can_switch_channel = !rollback_pending && status.state != "recovering";
        let last_checked = status.last_check.as_ref().and_then(|c| parse_utc(&c.time));
        let notice = status.last_result.as_ref().and_then(|r| match r.outcome.as_str() {
            "rollback-failed" => Some(Notice::RollbackFailed { detail: r.detail.clone() }),
            "rolled-back" => Some(Notice::RolledBack { requested: r.detail == "requested", detail: r.detail.clone() }),
            _ => None,
        });
        Summary { phase, channel: status.channel.clone(), can_roll_back, can_switch_channel, last_checked, notice }
    }

    fn phase(status: &Status) -> Phase {
        match status.state.as_str() {
            "recovering" => return Phase::Recovering,
            "awaiting-confirmation" => {
                let changes = status.last_applied.as_ref().map(|a| changes(&a.from, &a.to)).unwrap_or_default();
                return Phase::AwaitingConfirmation(changes);
            }
            "staged" => {
                if let Some(staged) = &status.staged {
                    return Phase::Staged(changes(&staged.from, &staged.target));
                }
            }
            _ => {}
        }
        let Some(check) = &status.last_check else { return Phase::NeverChecked };
        let applied = status.last_applied.as_ref().filter(|a| a.rolled_back.is_none());
        // last_check is a snapshot: drop entries that were installed since, or rolled back and skipped.
        let offered: Vec<Change> = check
            .updates
            .iter()
            .filter(|(name, v)| {
                let installed = applied.is_some_and(|a| a.to.get(*name) == Some(&v.to));
                let skipped = status.skipped.get(*name).is_some_and(|s| s.contains(&v.to));
                !installed && !skipped
            })
            .map(|(name, v)| Change { name: name.clone(), from: Some(v.from.clone()), to: v.to.clone() })
            .collect();
        if offered.is_empty() { Phase::UpToDate } else { Phase::Available(offered) }
    }
}

fn changes(from: &BTreeMap<String, String>, to: &BTreeMap<String, String>) -> Vec<Change> {
    to.iter()
        .map(|(name, version)| Change { name: name.clone(), from: from.get(name).cloned(), to: version.clone() })
        .collect()
}

/// `YYYY-MM-DDTHH:MM:SSZ` (what the updater writes) to Unix seconds.
pub fn parse_utc(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Age {
    JustNow,
    Minutes(i64),
    Hours(i64),
    Days(i64),
}

pub fn age(checked: i64, now: i64) -> Age {
    let secs = (now - checked).max(0);
    match secs {
        0..=89 => Age::JustNow,
        90..=3_599 => Age::Minutes((secs + 30) / 60),
        3_600..=86_399 => Age::Hours(secs / 3_600),
        _ => Age::Days(secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(json: &str) -> Status {
        Status::parse(json).unwrap_or_else(|e| panic!("{e}: {json}"))
    }

    #[test]
    fn a_real_idle_status_with_no_check_yet() {
        let s = status(
            r#"{"schema":1,"channel":"stable","auto_stage":false,"state":"idle","staged":null,
                "pending_confirmation":null,"last_applied":null,"last_check":null,"skipped":{},
                "last_result":null,"power":{"source":"ac","percent":null},"future_field":[1]}"#,
        );
        let summary = Summary::from_status(&s);
        assert_eq!(summary.phase, Phase::NeverChecked);
        assert!(!summary.can_roll_back);
        assert!(summary.can_switch_channel);
        assert_eq!(summary.last_checked, None);
    }

    #[test]
    fn available_updates_list_old_and_new_versions() {
        let s = status(
            r#"{"schema":1,"channel":"dev","state":"idle","last_check":{"time":"2026-10-21T20:07:11Z","channel":"dev",
                "updates":{"mseries-quick-settings":{"from":"1.4.0-0mseries3","to":"1.4.0-0mseries4"}}}}"#,
        );
        let summary = Summary::from_status(&s);
        assert_eq!(
            summary.phase,
            Phase::Available(vec![Change {
                name: "mseries-quick-settings".into(),
                from: Some("1.4.0-0mseries3".into()),
                to: "1.4.0-0mseries4".into()
            }])
        );
        assert_eq!(summary.last_checked, Some(1_792_613_231));
    }

    #[test]
    fn an_update_already_installed_or_rolled_back_is_not_offered_again() {
        let installed = status(
            r#"{"schema":1,"channel":"stable","state":"idle",
                "last_check":{"time":"2026-10-21T20:07:11Z","updates":{"a":{"from":"1","to":"2"}}},
                "last_applied":{"from":{"a":"1"},"to":{"a":"2"},"rolled_back":null}}"#,
        );
        assert_eq!(Summary::from_status(&installed).phase, Phase::UpToDate);
        let rolled_back = status(
            r#"{"schema":1,"channel":"stable","state":"idle","skipped":{"a":["2"]},
                "last_check":{"time":"2026-10-21T20:07:11Z","updates":{"a":{"from":"1","to":"2"}}},
                "last_applied":{"from":{"a":"1"},"to":{"a":"2"},"rolled_back":"2026-10-21T21:00:00Z"}}"#,
        );
        let summary = Summary::from_status(&rolled_back);
        assert_eq!(summary.phase, Phase::UpToDate);
        assert!(!summary.can_roll_back, "a rolled-back update cannot be rolled back twice");
    }

    #[test]
    fn staged_and_awaiting_confirmation_come_from_the_state_not_the_check() {
        let staged = status(
            r#"{"schema":1,"channel":"stable","state":"staged",
                "staged":{"id":"x","target":{"a":"2","b":"5"},"from":{"a":"1"}},
                "last_check":{"time":"2026-10-21T20:07:11Z","updates":{}}}"#,
        );
        let summary = Summary::from_status(&staged);
        assert_eq!(
            summary.phase,
            Phase::Staged(vec![
                Change { name: "a".into(), from: Some("1".into()), to: "2".into() },
                Change { name: "b".into(), from: None, to: "5".into() },
            ])
        );
        assert!(summary.can_switch_channel, "the updater discards a staged update on a switch");

        let waiting = status(
            r#"{"schema":1,"channel":"stable","state":"awaiting-confirmation",
                "pending_confirmation":{"id":"x"},
                "last_applied":{"from":{"a":"1"},"to":{"a":"2"},"rolled_back":null}}"#,
        );
        let summary = Summary::from_status(&waiting);
        assert!(matches!(summary.phase, Phase::AwaitingConfirmation(ref c) if c.len() == 1));
        assert!(summary.can_roll_back);
        assert!(!summary.can_switch_channel, "the updater refuses a switch before the update is confirmed");
    }

    #[test]
    fn recovery_blocks_rollback_and_channel_switch() {
        let s = status(r#"{"schema":1,"channel":"stable","state":"recovering",
            "last_applied":{"from":{"a":"1"},"to":{"a":"2"}}}"#);
        let summary = Summary::from_status(&s);
        assert_eq!(summary.phase, Phase::Recovering);
        assert!(!summary.can_roll_back && !summary.can_switch_channel);
    }

    #[test]
    fn last_results_become_notices() {
        let requested = status(r#"{"schema":1,"channel":"stable","state":"idle",
            "last_result":{"time":"2026-10-21T20:07:11Z","outcome":"rolled-back","detail":"requested"}}"#);
        assert_eq!(
            Summary::from_status(&requested).notice,
            Some(Notice::RolledBack { requested: true, detail: "requested".into() })
        );
        let failed = status(r#"{"schema":1,"channel":"stable","state":"idle",
            "last_result":{"time":"2026-10-21T20:07:11Z","outcome":"rollback-failed","detail":"copy missing"}}"#);
        assert_eq!(Summary::from_status(&failed).notice, Some(Notice::RollbackFailed { detail: "copy missing".into() }));
        let applied = status(r#"{"schema":1,"channel":"stable","state":"idle",
            "last_result":{"time":"2026-10-21T20:07:11Z","outcome":"applied","detail":""}}"#);
        assert_eq!(Summary::from_status(&applied).notice, None);
    }

    #[test]
    fn a_newer_or_broken_schema_is_reported_not_guessed() {
        assert_eq!(Status::parse(r#"{"schema":2,"channel":"stable","state":"idle"}"#), Err(ParseError::UnsupportedSchema(2)));
        assert!(matches!(Status::parse("not json"), Err(ParseError::Invalid(_))));
        assert!(matches!(Status::parse(r#"{"channel":"stable"}"#), Err(ParseError::Invalid(_))));
        assert!(matches!(Status::parse(r#"{"schema":1,"state":"idle"}"#), Err(ParseError::Invalid(_))));
    }

    #[test]
    fn utc_parsing_matches_known_instants() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_utc("2026-10-21T20:07:11Z"), Some(1_792_613_231));
        assert_eq!(parse_utc("2026-10-21 20:07:11"), None);
        assert_eq!(parse_utc("2026-13-21T20:07:11Z"), None);
        assert_eq!(parse_utc(""), None);
    }

    #[test]
    fn ages_round_to_what_a_person_would_say() {
        let t = 1_000_000;
        assert_eq!(age(t, t), Age::JustNow);
        assert_eq!(age(t, t + 89), Age::JustNow);
        assert_eq!(age(t, t + 120), Age::Minutes(2));
        assert_eq!(age(t, t + 7_300), Age::Hours(2));
        assert_eq!(age(t, t + 3 * 86_400 + 5), Age::Days(3));
        assert_eq!(age(t, t - 50), Age::JustNow, "a clock behind the check never shows a negative age");
    }
}
