//! Who may run what, and the audit trail (design: issue #649, "agent action layer").
//!
//! Every tool has a risk class. The owner's policy file decides per class whether a call runs, asks first or is refused;
//! model output can never widen it. Defaults: reads run, everything else asks. Tools or apps on the block lists are
//! refused before anything is shown. The audit log records tool, risk, decision and effect, never arguments or results.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Reads state; changes nothing.
    Read,
    /// Changes local state in a way the owner can undo (open an app, create a local task).
    Reversible,
    /// Acts in another app or beyond the machine (press a button, type into a field, open a link).
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    Run,
    Ask,
    Refuse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Run,
    Ask,
    Refuse(&'static str),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Policy {
    pub read: Rule,
    pub reversible: Rule,
    pub external: Rule,
    /// Tool names that are never run.
    pub blocked_tools: Vec<String>,
    /// App id or name fragments the driver never acts on (read or write).
    pub blocked_apps: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            read: Rule::Run,
            reversible: Rule::Ask,
            external: Rule::Ask,
            blocked_tools: Vec::new(),
            blocked_apps: [
                "terminal",
                "keepass",
                "bitwarden",
                "1password",
                "seahorse",
                "passwords",
                "lock",
                "greeter",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

impl Policy {
    pub fn path() -> PathBuf {
        config_dir().join("mseries/assistant-policy.json")
    }

    /// The owner's file, or the defaults. A malformed file blocks everything except reads, so a typo never widens access.
    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Self {
                reversible: Rule::Refuse,
                external: Rule::Refuse,
                ..Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn decide(&self, tool: &str, risk: Risk) -> Decision {
        if self.blocked_tools.iter().any(|name| name == tool) {
            return Decision::Refuse("blocked-tool");
        }
        match match risk {
            Risk::Read => self.read,
            Risk::Reversible => self.reversible,
            Risk::External => self.external,
        } {
            Rule::Run => Decision::Run,
            Rule::Ask => Decision::Ask,
            Rule::Refuse => Decision::Refuse("policy"),
        }
    }

    pub fn app_blocked(&self, app: &str) -> bool {
        let app = app.to_lowercase();
        !app.is_empty() && self.blocked_apps.iter().any(|fragment| app.contains(&fragment.to_lowercase()))
    }
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
        .join("mseries-assistant")
}

/// Appends one line to `~/.local/state/mseries-assistant/audit.jsonl`: no arguments, no results.
pub fn audit(client: &str, tool: &str, risk: Risk, decision: &str, effect: &str) {
    let dir = state_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = serde_json::json!({
        "t": seconds, "client": client, "tool": tool, "risk": risk, "decision": decision, "effect": effect,
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("audit.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_run_reads_and_ask_for_everything_else() {
        let policy = Policy::default();
        assert_eq!(policy.decide("list_windows", Risk::Read), Decision::Run);
        assert_eq!(policy.decide("launch_app", Risk::Reversible), Decision::Ask);
        assert_eq!(policy.decide("press_element", Risk::External), Decision::Ask);
    }

    #[test]
    fn blocked_tools_and_apps_are_refused() {
        let policy = Policy {
            blocked_tools: vec!["set_text".into()],
            ..Policy::default()
        };
        assert_eq!(policy.decide("set_text", Risk::Read), Decision::Refuse("blocked-tool"));
        assert!(policy.app_blocked("io.elementary.terminal"));
        assert!(policy.app_blocked("org.keepassxc.KeePassXC"));
        assert!(!policy.app_blocked("io.elementary.files"));
        assert!(!policy.app_blocked(""));
    }

    #[test]
    fn a_malformed_policy_never_widens_access() {
        let policy: Result<Policy, _> = serde_json::from_str("{\"reversible\": \"run\", \"external\": \"run\"}");
        assert_eq!(policy.unwrap().external, Rule::Run, "a valid file is honoured");
        assert!(serde_json::from_str::<Policy>("{not json").is_err());
    }
}
