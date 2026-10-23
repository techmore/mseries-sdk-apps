//! The checker. Walks a parsed document against the tables in [`crate::vocab`].

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use crate::vocab::{self, Kind, Payload, Prop, Want};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    /// JSON pointer to the offending value, `/surfaces/main/body/children/2`.
    pub path: String,
    /// Stable machine-readable code, `missing-property`.
    pub code: &'static str,
    pub message: String,
    pub fix: Option<String>,
}

impl Diagnostic {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "severity": if self.severity == Severity::Error { "error" } else { "warning" },
            "path": self.path, "code": self.code, "message": self.message, "fix": self.fix,
        })
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = if self.severity == Severity::Error { "error" } else { "warning" };
        write!(f, "{level}[{}] {}: {}", self.code, self.path, self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " (fix: {fix})")?;
        }
        Ok(())
    }
}

type Fields = BTreeMap<String, String>;

struct Cx {
    types: BTreeMap<String, Fields>,
    state: BTreeMap<String, String>,
    messages: BTreeMap<String, Payload>,
    ids: HashSet<String>,
    used_state: HashSet<String>,
    used_messages: HashSet<String>,
    suggested: BTreeMap<String, usize>,
    surfaces: Vec<String>,
    caps: Vec<String>,
    out: Vec<Diagnostic>,
}

/// What names are visible at a point in the tree: the state, plus the fields of the current row inside a `List` row.
#[derive(Clone, Default)]
struct Scope {
    row: Option<Fields>,
}

impl Cx {
    fn err(&mut self, path: &str, code: &'static str, message: String, fix: Option<String>) {
        self.out.push(Diagnostic { severity: Severity::Error, path: path.into(), code, message, fix });
    }
    fn warn(&mut self, path: &str, code: &'static str, message: String, fix: Option<String>) {
        self.out.push(Diagnostic { severity: Severity::Warning, path: path.into(), code, message, fix });
    }

    fn lookup(&mut self, name: &str, scope: &Scope) -> Option<String> {
        if let Some(t) = scope.row.as_ref().and_then(|r| r.get(name)) {
            return Some(t.clone());
        }
        let t = self.state.get(name).cloned();
        if t.is_some() {
            self.used_state.insert(name.to_string());
        }
        t
    }

    fn visible_names(&self, scope: &Scope) -> Vec<String> {
        let mut v: Vec<String> = scope.row.iter().flat_map(|r| r.keys().cloned()).collect();
        v.extend(self.state.keys().cloned());
        v
    }
}

/// Checks a parsed document and returns every diagnostic, errors first in document order.
pub fn check(doc: &Value) -> Vec<Diagnostic> {
    let mut cx = Cx {
        types: BTreeMap::new(),
        state: BTreeMap::new(),
        messages: BTreeMap::new(),
        ids: HashSet::new(),
        used_state: HashSet::new(),
        used_messages: HashSet::new(),
        suggested: BTreeMap::new(),
        surfaces: vec![],
        caps: vec![],
        out: vec![],
    };
    let Some(root) = doc.as_object() else {
        cx.err("", "not-object", "the document must be a JSON object".into(), None);
        return cx.out;
    };
    const TOP: &[&str] = &[
        "sdk",
        "app",
        "capabilities",
        "types",
        "state",
        "persist",
        "initial",
        "messages",
        "feeds",
        "update",
        "logic",
        "surfaces",
    ];
    for key in root.keys() {
        if !TOP.contains(&key.as_str()) {
            cx.err(&format!("/{key}"), "unknown-property", format!("unknown top-level key `{key}`"), suggest(key, TOP));
        }
    }
    match root.get("sdk").and_then(Value::as_u64) {
        Some(v) if v == crate::FORMAT_VERSION => {}
        _ => cx.err(
            "/sdk",
            "unsupported-sdk-version",
            format!("`sdk` must be {}", crate::FORMAT_VERSION),
            Some(format!("add \"sdk\": {}", crate::FORMAT_VERSION)),
        ),
    }
    cx.caps = root
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
        .unwrap_or_default();
    cx.surfaces =
        root.get("surfaces").and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default();
    check_app(root.get("app"), &mut cx);
    check_capabilities(root.get("capabilities"), &mut cx);
    check_types(root.get("types"), &mut cx);
    check_state(root.get("state"), &mut cx);
    check_messages(root.get("messages"), &mut cx);
    check_feeds(root.get("feeds"), root.get("capabilities"), &mut cx);
    check_logic(root, &mut cx);
    check_persist(root.get("persist"), &mut cx);
    check_initial(root.get("initial"), &mut cx);
    match root.get("surfaces").and_then(Value::as_object) {
        Some(s) if !s.is_empty() => {
            for (name, v) in s {
                check_surface(name, v, &mut cx);
            }
        }
        _ => cx.err(
            "/surfaces",
            "missing-property",
            "`surfaces` needs at least one surface".into(),
            Some("add \"main\": {\"kind\":\"window\",...}".into()),
        ),
    }
    let unused: Vec<String> = cx.state.keys().filter(|k| !cx.used_state.contains(*k)).cloned().collect();
    for k in unused {
        cx.warn(
            &format!("/state/{k}"),
            "unused-state",
            format!("state `{k}` is never used"),
            Some("remove it or bind it".into()),
        );
    }
    let unused: Vec<String> = cx.messages.keys().filter(|k| !cx.used_messages.contains(*k)).cloned().collect();
    for k in unused {
        cx.warn(
            &format!("/messages/{k}"),
            "unused-message",
            format!("message `{k}` is never sent"),
            Some("remove it or wire it to a control".into()),
        );
    }
    cx.out.sort_by_key(|d| d.severity == Severity::Warning);
    cx.out
}

fn check_app(app: Option<&Value>, cx: &mut Cx) {
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

fn check_capabilities(caps: Option<&Value>, cx: &mut Cx) {
    let Some(arr) = caps.and_then(Value::as_array) else {
        if caps.is_some() {
            cx.err("/capabilities", "wrong-type", "`capabilities` must be an array of strings".into(), None);
        }
        return;
    };
    for (i, c) in arr.iter().enumerate() {
        match c.as_str() {
            Some(s) if vocab::CAPABILITIES.contains(&s) => {}
            Some(s) => cx.err(
                &format!("/capabilities/{i}"),
                "unknown-capability",
                format!("unknown capability `{s}`"),
                suggest(s, vocab::CAPABILITIES),
            ),
            None => cx.err(&format!("/capabilities/{i}"), "wrong-type", "capability must be a string".into(), None),
        }
    }
}

fn check_types(types: Option<&Value>, cx: &mut Cx) {
    let Some(obj) = types.and_then(Value::as_object) else {
        if types.is_some() {
            cx.err("/types", "wrong-type", "`types` must be an object of record name: {field: type}".into(), None);
        }
        return;
    };
    for (name, fields) in obj {
        let Some(fobj) = fields.as_object() else {
            cx.err(&format!("/types/{name}"), "wrong-type", "a record type is an object of field: type".into(), None);
            continue;
        };
        let mut fs = Fields::new();
        for (f, t) in fobj {
            match t.as_str() {
                Some(t) if vocab::SCALARS.contains(&t) => {
                    fs.insert(f.clone(), t.to_string());
                }
                _ => cx.err(
                    &format!("/types/{name}/{f}"),
                    "bad-state-type",
                    format!("record field `{f}` must be one of {}", vocab::SCALARS.join(", ")),
                    None,
                ),
            }
        }
        cx.types.insert(name.clone(), fs);
    }
}

fn check_state(state: Option<&Value>, cx: &mut Cx) {
    let Some(obj) = state.and_then(Value::as_object) else {
        if state.is_some() {
            cx.err("/state", "wrong-type", "`state` must be an object of name: type".into(), None);
        }
        return;
    };
    for (k, v) in obj {
        let t = v.as_str().unwrap_or("");
        let ok = vocab::SCALARS.contains(&t)
            || t.strip_prefix("list<").and_then(|r| r.strip_suffix('>')).is_some_and(|rec| cx.types.contains_key(rec));
        if ok {
            cx.state.insert(k.clone(), t.to_string());
        } else {
            let fix = match t.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
                Some(rec) => {
                    let names: Vec<&str> = cx.types.keys().map(String::as_str).collect();
                    suggest(rec, &names).or(Some(format!("declare record `{rec}` under `types`")))
                }
                None => Some("use text, bool, number or list<RecordName>".into()),
            };
            cx.err(&format!("/state/{k}"), "bad-state-type", format!("state `{k}` has invalid type `{t}`"), fix);
        }
    }
}

fn check_messages(msgs: Option<&Value>, cx: &mut Cx) {
    let Some(obj) = msgs.and_then(Value::as_object) else {
        if msgs.is_some() {
            cx.err(
                "/messages",
                "wrong-type",
                "`messages` must be an object of Name: payload".into(),
                Some("{\"AddNote\":\"none\",\"EditBody\":\"text\"}".into()),
            );
        }
        return;
    };
    for (name, p) in obj {
        match p.as_str() {
            Some("none") => cx.messages.insert(name.clone(), Payload::None),
            Some("text") => cx.messages.insert(name.clone(), Payload::Text),
            Some("bool") => cx.messages.insert(name.clone(), Payload::Bool),
            _ => {
                cx.err(
                    &format!("/messages/{name}"),
                    "bad-payload",
                    format!("payload of `{name}` must be one of {}", vocab::PAYLOADS.join(", ")),
                    None,
                );
                None
            }
        };
    }
}

fn check_surface(name: &str, v: &Value, cx: &mut Cx) {
    let path = format!("/surfaces/{name}");
    let Some(obj) = v.as_object() else {
        cx.err(&path, "wrong-type", "a surface is an object with a `kind`".into(), None);
        return;
    };
    let Some(kind) = obj.get("kind").and_then(Value::as_str) else {
        cx.err(
            &path,
            "missing-property",
            "surface has no `kind`".into(),
            Some("add \"kind\": window, dialog, popover, panel or fullscreen".into()),
        );
        return;
    };
    let Some(sk) = vocab::surface_kind(kind) else {
        let names: Vec<&str> = vocab::SURFACES.iter().map(|s| s.name).collect();
        cx.err(
            &format!("{path}/kind"),
            "unknown-surface",
            format!("no surface kind `{kind}`"),
            suggest(kind, &names).or(Some(format!("use one of {}", names.join(", ")))),
        );
        return;
    };
    cx.suggested.clear();
    let mut props: Vec<Prop> = vec![Prop { name: "kind", kind: Kind::Str, required: true }];
    props.extend_from_slice(sk.props);
    check_props(obj, &path, &props, &Scope::default(), 1, kind, None, cx);
    if cx.suggested.values().any(|n| *n > 1) {
        cx.warn(
            &path,
            "multiple-suggested",
            "more than one `suggested` button visible at once; a surface has one primary action".into(),
            Some("keep one, make the others default or flat".into()),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check_props(
    obj: &Map<String, Value>,
    path: &str,
    props: &[Prop],
    scope: &Scope,
    depth: usize,
    owner: &str,
    item_type: Option<&str>,
    cx: &mut Cx,
) {
    let all: Vec<&Prop> = props
        .iter()
        .chain(vocab::COMMON.iter().filter(|c| owner_is_component(owner) && !props.iter().any(|p| p.name == c.name)))
        .collect();
    for key in obj.keys() {
        if !all.iter().any(|p| p.name == key) {
            let names: Vec<&str> = all.iter().map(|p| p.name).collect();
            cx.err(
                &format!("{path}/{key}"),
                "unknown-property",
                format!("{owner} has no property `{key}`"),
                suggest(key, &names),
            );
        }
    }
    for p in &all {
        if p.required && !obj.contains_key(p.name) {
            cx.err(
                path,
                "missing-property",
                format!("{owner} needs `{}`", p.name),
                Some(format!("add \"{}\"", p.name)),
            );
        }
    }
    for p in &all {
        let Some(v) = obj.get(p.name) else { continue };
        let vp = format!("{path}/{}", p.name);
        check_value(p, v, &vp, scope, depth, owner, item_type, cx);
    }
}

fn owner_is_component(owner: &str) -> bool {
    vocab::component(owner).is_some()
}

#[allow(clippy::too_many_arguments)]
fn check_value(
    p: &Prop,
    v: &Value,
    path: &str,
    scope: &Scope,
    depth: usize,
    owner: &str,
    item_type: Option<&str>,
    cx: &mut Cx,
) {
    let want_str = |cx: &mut Cx| -> Option<String> {
        match v.as_str() {
            Some(s) => Some(s.to_string()),
            None => {
                cx.err(path, "wrong-type", format!("`{}` must be a string", p.name), None);
                None
            }
        }
    };
    match p.kind {
        Kind::Str => {
            if let Some(s) = want_str(cx)
                && s.is_empty()
                && matches!(p.name, "label" | "title")
            {
                cx.err(path, "empty-label", format!("{owner} `{}` is empty; screen readers need a name", p.name), None);
            }
        }
        Kind::Template => {
            if let Some(s) = want_str(cx) {
                check_template(&s, path, scope, cx);
            }
        }
        Kind::Num => {
            if !v.is_number() {
                cx.err(path, "wrong-type", format!("`{}` must be a number", p.name), None);
            }
        }
        Kind::Bool => {
            if !v.is_boolean() {
                cx.err(path, "wrong-type", format!("`{}` must be true or false", p.name), None);
            }
        }
        Kind::Enum(set) => {
            if let Some(s) = want_str(cx) {
                check_enum(&s, set, path, p.name, cx);
            }
        }
        Kind::EnumOrTemplate(set) => {
            if let Some(s) = want_str(cx) {
                if s.contains('{') {
                    check_template(&s, path, scope, cx);
                } else {
                    check_enum(&s, set, path, p.name, cx);
                }
            }
        }
        Kind::Bind(want) => {
            if let Some(name) = want_str(cx) {
                check_bind(&name, want, path, p.name, owner, scope, cx);
            }
        }
        Kind::Msg(expected) => {
            // Inside a List row, an action sends the row's key as text.
            let expected = if p.name == "on" && scope.row.is_some() { Payload::Text } else { expected };
            if let Some(name) = want_str(cx) {
                cx.used_messages.insert(name.clone());
                match cx.messages.get(&name).copied() {
                    None => {
                        let names: Vec<&str> = cx.messages.keys().map(String::as_str).collect();
                        let fix = suggest(&name, &names)
                            .or(Some(format!("declare `{name}` under `messages` with payload {}", expected.name())));
                        cx.err(path, "unknown-message", format!("message `{name}` is not declared"), fix);
                    }
                    Some(have) if have != expected => cx.err(
                        path,
                        "message-payload-mismatch",
                        format!(
                            "`{}` sends payload {} but `{name}` is declared with {}",
                            p.name,
                            expected.name(),
                            have.name()
                        ),
                        Some(format!("declare \"{name}\": \"{}\"", expected.name())),
                    ),
                    Some(_) => {}
                }
            }
        }
        Kind::Field => {
            if let Some(name) = want_str(cx) {
                let fields = item_type.and_then(|t| cx.types.get(t)).cloned().unwrap_or_default();
                match fields.get(&name).map(String::as_str) {
                    Some("text") => {}
                    Some(_) => cx.err(path, "bad-key", format!("key field `{name}` must be text (a stable id)"), None),
                    None => {
                        let names: Vec<&str> = fields.keys().map(String::as_str).collect();
                        cx.err(
                            path,
                            "unknown-field",
                            format!("the list's records have no field `{name}`"),
                            suggest(&name, &names),
                        );
                    }
                }
            }
        }
        Kind::Node => node(v, path, scope, depth + 1, cx),
        Kind::Nodes => match v.as_array() {
            Some(items) => {
                items.iter().enumerate().for_each(|(i, n)| node(n, &format!("{path}/{i}"), scope, depth + 1, cx))
            }
            None => cx.err(path, "wrong-type", format!("`{}` must be an array of components", p.name), None),
        },
        Kind::Strs => {
            if !v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_string)) {
                cx.err(path, "wrong-type", format!("`{}` must be a non-empty array of strings", p.name), None);
            }
        }
    }
}

fn check_enum(s: &str, set: &str, path: &str, prop: &str, cx: &mut Cx) {
    let allowed = vocab::enum_values(set);
    if !allowed.contains(&s) {
        cx.err(
            path,
            "bad-value",
            format!("`{s}` is not valid for {prop}"),
            suggest(s, allowed).or(Some(format!("use one of {}", allowed.join(", ")))),
        );
    }
}

/// Checks `{name}` references against the visible names.
fn check_template(s: &str, path: &str, scope: &Scope, cx: &mut Cx) {
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else { break };
        let name = &after[..j];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            match cx.lookup(name, scope) {
                None => {
                    let names = cx.visible_names(scope);
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    cx.err(
                        path,
                        "unknown-field",
                        format!("`{{{name}}}` is not a state key or row field here"),
                        suggest(name, &refs),
                    );
                }
                Some(t) if t.starts_with("list<") => cx.err(
                    path,
                    "state-type-mismatch",
                    format!("`{{{name}}}` is a list and cannot be shown as text"),
                    None,
                ),
                Some(_) => {}
            }
        }
        rest = &after[j + 1..];
    }
}

fn check_bind(name: &str, want: Want, path: &str, prop: &str, owner: &str, scope: &Scope, cx: &mut Cx) {
    let Some(have) = cx.lookup(name, scope) else {
        let names = cx.visible_names(scope);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let fix = suggest(name, &refs).or(Some(format!("declare `{name}` under `state`")));
        cx.err(path, "unknown-state", format!("`{name}` is not in `state`"), fix);
        return;
    };
    let ok = match want {
        Want::Any => true,
        Want::Text => have == "text",
        Want::Bool => have == "bool",
        Want::Number => have == "number",
        Want::List => have.starts_with("list<"),
    };
    if !ok {
        cx.err(
            path,
            "state-type-mismatch",
            format!("{owner}.{prop} needs a `{}` state but `{name}` is `{have}`", format!("{want:?}").to_lowercase()),
            None,
        );
    }
}

fn node(v: &Value, path: &str, scope: &Scope, depth: usize, cx: &mut Cx) {
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
    let Some(spec) = vocab::component(ty) else {
        let names: Vec<&str> = vocab::COMPONENTS.iter().map(|c| c.name).collect();
        cx.err(
            &format!("{path}/type"),
            "unknown-component",
            format!("no component `{ty}`"),
            suggest(ty, &names).or(Some(format!("use one of {}", names.join(", ")))),
        );
        return;
    };
    if let Some(id) = obj.get("id").and_then(Value::as_str)
        && !cx.ids.insert(id.to_string())
    {
        cx.err(&format!("{path}/id"), "duplicate-id", format!("id `{id}` is used twice"), None);
    }
    // A List's rows see the fields of its record type; everything else sees the outer scope.
    let mut item_type: Option<String> = None;
    let mut row_scope = scope.clone();
    if ty == "List"
        && let Some(items) = obj.get("items").and_then(Value::as_str)
        && let Some(t) = cx.lookup(items, scope)
        && let Some(rec) = t.strip_prefix("list<").and_then(|r| r.strip_suffix('>'))
    {
        item_type = Some(rec.to_string());
        row_scope.row = cx.types.get(rec).cloned();
    }
    // Check properties; the row template gets the row scope.
    let all: Vec<&Prop> = spec.props.iter().chain(vocab::COMMON.iter()).collect();
    for key in obj.keys() {
        if !all.iter().any(|p| p.name == key) {
            let names: Vec<&str> = all.iter().map(|p| p.name).collect();
            cx.err(
                &format!("{path}/{key}"),
                "unknown-property",
                format!("{ty} has no property `{key}`"),
                suggest(key, &names),
            );
        }
    }
    for p in &all {
        if p.required && !obj.contains_key(p.name) {
            cx.err(path, "missing-property", format!("{ty} needs `{}`", p.name), Some(format!("add \"{}\"", p.name)));
        }
    }
    for p in &all {
        let Some(val) = obj.get(p.name) else { continue };
        let vp = format!("{path}/{}", p.name);
        let sc = if ty == "List" && p.name == "row" { &row_scope } else { scope };
        check_value(p, val, &vp, sc, depth, ty, item_type.as_deref(), cx);
    }
    design_rules(ty, obj, path, cx);
}

/// HIG rules from `.agents/skills/elementaryos-mseries-ui-specialist` that a machine can enforce.
fn design_rules(ty: &str, obj: &Map<String, Value>, path: &str, cx: &mut Cx) {
    let style = obj.get("button_style").and_then(Value::as_str);
    if matches!(ty, "Button" | "IconButton") {
        if style == Some("destructive") && !obj.contains_key("confirm") {
            cx.err(
                path,
                "destructive-needs-confirm",
                "a destructive button must say what it will do in `confirm`".into(),
                Some("add \"confirm\": \"Delete this note?\"".into()),
            );
        }
        if style == Some("suggested") {
            *cx.suggested
                .entry(obj.get("visible_if").and_then(Value::as_str).unwrap_or("").to_string())
                .or_insert(0) += 1;
        }
    }
    if ty == "Entry"
        && obj.get("entry_kind").and_then(Value::as_str) == Some("password")
        && obj.contains_key("on_change")
    {
        cx.warn(
            path,
            "password-change-handler",
            "a password entry should not send its text in messages".into(),
            Some("remove on_change; use on_submit and read the bound state".into()),
        );
    }
    if ty == "Custom" {
        cx.warn(
            path,
            "custom-component",
            "Custom escape hatch used; record the missing component in sdk-bench/GAPS.md".into(),
            None,
        );
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

fn needs_capability(cap: &str, path: &str, what: &str, cx: &mut Cx) {
    if !cx.caps.iter().any(|c| c == cap) {
        cx.err(
            path,
            "missing-capability",
            format!("{what} needs capability `{cap}`"),
            Some(format!("add \"{cap}\" to `capabilities`")),
        );
    }
}

fn check_feeds(feeds: Option<&Value>, _caps: Option<&Value>, cx: &mut Cx) {
    let Some(obj) = feeds.and_then(Value::as_object) else {
        if feeds.is_some() {
            cx.err("/feeds", "wrong-type", "`feeds` must be an object of stateKey: feedName".into(), None);
        }
        return;
    };
    for (key, f) in obj {
        let path = format!("/feeds/{key}");
        let Some(name) = f.as_str() else {
            cx.err(&path, "wrong-type", "a feed is a feed name".into(), None);
            continue;
        };
        let Some(feed) = vocab::feed(name) else {
            let names: Vec<&str> = vocab::FEEDS.iter().map(|f| f.name).collect();
            cx.err(
                &path,
                "unknown-feed",
                format!("no feed `{name}`"),
                suggest(name, &names).or(Some(format!("use one of {}", names.join(", ")))),
            );
            continue;
        };
        needs_capability(feed.capability, &path, &format!("feed `{name}`"), cx);
        cx.used_state.insert(key.clone());
        let Some(rec) =
            cx.state.get(key).and_then(|t| t.strip_prefix("list<")).and_then(|r| r.strip_suffix('>')).map(String::from)
        else {
            cx.err(&path, "state-type-mismatch", format!("feed state `{key}` must be declared as list<Record>"), None);
            continue;
        };
        let fields = cx.types.get(&rec).cloned().unwrap_or_default();
        for (fname, ftype) in feed.fields {
            if fields.get(*fname).map(String::as_str) != Some(*ftype) {
                cx.err(
                    &path,
                    "feed-record-mismatch",
                    format!("record `{rec}` needs field `{fname}: {ftype}` for feed `{name}`"),
                    Some(format!("add \"{fname}\": \"{ftype}\" to types/{rec}")),
                );
            }
        }
    }
}

/// `"logic": "rules"` (default; the `update` section) or `"logic": "rust"` (the app supplies a Rust `Logic`).
fn check_logic(root: &Map<String, Value>, cx: &mut Cx) {
    let mode = root.get("logic").and_then(Value::as_str).unwrap_or("rules");
    if !matches!(mode, "rules" | "rust") {
        cx.err("/logic", "bad-value", format!("`{mode}` is not valid for logic"), Some("use rules or rust".into()));
    }
    match (mode, root.get("update")) {
        ("rust", Some(_)) => cx.err(
            "/update",
            "update-with-rust-logic",
            "`update` rules are ignored when logic is rust".into(),
            Some("remove `update` or set logic to rules".into()),
        ),
        ("rules", Some(u)) => check_update(u, cx),
        ("rules", None) if !cx.messages.is_empty() => cx.err(
            "/update",
            "missing-property",
            "messages are declared but there are no `update` rules".into(),
            Some("add `update` with a rule per message, or set \"logic\": \"rust\"".into()),
        ),
        _ => {}
    }
    if mode == "rust" {
        // Rust logic handles every message itself; nothing to mark unused.
        let names: Vec<String> = cx.messages.keys().cloned().collect();
        cx.used_messages.extend(names);
    }
}

fn check_update(u: &Value, cx: &mut Cx) {
    let Some(obj) = u.as_object() else {
        cx.err("/update", "wrong-type", "`update` is an object of MessageName: [actions]".into(), None);
        return;
    };
    for (msg, actions) in obj {
        let path = format!("/update/{msg}");
        let Some(payload) = cx.messages.get(msg).copied() else {
            let names: Vec<&str> = cx.messages.keys().map(String::as_str).collect();
            cx.err(
                &path,
                "unknown-message",
                format!("rule for undeclared message `{msg}`"),
                suggest(msg, &names).or(Some("declare it under `messages`".into())),
            );
            continue;
        };
        cx.used_messages.insert(msg.clone());
        check_actions(actions, &path, payload, cx);
    }
    let missing: Vec<String> = cx.messages.keys().filter(|m| !obj.contains_key(*m)).cloned().collect();
    for m in missing {
        cx.warn(
            &format!("/update/{m}"),
            "no-rule",
            format!("message `{m}` has no update rule"),
            Some("add a rule or remove the message".into()),
        );
    }
}

fn check_actions(v: &Value, path: &str, payload: Payload, cx: &mut Cx) {
    let Some(list) = v.as_array() else {
        cx.err(path, "wrong-type", "a rule is an array of actions".into(), None);
        return;
    };
    for (i, a) in list.iter().enumerate() {
        let ap = format!("{path}/{i}");
        let Some(obj) = a.as_object() else {
            cx.err(&ap, "wrong-type", "an action is an object like {\"set\":\"key\",\"to\":...}".into(), None);
            continue;
        };
        let verbs: Vec<&String> = obj.keys().filter(|k| vocab::ACTIONS.contains(&k.as_str())).collect();
        let [verb] = verbs.as_slice() else {
            let fix = Some(format!("use exactly one of {}", vocab::ACTIONS.join(", ")));
            cx.err(&ap, "bad-action", "an action needs exactly one verb".into(), fix);
            continue;
        };
        let known = |k: &str| {
            matches!(
                (verb.as_str(), k),
                ("set", "to")
                    | ("add", "by")
                    | ("append", "record")
                    | ("patch", "where" | "fields")
                    | ("remove", "where")
                    | ("call", "args" | "then" | "error")
                    | ("when", "do")
            )
        };
        for k in obj.keys() {
            if k != *verb && !known(k) {
                cx.err(&format!("{ap}/{k}"), "unknown-property", format!("`{verb}` has no property `{k}`"), None);
            }
        }
        let target = obj.get(verb.as_str()).and_then(Value::as_str);
        let state_ty = |cx: &mut Cx, key: &str| -> Option<String> {
            let t = cx.state.get(key).cloned();
            if t.is_some() {
                cx.used_state.insert(key.to_string());
            } else {
                let names: Vec<&str> = cx.state.keys().map(String::as_str).collect();
                let fix = suggest(key, &names).or(Some(format!("declare `{key}` under `state`")));
                cx.err(&format!("{ap}/{verb}"), "unknown-state", format!("`{key}` is not in `state`"), fix);
            }
            t
        };
        match verb.as_str() {
            "set" => {
                let Some(key) = target else {
                    cx.err(&ap, "wrong-type", "`set` names a state key".into(), None);
                    continue;
                };
                let want = state_ty(cx, key);
                match obj.get("to") {
                    Some(e) => {
                        let have = expr_type(e, &format!("{ap}/to"), payload, cx);
                        if let (Some(w), Some(h)) = (want, have)
                            && SCALAR_OK.contains(&w.as_str())
                            && w != h
                        {
                            cx.err(
                                &format!("{ap}/to"),
                                "state-type-mismatch",
                                format!("`{key}` is `{w}` but the value is `{h}`"),
                                None,
                            );
                        }
                    }
                    None => {
                        cx.err(&ap, "missing-property", "`set` needs `to`".into(), Some("add \"to\": value".into()))
                    }
                }
            }
            "add" => {
                if let Some(key) = target
                    && let Some(t) = state_ty(cx, key)
                    && t != "number"
                {
                    cx.err(
                        &format!("{ap}/add"),
                        "state-type-mismatch",
                        format!("`add` needs a number but `{key}` is `{t}`"),
                        None,
                    );
                }
                if obj.get("by").is_some_and(|b| !b.is_number()) {
                    cx.err(&format!("{ap}/by"), "wrong-type", "`by` must be a number".into(), None);
                }
            }
            "toggle" => {
                if let Some(key) = target
                    && let Some(t) = state_ty(cx, key)
                    && t != "bool"
                {
                    cx.err(
                        &format!("{ap}/toggle"),
                        "state-type-mismatch",
                        format!("`toggle` needs a bool but `{key}` is `{t}`"),
                        None,
                    );
                }
            }
            "append" | "patch" | "remove" | "clear" => {
                let Some(key) = target else {
                    cx.err(&ap, "wrong-type", format!("`{verb}` names a list state key"), None);
                    continue;
                };
                let Some(t) = state_ty(cx, key) else { continue };
                let Some(rec) = t.strip_prefix("list<").and_then(|r| r.strip_suffix('>')).map(String::from) else {
                    cx.err(
                        &format!("{ap}/{verb}"),
                        "state-type-mismatch",
                        format!("`{verb}` needs a list but `{key}` is `{t}`"),
                        None,
                    );
                    continue;
                };
                let fields = cx.types.get(&rec).cloned().unwrap_or_default();
                if matches!(verb.as_str(), "patch" | "remove") && fields.get("id").map(String::as_str) != Some("text") {
                    cx.err(
                        &format!("{ap}/{verb}"),
                        "needs-id-field",
                        format!("record `{rec}` needs an `id: text` field for `{verb}`"),
                        Some(format!("add \"id\": \"text\" to types/{rec}")),
                    );
                }
                if verb.as_str() == "append" {
                    match obj.get("record").and_then(Value::as_object) {
                        Some(r) => {
                            for (f, e) in r {
                                match fields.get(f) {
                                    None => {
                                        let names: Vec<&str> = fields.keys().map(String::as_str).collect();
                                        cx.err(
                                            &format!("{ap}/record/{f}"),
                                            "unknown-field",
                                            format!("record `{rec}` has no field `{f}`"),
                                            suggest(f, &names),
                                        );
                                    }
                                    Some(ft) => {
                                        if let Some(h) = expr_type(e, &format!("{ap}/record/{f}"), payload, cx)
                                            && &h != ft
                                        {
                                            cx.err(
                                                &format!("{ap}/record/{f}"),
                                                "state-type-mismatch",
                                                format!("field `{f}` is `{ft}` but the value is `{h}`"),
                                                None,
                                            );
                                        }
                                    }
                                }
                            }
                            for f in fields.keys().filter(|f| !r.contains_key(*f)) {
                                cx.warn(
                                    &format!("{ap}/record"),
                                    "missing-field",
                                    format!("record `{rec}` field `{f}` is not set"),
                                    None,
                                );
                            }
                        }
                        None => cx.err(
                            &ap,
                            "missing-property",
                            "`append` needs `record`".into(),
                            Some("add \"record\": {field: value}".into()),
                        ),
                    }
                }
                if verb.as_str() == "patch" {
                    match obj.get("where") {
                        Some(e) => {
                            expr_type(e, &format!("{ap}/where"), payload, cx);
                        }
                        None => cx.err(
                            &ap,
                            "missing-property",
                            "`patch` needs `where` (the key value to change)".into(),
                            None,
                        ),
                    }
                    match obj.get("fields").and_then(Value::as_object) {
                        Some(r) => {
                            for (f, e) in r {
                                match fields.get(f) {
                                    None => {
                                        let names: Vec<&str> = fields.keys().map(String::as_str).collect();
                                        cx.err(
                                            &format!("{ap}/fields/{f}"),
                                            "unknown-field",
                                            format!("record `{rec}` has no field `{f}`"),
                                            suggest(f, &names),
                                        );
                                    }
                                    Some(ft) => {
                                        if let Some(h) = expr_type(e, &format!("{ap}/fields/{f}"), payload, cx)
                                            && &h != ft
                                        {
                                            cx.err(
                                                &format!("{ap}/fields/{f}"),
                                                "state-type-mismatch",
                                                format!("field `{f}` is `{ft}` but the value is `{h}`"),
                                                None,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        None => cx.err(&ap, "missing-property", "`patch` needs `fields`: {field: value}".into(), None),
                    }
                }
                if verb.as_str() == "remove" {
                    match obj.get("where") {
                        Some(e) => {
                            expr_type(e, &format!("{ap}/where"), payload, cx);
                        }
                        None => cx.err(
                            &ap,
                            "missing-property",
                            "`remove` needs `where` (the key value to remove)".into(),
                            None,
                        ),
                    }
                }
            }
            "call" => {
                let Some(name) = target else {
                    cx.err(&ap, "wrong-type", "`call` names a service".into(), None);
                    continue;
                };
                let Some(svc) = vocab::service(name) else {
                    let names: Vec<&str> = vocab::SERVICES.iter().map(|s| s.name).collect();
                    cx.err(
                        &format!("{ap}/call"),
                        "unknown-service",
                        format!("no service `{name}`"),
                        suggest(name, &names).or(Some(format!("use one of {}", names.join(", ")))),
                    );
                    continue;
                };
                needs_capability(svc.capability, &format!("{ap}/call"), &format!("service `{name}`"), cx);
                let args = obj.get("args").and_then(Value::as_object).cloned().unwrap_or_default();
                for (an, aw) in svc.args {
                    match args.get(*an) {
                        None => cx.err(
                            &ap,
                            "missing-property",
                            format!("`{name}` needs argument `{an}`"),
                            Some(format!("add \"{an}\" under args")),
                        ),
                        Some(e) => {
                            let h = expr_type(e, &format!("{ap}/args/{an}"), payload, cx);
                            let want = format!("{aw:?}").to_lowercase();
                            if let Some(h) = h
                                && h != want
                            {
                                cx.err(
                                    &format!("{ap}/args/{an}"),
                                    "state-type-mismatch",
                                    format!("argument `{an}` is `{want}` but the value is `{h}`"),
                                    None,
                                );
                            }
                        }
                    }
                }
                for an in args.keys().filter(|a| !svc.args.iter().any(|(n, _)| n == a)) {
                    cx.err(
                        &format!("{ap}/args/{an}"),
                        "unknown-property",
                        format!("`{name}` has no argument `{an}`"),
                        None,
                    );
                }
                for (slot, want) in [("then", svc.then), ("error", Payload::Text)] {
                    if let Some(m) = obj.get(slot).and_then(Value::as_str) {
                        cx.used_messages.insert(m.to_string());
                        match cx.messages.get(m).copied() {
                            None => cx.err(
                                &format!("{ap}/{slot}"),
                                "unknown-message",
                                format!("message `{m}` is not declared"),
                                None,
                            ),
                            Some(have) if have != want => cx.err(
                                &format!("{ap}/{slot}"),
                                "message-payload-mismatch",
                                format!(
                                    "`{slot}` of `{name}` carries {} but `{m}` is declared with {}",
                                    want.name(),
                                    have.name()
                                ),
                                None,
                            ),
                            Some(_) => {}
                        }
                    }
                }
            }
            "toast" => {
                if let Some(t) = target {
                    check_template(t, &format!("{ap}/toast"), &Scope::default(), cx);
                }
            }
            "open" | "close" => {
                if let Some(name) = target
                    && !cx.surfaces.iter().any(|s| s == name)
                {
                    let names: Vec<&str> = cx.surfaces.iter().map(String::as_str).collect();
                    cx.err(
                        &format!("{ap}/{verb}"),
                        "unknown-surface",
                        format!("no surface `{name}`"),
                        suggest(name, &names),
                    );
                }
            }
            "quit" => {}
            "when" => {
                if let Some(key) = target
                    && let Some(t) = state_ty(cx, key)
                    && t != "bool"
                {
                    cx.err(
                        &format!("{ap}/when"),
                        "state-type-mismatch",
                        format!("`when` needs a bool but `{key}` is `{t}`"),
                        None,
                    );
                }
                match obj.get("do") {
                    Some(d) => check_actions(d, &format!("{ap}/do"), payload, cx),
                    None => cx.err(&ap, "missing-property", "`when` needs `do`".into(), None),
                }
            }
            _ => {}
        }
    }
}

const SCALAR_OK: &[&str] = &["text", "bool", "number"];

/// The type of an expression: `"$payload"`, a literal, or a template over state keys.
fn expr_type(e: &Value, path: &str, payload: Payload, cx: &mut Cx) -> Option<String> {
    match e {
        Value::Bool(_) => Some("bool".into()),
        Value::Number(_) => Some("number".into()),
        Value::String(s) if s == "$id" || s == "$now" => Some("text".into()),
        Value::String(s) if s == "$payload" => match payload {
            Payload::None => {
                cx.err(
                    path,
                    "no-payload",
                    "`$payload` used but this message carries none".into(),
                    Some("declare the message with payload text or bool".into()),
                );
                None
            }
            p => Some(p.name().into()),
        },
        Value::String(s) => {
            check_template(s, path, &Scope::default(), cx);
            Some("text".into())
        }
        _ => {
            cx.err(path, "wrong-type", "a value is text, a number, true/false or \"$payload\"".into(), None);
            None
        }
    }
}

fn check_persist(p: Option<&Value>, cx: &mut Cx) {
    let Some(v) = p else { return };
    let Some(keys) = v.as_array() else {
        cx.err("/persist", "wrong-type", "`persist` is an array of state keys".into(), None);
        return;
    };
    if !keys.is_empty() {
        needs_capability("filesystem:data", "/persist", "persisting state", cx);
    }
    for (i, k) in keys.iter().enumerate() {
        match k.as_str() {
            Some(k) if cx.state.contains_key(k) => {
                cx.used_state.insert(k.to_string());
            }
            Some(k) => {
                let names: Vec<&str> = cx.state.keys().map(String::as_str).collect();
                cx.err(
                    &format!("/persist/{i}"),
                    "unknown-state",
                    format!("`{k}` is not in `state`"),
                    suggest(k, &names),
                );
            }
            None => cx.err(&format!("/persist/{i}"), "wrong-type", "a state key name".into(), None),
        }
    }
}

/// `initial`: starting values for state keys (settings defaults, pinned items).
fn check_initial(v: Option<&Value>, cx: &mut Cx) {
    let Some(v) = v else { return };
    let Some(obj) = v.as_object() else {
        cx.err("/initial", "wrong-type", "`initial` is an object of stateKey: value".into(), None);
        return;
    };
    for (k, val) in obj {
        let path = format!("/initial/{k}");
        let Some(t) = cx.state.get(k).cloned() else {
            let names: Vec<&str> = cx.state.keys().map(String::as_str).collect();
            cx.err(&path, "unknown-state", format!("`{k}` is not in `state`"), suggest(k, &names));
            continue;
        };
        cx.used_state.insert(k.clone());
        let ok = match t.as_str() {
            "text" => val.is_string(),
            "bool" => val.is_boolean(),
            "number" => val.is_number(),
            _ => val.is_array(),
        };
        if !ok {
            cx.err(&path, "state-type-mismatch", format!("`{k}` is `{t}` but the initial value is {val}"), None);
            continue;
        }
        if let (Some(rec), Some(items)) = (t.strip_prefix("list<").and_then(|r| r.strip_suffix('>')), val.as_array()) {
            let fields = cx.types.get(rec).cloned().unwrap_or_default();
            for (i, item) in items.iter().enumerate() {
                let Some(o) = item.as_object() else {
                    cx.err(&format!("{path}/{i}"), "wrong-type", format!("a `{rec}` record is an object"), None);
                    continue;
                };
                for f in o.keys().filter(|f| !fields.contains_key(*f)) {
                    cx.err(
                        &format!("{path}/{i}/{f}"),
                        "unknown-field",
                        format!("record `{rec}` has no field `{f}`"),
                        None,
                    );
                }
            }
        }
    }
}
