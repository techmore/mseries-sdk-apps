//! Interpreter runtime for `msdk` app descriptions.
//!
//! - [`logic`] (in `msdk`, no GUI): the store, templates, and the rules engine that runs `update` (or any [`logic::Logic`] written in Rust).
//! - [`render`]: builds GTK4 widgets from a surface, wires bindings and messages, and keeps the widgets in step with the store.
//! - [`host`]: ties them together, runs services, persists state and reloads the description when the file changes.

pub mod host;
pub use msdk::logic;
pub mod render;
pub mod services;
