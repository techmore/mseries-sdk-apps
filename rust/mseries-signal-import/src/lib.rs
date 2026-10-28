//! Imports the owner's Signal Desktop history into the Signal Pantheon client.
//! See the crate README for the data flow and the safety rules.

pub mod chromium;
pub mod history;
pub mod import;
pub mod keyring;
pub mod progress;
pub mod secret {
    pub use mseries_signal_history::Secret;
}
pub mod signal_db;
