//! The context packet (design section 2, tiers C0 and C2) and the gate in front of it.
//!
//! [`build_packet`] is the only way to get a [`Packet`], and it reads in a fixed order so a refusal at any step means
//! nothing later was read:
//! 1. window identity (C0, not content) and the chat marker: an unmarked app stops here;
//! 2. field role and label (no text): password, card, one-time-code and unknown fields stop here;
//! 3. rules decision from app and field alone: a blocked app stops here, before any message is read;
//! 4. the minimum header (conversation title), then the rules decision again with the contact: a blocked contact stops
//!    here and the header is dropped, never logged;
//! 5. the transcript, bounded by the grant's cap and the global limits.
//!
//! The composer's current text (C1) is deliberately not read in this slice: the draft does not use it.

use crate::marker::ChatMarker;
use crate::rules::{BlockReason, ContextCap, Decision, FieldKind, Query, Rules};
use serde::Serialize;

pub const MAX_LINES: usize = 12;
pub const MAX_CHARS: usize = 12_000;

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub pid: u32,
}

/// What the accessibility layer (or the app's own structured source) says about the focused field, without its text.
#[derive(Clone, Debug, Default)]
pub struct FieldDescriptor {
    pub role: String,
    pub label: String,
    /// GTK/AT-SPI input purpose, for example `password`, `pin`, `terminal`.
    pub purpose: Option<String>,
    /// The chat app itself named this field as its composer (structured source).
    pub declared_composer: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranscriptSource {
    /// The app named its transcript.
    Structured,
    /// A bounded accessibility-tree read above the composer.
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub speaker: String,
    pub from_owner: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Transcript {
    pub source: TranscriptSource,
    /// Oldest first.
    pub lines: Vec<Line>,
}

/// Where the content comes from. Implemented by the AT-SPI reader and the app adapters; tests use a counting fake.
pub trait ChatSource {
    fn window(&self) -> WindowInfo;
    fn field(&self) -> FieldDescriptor;
    fn header(&self) -> Option<String>;
    /// At most `max_lines` of the newest messages, oldest first.
    fn transcript(&self, max_lines: usize) -> Transcript;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    NotChatApp,
    Blocked(BlockReason),
    /// The grant allows the app but caps the context so low there is nothing to reply to.
    CapLeavesNoContext,
    /// The focused field is not a chat composer (a search box or address bar), so there is nothing to reply in.
    NotComposer,
    EmptyTranscript,
    NoIncomingMessage,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tier {
    pub tier: &'static str,
    pub source: &'static str,
    pub chars: usize,
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub app_id: String,
    pub window_title: String,
    pub contact: Option<String>,
    pub lines: Vec<Line>,
    pub tiers: Vec<Tier>,
    pub truncated: bool,
    pub local_only: bool,
}

pub fn classify(field: &FieldDescriptor) -> FieldKind {
    let text = format!(
        "{} {} {}",
        field.role,
        field.label,
        field.purpose.as_deref().unwrap_or("")
    )
    .to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| text.contains(word));
    if has(&[
        "password",
        "passcode",
        "passphrase",
        "secret",
        "token",
        "pin ",
    ]) || text.trim_end().ends_with(" pin")
    {
        FieldKind::Password
    } else if has(&["card", "cvv", "cvc", "credit", "iban", "routing"]) {
        FieldKind::Card
    } else if has(&[
        "otp",
        "one-time",
        "one time",
        "verification code",
        "security code",
        "2fa",
    ]) {
        FieldKind::Otp
    } else if field.declared_composer {
        FieldKind::Chat
    } else if has(&["search"]) {
        FieldKind::Search
    } else if has(&["address", "url", "location bar"]) {
        FieldKind::Address
    } else {
        FieldKind::Unknown
    }
}

pub fn build_packet(
    source: &dyn ChatSource,
    rules: &Rules,
    marker_for: &dyn Fn(&str) -> Option<ChatMarker>,
) -> Result<Packet, Refusal> {
    let window = source.window();
    marker_for(&window.app_id).ok_or(Refusal::NotChatApp)?;

    let kind = classify(&source.field());
    let first = Query {
        app_id: &window.app_id,
        field: kind,
        contact: None,
    };
    if let Decision::Block(reason) = rules.decide(&first) {
        return Err(Refusal::Blocked(reason));
    }
    if kind != FieldKind::Chat {
        return Err(Refusal::NotComposer);
    }

    let header = source.header();
    let second = Query {
        contact: header.as_deref(),
        ..first
    };
    let grant = match rules.decide(&second) {
        Decision::Allow(grant) => grant,
        Decision::Block(reason) => return Err(Refusal::Blocked(reason)),
    };

    let max_lines = match grant.cap {
        Some(ContextCap::TitleOnly) => return Err(Refusal::CapLeavesNoContext),
        Some(ContextCap::Messages(0)) => return Err(Refusal::CapLeavesNoContext),
        Some(ContextCap::Messages(n)) => n.min(MAX_LINES),
        None => MAX_LINES,
    };

    let transcript = source.transcript(max_lines);
    if transcript.lines.is_empty() {
        return Err(Refusal::EmptyTranscript);
    }
    let (lines, truncated) = bound(transcript.lines, max_lines);
    if lines.last().is_none_or(|line| line.from_owner) {
        return Err(Refusal::NoIncomingMessage);
    }

    let chars = lines.iter().map(|line| line.text.chars().count()).sum();
    let mut tiers = vec![Tier {
        tier: "C0",
        source: "window",
        chars: window.app_id.chars().count() + window.title.chars().count(),
    }];
    tiers.push(Tier {
        tier: "C2",
        source: match transcript.source {
            TranscriptSource::Structured => "structured",
            TranscriptSource::Fallback => "fallback",
        },
        chars,
    });
    Ok(Packet {
        app_id: window.app_id,
        window_title: window.title,
        contact: header,
        lines,
        tiers,
        truncated,
        local_only: grant.local_only,
    })
}

/// Newest `max_lines`, trimmed to [`MAX_CHARS`] from the old end. Returns whether anything was cut.
fn bound(mut lines: Vec<Line>, max_lines: usize) -> (Vec<Line>, bool) {
    let mut truncated = false;
    if lines.len() > max_lines {
        lines.drain(..lines.len() - max_lines);
        truncated = true;
    }
    let mut remaining = MAX_CHARS;
    let mut kept = Vec::with_capacity(lines.len());
    for mut line in lines.into_iter().rev() {
        if remaining == 0 {
            truncated = true;
            break;
        }
        let length = line.text.chars().count();
        if length > remaining {
            line.text = line.text.chars().skip(length - remaining).collect();
            truncated = true;
        }
        remaining -= line.text.chars().count();
        kept.push(line);
    }
    kept.reverse();
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Action, Policy, RULES_VERSION, Rule, RuleScope, Settings};
    use std::cell::Cell;

    /// Counts every read that touches content, so a test can prove a refusal happened before it.
    struct Fake {
        app_id: &'static str,
        field: FieldDescriptor,
        header: Option<&'static str>,
        lines: Vec<Line>,
        header_reads: Cell<u32>,
        transcript_reads: Cell<u32>,
    }

    impl Fake {
        fn chat(app_id: &'static str) -> Self {
            Fake {
                app_id,
                field: FieldDescriptor {
                    role: "text".into(),
                    label: "Message".into(),
                    purpose: None,
                    declared_composer: true,
                },
                header: Some("Sam"),
                lines: vec![
                    line("Me", true, "On my way"),
                    line("Sam", false, "Great, what time do you land?"),
                ],
                header_reads: Cell::new(0),
                transcript_reads: Cell::new(0),
            }
        }
    }

    fn line(speaker: &str, from_owner: bool, text: &str) -> Line {
        Line {
            speaker: speaker.into(),
            from_owner,
            text: text.into(),
        }
    }

    impl ChatSource for Fake {
        fn window(&self) -> WindowInfo {
            WindowInfo {
                app_id: self.app_id.into(),
                title: "Chat".into(),
                pid: 1,
            }
        }
        fn field(&self) -> FieldDescriptor {
            self.field.clone()
        }
        fn header(&self) -> Option<String> {
            self.header_reads.set(self.header_reads.get() + 1);
            self.header.map(str::to_string)
        }
        fn transcript(&self, max_lines: usize) -> Transcript {
            self.transcript_reads.set(self.transcript_reads.get() + 1);
            let skip = self.lines.len().saturating_sub(max_lines);
            Transcript {
                source: TranscriptSource::Structured,
                lines: self.lines[skip..].to_vec(),
            }
        }
    }

    fn allow_all() -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy: Policy::EverywhereExceptBlocked,
            hotkey: None,
            rules: vec![],
        })
    }

    fn with_rules(rules: Vec<Rule>) -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy: Policy::EverywhereExceptBlocked,
            hotkey: None,
            rules,
        })
    }

    fn marked(_: &str) -> Option<ChatMarker> {
        Some(ChatMarker::default())
    }

    fn rule(scope: RuleScope, value: &str, action: Action) -> Rule {
        Rule {
            scope,
            value: value.into(),
            action,
            context_cap: None,
            local_only: false,
            contains: false,
        }
    }

    #[test]
    fn a_marked_chat_builds_a_bounded_packet_with_its_tiers() {
        let fake = Fake::chat("chat.app");
        let packet = build_packet(&fake, &allow_all(), &marked).unwrap();
        assert_eq!(packet.contact.as_deref(), Some("Sam"));
        assert_eq!(packet.lines.len(), 2);
        assert!(!packet.truncated);
        assert_eq!(
            packet.tiers.iter().map(|t| t.tier).collect::<Vec<_>>(),
            ["C0", "C2"]
        );
    }

    #[test]
    fn an_unmarked_app_gets_nothing_read() {
        let fake = Fake::chat("plain.app");
        let result = build_packet(&fake, &allow_all(), &|_| None);
        assert_eq!(result.unwrap_err(), Refusal::NotChatApp);
        assert_eq!(
            (fake.header_reads.get(), fake.transcript_reads.get()),
            (0, 0)
        );
    }

    #[test]
    fn password_card_otp_and_unknown_fields_build_no_packet_and_read_nothing() {
        for (role, label, declared) in [
            ("password text", "", true),
            ("text", "Password", true),
            ("text", "Card number", true),
            ("text", "Verification code", true),
            ("text", "", false),
        ] {
            let mut fake = Fake::chat("chat.app");
            fake.field = FieldDescriptor {
                role: role.into(),
                label: label.into(),
                purpose: None,
                declared_composer: declared,
            };
            let result = build_packet(&fake, &allow_all(), &marked);
            assert!(
                matches!(result, Err(Refusal::Blocked(BlockReason::BuiltInField))),
                "{role}/{label}"
            );
            assert_eq!(
                (fake.header_reads.get(), fake.transcript_reads.get()),
                (0, 0),
                "{role}/{label}"
            );
        }
    }

    #[test]
    fn a_blocked_app_causes_no_content_read() {
        let fake = Fake::chat("chat.app");
        let rules = with_rules(vec![rule(RuleScope::App, "chat.app", Action::Block)]);
        assert_eq!(
            build_packet(&fake, &rules, &marked).unwrap_err(),
            Refusal::Blocked(BlockReason::Rule)
        );
        assert_eq!(
            (fake.header_reads.get(), fake.transcript_reads.get()),
            (0, 0)
        );
    }

    #[test]
    fn a_blocked_contact_reads_only_the_header_and_discards_it() {
        let fake = Fake::chat("chat.app");
        let rules = with_rules(vec![rule(RuleScope::Contact, "sam", Action::Block)]);
        assert_eq!(
            build_packet(&fake, &rules, &marked).unwrap_err(),
            Refusal::Blocked(BlockReason::Rule)
        );
        assert_eq!(
            (fake.header_reads.get(), fake.transcript_reads.get()),
            (1, 0)
        );
    }

    #[test]
    fn caps_limit_what_is_read_and_title_only_leaves_nothing() {
        let mut fake = Fake::chat("chat.app");
        fake.lines = (0..10)
            .map(|i| line("Sam", false, &format!("message {i}")))
            .collect();
        let mut capped = rule(RuleScope::App, "chat.app", Action::Allow);
        capped.context_cap = Some(ContextCap::Messages(3));
        let packet = build_packet(&fake, &with_rules(vec![capped]), &marked).unwrap();
        assert_eq!(packet.lines.len(), 3);
        assert_eq!(packet.lines[0].text, "message 7");

        let mut title = rule(RuleScope::App, "chat.app", Action::Allow);
        title.context_cap = Some(ContextCap::TitleOnly);
        let fake = Fake::chat("chat.app");
        assert_eq!(
            build_packet(&fake, &with_rules(vec![title]), &marked).unwrap_err(),
            Refusal::CapLeavesNoContext
        );
        assert_eq!(fake.transcript_reads.get(), 0);
    }

    #[test]
    fn nothing_to_reply_to_is_refused() {
        let mut fake = Fake::chat("chat.app");
        fake.lines = vec![line("Me", true, "Already replied")];
        assert_eq!(
            build_packet(&fake, &allow_all(), &marked).unwrap_err(),
            Refusal::NoIncomingMessage
        );
        fake.lines.clear();
        assert_eq!(
            build_packet(&fake, &allow_all(), &marked).unwrap_err(),
            Refusal::EmptyTranscript
        );
    }

    #[test]
    fn long_transcripts_are_cut_from_the_old_end_and_flagged() {
        let mut fake = Fake::chat("chat.app");
        fake.lines = (0..12)
            .map(|_| line("Sam", false, &"x".repeat(1_500)))
            .collect();
        let packet = build_packet(&fake, &allow_all(), &marked).unwrap();
        assert!(packet.truncated);
        assert!(
            packet
                .lines
                .iter()
                .map(|l| l.text.chars().count())
                .sum::<usize>()
                <= MAX_CHARS
        );
    }

    #[test]
    fn classification_prefers_sensitive_kinds_over_a_declared_composer() {
        let field = |role: &str, label: &str, declared| FieldDescriptor {
            role: role.into(),
            label: label.into(),
            purpose: None,
            declared_composer: declared,
        };
        assert_eq!(classify(&field("text", "Message", true)), FieldKind::Chat);
        assert_eq!(classify(&field("text", "Search", false)), FieldKind::Search);
        assert_eq!(
            classify(&field("text", "Address bar", false)),
            FieldKind::Address
        );
        assert_eq!(
            classify(&field("password text", "", true)),
            FieldKind::Password
        );
        assert_eq!(
            classify(&field("text", "Enter PIN", true)),
            FieldKind::Password
        );
        assert_eq!(classify(&field("text", "", false)), FieldKind::Unknown);
    }
}
