mod client;
mod persona;
mod retrieve;
mod store;
#[cfg(test)]
mod test_support;

pub use client::{
    ChatMessage, ChatRole, Instruction, MAX_NOTE_CHARS, Revision, draft_reply, draft_reply_with,
    strip_thinking,
};
pub use persona::{
    Contact, ContactInput, ContactPatch, Export, Note, NoteInput, NoteKind, NotePatch, clean_line,
    one_line,
};
pub use retrieve::{DEFAULT_BUDGET_CHARS, PersonalContext, RetrievalCounts, RetrievalQuery};
pub use store::{AiMemory, AiPreferences, AiStore, LocalModelSettings};
