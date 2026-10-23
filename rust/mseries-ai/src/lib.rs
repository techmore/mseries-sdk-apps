mod client;
mod store;

pub use client::{
    ChatMessage, ChatRole, Instruction, MAX_NOTE_CHARS, Revision, draft_reply, draft_reply_revised,
};
pub use store::{AiMemory, AiPreferences, AiStore, LocalModelSettings};
