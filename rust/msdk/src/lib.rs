//! The closed UI vocabulary of the M-series SDK and the checker for `*.ui.json` files.
//!
//! An app description is one JSON document: identity, capabilities, state, messages and a window
//! built only from the components in [`COMPONENTS`]. [`check`] reports every problem as one line
//! with a JSON path, a stable code and a suggested fix, so a small model can repair its own output.

use serde_json::Value;

/// One component of the vocabulary. `nodes` are properties holding a component (or a list of them).
pub struct Component {
    pub name: &'static str,
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
    pub nodes: &'static [&'static str],
}

/// Properties any component accepts.
const COMMON: &[&str] = &["type", "id", "visible_if", "tooltip"];

/// Allowed values for properties that take one of a fixed set.
const ENUMS: &[(&str, &[&str])] = &[
    ("direction", &["row", "column"]),
    ("spacing", &["small", "medium", "large"]),
    ("text_style", &["title", "heading", "body", "caption", "muted"]),
    ("button_style", &["suggested", "destructive", "flat"]),
    ("entry_kind", &["text", "search", "multiline", "password"]),
];

const CAPABILITIES: &[&str] = &[
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
];

const STATE_TYPES: &[&str] = &["text", "bool", "number", "list"];

pub const COMPONENTS: &[Component] = &[
    Component { name: "Box", required: &[], optional: &["direction", "spacing"], nodes: &["children"] },
    Component { name: "Text", required: &["text"], optional: &["text_style"], nodes: &[] },
    Component {
        name: "Entry",
        required: &["label", "bind"],
        optional: &["placeholder", "entry_kind", "on_change"],
        nodes: &[],
    },
    Component {
        name: "Button",
        required: &["label", "on"],
        optional: &["icon", "button_style", "shortcut"],
        nodes: &[],
    },
    Component { name: "Switch", required: &["label", "bind"], optional: &["on_change"], nodes: &[] },
    Component { name: "Dropdown", required: &["label", "bind", "options"], optional: &["on_change"], nodes: &[] },
    Component {
        name: "List",
        required: &["items", "row", "empty"],
        optional: &["on_select"],
        nodes: &["row", "empty"],
    },
    Component { name: "Card", required: &[], optional: &["title"], nodes: &["children"] },
    Component { name: "EmptyState", required: &["title"], optional: &["description", "icon"], nodes: &["action"] },
    Component { name: "Progress", required: &["label", "bind"], optional: &[], nodes: &[] },
    Component { name: "Gauge", required: &["label", "bind", "state_text"], optional: &["max"], nodes: &[] },
    Component { name: "Split", required: &["sidebar", "content"], optional: &[], nodes: &["sidebar", "content"] },
    Component { name: "Custom", required: &["name", "reason"], optional: &["bind"], nodes: &[] },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    /// JSON pointer to the offending value, `/window/body/children/2`.
    pub path: String,
    /// Stable machine-readable code, `missing-property`.
    pub code: &'static str,
    pub message: String,
    pub fix: Option<String>,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}[{}] {}: {}", self.code, self.path, self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " (fix: {fix})")?;
        }
        Ok(())
    }
}

struct Ctx {
    state: Vec<(String, String)>,
    messages: Vec<String>,
    ids: Vec<String>,
    out: Vec<Diagnostic>,
}

impl Ctx {
    fn err(&mut self, path: &str, code: &'static str, message: String, fix: Option<String>) {
        self.out.push(Diagnostic { severity: Severity::Error, path: path.into(), code, message, fix });
    }
    fn warn(&mut self, path: &str, code: &'static str, message: String, fix: Option<String>) {
        self.out.push(Diagnostic { severity: Severity::Warning, path: path.into(), code, message, fix });
    }
}

/// Checks a parsed document and returns every diagnostic, errors first in document order.
pub fn check(doc: &Value) -> Vec<Diagnostic> {
    let mut cx = Ctx { state: vec![], messages: vec![], ids: vec![], out: vec![] };
    let Some(root) = doc.as_object() else {
        cx.err("", "not-object", "the document must be a JSON object".into(), None);
        return cx.out;
    };
    for key in root.keys() {
        if !["app", "capabilities", "state", "messages", "window"].contains(&key.as_str()) {
            let fix = suggest(key, &["app", "capabilities", "state", "messages", "window"]);
            cx.err(&format!("/{key}"), "unknown-property", format!("unknown top-level key `{key}`"), fix);
        }
    }
    check_app(root.get("app"), &mut cx);
    check_capabilities(root.get("capabilities"), &mut cx);
    check_state(root.get("state"), &mut cx);
    check_messages(root.get("messages"), &mut cx);
    match root.get("window") {
        Some(w) => check_window(w, &mut cx),
        None => cx.err(
            "/window",
            "missing-property",
            "the document has no `window`".into(),
            Some("add a window with title and body".into()),
        ),
    }
    cx.out.sort_by_key(|d| d.severity == Severity::Warning);
    cx.out
}

fn check_app(app: Option<&Value>, cx: &mut Ctx) {
    let Some(app) = app.and_then(Value::as_object) else {
        cx.err(
            "/app",
            "missing-property",
            "`app` object is required".into(),
            Some("add {\"id\",\"name\",\"icon\"}".into()),
        );
        return;
    };
    for k in ["id", "name", "icon"] {
        if !app.get(k).is_some_and(Value::is_string) {
            cx.err(&format!("/app/{k}"), "missing-property", format!("`app.{k}` must be a string"), None);
        }
    }
    if let Some(id) = app.get("id").and_then(Value::as_str) {
        let parts: Vec<&str> = id.split('.').collect();
        if parts.len() < 3
            || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            cx.err(
                "/app/id",
                "bad-app-id",
                format!("`{id}` is not a reverse-DNS id"),
                Some("use io.github.techmore.AppName".into()),
            );
        }
    }
}

fn check_capabilities(caps: Option<&Value>, cx: &mut Ctx) {
    let Some(arr) = caps.and_then(Value::as_array) else {
        if caps.is_some() {
            cx.err("/capabilities", "wrong-type", "`capabilities` must be an array of strings".into(), None);
        }
        return;
    };
    for (i, c) in arr.iter().enumerate() {
        match c.as_str() {
            Some(s) if CAPABILITIES.contains(&s) => {}
            Some(s) => cx.err(
                &format!("/capabilities/{i}"),
                "unknown-capability",
                format!("unknown capability `{s}`"),
                suggest(s, CAPABILITIES),
            ),
            None => cx.err(&format!("/capabilities/{i}"), "wrong-type", "capability must be a string".into(), None),
        }
    }
}

fn check_state(state: Option<&Value>, cx: &mut Ctx) {
    let Some(obj) = state.and_then(Value::as_object) else {
        if state.is_some() {
            cx.err("/state", "wrong-type", "`state` must be an object of name: type".into(), None);
        }
        return;
    };
    for (k, v) in obj {
        match v.as_str() {
            Some(t) if STATE_TYPES.contains(&t) => cx.state.push((k.clone(), t.to_string())),
            _ => cx.err(
                &format!("/state/{k}"),
                "bad-state-type",
                format!("state `{k}` must be one of {}", STATE_TYPES.join(", ")),
                None,
            ),
        }
    }
}

fn check_messages(msgs: Option<&Value>, cx: &mut Ctx) {
    let Some(arr) = msgs.and_then(Value::as_array) else {
        if msgs.is_some() {
            cx.err("/messages", "wrong-type", "`messages` must be an array of names".into(), None);
        }
        return;
    };
    cx.messages = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
    for (i, v) in arr.iter().enumerate() {
        if !v.is_string() {
            cx.err(&format!("/messages/{i}"), "wrong-type", "message name must be a string".into(), None);
        }
    }
}

fn check_window(w: &Value, cx: &mut Ctx) {
    let Some(obj) = w.as_object() else {
        cx.err("/window", "wrong-type", "`window` must be an object".into(), None);
        return;
    };
    if !obj.get("title").is_some_and(Value::is_string) {
        cx.err("/window/title", "missing-property", "`window.title` must be a string".into(), None);
    }
    for key in obj.keys() {
        if !["title", "header", "body"].contains(&key.as_str()) {
            cx.err(
                &format!("/window/{key}"),
                "unknown-property",
                format!("unknown window key `{key}`"),
                suggest(key, &["title", "header", "body"]),
            );
        }
    }
    if let Some(h) = obj.get("header") {
        match h.as_array() {
            Some(items) => items.iter().enumerate().for_each(|(i, n)| node(n, &format!("/window/header/{i}"), 1, cx)),
            None => cx.err("/window/header", "wrong-type", "`header` must be an array of components".into(), None),
        }
    }
    match obj.get("body") {
        Some(b) => node(b, "/window/body", 1, cx),
        None => cx.err("/window/body", "missing-property", "`window.body` is required".into(), None),
    }
}

fn node(v: &Value, path: &str, depth: usize, cx: &mut Ctx) {
    let Some(obj) = v.as_object() else {
        cx.err(path, "wrong-type", "a component must be an object with a `type`".into(), None);
        return;
    };
    if depth > 8 {
        cx.warn(
            path,
            "too-deep",
            "components nest deeper than 8 levels".into(),
            Some("flatten with a Card or split the screen".into()),
        );
    }
    let Some(ty) = obj.get("type").and_then(Value::as_str) else {
        cx.err(
            path,
            "missing-property",
            "component has no `type`".into(),
            Some("add \"type\" with one of the vocabulary names".into()),
        );
        return;
    };
    let Some(spec) = COMPONENTS.iter().find(|c| c.name == ty) else {
        let names: Vec<&str> = COMPONENTS.iter().map(|c| c.name).collect();
        cx.err(
            &format!("{path}/type"),
            "unknown-component",
            format!("no component `{ty}`"),
            suggest(ty, &names).or(Some(format!("use one of {}", names.join(", ")))),
        );
        return;
    };
    for key in obj.keys() {
        let k = key.as_str();
        if !(COMMON.contains(&k) || spec.required.contains(&k) || spec.optional.contains(&k) || spec.nodes.contains(&k))
        {
            let mut all: Vec<&str> = COMMON.to_vec();
            all.extend(spec.required);
            all.extend(spec.optional);
            all.extend(spec.nodes);
            cx.err(
                &format!("{path}/{key}"),
                "unknown-property",
                format!("{ty} has no property `{key}`"),
                suggest(key, &all),
            );
        }
    }
    for req in spec.required {
        if !obj.contains_key(*req) {
            cx.err(path, "missing-property", format!("{ty} needs `{req}`"), Some(format!("add \"{req}\"")));
        }
    }
    if let Some(id) = obj.get("id").and_then(Value::as_str) {
        if cx.ids.iter().any(|i| i == id) {
            cx.err(&format!("{path}/id"), "duplicate-id", format!("id `{id}` is used twice"), None);
        }
        cx.ids.push(id.to_string());
    }
    for key in ["bind", "visible_if", "items"] {
        if let Some(name) = obj.get(key).and_then(Value::as_str) {
            check_bind(path, key, name, ty, cx);
        }
    }
    for key in ["on", "on_change", "on_select"] {
        if let Some(name) = obj.get(key).and_then(Value::as_str)
            && !cx.messages.iter().any(|m| m == name)
        {
            let refs: Vec<&str> = cx.messages.iter().map(String::as_str).collect();
            cx.err(
                &format!("{path}/{key}"),
                "unknown-message",
                format!("message `{name}` is not declared"),
                suggest(name, &refs).or(Some("add it to `messages`".into())),
            );
        }
    }
    check_enums(obj, path, ty, cx);
    if ty == "Custom" {
        cx.warn(
            path,
            "custom-component",
            "Custom escape hatch used; record the missing component in sdk-bench/GAPS.md".into(),
            None,
        );
    }
    for slot in spec.nodes {
        let Some(child) = obj.get(*slot) else {
            continue;
        };
        let p = format!("{path}/{slot}");
        match child {
            Value::Array(items) => {
                items.iter().enumerate().for_each(|(i, n)| node(n, &format!("{p}/{i}"), depth + 1, cx))
            }
            other => node(other, &p, depth + 1, cx),
        }
    }
    if matches!(ty, "Entry" | "Switch" | "Dropdown" | "Button")
        && obj.get("label").and_then(Value::as_str).is_some_and(str::is_empty)
    {
        cx.err(
            &format!("{path}/label"),
            "empty-label",
            format!("{ty} label is empty; screen readers need a name"),
            None,
        );
    }
    if ty == "Gauge" && obj.get("state_text").and_then(Value::as_str).is_some_and(str::is_empty) {
        cx.err(
            &format!("{path}/state_text"),
            "empty-label",
            "Gauge must say its state in text, not colour alone".into(),
            None,
        );
    }
}

fn check_bind(path: &str, key: &str, name: &str, ty: &str, cx: &mut Ctx) {
    let found = cx.state.iter().find(|(k, _)| k == name).map(|(_, t)| t.clone());
    let Some(kind) = found else {
        let names: Vec<&str> = cx.state.iter().map(|(k, _)| k.as_str()).collect();
        let fix = suggest(name, &names).or(Some(format!("declare `{name}` under `state`")));
        cx.err(&format!("{path}/{key}"), "unknown-state", format!("`{name}` is not in `state`"), fix);
        return;
    };
    let want: Option<&str> = match (ty, key) {
        (_, "visible_if") | ("Switch", "bind") => Some("bool"),
        ("List", "items") => Some("list"),
        ("Entry", "bind") => Some("text"),
        ("Progress" | "Gauge", "bind") => Some("number"),
        _ => None,
    };
    if let Some(w) = want
        && w != kind.as_str()
    {
        cx.err(
            &format!("{path}/{key}"),
            "state-type-mismatch",
            format!("{ty}.{key} needs a `{w}` state but `{name}` is `{kind}`"),
            None,
        );
    }
}

fn check_enums(obj: &serde_json::Map<String, Value>, path: &str, ty: &str, cx: &mut Ctx) {
    let pairs: &[(&str, &str)] = match ty {
        "Box" => &[("direction", "direction"), ("spacing", "spacing")],
        "Text" => &[("text_style", "text_style")],
        "Button" => &[("button_style", "button_style")],
        "Entry" => &[("entry_kind", "entry_kind")],
        _ => &[],
    };
    for (prop, set) in pairs {
        let Some(v) = obj.get(*prop) else { continue };
        let allowed = ENUMS.iter().find(|(n, _)| n == set).map(|(_, a)| *a).unwrap_or(&[]);
        match v.as_str() {
            Some(s) if allowed.contains(&s) => {}
            Some(s) => cx.err(
                &format!("{path}/{prop}"),
                "bad-value",
                format!("`{s}` is not valid for {prop}"),
                suggest(s, allowed).or(Some(format!("use one of {}", allowed.join(", ")))),
            ),
            None => cx.err(&format!("{path}/{prop}"), "wrong-type", format!("{prop} must be a string"), None),
        }
    }
}

/// Closest candidate by edit distance, when it is close enough to be a likely typo.
fn suggest(word: &str, candidates: &[&str]) -> Option<String> {
    let (best, dist) = candidates
        .iter()
        .map(|c| (*c, edit_distance(&word.to_lowercase(), &c.to_lowercase())))
        .min_by_key(|(_, d)| *d)?;
    (dist <= 2.max(word.len() / 3)).then(|| format!("did you mean `{best}`?"))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The vocabulary as JSON, for prompts and for building a constrained-generation grammar.
pub fn vocabulary() -> Value {
    let mut out = serde_json::Map::new();
    for c in COMPONENTS {
        out.insert(
            c.name.into(),
            serde_json::json!({ "required": c.required, "optional": c.optional, "components_in": c.nodes }),
        );
    }
    let enums: serde_json::Map<String, Value> =
        ENUMS.iter().map(|(n, v)| ((*n).into(), serde_json::json!(v))).collect();
    serde_json::json!({ "components": out, "enums": enums, "capabilities": CAPABILITIES, "state_types": STATE_TYPES })
}
