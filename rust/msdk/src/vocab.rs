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
    // Layout. `align` is on the cross axis of the parent box (a vertical box aligns horizontally, a horizontal one
    // vertically); `grow` takes the free space along the main axis; `width` is a minimum width in logical pixels.
    opt("align", Kind::Enum("align")),
    opt("grow", Kind::Bool),
    opt("width", Kind::Num),
];

/// The common properties written as named arguments in the text form (the others are `.visible()`, `.id()`, `.tooltip()`).
pub const LAYOUT_PROPS: &[&str] = &["align", "grow", "width"];

/// Named value sets. Values mirror the compositor protocol where one exists.
pub const ENUMS: &[(&str, &[&str])] = &[
    ("direction", &["row", "column"]),
    ("spacing", &["small", "medium", "large"]),
    ("text_style", &["title", "heading", "body", "caption", "muted"]),
    ("button_style", &["suggested", "destructive", "flat"]),
    ("entry_kind", &["text", "search", "multiline", "password"]),
    ("indicator", &["none", "open", "focused"]),
    ("align", &["start", "center", "end", "fill"]),
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
        props: &[req("text", Kind::Template), opt("text_style", Kind::Enum("text_style")), opt("lines", Kind::Num)],
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
            opt("enter_sends", Kind::Bool),
            opt("lines", Kind::Num),
            opt("max_lines", Kind::Num),
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
            opt("group_by", Kind::Field),
            opt("selected", Kind::Bind(Want::Text)),
            opt("scroll", Kind::Bool),
        ],
    },
    Component {
        name: "Transcript",
        about: "A conversation: entries with a `kind` (user, assistant, code, tool, permission). Permission entries show Allow and Deny buttons that send the entry id. The record needs id, kind and text.",
        props: &[
            req("items", Kind::Bind(Want::List)),
            req("key", Kind::Field),
            opt("on_allow", Kind::Msg(Payload::Text)),
            opt("on_deny", Kind::Msg(Payload::Text)),
            opt("empty", Kind::Node),
        ],
    },
    Component {
        name: "Tabs",
        about: "A segmented switcher for a few pages. Put it in a window's header to get tabs in the title bar; it shows `options` and sets the bound text state.",
        props: &[
            req("label", Kind::Str),
            req("bind", Kind::Bind(Want::Text)),
            req("options", Kind::Strs),
            opt("on_change", Kind::Msg(Payload::Text)),
        ],
    },
    Component {
        name: "Stack",
        about: "Pages: shows the child whose `id` equals the bound text state (pair it with Tabs). Every child needs an id.",
        props: &[req("bind", Kind::Bind(Want::Text)), opt("children", Kind::Nodes)],
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
        props: &[req("sidebar", Kind::Node), req("content", Kind::Node), opt("autohide", Kind::Bool)],
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
    fields: &[("id", "text"), ("app_id", "text"), ("title", "text"), ("focused", "bool"), ("attention", "bool")],
    about: "Open top-level windows, from the compositor (org.pantheon.gala.DesktopIntegration).",
}];

/// Action verbs of an `update` rule.
pub const ACTIONS: &[&str] =
    &["set", "toggle", "add", "append", "patch", "remove", "clear", "call", "toast", "open", "close", "quit", "when"];

/// Services and feeds that Rust crates (or a manifest file) add at run time, for example the T3 client's `t3.threads`.
static REGISTRY: std::sync::RwLock<(Vec<&'static Service>, Vec<&'static Feed>)> =
    std::sync::RwLock::new((Vec::new(), Vec::new()));

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

pub fn register_service(s: Service) {
    if let Ok(mut r) = REGISTRY.write()
        && !r.0.iter().any(|x| x.name == s.name)
        && !SERVICES.iter().any(|x| x.name == s.name)
    {
        r.0.push(Box::leak(Box::new(s)));
    }
}

pub fn register_feed(f: Feed) {
    if let Ok(mut r) = REGISTRY.write()
        && !r.1.iter().any(|x| x.name == f.name)
        && !FEEDS.iter().any(|x| x.name == f.name)
    {
        r.1.push(Box::leak(Box::new(f)));
    }
}

/// Built-in services followed by registered ones.
pub fn services() -> Vec<&'static Service> {
    let mut v: Vec<&'static Service> = SERVICES.iter().collect();
    if let Ok(r) = REGISTRY.read() {
        v.extend(r.0.iter().copied());
    }
    v
}

pub fn feeds() -> Vec<&'static Feed> {
    let mut v: Vec<&'static Feed> = FEEDS.iter().collect();
    if let Ok(r) = REGISTRY.read() {
        v.extend(r.1.iter().copied());
    }
    v
}

pub fn service(name: &str) -> Option<&'static Service> {
    services().into_iter().find(|s| s.name == name)
}

pub fn feed(name: &str) -> Option<&'static Feed> {
    feeds().into_iter().find(|f| f.name == name)
}

fn want_of(s: &str) -> Result<Want, String> {
    Ok(match s {
        "text" => Want::Text,
        "bool" => Want::Bool,
        "number" => Want::Number,
        "list" => Want::List,
        "any" => Want::Any,
        other => return Err(format!("unknown argument type `{other}`")),
    })
}

/// Registers the services and feeds a manifest declares. Format:
/// `{"services":[{"name","capability","about","then":"none|text|bool","args":{"thread":"text"}}],
///   "feeds":[{"name","capability","about","fields":{"id":"text"}}]}`.
/// A crate that provides services ships this file next to its code so `msdk check` knows them without loading the crate.
pub const SERVICE_MANIFEST_VERSION: u64 = 1;

fn manifest_object<'a>(
    value: &'a serde_json::Value,
    allowed: &[&str],
    context: &str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, String> {
    let object = value.as_object().ok_or_else(|| format!("{context}: expected an object"))?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{context}: unknown property `{key}`"));
    }
    Ok(object)
}

/// Validates the whole manifest and conflicts before publishing any declarations.
/// Omitted manifest_version is the legacy v1 representation. This version does
/// not identify an OS/network backend protocol or the Services extension ABI.
pub fn load_manifest(v: &serde_json::Value) -> Result<usize, String> {
    manifest_object(v, &["manifest_version", "services", "feeds"], "manifest")?;
    if let Some(version) = v.get("manifest_version")
        && version.as_u64() != Some(SERVICE_MANIFEST_VERSION)
    {
        return Err(format!(
            "unsupported service manifest version; expected {SERVICE_MANIFEST_VERSION}; use a compatible SDK or migrate the manifest, do not relabel it"
        ));
    }
    for key in ["services", "feeds"] {
        if v.get(key).is_some_and(|value| !value.is_array()) {
            return Err(format!("manifest: `{key}` must be an array"));
        }
    }
    let mut services = Vec::new();
    let mut feeds = Vec::new();
    let mut service_names = std::collections::HashSet::new();
    let mut feed_names = std::collections::HashSet::new();
    for s in v["services"].as_array().into_iter().flatten() {
        manifest_object(s, &["name", "capability", "about", "then", "args"], "service")?;
        let name =
            s["name"].as_str().filter(|name| !name.trim().is_empty()).ok_or("a service needs a nonempty name")?;
        if !service_names.insert(name) {
            return Err(format!("duplicate service `{name}`"));
        }
        let cap = s["capability"].as_str().ok_or_else(|| format!("{name}: capability missing"))?;
        if !CAPABILITIES.contains(&cap) {
            return Err(format!("{name}: unknown capability `{cap}`"));
        }
        let mut args = Vec::new();
        if let Some(value) = s.get("args") {
            let object = value.as_object().ok_or_else(|| format!("{name}: args must be an object"))?;
            for (key, ty) in object {
                args.push((key.as_str(), want_of(ty.as_str().unwrap_or(""))?));
            }
        }
        let then = match s.get("then") {
            None => Payload::None,
            Some(value) => match value.as_str() {
                Some("none") => Payload::None,
                Some("text") => Payload::Text,
                Some("bool") => Payload::Bool,
                _ => return Err(format!("{name}: then must be none, text or bool")),
            },
        };
        let about = match s.get("about") {
            None => "",
            Some(value) => value.as_str().ok_or_else(|| format!("{name}: about must be text"))?,
        };
        services.push((name, cap, args, then, about));
    }
    for f in v["feeds"].as_array().into_iter().flatten() {
        manifest_object(f, &["name", "capability", "about", "fields"], "feed")?;
        let name = f["name"].as_str().filter(|name| !name.trim().is_empty()).ok_or("a feed needs a nonempty name")?;
        if !feed_names.insert(name) {
            return Err(format!("duplicate feed `{name}`"));
        }
        let cap = f["capability"].as_str().ok_or_else(|| format!("{name}: capability missing"))?;
        if !CAPABILITIES.contains(&cap) {
            return Err(format!("{name}: unknown capability `{cap}`"));
        }
        let mut fields = Vec::new();
        if let Some(value) = f.get("fields") {
            let object = value.as_object().ok_or_else(|| format!("{name}: fields must be an object"))?;
            for (key, value) in object {
                let ty = value.as_str().unwrap_or("");
                if !SCALARS.contains(&ty) {
                    return Err(format!("{name}: field `{key}` has bad type `{ty}`"));
                }
                fields.push((key.as_str(), ty));
            }
        }
        let about = match f.get("about") {
            None => "",
            Some(value) => value.as_str().ok_or_else(|| format!("{name}: about must be text"))?,
        };
        feeds.push((name, cap, fields, about));
    }
    let mut registry = REGISTRY.write().map_err(|_| "service registry is unavailable")?;
    // Conflict validation and commit share a write lock, including concurrent
    // loaders. No leaked strings or partially visible declarations on failure.
    for (name, cap, args, then, about) in &services {
        if let Some(old) = SERVICES.iter().chain(registry.0.iter().copied()).find(|old| old.name == *name)
            && (old.capability != *cap
                || (old.args.len() != args.len() || !args.iter().all(|arg| old.args.contains(arg)))
                || old.then != *then
                || old.about != *about)
        {
            return Err(format!("conflicting service declaration `{name}`"));
        }
    }
    for (name, cap, fields, about) in &feeds {
        if let Some(old) = FEEDS.iter().chain(registry.1.iter().copied()).find(|old| old.name == *name)
            && (old.capability != *cap
                || (old.fields.len() != fields.len() || !fields.iter().all(|field| old.fields.contains(field)))
                || old.about != *about)
        {
            return Err(format!("conflicting feed declaration `{name}`"));
        }
    }
    let count = services.len() + feeds.len();
    for (name, cap, args, then, about) in services {
        if !SERVICES.iter().chain(registry.0.iter().copied()).any(|old| old.name == name) {
            registry.0.push(Box::leak(Box::new(Service {
                name: leak(name),
                capability: leak(cap),
                then,
                about: leak(about),
                args: Box::leak(
                    args.into_iter().map(|(key, ty)| (leak(key), ty)).collect::<Vec<_>>().into_boxed_slice(),
                ),
            })));
        }
    }
    for (name, cap, fields, about) in feeds {
        if !FEEDS.iter().chain(registry.1.iter().copied()).any(|old| old.name == name) {
            registry.1.push(Box::leak(Box::new(Feed {
                name: leak(name),
                capability: leak(cap),
                about: leak(about),
                fields: Box::leak(
                    fields.into_iter().map(|(key, ty)| (leak(key), leak(ty))).collect::<Vec<_>>().into_boxed_slice(),
                ),
            })));
        }
    }
    Ok(count)
}
