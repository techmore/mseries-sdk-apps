//! M-series SDK core: the closed UI vocabulary, the app description checker and the derived artefacts
//! (JSON Schema, design tokens). No GUI dependencies.
//!
//! An app is one `*.ui.json` document: identity, capabilities, record types, typed state, typed messages and one or
//! more surfaces (window, dialog, popover, panel, fullscreen) built only from the components in [`vocab::COMPONENTS`].
//! [`check`] reports every problem as one line with a JSON path, a stable code and a suggested fix, so a small model can
//! repair its own output.

pub mod agent;
mod check;
pub mod gate;
pub mod logic;
pub mod schema;
pub mod templates;
pub mod text;
pub mod tokens;
pub mod vocab;

pub use check::{Diagnostic, Severity, check};

/// The format version a document must declare as `"sdk"`.
pub const FORMAT_VERSION: u64 = 1;

/// The vocabulary as JSON, for prompts.
pub fn vocabulary() -> serde_json::Value {
    use vocab::*;
    let props = |ps: &[Prop]| -> serde_json::Value {
        ps.iter()
            .map(|p| (p.name.to_string(), serde_json::json!({ "kind": kind_name(p.kind), "required": p.required })))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    let components: serde_json::Map<_, _> = COMPONENTS
        .iter()
        .map(|c| (c.name.to_string(), serde_json::json!({ "about": c.about, "props": props(c.props) })))
        .collect();
    let surfaces: serde_json::Map<_, _> = SURFACES
        .iter()
        .map(|s| (s.name.to_string(), serde_json::json!({ "role": s.role, "props": props(s.props) })))
        .collect();
    let enums: serde_json::Map<_, _> = ENUMS.iter().map(|(n, v)| ((*n).to_string(), serde_json::json!(v))).collect();
    serde_json::json!({
        "sdk": FORMAT_VERSION,
        "components": components,
        "surfaces": surfaces,
        "enums": enums,
        "services": SERVICES.iter().map(|s| (s.name.to_string(), serde_json::json!({
            "about": s.about, "capability": s.capability, "then_payload": s.then.name(),
            "args": s.args.iter().map(|(n, w)| ((*n).to_string(), serde_json::json!(format!("{w:?}").to_lowercase()))).collect::<serde_json::Map<_, _>>(),
        }))).collect::<serde_json::Map<_, _>>(),
        "feeds": FEEDS.iter().map(|f| (f.name.to_string(), serde_json::json!({
            "about": f.about, "capability": f.capability,
            "fields": f.fields.iter().map(|(n, t)| ((*n).to_string(), serde_json::json!(t))).collect::<serde_json::Map<_, _>>(),
        }))).collect::<serde_json::Map<_, _>>(),
        "actions": ACTIONS,
        "capabilities": CAPABILITIES,
        "scalar_types": SCALARS,
        "message_payloads": PAYLOADS,
    })
}

pub(crate) fn kind_name(k: vocab::Kind) -> String {
    use vocab::Kind::*;
    match k {
        Str => "string".into(),
        Template => "string with {field} references".into(),
        Num => "number".into(),
        Bool => "boolean".into(),
        Enum(n) => format!("one of enum {n}"),
        EnumOrTemplate(n) => format!("one of enum {n}, or a {{field}} reference"),
        Bind(w) => format!("state key of type {}", format!("{w:?}").to_lowercase()),
        Msg(p) => format!("message with payload {}", p.name()),
        Field => "field of the list's record type".into(),
        Node => "component".into(),
        Nodes => "array of components".into(),
        Strs => "array of strings".into(),
    }
}
