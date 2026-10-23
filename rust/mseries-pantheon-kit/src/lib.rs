//! Shared UI foundations for first-party Rust applications on elementary OS.
//!
//! GTK already converts logical coordinates to device pixels. The kit keeps
//! application spacing and layout breakpoints tied to the active text metrics,
//! while leaving colors, controls, and platform behavior to GTK and Granite.

pub use granite;
pub use gtk;

mod blueferry;
mod dock_badge;
mod images;
mod layout;
mod messages;
mod phone_pins;
mod polling;
mod system;
pub mod updates;

pub use blueferry::{
    BLUEFERRY_MESSAGES_API_VERSION, BlueFerryMessagesClient, BlueFerrySendError, BlueFerryStatus,
};
pub use dock_badge::DockBadge;
pub use images::decode_contact_avatar;
pub use layout::{
    LayoutClass, has_focus_within, install_css, set_accessible_label, set_box_spacing_text_units,
    set_margins_text_units, text_unit, watch_responsive_width,
};
pub use messages::{
    ContactMatch, Message, MessageLayout, MessagePosition, Reaction, RecentMessage, Thread,
    format_message_timestamp, layout_messages, merge_recent_inbox, recipients_share_inbox_identity,
};
pub use phone_pins::{
    APPLE_PINS_SCHEMA, ApplePin, ImportedPinState, PhonePinStore, PinSnapshot,
    is_valid_phone_profile_id, suggested_pin_matches,
};
pub use polling::Polling;
pub use system::{
    CpuUsage, MemoryUsage, ResumeWatcher, SystemSample, SystemSampler, poll_system_metrics,
};
