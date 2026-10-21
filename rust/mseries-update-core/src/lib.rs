//! The contract between a UI and `mseries-update`, and nothing else.
//!
//! * [`command`]: the closed set of operations a UI may request. The root helper accepts exactly
//!   these, so a UI bug or a hostile caller cannot reach `init`, `config` or `apply --now`.
//! * [`model`]: `mseries-update status --json` (schema 1) and the display state derived from it.
//!   No update logic lives here: the updater decides what is installed, staged and rolled back.

pub mod command;
pub mod model;
