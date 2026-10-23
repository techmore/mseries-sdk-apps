//! The fallback chat source: the focused window read through accessibility, under the context gate.
//!
//! Content is read in the gate's order: [`ChatSource::field`] walks the window for the focused editable field and the
//! conversation nodes above it, reading roles, states and the field's label only; [`ChatSource::transcript`] reads the
//! text of the newest few of those nodes, and the gate calls it only after the rules allowed the read. The field's own
//! text is never read.

use crate::context::{ChatSource, FieldDescriptor, Line, Transcript, TranscriptSource, WindowInfo};
use crate::tree::{self, Composer, Limits, Scan, Tree, role};
use std::cell::RefCell;

pub struct AtspiSource<'a, T: Tree> {
    tree: &'a T,
    window: WindowInfo,
    scanned: RefCell<Option<Option<Scan<T::Node>>>>,
}

impl<'a, T: Tree> AtspiSource<'a, T> {
    pub fn new(tree: &'a T, window: WindowInfo) -> Self {
        Self {
            tree,
            window,
            scanned: RefCell::new(None),
        }
    }

    /// Walks the active window once. `None` when no accessible window is active.
    fn scan<R>(&self, use_scan: impl FnOnce(Option<&Scan<T::Node>>) -> R) -> R {
        let mut slot = self.scanned.borrow_mut();
        let scanned = slot.get_or_insert_with(|| {
            tree::active_window(self.tree, Limits::READ)
                .map(|window| tree::scan_window(self.tree, &window, Limits::READ))
        });
        use_scan(scanned.as_ref())
    }

    /// Whether any accessible window is active at all (the trigger says so instead of "not a chat field").
    pub fn has_active_window(&self) -> bool {
        self.scan(|scan| scan.is_some())
    }
}

/// A label that says the field is for something other than replying.
fn looks_like_search_or_address(label: &str) -> bool {
    let label = label.to_lowercase();
    ["search", "address", "url", "location bar", "find"]
        .iter()
        .any(|word| label.contains(word))
}

impl<T: Tree> ChatSource for AtspiSource<'_, T> {
    fn window(&self) -> WindowInfo {
        self.window.clone()
    }

    fn field(&self) -> FieldDescriptor {
        self.scan(|scan| match scan.and_then(|scan| scan.composer.as_ref()) {
            // No focused editable field was reached: unknown, so the gate refuses.
            None => FieldDescriptor::default(),
            Some(Composer {
                role: found, label, ..
            }) => FieldDescriptor {
                role: role::name(*found),
                label: label.clone(),
                purpose: None,
                // The app is already a chat app (marker or the owner's allow rule); a text-like field that is not a
                // search box or address bar is its composer.
                declared_composer: role::EDITABLE_FIELD.contains(found)
                    && !looks_like_search_or_address(label),
            },
        })
    }

    fn header(&self) -> Option<String> {
        (!self.window.title.is_empty()).then(|| self.window.title.clone())
    }

    fn transcript(&self, max_lines: usize) -> Transcript {
        let texts = self.scan(|scan| match scan {
            Some(scan) => tree::read_lines(self.tree, &scan.candidates, max_lines),
            None => Vec::new(),
        });
        Transcript {
            source: TranscriptSource::Fallback,
            // The accessibility tree does not say who wrote a line, so none is attributed to the owner.
            lines: texts
                .into_iter()
                .map(|text| Line {
                    speaker: "Contact".into(),
                    from_owner: false,
                    text,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Refusal, build_packet};
    use crate::marker::ChatMarker;
    use crate::rules::{
        Action, BlockReason, Policy, RULES_VERSION, Rule, RuleScope, Rules, Settings,
    };
    use crate::tree::fake::{FakeTree, bits};
    use crate::tree::state;

    fn window(app: &str, title: &str) -> WindowInfo {
        WindowInfo {
            app_id: app.into(),
            title: title.into(),
            pid: 0,
        }
    }

    fn allow(app: &str) -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy: Policy::AskFirst,
            hotkey: None,
            rules: vec![Rule {
                scope: RuleScope::App,
                value: app.into(),
                action: Action::Allow,
                context_cap: None,
                local_only: false,
                contains: false,
            }],
        })
    }

    fn marked(_: &str) -> Option<ChatMarker> {
        Some(ChatMarker::default())
    }

    #[test]
    fn a_chat_window_becomes_a_packet_of_the_newest_text_above_the_composer() {
        let (tree, _) = FakeTree::chat(
            &["old one", "Are we still on for lunch?"],
            role::ENTRY,
            "Type a message",
        );
        let source = AtspiSource::new(&tree, window("chat.app", "Chat | Sam"));
        let packet = build_packet(&source, &allow("chat.app"), &marked).unwrap();
        assert_eq!(packet.lines.len(), 2);
        assert_eq!(packet.lines[1].text, "Are we still on for lunch?");
        assert_eq!(packet.tiers[1].source, "fallback");
        // Only the two messages were read; the text typed in the composer never was.
        assert_eq!(tree.text_reads.get(), 2);
    }

    #[test]
    fn sensitive_and_unreadable_fields_read_no_message_text() {
        for (composer_role, label) in [
            (role::PASSWORD_TEXT, "Message"),
            (role::ENTRY, "Password"),
            (role::ENTRY, "Enter your PIN"),
            (role::ENTRY, "Card number"),
            (role::ENTRY, "One-time code"),
            (role::ENTRY, "API token"),
            (role::FILLER, "Message"),
        ] {
            let (tree, _) = FakeTree::chat(&["secret chat text"], composer_role, label);
            let source = AtspiSource::new(&tree, window("chat.app", "Chat"));
            let refusal = build_packet(&source, &allow("chat.app"), &marked).unwrap_err();
            assert!(
                matches!(refusal, Refusal::Blocked(BlockReason::BuiltInField)),
                "{label}: {refusal:?}"
            );
            assert_eq!(tree.text_reads.get(), 0, "{label}");
        }
    }

    #[test]
    fn a_search_box_is_not_a_composer_and_reads_nothing() {
        let (tree, _) = FakeTree::chat(&["chat text"], role::ENTRY, "Search messages");
        let source = AtspiSource::new(&tree, window("chat.app", "Chat"));
        assert_eq!(
            build_packet(&source, &allow("chat.app"), &marked).unwrap_err(),
            Refusal::NotComposer
        );
        assert_eq!(tree.text_reads.get(), 0);
    }

    #[test]
    fn a_blocked_app_or_contact_or_unmarked_app_reads_no_text() {
        let (tree, _) = FakeTree::chat(&["chat text"], role::ENTRY, "Message");
        // Unmarked: the gate stops before the window is even walked.
        let source = AtspiSource::new(&tree, window("chat.app", "Chat"));
        let unmarked = |_: &str| None;
        assert_eq!(
            build_packet(&source, &allow("chat.app"), &unmarked).unwrap_err(),
            Refusal::NotChatApp
        );
        assert_eq!(tree.name_reads.get(), 0);

        // Not on the allow list.
        let source = AtspiSource::new(&tree, window("other.app", "Chat"));
        assert!(build_packet(&source, &allow("chat.app"), &marked).is_err());
        assert_eq!(tree.text_reads.get(), 0);

        // A blocked contact (substring of the window title, as in the first slice).
        let mut settings = allow("chat.app").settings().clone();
        settings.rules.push(Rule {
            scope: RuleScope::Contact,
            value: "Ex Partner".into(),
            action: Action::Block,
            context_cap: None,
            local_only: false,
            contains: true,
        });
        let source = AtspiSource::new(&tree, window("chat.app", "Chat with ex partner"));
        assert_eq!(
            build_packet(&source, &Rules::new(settings), &marked).unwrap_err(),
            Refusal::Blocked(BlockReason::Rule)
        );
        assert_eq!(tree.text_reads.get(), 0);
    }

    #[test]
    fn no_focused_field_means_no_packet() {
        let (mut tree, composer) = FakeTree::chat(&["chat text"], role::ENTRY, "Message");
        tree.nodes[composer].states = bits(&[state::EDITABLE]);
        let source = AtspiSource::new(&tree, window("chat.app", "Chat"));
        assert_eq!(
            build_packet(&source, &allow("chat.app"), &marked).unwrap_err(),
            Refusal::Blocked(BlockReason::BuiltInField)
        );
        assert_eq!(tree.text_reads.get(), 0);
    }
}
