//! Software Updates: shows `mseries-update status --json` and asks the updater to act.
//! There is no update logic here; see `mseries/UPDATER-DESIGN.md`.

pub mod backend;
pub mod strings;
pub mod ui;
