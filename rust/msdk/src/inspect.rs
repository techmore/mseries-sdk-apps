//! Introspection: what an app looks like right now, as data. [`render`] turns a description and a store into the tree of
//! visible components with resolved text, values and the message each control would send. It needs no window, so an agent
//! (or a gate) can "see" any SDK app, and the live runtime serves the same tree. [`outline`] prints it as indented text.

use serde_json::{Map, Value, json};

use crate::logic::{Store, display, initial_store, interpolate};

fn s<'a>(n: &'a Value, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

fn is_visible(n: &Value, store: &Store, row: Option<&Value>) -> bool {
    match s(n, "visible_if") {
        Some(k) => row.and_then(|r| r.get(k)).or_else(|| store.get(k)).and_then(Value::as_bool).unwrap_or(false),
        None => true,
    }
}

/// The visible tree of `surface`, with the store the runtime would hold (`overrides` replace starting values).
pub fn render(doc: &Value, surface: &str, overrides: Option<&Store>) -> Result<Value, String> {
    let surf = doc["surfaces"].get(surface).ok_or_else(|| format!("no surface `{surface}`"))?;
    let mut store = initial_store(doc);
    for (k, v) in overrides.into_iter().flatten() {
        store.insert(k.clone(), v.clone());
    }
    let base = format!("/surfaces/{surface}");
    let mut out = Map::new();
    out.insert("surface".into(), json!(surface));
    out.insert("kind".into(), surf["kind"].clone());
    for k in ["title", "anchor", "size", "hide_mode"] {
        if let Some(v) = surf.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    for slot in ["header", "actions"] {
        if let Some(items) = surf.get(slot).and_then(Value::as_array) {
            let nodes: Vec<Value> = items
                .iter()
                .enumerate()
                .filter_map(|(i, n)| node(n, &format!("{base}/{slot}/{i}"), &store, None, None))
                .collect();
            out.insert(slot.into(), Value::Array(nodes));
        }
    }
    if let Some(b) = surf.get("body") {
        out.insert("body".into(), node(b, &format!("{base}/body"), &store, None, None).unwrap_or(Value::Null));
    }
    Ok(Value::Object(out))
}

fn children(n: &Value, slot: &str, path: &str, store: &Store, row: Option<&Value>) -> Vec<Value> {
    n.get(slot)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, c)| node(c, &format!("{path}/{slot}/{i}"), store, row, None))
        .collect()
}

fn put(o: &mut Map<String, Value>, k: &str, v: Option<String>) {
    if let Some(v) = v {
        o.insert(k.into(), json!(v));
    }
}

fn node(n: &Value, path: &str, store: &Store, row: Option<&Value>, key: Option<&str>) -> Option<Value> {
    if !is_visible(n, store, row) {
        return None;
    }
    let ty = s(n, "type")?;
    let mut o = Map::new();
    o.insert("type".into(), json!(ty));
    o.insert("path".into(), json!(path));
    let text = |k: &str| s(n, k).map(|t| interpolate(t, store, row));
    let state = |k: &str| row.and_then(|r| r.get(k)).or_else(|| store.get(k)).cloned().unwrap_or(Value::Null);
    let mut actions = Map::new();
    let mut act = |verb: &str, prop: &str| {
        if let Some(m) = s(n, prop) {
            actions.insert(
                verb.into(),
                json!(if key.is_some() && prop == "on" {
                    format!("{m}({})", key.unwrap_or(""))
                } else {
                    m.to_string()
                }),
            );
        }
    };
    let mut kids: Vec<Value> = Vec::new();
    match ty {
        "Text" => put(&mut o, "text", text("text")),
        "Button" => {
            put(&mut o, "label", text("label"));
            put(&mut o, "style", text("button_style"));
            act("click", "on");
        }
        "IconButton" => {
            put(&mut o, "label", text("label"));
            put(&mut o, "icon", text("icon"));
            put(&mut o, "indicator", text("indicator"));
            put(&mut o, "badge", text("badge").filter(|b| !b.is_empty()));
            act("click", "on");
        }
        "Entry" => {
            put(&mut o, "label", text("label"));
            put(&mut o, "kind", text("entry_kind"));
            o.insert(
                "value".into(),
                if s(n, "entry_kind") == Some("password") {
                    json!("••••")
                } else {
                    state(s(n, "bind").unwrap_or(""))
                },
            );
            act("change", "on_change");
            act("submit", "on_submit");
        }
        "Switch" => {
            put(&mut o, "label", text("label"));
            o.insert("value".into(), state(s(n, "bind").unwrap_or("")));
            act("toggle", "on_change");
        }
        "Dropdown" => {
            put(&mut o, "label", text("label"));
            o.insert("value".into(), state(s(n, "bind").unwrap_or("")));
            o.insert("options".into(), n["options"].clone());
            act("choose", "on_change");
        }
        "Tabs" => {
            put(&mut o, "label", text("label"));
            o.insert("value".into(), state(s(n, "bind").unwrap_or("")));
            o.insert("options".into(), n["options"].clone());
            act("choose", "on_change");
        }
        "Stack" => {
            let page = state(s(n, "bind").unwrap_or("")).as_str().unwrap_or("").to_string();
            o.insert("page".into(), json!(page));
            for (i, c) in n.get("children").and_then(Value::as_array).into_iter().flatten().enumerate() {
                if s(c, "id") == Some(page.as_str())
                    && let Some(v) = node(c, &format!("{path}/children/{i}"), store, row, None)
                {
                    kids.push(v);
                }
            }
        }
        "Progress" => {
            put(&mut o, "label", text("label"));
            o.insert("value".into(), state(s(n, "bind").unwrap_or("")));
        }
        "Gauge" => {
            put(&mut o, "label", text("label"));
            o.insert("value".into(), state(s(n, "bind").unwrap_or("")));
            o.insert("max".into(), n.get("max").cloned().unwrap_or(json!(1)));
            put(&mut o, "state_text", text("state_text"));
        }
        "Card" => {
            put(&mut o, "title", text("title"));
            kids = children(n, "children", path, store, row);
        }
        "Box" => {
            put(&mut o, "direction", s(n, "direction").map(String::from));
            kids = children(n, "children", path, store, row);
        }
        "EmptyState" => {
            put(&mut o, "title", text("title"));
            put(&mut o, "description", text("description"));
            if let Some(a) = n.get("action").and_then(|a| node(a, &format!("{path}/action"), store, row, None)) {
                kids.push(a);
            }
        }
        "Split" => {
            if n.get("autohide").and_then(Value::as_bool) == Some(true) {
                o.insert("autohide".into(), json!(true));
            }
            for slot in ["sidebar", "content"] {
                if let Some(c) = n.get(slot).and_then(|c| node(c, &format!("{path}/{slot}"), store, row, None)) {
                    kids.push(json!({ "slot": slot, "node": c }));
                }
            }
        }
        "List" | "Transcript" => {
            let items = state(s(n, "items").unwrap_or("")).as_array().cloned().unwrap_or_default();
            if ty == "Transcript" {
                o.insert(
                    "entries".into(),
                    json!(
                        items
                            .iter()
                            .map(|e| json!({ "id": e["id"], "kind": e["kind"], "text": e["text"] }))
                            .collect::<Vec<_>>()
                    ),
                );
                act("allow", "on_allow");
                act("deny", "on_deny");
            } else {
                act("select", "on_select");
                let group = s(n, "group_by");
                let mut last_group: Option<String> = None;
                let mut rows = Vec::new();
                for (i, it) in items.iter().enumerate() {
                    if let Some(g) = group {
                        let gv = it.get(g).map(display).unwrap_or_default();
                        if last_group.as_deref() != Some(gv.as_str()) {
                            rows.push(json!({ "section": gv }));
                            last_group = Some(gv);
                        }
                    }
                    let k = n.get("key").and_then(Value::as_str).and_then(|kf| it.get(kf)).map(display);
                    if let Some(r) =
                        n.get("row").and_then(|t| node(t, &format!("{path}/row"), store, Some(it), k.as_deref()))
                    {
                        rows.push(json!({ "index": i, "key": k, "node": r }));
                    }
                }
                o.insert("rows".into(), Value::Array(rows));
            }
            if items.is_empty()
                && let Some(e) = n.get("empty").and_then(|e| node(e, &format!("{path}/empty"), store, row, None))
            {
                o.insert("empty".into(), e);
            }
        }
        "Custom" => {
            put(&mut o, "name", text("name"));
            put(&mut o, "reason", text("reason"));
        }
        _ => {}
    }
    if let Some(t) = text("tooltip") {
        o.insert("tooltip".into(), json!(t));
    }
    if !actions.is_empty() {
        o.insert("actions".into(), Value::Object(actions));
    }
    if !kids.is_empty() {
        o.insert("children".into(), Value::Array(kids));
    }
    Some(Value::Object(o))
}

/// The tree as indented text, one line per control: `Button "New note" [click: AddNote]`.
pub fn outline(tree: &Value) -> String {
    let mut out = String::new();
    let head = format!(
        "{} {}",
        tree["kind"].as_str().unwrap_or("surface"),
        tree["title"].as_str().map(|t| format!("\"{t}\"")).unwrap_or_default()
    );
    out.push_str(head.trim_end());
    out.push('\n');
    for slot in ["header", "actions"] {
        if let Some(items) = tree.get(slot).and_then(Value::as_array) {
            out.push_str(&format!("  {slot}:\n"));
            for n in items {
                line(n, 2, &mut out);
            }
        }
    }
    if let Some(b) = tree.get("body").filter(|b| !b.is_null()) {
        line(b, 1, &mut out);
    }
    out
}

fn line(n: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let mut parts = vec![n["type"].as_str().unwrap_or("?").to_string()];
    for k in ["label", "title", "text", "name"] {
        if let Some(v) = n.get(k).and_then(Value::as_str) {
            parts.push(format!("\"{v}\""));
            break;
        }
    }
    for k in ["value", "page", "state_text", "indicator", "badge"] {
        if let Some(v) = n.get(k).filter(|v| !v.is_null()) {
            parts.push(format!("{k}={}", display(v)));
        }
    }
    if let Some(a) = n.get("actions").and_then(Value::as_object) {
        parts.push(format!(
            "[{}]",
            a.iter().map(|(k, v)| format!("{k}: {}", display(v))).collect::<Vec<_>>().join(", ")
        ));
    }
    out.push_str(&format!("{pad}{}\n", parts.join(" ")));
    if let Some(entries) = n.get("entries").and_then(Value::as_array) {
        for e in entries {
            out.push_str(&format!("{pad}  {}: {}\n", e["kind"].as_str().unwrap_or(""), display(&e["text"])));
        }
    }
    if let Some(rows) = n.get("rows").and_then(Value::as_array) {
        for r in rows {
            match r.get("section").and_then(Value::as_str) {
                Some(sec) => out.push_str(&format!("{pad}  -- {sec} --\n")),
                None => {
                    out.push_str(&format!("{pad}  row {}:\n", r["key"].as_str().unwrap_or("")));
                    line(&r["node"], depth + 2, out);
                }
            }
        }
    }
    if let Some(e) = n.get("empty") {
        out.push_str(&format!("{pad}  (empty)\n"));
        line(e, depth + 2, out);
    }
    for c in n.get("children").and_then(Value::as_array).into_iter().flatten() {
        match c.get("slot").and_then(Value::as_str) {
            Some(slot) => {
                out.push_str(&format!("{pad}  {slot}:\n"));
                line(&c["node"], depth + 2, out);
            }
            None => line(c, depth + 1, out),
        }
    }
}
