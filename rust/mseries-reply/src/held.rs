//! The one draft the input service holds, and what each key does while it is held. Pure state, so the Tab/Esc rules are
//! unit-tested without IBus.

use mseries_ai::Instruction;
use std::time::{Duration, Instant};

/// A draft is held for at most this long.
pub const HOLD: Duration = Duration::from_secs(60);
pub const MAX_CHARS: usize = 1_500;

pub const KEY_TAB: u32 = 0xff09;
pub const KEY_ESCAPE: u32 = 0xff1b;
pub const KEY_RETURN: u32 = 0xff0d;
pub const KEY_KP_ENTER: u32 = 0xff8d;

pub const KEY_BACKSPACE: u32 = 0xff08;
pub const KEY_KP_ADD: u32 = 0xffab;

/// IBus key-event modifier bits.
pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_CONTROL: u32 = 1 << 2;
pub const MOD_ALT: u32 = 1 << 3;
pub const MOD_SUPER: u32 = 1 << 26 | 1 << 6;

/// The quick action a chord asks for. Only Alt chords count, so they never collide with typing: Alt+R reword, Alt+P more
/// polite, Alt+C more concise, Alt+= (or Alt++) more context. Ctrl or Super in the chord make it somebody else's shortcut.
pub fn chord(keyval: u32, modifiers: u32) -> Option<Instruction> {
    if modifiers & MOD_ALT == 0 || modifiers & (MOD_CONTROL | MOD_SUPER) != 0 {
        return None;
    }
    match keyval {
        0x72 | 0x52 => Some(Instruction::Reword),
        0x70 | 0x50 => Some(Instruction::MorePolite),
        0x63 | 0x43 => Some(Instruction::MoreConcise),
        0x3d | 0x2b | KEY_KP_ADD => Some(Instruction::AddContext),
        _ => None,
    }
}

/// The line under or after the draft that names the keys, so they are never a secret.
pub fn hint(actions: bool) -> &'static str {
    if actions {
        "Tab insert · Alt+R reword · Alt+P polite · Alt+C concise · Alt+= more context · Esc discard"
    } else {
        "Tab inserts the draft, Esc discards it"
    }
}

/// What the field shows for a held draft: the draft and the keys (inline), or just the keys (beside the field).
pub fn draft_view(draft: &str, inline: bool, actions: bool) -> String {
    if inline {
        format!("{draft}   — {}", hint(actions))
    } else {
        hint(actions).to_string()
    }
}

/// What the field shows while the owner types extra context.
pub fn entry_view(note: &str) -> String {
    format!("Extra context (Enter redrafts · Esc back): {note}")
}

/// Title of the notification that carries the draft where it cannot be shown in the field.
pub const NOTICE_TITLE: &str =
    "Smart Reply draft — Tab inserts · Alt+R/P/C/= revises · Esc discards";

/// Keys that accept the draft. Enter only inserts it (the key is consumed), so it can never send the message.
pub fn accepts(keyval: u32) -> bool {
    matches!(keyval, KEY_TAB | KEY_RETURN | KEY_KP_ENTER)
}

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

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
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

    /// Gives the owner a fresh [`HOLD`] (typing extra context takes a while).
    pub fn extend(&mut self, now: Instant) {
        if self.text.is_some() {
            self.deadline = Some(now + HOLD);
        }
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
    /// Alt+R/P/C/=: change the draft. `AddContext` first asks the owner for a few words.
    Revise(Instruction),
    /// The release of a key already consumed.
    Swallow,
    /// No draft is held: the key is none of our business.
    Ignore,
}

pub fn classify_key(
    held: &Held,
    keyval: u32,
    modifiers: u32,
    released: bool,
    now: Instant,
) -> KeyAction {
    if !held.active(now) || is_modifier(keyval) {
        return KeyAction::Ignore;
    }
    let revision = chord(keyval, modifiers);
    if released {
        return if accepts(keyval) || keyval == KEY_ESCAPE || revision.is_some() {
            KeyAction::Swallow
        } else {
            KeyAction::Ignore
        };
    }
    if let Some(instruction) = revision {
        KeyAction::Revise(instruction)
    } else if accepts(keyval) {
        KeyAction::Insert
    } else if keyval == KEY_ESCAPE {
        KeyAction::DiscardConsume
    } else {
        KeyAction::DiscardPass
    }
}

/// The longest note the owner can type for "more context".
pub const MAX_NOTE: usize = mseries_ai::MAX_NOTE_CHARS;

/// The character a plain key types, or `None` for keys that type nothing (arrows, function keys, controls).
pub fn typed_char(keyval: u32) -> Option<char> {
    let code = match keyval {
        0x20..=0x7e | 0xa0..=0xff => keyval,
        0x0100_0000..=0x0110_ffff => keyval - 0x0100_0000,
        _ => return None,
    };
    char::from_u32(code).filter(|c| !c.is_control())
}

/// What a key did to the note being typed.
#[derive(Debug, Eq, PartialEq)]
pub enum EntryKey {
    /// The note changed (a character, or Backspace).
    Edited,
    /// Enter or Tab: use the note (an empty note means "never mind").
    Submit,
    /// Esc: back to the draft.
    Cancel,
    /// Anything else; nothing changed.
    Ignored,
}

/// The few words the owner types after Alt+=. The engine owns this line (shown as composing text), so nothing is typed
/// into the app, nothing needs reading back from it, and nothing is left behind to clean up.
#[derive(Debug, Default)]
pub struct ContextEntry {
    note: String,
}

impl ContextEntry {
    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn key(&mut self, keyval: u32, modifiers: u32) -> EntryKey {
        if keyval == KEY_ESCAPE {
            return EntryKey::Cancel;
        }
        if accepts(keyval) {
            return EntryKey::Submit;
        }
        if keyval == KEY_BACKSPACE {
            return if self.note.pop().is_some() {
                EntryKey::Edited
            } else {
                EntryKey::Ignored
            };
        }
        // Ctrl, Alt and Super chords type nothing (Shift and AltGr are part of typing).
        if modifiers & (MOD_CONTROL | MOD_ALT | MOD_SUPER) != 0 {
            return EntryKey::Ignored;
        }
        match typed_char(keyval) {
            Some(c) if self.note.chars().count() < MAX_NOTE => {
                self.note.push(c);
                EntryKey::Edited
            }
            _ => EntryKey::Ignored,
        }
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
        assert_eq!(
            classify_key(&held, KEY_TAB, 0, false, now),
            KeyAction::Insert
        );
        assert_eq!(
            classify_key(&held, KEY_ESCAPE, 0, false, now),
            KeyAction::DiscardConsume
        );
        assert_eq!(
            classify_key(&held, A, 0, false, now),
            KeyAction::DiscardPass
        );
        assert_eq!(classify_key(&held, A, 0, true, now), KeyAction::Ignore);
        assert_eq!(
            classify_key(&held, KEY_TAB, 0, true, now),
            KeyAction::Swallow
        );
    }

    #[test]
    fn enter_inserts_the_draft_but_is_consumed_so_it_never_sends() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        for key in [KEY_RETURN, KEY_KP_ENTER] {
            assert_eq!(classify_key(&held, key, 0, false, now), KeyAction::Insert);
            assert_eq!(classify_key(&held, key, 0, true, now), KeyAction::Swallow);
        }
    }

    #[test]
    fn bare_modifier_keys_never_drop_a_draft() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        for key in [0xffe1, 0xffe3, 0xffe9, 0xffeb] {
            assert_eq!(classify_key(&held, key, 0, false, now), KeyAction::Ignore);
            assert_eq!(classify_key(&held, key, 0, true, now), KeyAction::Ignore);
        }
        assert!(is_modifier(0xffe1) && is_modifier(0xffee));
        assert!(!is_modifier(KEY_TAB) && !is_modifier(0x61));
    }

    #[test]
    fn with_no_draft_every_key_is_untouched() {
        let now = Instant::now();
        let held = Held::default();
        for key in [KEY_TAB, KEY_ESCAPE, A] {
            assert_eq!(classify_key(&held, key, 0, false, now), KeyAction::Ignore);
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
        assert_eq!(
            classify_key(&held, KEY_TAB, 0, false, late),
            KeyAction::Ignore
        );
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

    #[test]
    fn alt_chords_ask_for_a_revision_and_nothing_else_does() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        let cases = [
            (0x72, Instruction::Reword),
            (0x52, Instruction::Reword),
            (0x70, Instruction::MorePolite),
            (0x63, Instruction::MoreConcise),
            (0x3d, Instruction::AddContext),
            (0x2b, Instruction::AddContext),
            (KEY_KP_ADD, Instruction::AddContext),
        ];
        for (key, instruction) in cases {
            assert_eq!(
                classify_key(&held, key, MOD_ALT, false, now),
                KeyAction::Revise(instruction),
                "{key:#x}"
            );
            // The release of a consumed chord must not reach the app either.
            assert_eq!(
                classify_key(&held, key, MOD_ALT, true, now),
                KeyAction::Swallow
            );
        }
        // The same letters without Alt are typing: they abandon the draft and go through.
        for key in [0x72, 0x70, 0x63, 0x3d] {
            assert_eq!(
                classify_key(&held, key, 0, false, now),
                KeyAction::DiscardPass
            );
            assert_eq!(
                classify_key(&held, key, MOD_SHIFT, false, now),
                KeyAction::DiscardPass
            );
        }
        // Other people's shortcuts stay theirs.
        for modifiers in [MOD_ALT | MOD_CONTROL, MOD_ALT | MOD_SUPER, MOD_CONTROL] {
            assert_eq!(chord(0x72, modifiers), None, "{modifiers:#x}");
        }
        assert_eq!(chord(0x78, MOD_ALT), None);
        // Alt + Tab is not an accept key to us either way; Alt+Esc still discards.
        assert_eq!(
            classify_key(&held, KEY_ESCAPE, MOD_ALT, false, now),
            KeyAction::DiscardConsume
        );
    }

    #[test]
    fn the_hint_names_every_action_key() {
        let hint = hint(true);
        for needle in ["Tab", "Alt+R", "Alt+P", "Alt+C", "Alt+=", "Esc"] {
            assert!(hint.contains(needle), "{needle}");
        }
        assert!(!super::hint(false).contains("Alt"));
    }

    #[test]
    fn typing_a_note_edits_submits_or_cancels_and_never_exceeds_the_limit() {
        let mut entry = ContextEntry::default();
        assert_eq!(entry.key(KEY_BACKSPACE, 0), EntryKey::Ignored);
        for key in "hi!".chars() {
            assert_eq!(entry.key(key as u32, 0), EntryKey::Edited);
        }
        assert_eq!(entry.key(0x58, MOD_SHIFT), EntryKey::Edited);
        assert_eq!(entry.key(0x0100_00e9, 0), EntryKey::Edited); // é as a Unicode keysym
        assert_eq!(entry.note(), "hi!Xé");
        assert_eq!(entry.key(KEY_BACKSPACE, 0), EntryKey::Edited);
        assert_eq!(entry.note(), "hi!X");
        // Chords and non-typing keys change nothing.
        assert_eq!(entry.key(0x76, MOD_CONTROL), EntryKey::Ignored);
        assert_eq!(entry.key(0x72, MOD_ALT), EntryKey::Ignored);
        assert_eq!(entry.key(0xff51, 0), EntryKey::Ignored); // Left
        assert_eq!(entry.key(0xffbe, 0), EntryKey::Ignored); // F1
        assert_eq!(entry.note(), "hi!X");
        assert_eq!(entry.key(KEY_RETURN, 0), EntryKey::Submit);
        assert_eq!(entry.key(KEY_TAB, 0), EntryKey::Submit);
        assert_eq!(entry.key(KEY_ESCAPE, 0), EntryKey::Cancel);

        let mut long = ContextEntry::default();
        for _ in 0..MAX_NOTE + 20 {
            long.key(0x78, 0);
        }
        assert_eq!(long.note().chars().count(), MAX_NOTE);
        assert_eq!(typed_char(0x0a), None);
        assert_eq!(typed_char(0x7f), None);
    }

    #[test]
    fn typing_a_note_gives_the_draft_a_fresh_hold() {
        let now = Instant::now();
        let mut held = Held::default();
        held.hold("hello", now).unwrap();
        let later = now + HOLD - Duration::from_secs(1);
        held.extend(later);
        assert!(held.active(later + Duration::from_secs(30)));
        let mut empty = Held::default();
        empty.extend(now);
        assert!(!empty.is_holding());
    }
}
