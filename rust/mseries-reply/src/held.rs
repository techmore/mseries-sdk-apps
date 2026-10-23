//! The one draft the input service holds, and what each key does while it is held. Pure state, so the Tab/Esc rules are
//! unit-tested without IBus.

use std::time::{Duration, Instant};

/// A draft is held for at most this long.
pub const HOLD: Duration = Duration::from_secs(60);
pub const MAX_CHARS: usize = 1_500;

pub const KEY_TAB: u32 = 0xff09;
pub const KEY_ESCAPE: u32 = 0xff1b;

/// Shift, Control, Caps Lock, Alt, Meta, Super and Hyper on their own. Pressing one is not a reason to drop a draft
/// (the shortcut's own modifiers are still going up when the draft starts).
pub fn is_modifier(keyval: u32) -> bool {
    (0xffe1..=0xffee).contains(&keyval)
}

/// A draft must be non-empty, bounded and free of control characters other than newline and tab.
pub fn valid_offer(text: &str) -> bool {
    let length = text.chars().count();
    length > 0
        && length <= MAX_CHARS
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\t'))
}

#[derive(Debug, Default)]
pub struct Held {
    text: Option<String>,
    deadline: Option<Instant>,
}

impl Held {
    pub fn hold(&mut self, text: &str, now: Instant) -> Result<(), &'static str> {
        if !valid_offer(text) {
            return Err("Invalid draft");
        }
        self.text = Some(text.to_string());
        self.deadline = Some(now + HOLD);
        Ok(())
    }

    pub fn is_holding(&self) -> bool {
        self.text.is_some()
    }

    pub fn active(&self, now: Instant) -> bool {
        self.text.is_some() && self.deadline.is_some_and(|deadline| now <= deadline)
    }

    /// The draft, once. An expired draft is dropped and gives nothing.
    pub fn take(&mut self, now: Instant) -> Option<String> {
        let text = if self.active(now) {
            self.text.take()
        } else {
            None
        };
        self.text = None;
        text
    }

    pub fn drop_draft(&mut self) {
        self.text = None;
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum KeyAction {
    /// Tab: commit the draft once.
    Insert,
    /// Esc: discard and consume the key.
    DiscardConsume,
    /// Any other key: discard, and let the key through to the app.
    DiscardPass,
    /// The release of a key already consumed.
    Swallow,
    /// No draft is held: the key is none of our business.
    Ignore,
}

pub fn classify_key(held: &Held, keyval: u32, released: bool, now: Instant) -> KeyAction {
    if !held.active(now) || is_modifier(keyval) {
        return KeyAction::Ignore;
    }
    if released {
        return if matches!(keyval, KEY_TAB | KEY_ESCAPE) {
            KeyAction::Swallow
        } else {
            KeyAction::Ignore
        };
    }
    match keyval {
        KEY_TAB => KeyAction::Insert,
        KEY_ESCAPE => KeyAction::DiscardConsume,
        _ => KeyAction::DiscardPass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 0x61;

    #[test]
    fn tab_inserts_escape_discards_and_other_keys_pass_through() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        assert_eq!(classify_key(&held, KEY_TAB, false, now), KeyAction::Insert);
        assert_eq!(
            classify_key(&held, KEY_ESCAPE, false, now),
            KeyAction::DiscardConsume
        );
        assert_eq!(classify_key(&held, A, false, now), KeyAction::DiscardPass);
        assert_eq!(classify_key(&held, A, true, now), KeyAction::Ignore);
        assert_eq!(classify_key(&held, KEY_TAB, true, now), KeyAction::Swallow);
    }

    #[test]
    fn bare_modifier_keys_never_drop_a_draft() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        for key in [0xffe1, 0xffe3, 0xffe9, 0xffeb] {
            assert_eq!(classify_key(&held, key, false, now), KeyAction::Ignore);
            assert_eq!(classify_key(&held, key, true, now), KeyAction::Ignore);
        }
        assert!(is_modifier(0xffe1) && is_modifier(0xffee));
        assert!(!is_modifier(KEY_TAB) && !is_modifier(0x61));
    }

    #[test]
    fn with_no_draft_every_key_is_untouched() {
        let now = Instant::now();
        let held = Held::default();
        for key in [KEY_TAB, KEY_ESCAPE, A] {
            assert_eq!(classify_key(&held, key, false, now), KeyAction::Ignore);
        }
    }

    #[test]
    fn a_draft_is_taken_once_and_expires_after_sixty_seconds() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        assert_eq!(held.take(now).as_deref(), Some("hello"));
        assert_eq!(held.take(now), None);

        held.hold("again", now).unwrap();
        let late = now + HOLD + Duration::from_secs(1);
        assert!(!held.active(late));
        assert_eq!(classify_key(&held, KEY_TAB, false, late), KeyAction::Ignore);
        assert_eq!(held.take(late), None);
        assert!(!held.is_holding());
    }

    #[test]
    fn offers_are_bounded_and_free_of_control_characters() {
        for bad in ["", "a\u{1b}b", "a\u{7f}", "\u{0}"] {
            assert!(!valid_offer(bad), "{bad:?}");
        }
        assert!(!valid_offer(&"x".repeat(MAX_CHARS + 1)));
        assert!(valid_offer(&"x".repeat(MAX_CHARS)));
        assert!(valid_offer("line one\nline two\ttab"));
        assert!(Held::default().hold("a\u{1b}b", Instant::now()).is_err());
    }
}
