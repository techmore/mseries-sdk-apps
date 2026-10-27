//! Diffing for message timelines.
//!
//! Rebuilding every bubble whenever one message arrives makes the history
//! flicker and move under the reader. A timeline instead remembers one
//! signature per rendered row; on the next render only the rows after the first
//! difference are removed and rebuilt, so unchanged rows (and any text the
//! reader has selected) are never touched.

use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

/// A stable fingerprint of everything that influences how one row looks.
pub fn signature(row: &impl Debug) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{row:?}").hash(&mut hasher);
    hasher.finish()
}

/// Number of leading rows that are identical in `old` and `new`.
pub fn common_prefix(old: &[u64], new: &[u64]) -> usize {
    old.iter().zip(new).take_while(|(a, b)| a == b).count()
}

/// Rows kept from the previous render, the rows to discard, and the rows to build.
#[derive(Debug, Eq, PartialEq)]
pub struct TimelineDiff {
    pub keep: usize,
    pub remove: usize,
    pub build: usize,
}

pub fn diff(old: &[u64], new: &[u64]) -> TimelineDiff {
    let keep = common_prefix(old, new);
    TimelineDiff {
        keep,
        remove: old.len() - keep,
        build: new.len() - keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_timelines_change_nothing() {
        let rows = [1, 2, 3];
        assert_eq!(
            diff(&rows, &rows),
            TimelineDiff {
                keep: 3,
                remove: 0,
                build: 0
            }
        );
    }

    #[test]
    fn a_new_message_builds_only_the_tail() {
        assert_eq!(
            diff(&[1, 2, 3], &[1, 2, 3, 4]),
            TimelineDiff {
                keep: 3,
                remove: 0,
                build: 1
            }
        );
    }

    #[test]
    fn a_changed_tail_row_is_replaced_without_touching_earlier_rows() {
        // The previous last message gains a timestamp when a newer one follows.
        assert_eq!(
            diff(&[1, 2, 3], &[1, 2, 30, 4]),
            TimelineDiff {
                keep: 2,
                remove: 1,
                build: 2
            }
        );
    }

    #[test]
    fn a_change_at_the_top_rebuilds_everything() {
        assert_eq!(
            diff(&[1, 2, 3], &[9, 2, 3]),
            TimelineDiff {
                keep: 0,
                remove: 3,
                build: 3
            }
        );
    }

    #[test]
    fn shrinking_removes_the_extra_rows() {
        assert_eq!(
            diff(&[1, 2, 3], &[1, 2]),
            TimelineDiff {
                keep: 2,
                remove: 1,
                build: 0
            }
        );
    }

    #[test]
    fn signatures_follow_content() {
        assert_eq!(signature(&("a", 1)), signature(&("a", 1)));
        assert_ne!(signature(&("a", 1)), signature(&("a", 2)));
    }
}
