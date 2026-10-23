//! Smart Reply core (first slice of `mseries/SMART-REPLY-DESIGN.md`).
//!
//! The order of the modules is the order of the pipeline, and each step may only run if the one before allowed it:
//! [`marker`] says whether an app is a chat app at all, [`rules`] decides whether it may be read, [`context`] reads the
//! minimum needed and builds a bounded packet, [`agent`] turns the packet into a plain-text draft with no tools, and
//! [`audit`] records what happened without recording any conversation text.
//!
//! Not in this crate yet: the hotkey, the IBus lease/popup and the Settings panel. Nothing here reads a window by itself;
//! a [`context::ChatSource`] has to be supplied.

pub mod agent;
pub mod audit;
pub mod context;
pub mod fixture;
pub mod marker;
pub mod rules;
