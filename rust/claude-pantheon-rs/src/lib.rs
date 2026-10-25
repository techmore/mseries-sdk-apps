//! Claude for elementary OS: a native client of the T3 Code server (Claude Code, Codex, OpenCode, Grok,
//! Cursor…) with the local Claude Code JSONL transcripts as the offline view.
//! The library exists so the render test can drive the real window; the app is `src/main.rs`.

pub mod activity;
pub mod activity_collect;
pub mod activity_view;
pub mod app;
pub mod changes;
pub mod chat;
pub mod claude_cli;
pub mod composer;
pub mod diagnostics;
pub mod environments;
pub mod handoff;
pub mod index;
pub mod look;
pub mod markdown;
pub mod notify;
pub mod palette;
pub mod probe;
pub mod quota;
pub mod quota_view;
pub mod render;
pub mod repos;
pub mod sdk_services;
pub mod settings;
pub mod sidebar;
pub mod skeleton;
pub mod snapshot;
pub mod status;
pub mod t3;
pub mod terminal;
pub mod transcript;
pub mod usage;
pub mod voice;
pub mod webview;
pub mod winstate;
pub mod workflow;

pub const APP_ID: &str = "io.github.techmore.ClaudePantheonRs";
