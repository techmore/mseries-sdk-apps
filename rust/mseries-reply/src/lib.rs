//! Smart Reply core (first slice of `mseries/SMART-REPLY-DESIGN.md`).
//!
//! The order of the modules is the order of the pipeline, and each step may only run if the one before allowed it:
//! [`marker`] says whether an app is a chat app at all, [`rules`] decides whether it may be read, [`context`] reads the
//! minimum needed and builds a bounded packet, [`agent`] turns the packet into a plain-text draft with no tools, and
//! [`audit`] records what happened without recording any conversation text.
//!
//! The desktop side: [`window`] (focused window from the compositor), [`a11y`] and [`tree`] (bounded AT-SPI reads),
//! [`source`] (the AT-SPI chat source behind the gate), [`trigger`] (the hotkey command) and [`ibus`] with [`held`]
//! (the input service that holds a draft until Tab or Esc). Not here yet: the Settings panel.

pub mod a11y;
pub mod agent;
pub mod audit;
pub mod context;
pub mod fixture;
pub mod held;
pub mod ibus;
pub mod marker;
pub mod probe;
pub mod rules;
pub mod source;
pub mod tree;
pub mod trigger;
pub mod window;
