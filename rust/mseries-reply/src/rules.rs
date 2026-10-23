//! Allow/block rules for Smart Reply (design section 6).
//!
//! Evaluation order, so behaviour is predictable:
//! 1. Smart Reply off, or a missing/malformed/loosely-permissioned rules file: everything is blocked.
//! 2. Built-in blocks that no rule can lift: password, card, one-time-code and unknown fields; terminals, password
//!    managers and the lock screen.
//! 3. Most specific matching rule wins: contact over field kind over app. At equal specificity block beats allow.
//! 4. No matching rule: the default policy (ask-first allow list unless the owner chose everywhere-except-blocked).
//!
//! Caps (`context_cap`) and `local_only` only ever tighten: they accumulate from every matching allow rule, whichever
//! specificity decided the outcome.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RULES_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Only apps the owner allowed.
    #[default]
    AskFirst,
    EverywhereExceptBlocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Allow,
    Block,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleScope {
    App,
    Contact,
    FieldKind,
}

impl RuleScope {
    fn specificity(self) -> u8 {
        match self {
            RuleScope::App => 1,
            RuleScope::FieldKind => 2,
            RuleScope::Contact => 3,
        }
    }
}

/// How much of the conversation a rule lets the context service read.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextCap {
    /// Window identity only, never the conversation.
    TitleOnly,
    /// At most this many messages.
    Messages(usize),
}

impl ContextCap {
    fn tighter(self, other: ContextCap) -> ContextCap {
        match (self, other) {
            (ContextCap::TitleOnly, _) | (_, ContextCap::TitleOnly) => ContextCap::TitleOnly,
            (ContextCap::Messages(a), ContextCap::Messages(b)) => ContextCap::Messages(a.min(b)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub scope: RuleScope,
    /// App id, contact/conversation name, or field kind name (`search`, `address`).
    pub value: String,
    pub action: Action,
    #[serde(default)]
    pub context_cap: Option<ContextCap>,
    #[serde(default)]
    pub local_only: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    /// Master switch. Nothing is read until it is on.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub hotkey: Option<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldKind {
    /// A composer the chat app itself declared.
    Chat,
    Search,
    Address,
    Password,
    Card,
    Otp,
    Unknown,
}

impl FieldKind {
    pub fn name(self) -> &'static str {
        match self {
            FieldKind::Chat => "chat",
            FieldKind::Search => "search",
            FieldKind::Address => "address",
            FieldKind::Password => "password",
            FieldKind::Card => "card",
            FieldKind::Otp => "otp",
            FieldKind::Unknown => "unknown",
        }
    }

    fn built_in_blocked(self) -> bool {
        matches!(
            self,
            FieldKind::Password | FieldKind::Card | FieldKind::Otp | FieldKind::Unknown
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockReason {
    Disabled,
    /// The rules file is missing, malformed, from a newer version or writable by others.
    RulesUnreadable,
    BuiltInApp,
    BuiltInField,
    Rule,
    /// Not on the allow list under the ask-first policy.
    NotAllowed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Grant {
    pub cap: Option<ContextCap>,
    pub local_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Allow(Grant),
    Block(BlockReason),
}

#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub app_id: &'a str,
    pub field: FieldKind,
    /// Known only after the minimum header has been read; `None` in the first phase.
    pub contact: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct Rules {
    settings: Settings,
    unreadable: bool,
}

impl Rules {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            unreadable: false,
        }
    }

    /// Everything blocked: the state for a missing or malformed rules file.
    pub fn locked() -> Self {
        Self {
            settings: Settings {
                version: RULES_VERSION,
                enabled: false,
                policy: Policy::AskFirst,
                hotkey: None,
                rules: Vec::new(),
            },
            unreadable: true,
        }
    }

    pub fn default_path() -> PathBuf {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_default();
        config.join("mseries").join("smart-reply.json")
    }

    /// Loads the rules. A missing file, bad JSON, an unknown key, a version other than [`RULES_VERSION`] or a file that
    /// group/other can write all give [`Rules::locked`].
    pub fn load(path: &Path) -> Self {
        Self::try_load(path).unwrap_or_else(|_| Self::locked())
    }

    fn try_load(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .permissions()
            .mode();
        if mode & 0o022 != 0 {
            return Err("rules file is writable by others".into());
        }
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let settings: Settings = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if settings.version != RULES_VERSION {
            return Err("unsupported rules version".into());
        }
        Ok(Self::new(settings))
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn decide(&self, query: &Query<'_>) -> Decision {
        if self.unreadable {
            return Decision::Block(BlockReason::RulesUnreadable);
        }
        if !self.settings.enabled {
            return Decision::Block(BlockReason::Disabled);
        }
        if built_in_blocked_app(query.app_id) {
            return Decision::Block(BlockReason::BuiltInApp);
        }
        if query.field.built_in_blocked() {
            return Decision::Block(BlockReason::BuiltInField);
        }

        let matching: Vec<&Rule> = self
            .settings
            .rules
            .iter()
            .filter(|rule| matches_rule(rule, query))
            .collect();

        let mut grant = Grant::default();
        for rule in matching.iter().filter(|rule| rule.action == Action::Allow) {
            grant.local_only |= rule.local_only;
            grant.cap = match (grant.cap, rule.context_cap) {
                (Some(a), Some(b)) => Some(a.tighter(b)),
                (a, b) => a.or(b),
            };
        }

        let top = matching.iter().map(|rule| rule.scope.specificity()).max();
        match top {
            Some(top) => {
                let at_top = matching
                    .iter()
                    .filter(|rule| rule.scope.specificity() == top);
                if at_top.clone().any(|rule| rule.action == Action::Block) {
                    Decision::Block(BlockReason::Rule)
                } else {
                    Decision::Allow(grant)
                }
            }
            None => match self.settings.policy {
                Policy::AskFirst => Decision::Block(BlockReason::NotAllowed),
                Policy::EverywhereExceptBlocked => Decision::Allow(grant),
            },
        }
    }
}

fn matches_rule(rule: &Rule, query: &Query<'_>) -> bool {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    match rule.scope {
        RuleScope::App => same(&rule.value, query.app_id),
        RuleScope::FieldKind => same(&rule.value, query.field.name()),
        RuleScope::Contact => query
            .contact
            .is_some_and(|contact| same(&rule.value, contact)),
    }
}

/// Terminals, password managers and the lock screen. No rule can lift these.
fn built_in_blocked_app(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    const FRAGMENTS: &[&str] = &[
        "terminal",
        "konsole",
        "kitty",
        "alacritty",
        "wezterm",
        "ghostty",
        "xterm",
        "foot",
        "keepass",
        "bitwarden",
        "1password",
        "seahorse",
        "passwords",
        "gtklock",
        "pantheon-lock",
        "screensaver",
        "greeter",
    ];
    FRAGMENTS.iter().any(|fragment| id.contains(fragment))
}

/// Checks a hotkey against the shortcuts Smart Reply must never shadow.
pub fn validate_hotkey(hotkey: &str) -> Result<(), String> {
    const RESERVED: &[&str] = &[
        "F5",
        "<Control>space",
        "<Super>space",
        "<Control><Alt>t",
        "<Super>l",
        "<Super>Tab",
        "<Alt>Tab",
        "<Control>c",
        "<Control>v",
        "<Control>x",
    ];
    let normalised = hotkey.replace(' ', "");
    if normalised.is_empty() {
        return Err("the hotkey is empty".into());
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&normalised)) {
        return Err(format!(
            "{hotkey} is already used by dictation or the shell"
        ));
    }
    if !normalised.contains('<') {
        return Err("use at least one modifier so typing is never intercepted".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(scope: RuleScope, value: &str, action: Action) -> Rule {
        Rule {
            scope,
            value: value.into(),
            action,
            context_cap: None,
            local_only: false,
        }
    }

    fn rules(policy: Policy, list: Vec<Rule>) -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy,
            hotkey: None,
            rules: list,
        })
    }

    fn chat<'a>(app: &'a str, contact: Option<&'a str>) -> Query<'a> {
        Query {
            app_id: app,
            field: FieldKind::Chat,
            contact,
        }
    }

    #[test]
    fn off_by_default_and_ask_first_blocks_unlisted_apps() {
        let off = Rules::new(Settings {
            version: RULES_VERSION,
            enabled: false,
            policy: Policy::EverywhereExceptBlocked,
            hotkey: None,
            rules: vec![],
        });
        assert_eq!(
            off.decide(&chat("a", None)),
            Decision::Block(BlockReason::Disabled)
        );

        let r = rules(
            Policy::AskFirst,
            vec![rule(RuleScope::App, "chat.app", Action::Allow)],
        );
        assert!(matches!(
            r.decide(&chat("chat.app", None)),
            Decision::Allow(_)
        ));
        assert_eq!(
            r.decide(&chat("other", None)),
            Decision::Block(BlockReason::NotAllowed)
        );
    }

    #[test]
    fn everywhere_except_blocked_still_honours_blocks() {
        let r = rules(
            Policy::EverywhereExceptBlocked,
            vec![rule(RuleScope::App, "chat.app", Action::Block)],
        );
        assert_eq!(
            r.decide(&chat("chat.app", None)),
            Decision::Block(BlockReason::Rule)
        );
        assert!(matches!(r.decide(&chat("other", None)), Decision::Allow(_)));
    }

    #[test]
    fn most_specific_rule_wins_and_block_beats_allow_at_equal_specificity() {
        let r = rules(
            Policy::AskFirst,
            vec![
                rule(RuleScope::App, "chat.app", Action::Block),
                rule(RuleScope::Contact, "Sam", Action::Allow),
                rule(RuleScope::Contact, "Pat", Action::Block),
                rule(RuleScope::Contact, "Pat", Action::Allow),
            ],
        );
        // Contact allow overrides the app block ...
        assert!(matches!(
            r.decide(&chat("chat.app", Some("sam"))),
            Decision::Allow(_)
        ));
        // ... a contact block overrides nothing less than itself, and block beats allow at the same level.
        assert_eq!(
            r.decide(&chat("chat.app", Some("Pat"))),
            Decision::Block(BlockReason::Rule)
        );
        // Before the header is read the app block applies.
        assert_eq!(
            r.decide(&chat("chat.app", None)),
            Decision::Block(BlockReason::Rule)
        );
    }

    #[test]
    fn built_ins_cannot_be_lifted_by_any_rule() {
        let r = rules(
            Policy::EverywhereExceptBlocked,
            vec![
                rule(RuleScope::App, "io.elementary.terminal", Action::Allow),
                rule(RuleScope::FieldKind, "password", Action::Allow),
            ],
        );
        assert_eq!(
            r.decide(&chat("io.elementary.terminal", None)),
            Decision::Block(BlockReason::BuiltInApp)
        );
        for field in [
            FieldKind::Password,
            FieldKind::Card,
            FieldKind::Otp,
            FieldKind::Unknown,
        ] {
            let query = Query {
                app_id: "chat.app",
                field,
                contact: None,
            };
            assert_eq!(
                r.decide(&query),
                Decision::Block(BlockReason::BuiltInField),
                "{field:?}"
            );
        }
        for app in [
            "org.keepassxc.KeePassXC",
            "com.bitwarden.desktop",
            "gtklock",
            "org.techmore.mseries.ApplePasswords",
        ] {
            assert_eq!(
                r.decide(&chat(app, None)),
                Decision::Block(BlockReason::BuiltInApp),
                "{app}"
            );
        }
    }

    #[test]
    fn caps_and_local_only_accumulate_and_only_tighten() {
        let mut app = rule(RuleScope::App, "chat.app", Action::Allow);
        app.context_cap = Some(ContextCap::Messages(8));
        let mut contact = rule(RuleScope::Contact, "Sam", Action::Allow);
        contact.context_cap = Some(ContextCap::Messages(3));
        contact.local_only = true;
        let r = rules(Policy::AskFirst, vec![app, contact]);
        assert_eq!(
            r.decide(&chat("chat.app", Some("Sam"))),
            Decision::Allow(Grant {
                cap: Some(ContextCap::Messages(3)),
                local_only: true
            })
        );
        assert_eq!(
            ContextCap::Messages(3).tighter(ContextCap::TitleOnly),
            ContextCap::TitleOnly
        );
    }

    fn write_rules(name: &str, text: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("mseries-reply-rules-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("smart-reply.json");
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn missing_malformed_or_loose_rules_files_block_everything() {
        let good =
            r#"{"version":1,"enabled":true,"policy":"everywhere-except-blocked","rules":[]}"#;
        let ok = write_rules("ok", good, 0o600);
        assert!(matches!(
            Rules::load(&ok).decide(&chat("a", None)),
            Decision::Allow(_)
        ));

        for (name, text, mode) in [
            ("malformed", "{ not json", 0o600),
            (
                "unknown-key",
                r#"{"version":1,"enabled":true,"surprise":1}"#,
                0o600,
            ),
            ("newer", r#"{"version":2,"enabled":true}"#, 0o600),
            ("loose", good, 0o666),
        ] {
            let path = write_rules(name, text, mode);
            assert_eq!(
                Rules::load(&path).decide(&chat("a", None)),
                Decision::Block(BlockReason::RulesUnreadable),
                "{name}"
            );
        }
        let missing = std::env::temp_dir().join("mseries-reply-does-not-exist.json");
        assert_eq!(
            Rules::load(&missing).decide(&chat("a", None)),
            Decision::Block(BlockReason::RulesUnreadable)
        );
    }

    #[test]
    fn hotkey_must_not_shadow_dictation_or_plain_typing() {
        assert!(validate_hotkey("<Super>period").is_ok());
        assert!(validate_hotkey("F5").is_err());
        assert!(validate_hotkey("<Control>space").is_err());
        assert!(validate_hotkey("a").is_err());
        assert!(validate_hotkey("").is_err());
    }
}
