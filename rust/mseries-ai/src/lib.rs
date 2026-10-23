mod client;
mod store;

pub use client::{ChatMessage, ChatRole, draft_reply};
pub use store::{AiMemory, AiPreferences, AiStore, LocalModelSettings};
