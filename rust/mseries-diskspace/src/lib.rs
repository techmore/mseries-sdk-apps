//! Disk Space for elementary OS: scan a folder or filesystem, then find and recover space.
//! The library exists so the render test can drive the real window; the app is `src/main.rs`.

pub mod app;
pub mod layout;
pub mod look;
pub mod model;
pub mod ops;
pub mod paint;
pub mod scan;
pub mod sys;
