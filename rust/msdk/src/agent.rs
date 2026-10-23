//! Discovery for a looping model (and for the editor): describe things, complete at a JSON path, explain a diagnostic.
//! Everything is derived from [`crate::vocab`] and the document itself, so answers cannot go stale.

use serde_json::{Value, json};

use crate::vocab::{self, Kind, Prop};

pub struct Completion {
    pub label: String,
    pub detail: String,
}

fn placeholder(p: &Prop) -> Value {
    match p.kind {
        Kind::Str => json!("Label"),
        Kind::Template => json!("{field}"),
        Kind::Num => json!(0),
        Kind::Bool => json!(true),
        Kind::Enum(set) | Kind::EnumOrTemplate(set) => json!(vocab::enum_values(set).first().copied().unwrap_or("")),
        Kind::Bind(_) => json!("stateKey"),
        Kind::Msg(_) => json!("MessageName"),
        Kind::Field => json!("id"),
        Kind::Node => json!({ "type": "Text", "text": "…" }),
        Kind::Nodes => json!([]),
        Kind::Strs => json!(["One", "Two"]),
    }
}

fn props_json(ps: &[Prop]) -> Value {
    ps.iter()
        .map(|p| (p.name.to_string(), json!({ "kind": crate::kind_name(p.kind), "required": p.required })))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// Describes a component, surface kind, service, feed or enum, with a minimal valid example.
pub fn describe(name: &str) -> Option<Value> {
    if let Some(c) = vocab::component(name) {
        let mut ex = serde_json::Map::new();
        ex.insert("type".into(), json!(c.name));
        for p in c.props.iter().filter(|p| p.required) {
            ex.insert(p.name.into(), placeholder(p));
        }
        return Some(
            json!({ "component": c.name, "about": c.about, "props": props_json(c.props), "common_props": props_json(vocab::COMMON), "example": ex }),
        );
    }
    if let Some(s) = vocab::surface_kind(name) {
        let mut ex = serde_json::Map::new();
        ex.insert("kind".into(), json!(s.name));
        for p in s.props.iter().filter(|p| p.required) {
            ex.insert(p.name.into(), placeholder(p));
        }
        return Some(json!({ "surface": s.name, "role": s.role, "props": props_json(s.props), "example": ex }));
    }
    if let Some(s) = vocab::service(name) {
        let args: serde_json::Map<_, _> =
            s.args.iter().map(|(n, w)| ((*n).to_string(), json!(format!("{w:?}").to_lowercase()))).collect();
        return Some(
            json!({ "service": s.name, "about": s.about, "capability": s.capability, "args": args, "then_payload": s.then.name(),
            "example": { "call": s.name, "args": s.args.iter().map(|(n, _)| ((*n).to_string(), json!("…"))).collect::<serde_json::Map<_, _>>() } }),
        );
    }
    if let Some(f) = vocab::feed(name) {
        return Some(
            json!({ "feed": f.name, "about": f.about, "capability": f.capability, "record_fields": f.fields.iter().map(|(n, t)| ((*n).to_string(), json!(t))).collect::<serde_json::Map<_, _>>() }),
        );
    }
    let vals = vocab::enum_values(name);
    (!vals.is_empty()).then(|| json!({ "enum": name, "values": vals }))
}

/// Every name `describe` knows, for "what can I use?".
pub fn catalogue() -> Value {
    json!({
        "components": vocab::COMPONENTS.iter().map(|c| json!({ "name": c.name, "about": c.about })).collect::<Vec<_>>(),
        "surfaces": vocab::SURFACES.iter().map(|s| json!({ "name": s.name, "role": s.role })).collect::<Vec<_>>(),
        "services": vocab::services().into_iter().map(|s| json!({ "name": s.name, "about": s.about })).collect::<Vec<_>>(),
        "feeds": vocab::feeds().into_iter().map(|f| json!({ "name": f.name, "about": f.about })).collect::<Vec<_>>(),
        "enums": vocab::ENUMS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "actions": vocab::ACTIONS,
    })
}

/// What is valid at `pointer` (RFC 6901) in `doc`: the keys an object there may still take, or the values a property there accepts.
pub fn complete(doc: &Value, pointer: &str) -> Vec<Completion> {
    let segs: Vec<String> = pointer.split('/').skip(1).map(|s| s.replace("~1", "/").replace("~0", "~")).collect();
    let c = |l: &str, d: &str| Completion { label: l.into(), detail: d.into() };
    let state = |doc: &Value| -> Vec<(String, String)> {
        doc.get("state")
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default()
    };
    match segs.first().map(String::as_str) {
        None | Some("") => {
            return [
                "sdk",
                "app",
                "capabilities",
                "types",
                "state",
                "persist",
                "messages",
                "feeds",
                "update",
                "logic",
                "surfaces",
            ]
            .iter()
            .map(|k| c(k, "top-level section"))
            .collect();
        }
        Some("capabilities") => return vocab::CAPABILITIES.iter().map(|k| c(k, "capability")).collect(),
        Some("state") if segs.len() == 2 => {
            let mut v: Vec<Completion> = vocab::SCALARS.iter().map(|t| c(t, "scalar type")).collect();
            for r in doc.get("types").and_then(Value::as_object).into_iter().flat_map(|o| o.keys()) {
                v.push(c(&format!("list<{r}>"), "list of records"));
            }
            return v;
        }
        Some("messages") if segs.len() == 2 => {
            return vocab::PAYLOADS.iter().map(|p| c(p, "message payload")).collect();
        }
        Some("feeds") if segs.len() == 2 => return vocab::feeds().into_iter().map(|f| c(f.name, f.about)).collect(),
        Some("update") => return complete_update(doc, &segs, &state(doc)),
        Some("surfaces") => {}
        _ => return vec![],
    }
    // /surfaces/<name>[/...]
    let Some(surf) = segs.get(1).and_then(|n| doc["surfaces"].get(n)) else {
        return if segs.len() == 1 { vec![c("main", "name of a surface")] } else { vec![] };
    };
    let Some(kind) = surf["kind"].as_str().and_then(vocab::surface_kind) else {
        return if segs.len() == 2 { vocab::SURFACES.iter().map(|s| c(s.name, s.role)).collect() } else { vec![] };
    };
    walk(doc, surf, Owner::Surface(kind), &segs[2..], &[])
}

#[derive(Clone, Copy)]
enum Owner {
    Surface(&'static vocab::SurfaceKind),
    Comp(&'static vocab::Component),
}

impl Owner {
    fn props(self) -> Vec<Prop> {
        match self {
            Owner::Surface(k) => k
                .props
                .iter()
                .copied()
                .chain(std::iter::once(Prop { name: "kind", kind: Kind::Str, required: true }))
                .collect(),
            Owner::Comp(k) => k.props.iter().chain(vocab::COMMON.iter()).copied().collect(),
        }
    }
}

fn comp_of(v: &Value) -> Option<Owner> {
    v["type"].as_str().and_then(vocab::component).map(Owner::Comp)
}

fn keys_missing(cur: &Value, owner: Owner) -> Vec<Completion> {
    owner
        .props()
        .iter()
        .filter(|p| cur.get(p.name).is_none())
        .map(|p| Completion {
            label: p.name.into(),
            detail: format!("{}{}", crate::kind_name(p.kind), if p.required { " (required)" } else { "" }),
        })
        .collect()
}

/// `row`: fields visible inside a List row (name, type), empty outside one.
fn walk(doc: &Value, cur: &Value, owner: Owner, path: &[String], row: &[(String, String)]) -> Vec<Completion> {
    let all_components = || {
        vocab::COMPONENTS
            .iter()
            .map(|k| Completion { label: k.name.into(), detail: k.about.into() })
            .collect::<Vec<_>>()
    };
    let Some((key, rest)) = path.split_first() else {
        return if matches!(owner, Owner::Comp(_)) || cur.is_object() {
            keys_missing(cur, owner)
        } else {
            all_components()
        };
    };
    let Some(prop) = owner.props().into_iter().find(|p| p.name == key) else { return vec![] };
    let value = cur.get(key.as_str());
    if rest.is_empty() {
        return match prop.kind {
            Kind::Node | Kind::Nodes => all_components(),
            _ => values_for(&prop, doc, row),
        };
    }
    let Some(value) = value else { return vec![] };
    match prop.kind {
        Kind::Nodes => {
            let Some(item) = rest[0].parse::<usize>().ok().and_then(|i| value.get(i)) else { return vec![] };
            match comp_of(item) {
                Some(o) => walk(doc, item, o, &rest[1..], row),
                None => all_components(),
            }
        }
        Kind::Node => {
            let row2 = if prop.name == "row" { row_fields(doc, cur) } else { row.to_vec() };
            match comp_of(value) {
                Some(o) => walk(doc, value, o, rest, &row2),
                None => all_components(),
            }
        }
        _ => vec![],
    }
}

/// The fields of the record type of a List's `items`.
fn row_fields(doc: &Value, list: &Value) -> Vec<(String, String)> {
    let rec = list["items"]
        .as_str()
        .and_then(|k| doc["state"][k].as_str())
        .and_then(|t| t.strip_prefix("list<"))
        .and_then(|t| t.strip_suffix('>'));
    rec.and_then(|r| doc["types"][r].as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default()
}

fn values_for(p: &Prop, doc: &Value, row: &[(String, String)]) -> Vec<Completion> {
    let c = |l: &str, d: &str| Completion { label: l.into(), detail: d.into() };
    let state: Vec<(String, String)> = doc
        .get("state")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let visible: Vec<&(String, String)> = row.iter().chain(state.iter()).collect();
    match p.kind {
        Kind::Enum(set) | Kind::EnumOrTemplate(set) => vocab::enum_values(set).iter().map(|v| c(v, set)).collect(),
        Kind::Bind(want) => visible
            .iter()
            .filter(|(_, t)| match want {
                vocab::Want::Text => t == "text",
                vocab::Want::Bool => t == "bool",
                vocab::Want::Number => t == "number",
                vocab::Want::List => t.starts_with("list<"),
                vocab::Want::Any => true,
            })
            .map(|(k, t)| c(k, t))
            .collect(),
        Kind::Msg(want) => doc
            .get("messages")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, v)| v.as_str() == Some(want.name()) || (p.name == "on" && v.as_str() == Some("text")))
            .map(|(k, v)| c(k, &format!("payload {}", v.as_str().unwrap_or("?"))))
            .collect(),
        Kind::Node | Kind::Nodes => vocab::COMPONENTS.iter().map(|k| c(k.name, k.about)).collect(),
        Kind::Field => row.iter().filter(|(_, t)| t == "text").map(|(k, t)| c(k, t)).collect(),
        Kind::Template => {
            visible.iter().filter(|(_, t)| !t.starts_with("list<")).map(|(k, t)| c(&format!("{{{k}}}"), t)).collect()
        }
        _ => vec![],
    }
}

fn complete_update(doc: &Value, segs: &[String], state: &[(String, String)]) -> Vec<Completion> {
    let c = |l: &str, d: &str| Completion { label: l.into(), detail: d.into() };
    match segs.len() {
        1 => doc
            .get("messages")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| c(k, &format!("payload {}", v.as_str().unwrap_or("?"))))
            .collect(),
        2 | 3 => vocab::ACTIONS.iter().map(|a| c(a, "action verb")).collect(),
        _ => match segs.last().map(String::as_str) {
            Some("set") | Some("toggle") | Some("when") => state.iter().map(|(k, t)| c(k, t)).collect(),
            Some("append") | Some("patch") | Some("remove") | Some("clear") => {
                state.iter().filter(|(_, t)| t.starts_with("list<")).map(|(k, t)| c(k, t)).collect()
            }
            Some("call") => vocab::services().into_iter().map(|s| c(s.name, s.about)).collect(),
            Some("then") | Some("error") => doc
                .get("messages")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(k, v)| c(k, v.as_str().unwrap_or("")))
                .collect(),
            _ => vec![
                c("$payload", "the message payload"),
                c("$id", "a new unique id"),
                c("$now", "the current date and time"),
            ],
        },
    }
}

/// One-paragraph explanation and the usual repair for each diagnostic code.
pub fn explain(code: &str) -> Option<&'static str> {
    Some(match code {
        "bad-json" => "The file is not valid JSON. Check commas, quotes and brackets near the reported line.",
        "not-object" => "The document, a component or an action must be a JSON object { ... }.",
        "unsupported-sdk-version" => "Add \"sdk\": 1 at the top of the document.",
        "unknown-property" => {
            "That key is not part of this component or section. The fix line suggests the nearest valid name; `msdk describe <Name>` lists them all."
        }
        "missing-property" => {
            "A required property is absent. `msdk describe <Name>` shows which are required and an example."
        }
        "wrong-type" => "The value has the wrong JSON type (for example a number where text is needed).",
        "unknown-component" => "No component has that name. `msdk describe` with no name lists every component.",
        "unknown-surface" => "Surface kinds are window, dialog, popover, panel and fullscreen.",
        "bad-value" => {
            "The value is not in the allowed set. The fix line lists the choices; `msdk describe <enum>` shows them too."
        }
        "bad-app-id" => "Use a reverse-DNS id with at least three parts, such as io.github.techmore.QuickNotes.",
        "unknown-capability" => {
            "Capabilities are fixed names such as filesystem:data or subprocess. `msdk components` lists them."
        }
        "missing-capability" => {
            "The service, feed or persistence you use needs a capability. Add it to `capabilities`."
        }
        "bad-state-type" => {
            "State and record fields are text, bool or number; lists are list<RecordName> where RecordName is under `types`."
        }
        "bad-payload" => "A message payload is none, text or bool.",
        "unknown-state" => {
            "The name is not in `state` (or, inside a List row, not a field of the row's record). Declare it or fix the spelling."
        }
        "state-type-mismatch" => {
            "The control or value needs a different type: Switch needs bool, Entry text, Progress number, List list<Record>."
        }
        "unknown-message" => "The message is not declared under `messages`. Add it with its payload type.",
        "message-payload-mismatch" => {
            "The control sends a different payload than the message declares: buttons send none (or the row key inside a List), Entry sends text, Switch sends bool."
        }
        "unknown-field" => {
            "The record has no field with that name, or a {template} refers to something not visible here. Row fields only exist inside the List row."
        }
        "bad-key" => "A List `key` must name a text field of the record; it is the stable id of each row.",
        "needs-id-field" => "patch and remove find records by `id`. Give the record type an \"id\": \"text\" field.",
        "duplicate-id" => "Component ids must be unique in the document.",
        "empty-label" => "Labels and titles must not be empty; screen readers announce them.",
        "destructive-needs-confirm" => {
            "A destructive button must carry a `confirm` question such as \"Delete this note?\"."
        }
        "multiple-suggested" => {
            "Only one suggested (primary) button should be visible at a time. Make the others default or flat."
        }
        "password-change-handler" => {
            "A password entry must not send its text in messages. Remove on_change, use on_submit, and read the bound state."
        }
        "custom-component" => {
            "You used the Custom escape hatch. Allowed, but record the missing component so the vocabulary can grow."
        }
        "too-deep" => "Nesting is more than 8 levels. Group content in a Card or split it.",
        "bad-action" => {
            "An update action has exactly one verb (set, toggle, add, append, patch, remove, clear, call, toast, open, close, quit, when) plus its modifiers."
        }
        "no-payload" => {
            "$payload was used in a rule for a message with payload none. Declare the message with text or bool."
        }
        "no-rule" => "A declared message has no rule under `update`. Add one or delete the message.",
        "unused-message" | "unused-state" => "Declared but never used. Remove it or wire it up.",
        "missing-field" => "An appended record leaves a field unset. Set every field.",
        "unknown-service" => {
            "Services are fixed names. `msdk describe` lists them; `msdk describe process.run` shows arguments."
        }
        "transcript-record-mismatch" => {
            "A Transcript needs a record type with id, kind and text, all text. kind selects how an entry is drawn: user, assistant, code, tool or permission."
        }
        "stack-child-needs-id" => {
            "A Stack shows the child whose id equals the bound text. Give each child an id (in text form: `.id(\"Chat\")` after it) and list the same names as the Tabs options."
        }
        "unknown-feed" => {
            "Feeds are fixed names such as windows.list. `msdk describe windows.list` shows the record fields it needs."
        }
        "feed-record-mismatch" => {
            "The list's record type must declare every field the feed provides, with the same types."
        }
        "update-with-rust-logic" => "`update` rules and \"logic\": \"rust\" cannot be mixed. Remove one.",
        _ => return None,
    })
}
