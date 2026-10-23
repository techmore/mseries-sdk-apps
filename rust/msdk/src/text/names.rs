//! How JSON property names are spelled in the text form. Everything is derived from `vocab` plus the small tables here.

use crate::vocab::Prop;

/// The property written as the first, unlabelled argument: `Button("New note")`.
pub fn positional(component: &str) -> Option<&'static str> {
    Some(match component {
        "Text" => "text",
        "Button" | "IconButton" | "Entry" | "Switch" | "Dropdown" | "Progress" | "Gauge" | "Tabs" => "label",
        "List" | "Transcript" => "items",
        "Card" | "EmptyState" => "title",
        "Custom" => "name",
        _ => return None,
    })
}

/// Components whose single action is written as a trailing closure: `Button("New") { AddNote }`.
pub fn closure_prop(component: &str) -> Option<&'static str> {
    matches!(component, "Button" | "IconButton").then_some("on")
}

/// The text spelling of a property: camelCase, with the style/kind families shortened.
pub fn text_name(json: &str) -> String {
    match json {
        "entry_kind" => return "kind".into(),
        "button_style" | "text_style" => return "style".into(),
        "hide_mode" => return "hide".into(),
        _ => {}
    }
    let mut out = String::new();
    let mut up = false;
    for c in json.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Finds the property of `props` whose text spelling is `name`.
pub fn prop_by_text<'a>(props: &'a [Prop], name: &str) -> Option<&'a Prop> {
    props.iter().find(|p| text_name(p.name) == name)
}

/// Names accepted on input that map to a canonical component, with the property they imply.
/// `(alias, component, implied property, implied value)`.
/// `(alias, component, implied property and value)`.
pub type Alias = (&'static str, &'static str, Option<(&'static str, &'static str)>);

pub const ALIASES: &[Alias] = &[
    ("VStack", "Box", Some(("direction", "column"))),
    ("HStack", "Box", Some(("direction", "row"))),
    ("Toggle", "Switch", None),
    ("TextField", "Entry", None),
    ("SearchField", "Entry", Some(("entry_kind", "search"))),
    ("SecureField", "Entry", Some(("entry_kind", "password"))),
    ("TextEditor", "Entry", Some(("entry_kind", "multiline"))),
    ("ListView", "List", None),
    ("SplitView", "Split", None),
    ("NavigationSplitView", "Split", None),
];

pub const COMMON_MODIFIERS: &[(&str, &str)] = &[("visible", "visible_if"), ("id", "id"), ("tooltip", "tooltip")];
