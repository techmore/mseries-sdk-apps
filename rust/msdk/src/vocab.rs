//! The single source of truth for the SDK: every component, surface kind, enum and property is declared here once.
//! The checker, the JSON Schema and the prompts all derive from these tables, so they cannot drift apart.

/// What a message carries to the app's `update` function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    None,
    Text,
    Bool,
}

impl Payload {
    pub fn name(self) -> &'static str {
        match self {
            Payload::None => "none",
            Payload::Text => "text",
            Payload::Bool => "bool",
        }
    }
}

/// The state type a binding expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Text,
    Bool,
    Number,
    List,
    Any,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Literal text shown to the user (translatable).
    Str,
    /// Literal text that may reference fields as `{name}`.
    Template,
    Num,
    Bool,
    Enum(&'static str),
    /// A fixed value or a `{field}` reference (for per-row values such as an indicator).
    EnumOrTemplate(&'static str),
    /// Names a state key, or a field of the current row inside a `List` row.
    Bind(Want),
    /// Names a declared message whose payload must match.
    Msg(Payload),
    /// Names a field of the item type of the surrounding `List`.
    Field,
    Node,
    Nodes,
    Strs,
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
}

const fn opt(name: &'static str, kind: Kind) -> Prop {
    Prop { name, kind, required: false }
}
const fn req(name: &'static str, kind: Kind) -> Prop {
    Prop { name, kind, required: true }
}

pub struct Component {
    pub name: &'static str,
    pub about: &'static str,
    pub props: &'static [Prop],
}

pub struct SurfaceKind {
    pub name: &'static str,
    /// The compositor role it maps to (pantheon shell protocol, `pantheon-desktop-shell-v1.xml`).
    pub role: &'static str,
    pub props: &'static [Prop],
}

/// Properties every component accepts.
pub const COMMON: &[Prop] = &[
    opt("type", Kind::Str),
    opt("id", Kind::Str),
    opt("tooltip", Kind::Str),
    opt("visible_if", Kind::Bind(Want::Bool)),
];

/// Named value sets. Values mirror the compositor protocol where one exists.
pub const ENUMS: &[(&str, &[&str])] = &[
    ("direction", &["row", "column"]),
    ("spacing", &["small", "medium", "large"]),
    ("text_style", &["title", "heading", "body", "caption", "muted"]),
    ("button_style", &["suggested", "destructive", "flat"]),
    ("entry_kind", &["text", "search", "multiline", "password"]),
    ("indicator", &["none", "open", "focused"]),
    ("anchor", &["top", "bottom", "left", "right"]),
    ("hide_mode", &["never", "maximized_focus_window", "overlapping_focus_window", "overlapping_window", "always"]),
];

pub const CAPABILITIES: &[&str] = &[
    "filesystem:data",
    "filesystem:user-chosen",
    "gsettings",
    "dbus:session",
    "subprocess",
    "network:localhost",
    "network",
    "keyring",
    "notifications",
    "timers",
    "trash",
    "windows",
    "session",
];

pub const SCALARS: &[&str] = &["text", "bool", "number"];
pub const PAYLOADS: &[&str] = &["none", "text", "bool"];

pub const COMPONENTS: &[Component] = &[
    Component {
        name: "Box",
        about: "Lays children out in a row or column.",
        props: &[
            opt("direction", Kind::Enum("direction")),
            opt("spacing", Kind::Enum("spacing")),
            opt("children", Kind::Nodes),
        ],
    },
    Component {
        name: "Text",
        about: "A line or paragraph of text.",
        props: &[req("text", Kind::Template), opt("text_style", Kind::Enum("text_style"))],
    },
    Component {
        name: "Entry",
        about: "Single- or multi-line text input bound to a text state key.",
        props: &[
            req("label", Kind::Str),
            req("bind", Kind::Bind(Want::Text)),
            opt("placeholder", Kind::Str),
            opt("entry_kind", Kind::Enum("entry_kind")),
            opt("on_change", Kind::Msg(Payload::Text)),
            opt("on_submit", Kind::Msg(Payload::None)),
        ],
    },
    Component {
        name: "Button",
        about: "Runs an action. Destructive buttons must carry a confirmation text.",
        props: &[
            req("label", Kind::Str),
            req("on", Kind::Msg(Payload::None)),
            opt("icon", Kind::Str),
            opt("button_style", Kind::Enum("button_style")),
            opt("shortcut", Kind::Str),
            opt("confirm", Kind::Str),
        ],
    },
    Component {
        name: "IconButton",
        about: "An icon-only action with an accessible name, optional running indicator and badge (dock, panel).",
        props: &[
            req("label", Kind::Str),
            req("icon", Kind::Template),
            req("on", Kind::Msg(Payload::None)),
            opt("indicator", Kind::EnumOrTemplate("indicator")),
            opt("badge", Kind::Template),
            opt("button_style", Kind::Enum("button_style")),
        ],
    },
    Component {
        name: "Switch",
        about: "A persistent on/off setting bound to a bool state key.",
        props: &[
            req("label", Kind::Str),
            req("bind", Kind::Bind(Want::Bool)),
            opt("on_change", Kind::Msg(Payload::Bool)),
        ],
    },
    Component {
        name: "Dropdown",
        about: "Choose one of several options; bound to a text state key.",
        props: &[
            req("label", Kind::Str),
            req("bind", Kind::Bind(Want::Text)),
            req("options", Kind::Strs),
            opt("on_change", Kind::Msg(Payload::Text)),
        ],
    },
    Component {
        name: "List",
        about: "A collection of records. `key` names the stable id field; `row` is the template; `empty` is required.",
        props: &[
            req("items", Kind::Bind(Want::List)),
            req("key", Kind::Field),
            req("row", Kind::Node),
            req("empty", Kind::Node),
            opt("on_select", Kind::Msg(Payload::Text)),
            opt("direction", Kind::Enum("direction")),
        ],
    },
    Component {
        name: "Card",
        about: "A grouped surface with an optional title.",
        props: &[opt("title", Kind::Template), opt("children", Kind::Nodes)],
    },
    Component {
        name: "EmptyState",
        about: "Explains why there is nothing to show and offers the next action.",
        props: &[
            req("title", Kind::Str),
            opt("description", Kind::Str),
            opt("icon", Kind::Str),
            opt("action", Kind::Node),
        ],
    },
    Component {
        name: "Progress",
        about: "Progress of an operation, bound to a number state key (0 to 1).",
        props: &[req("label", Kind::Str), req("bind", Kind::Bind(Want::Number))],
    },
    Component {
        name: "Gauge",
        about: "A quantity against a maximum; must also state its level in text, never colour alone.",
        props: &[
            req("label", Kind::Str),
            req("bind", Kind::Bind(Want::Number)),
            req("state_text", Kind::Template),
            opt("max", Kind::Num),
        ],
    },
    Component {
        name: "Split",
        about: "Sidebar and detail layout.",
        props: &[req("sidebar", Kind::Node), req("content", Kind::Node)],
    },
    Component {
        name: "Custom",
        about: "Escape hatch for a component the vocabulary lacks. Always raises a warning; record the gap.",
        props: &[req("name", Kind::Str), req("reason", Kind::Str), opt("bind", Kind::Bind(Want::Any))],
    },
];

pub const SURFACES: &[SurfaceKind] = &[
    SurfaceKind {
        name: "window",
        role: "xdg_toplevel",
        props: &[req("title", Kind::Str), opt("header", Kind::Nodes), req("body", Kind::Node)],
    },
    SurfaceKind {
        name: "dialog",
        role: "extended_behavior: make_centered + make_modal",
        props: &[req("title", Kind::Str), req("body", Kind::Node), req("actions", Kind::Nodes)],
    },
    SurfaceKind { name: "popover", role: "pantheon widget", props: &[req("body", Kind::Node)] },
    SurfaceKind {
        name: "panel",
        role: "pantheon panel: set_anchor, set_size, set_hide_mode",
        props: &[
            req("anchor", Kind::Enum("anchor")),
            opt("size", Kind::Num),
            opt("hide_mode", Kind::Enum("hide_mode")),
            req("body", Kind::Node),
        ],
    },
    SurfaceKind { name: "fullscreen", role: "pantheon greeter / kiosk", props: &[req("body", Kind::Node)] },
];

pub fn enum_values(name: &str) -> &'static [&'static str] {
    ENUMS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or(&[])
}

pub fn component(name: &str) -> Option<&'static Component> {
    COMPONENTS.iter().find(|c| c.name == name)
}

pub fn surface_kind(name: &str) -> Option<&'static SurfaceKind> {
    SURFACES.iter().find(|s| s.name == name)
}

/// A named capability the runtime implements in Rust. Apps reach the system only through these.
pub struct Service {
    pub name: &'static str,
    pub capability: &'static str,
    /// Argument names and the types they take.
    pub args: &'static [(&'static str, Want)],
    /// Payload of the `then` message (what the call produces).
    pub then: Payload,
    pub about: &'static str,
}

pub const SERVICES: &[Service] = &[
    Service {
        name: "process.run",
        capability: "subprocess",
        args: &[("command", Want::Text)],
        then: Payload::Text,
        about: "Run a command; `then` receives its output.",
    },
    Service {
        name: "power.poweroff",
        capability: "session",
        args: &[],
        then: Payload::None,
        about: "Shut the machine down.",
    },
    Service {
        name: "power.reboot",
        capability: "session",
        args: &[],
        then: Payload::None,
        about: "Restart the machine.",
    },
    Service {
        name: "windows.activate",
        capability: "windows",
        args: &[("id", Want::Text)],
        then: Payload::None,
        about: "Focus (and unminimise) a window.",
    },
    Service {
        name: "windows.close",
        capability: "windows",
        args: &[("id", Want::Text)],
        then: Payload::None,
        about: "Ask a window to close.",
    },
    Service {
        name: "windows.launch",
        capability: "windows",
        args: &[("app", Want::Text)],
        then: Payload::None,
        about: "Start an application by desktop id.",
    },
];

/// A live list the runtime keeps up to date. Declared in the document as `"feeds": {"stateKey": "windows.list"}`.
pub struct Feed {
    pub name: &'static str,
    pub capability: &'static str,
    /// Fields every record carries; the app's record type must declare each with the same type.
    pub fields: &'static [(&'static str, &'static str)],
    pub about: &'static str,
}

pub const FEEDS: &[Feed] = &[Feed {
    name: "windows.list",
    capability: "windows",
    fields: &[("id", "text"), ("app_id", "text"), ("title", "text"), ("focused", "bool"), ("minimized", "bool")],
    about: "Open top-level windows, from the compositor.",
}];

/// Action verbs of an `update` rule.
pub const ACTIONS: &[&str] =
    &["set", "toggle", "append", "patch", "remove", "clear", "call", "toast", "open", "close", "quit", "when"];

pub fn service(name: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.name == name)
}

pub fn feed(name: &str) -> Option<&'static Feed> {
    FEEDS.iter().find(|f| f.name == name)
}
