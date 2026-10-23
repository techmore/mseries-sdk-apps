//! Imports the owner's Signal Desktop history into the Signal Pantheon client.
//! See the crate README for the data flow and the safety rules.

pub mod chromium;
pub mod history;
pub mod import;
pub mod keyring;
pub mod secret;
pub mod signal_db;
