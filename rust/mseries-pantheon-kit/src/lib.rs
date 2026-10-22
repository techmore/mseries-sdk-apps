//! Shared UI foundations for first-party Rust applications on elementary OS.
//!
//! GTK already converts logical coordinates to device pixels. The kit keeps
//! application spacing and layout breakpoints tied to the active text metrics,
//! while leaving colors, controls, and platform behavior to GTK and Granite.

pub use granite;
pub use gtk;

mod layout;
mod polling;
mod system;

pub use layout::{
    LayoutClass, install_css, set_accessible_label, set_box_spacing_text_units,
    set_margins_text_units, text_unit, watch_responsive_width,
};
pub use polling::Polling;
pub use system::{CpuUsage, MemoryUsage, SystemSample, SystemSampler, poll_system_metrics};
