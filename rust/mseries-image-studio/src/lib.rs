//! Native Pantheon image creation workspace.

pub mod assistant;
pub mod host;
pub mod recipe;
pub mod ui;

pub use recipe::{Recipe, RecipeStore};

pub const APP_ID: &str = "org.elementary.mseries.ImageStudio";
pub const DESKTOP_RESERVE_BYTES: u64 = 6 * 1024 * 1024 * 1024;

pub fn can_run_generation(qualified: bool, available_bytes: u64, measured_peak_bytes: u64) -> bool {
    qualified
        && measured_peak_bytes > 0
        && available_bytes >= DESKTOP_RESERVE_BYTES
        && available_bytes - DESKTOP_RESERVE_BYTES >= measured_peak_bytes
}

pub fn generation_block_reason(
    qualified: bool,
    available_bytes: u64,
    measured_peak_bytes: u64,
) -> &'static str {
    if !qualified {
        "Connect and qualify an image engine/model on this device."
    } else if available_bytes < DESKTOP_RESERVE_BYTES {
        "Waiting for the 6 GiB desktop memory reserve."
    } else if measured_peak_bytes == 0 {
        "A measured memory budget is required before generation."
    } else if available_bytes - DESKTOP_RESERVE_BYTES < measured_peak_bytes {
        "This run would use the protected desktop memory reserve."
    } else {
        "Ready for the calibrated model and image size."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_gate_requires_qualification_budget_and_reserve() {
        assert!(!can_run_generation(
            false,
            DESKTOP_RESERVE_BYTES + 8 * 1024 * 1024 * 1024,
            2 * 1024 * 1024 * 1024
        ));
        assert!(!can_run_generation(true, DESKTOP_RESERVE_BYTES - 1, 1));
        assert!(!can_run_generation(true, DESKTOP_RESERVE_BYTES + 1, 0));
        assert!(!can_run_generation(true, DESKTOP_RESERVE_BYTES + 1, 2));
        assert!(can_run_generation(
            true,
            DESKTOP_RESERVE_BYTES + 3 * 1024 * 1024 * 1024,
            2 * 1024 * 1024 * 1024
        ));
    }
}

pub mod models;

pub mod study;

pub mod model_notes;

pub mod logs;

pub mod style;

/// Embed diagnostics with the worker so development and installed builds agree.
pub fn local_generation_worker() -> String {
    format!(
        "{}\n{}",
        include_str!("run_logging.py"),
        include_str!("local_generation.py")
    )
}
