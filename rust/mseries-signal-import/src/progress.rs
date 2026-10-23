//! Plain-language progress text shared by the command line and the window.
//! Counts and stage names only.

use crate::import::{Progress, Stage};
use std::time::Duration;

pub fn elapsed_text(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// One line, e.g. `[writing] chats 3/4 · messages 1203/1903 · already there 0 · 0:12`.
pub fn line(progress: &Progress, elapsed: Duration) -> String {
    match progress.stage {
        Stage::Writing | Stage::Done | Stage::Cancelled | Stage::Matching => format!(
            "[{}] chats {}/{} · messages {}/{} · added {} · already there {} · {}",
            progress.stage.label(),
            progress.conversations_done,
            progress.conversations_found,
            progress.messages_done,
            progress.messages_total,
            progress.messages_added,
            progress.duplicates_skipped,
            elapsed_text(elapsed)
        ),
        stage => format!("[{}] {}", stage.label(), elapsed_text(elapsed)),
    }
}

/// Fraction 0..=1 for a progress bar, or `None` while the total is unknown.
pub fn fraction(progress: &Progress) -> Option<f64> {
    match progress.stage {
        Stage::Done => Some(1.0),
        Stage::Writing | Stage::Cancelled if progress.messages_total > 0 => {
            Some((progress.messages_done as f64 / progress.messages_total as f64).clamp(0.0, 1.0))
        }
        _ => None,
    }
}
