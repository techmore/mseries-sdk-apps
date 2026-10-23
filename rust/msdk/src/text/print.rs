//! Canonical text printer: JSON description to the SwiftUI/QML-style text. `parse(print(doc)) == doc` for every valid document.

use serde_json::{Map, Value};

use super::names::{COMMON_MODIFIERS, closure_prop, positional, text_name};
use crate::vocab::{self, Kind, Prop};

type R<T> = Result<T, String>;

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A word the lexer reads back as one bare token.
fn word_ok(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !s.ends_with('.')
        && !s.contains("..")
        && !matches!(s, "true" | "false")
}

fn bare(s: &str) -> String {
    if word_ok(s) { s.to_string() } else { quote(s) }
}

fn num(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.fract() == 0.0 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => "0".into(),
    }
}

/// An expression or literal value on one line.
fn value(v: &Value) -> String {
    match v {
        Value::String(s) if matches!(s.as_str(), "$payload" | "$id" | "$now") => s.clone(),
        Value::String(s) => quote(s),
        Value::Number(_) => num(v),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => format!("[{}]", a.iter().map(value).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{ {} }}",
            o.iter().map(|(k, x)| format!("{}: {}", bare(k), value(x))).collect::<Vec<_>>().join(", ")
        ),
        Value::Null => "\"\"".into(),
    }
}

fn pad(n: usize) -> String {
    "  ".repeat(n)
}

fn scalar_type(t: &str) -> R<String> {
    Ok(match t {
        "text" => "Text".into(),
        "bool" => "Bool".into(),
        "number" => "Number".into(),
        other => match other.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
            Some(r) => format!("[{r}]"),
            None => return Err(format!("unknown type `{other}`")),
        },
    })
}

pub fn print(doc: &Value) -> R<String> {
    let root = doc.as_object().ok_or("the document is not an object")?;
    let mut out = String::new();
    let section = |out: &mut String, s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    if let Some(app) = root.get("app") {
        let g = |k: &str| app.get(k).and_then(Value::as_str).ok_or(format!("app.{k} is missing"));
        let mut line = format!("app {} id {} icon {}", quote(g("name")?), bare(g("id")?), bare(g("icon")?));
        for k in ["version", "license", "website"] {
            if let Some(v) = app.get(k).and_then(Value::as_str) {
                line.push_str(&format!(" {k} {}", if k == "website" { quote(v) } else { bare(v) }));
            }
        }
        section(&mut out, line);
    }
    if let Some(caps) = root.get("capabilities").and_then(Value::as_array).filter(|c| !c.is_empty()) {
        section(
            &mut out,
            format!("needs {}", caps.iter().filter_map(Value::as_str).map(quote).collect::<Vec<_>>().join(", ")),
        );
    }
    if let Some(l) = root.get("logic").and_then(Value::as_str) {
        section(&mut out, format!("logic {l}"));
    }
    if let Some(types) = root.get("types").and_then(Value::as_object) {
        out.push('\n');
        for (name, fields) in types {
            let fs: Vec<String> = fields
                .as_object()
                .ok_or("bad record")?
                .iter()
                .map(|(k, t)| Ok(format!("{k}: {}", scalar_type(t.as_str().unwrap_or(""))?)))
                .collect::<R<_>>()?;
            let one = format!("type {name} {{ {} }}", fs.join(", "));
            if one.len() <= 90 {
                section(&mut out, one);
            } else {
                section(
                    &mut out,
                    format!(
                        "type {name} {{\n{}\n}}",
                        fs.iter().map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n")
                    ),
                );
            }
        }
    }
    if let Some(state) = root.get("state").and_then(Value::as_object) {
        out.push('\n');
        let rows: Vec<String> = state
            .iter()
            .map(|(k, t)| Ok(format!("  {}: {}", bare(k), scalar_type(t.as_str().unwrap_or(""))?)))
            .collect::<R<_>>()?;
        section(&mut out, format!("state {{\n{}\n}}", rows.join("\n")));
    }
    if let Some(init) = root.get("initial").and_then(Value::as_object) {
        let rows: Vec<String> = init.iter().map(|(k, v)| format!("  {}: {}", bare(k), initial_value(v, 1))).collect();
        section(&mut out, format!("initial {{\n{}\n}}", rows.join("\n")));
    }
    if let Some(p) = root.get("persist").and_then(Value::as_array).filter(|p| !p.is_empty()) {
        section(
            &mut out,
            format!("persist {}", p.iter().filter_map(Value::as_str).map(bare).collect::<Vec<_>>().join(", ")),
        );
    }
    if let Some(feeds) = root.get("feeds").and_then(Value::as_object) {
        for (k, f) in feeds {
            section(&mut out, format!("feed {} = {}", bare(k), bare(f.as_str().unwrap_or(""))));
        }
    }
    if let Some(msgs) = root.get("messages").and_then(Value::as_object).filter(|m| !m.is_empty()) {
        out.push('\n');
        let items: Vec<String> = msgs
            .iter()
            .map(|(n, p)| match p.as_str() {
                Some("text") => format!("{n}(Text)"),
                Some("bool") => format!("{n}(Bool)"),
                _ => n.clone(),
            })
            .collect();
        section(&mut out, format!("message {}", items.join(", ")));
    }
    if let Some(update) = root.get("update").and_then(Value::as_object) {
        out.push('\n');
        for (msg, acts) in update {
            section(&mut out, format!("on {} {{\n{}}}", bare(msg), actions(acts, 1)?));
        }
    }
    if let Some(surfaces) = root.get("surfaces").and_then(Value::as_object) {
        for (name, s) in surfaces {
            out.push('\n');
            out.push_str(&surface(name, s)?);
        }
    }
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    Ok(out.trim_start_matches('\n').to_string())
}

fn initial_value(v: &Value, ind: usize) -> String {
    match v {
        Value::Array(a) if !a.is_empty() && a.iter().any(|x| x.is_object()) => {
            let rows: Vec<String> = a.iter().map(|x| format!("{}{}", pad(ind + 1), value(x))).collect();
            format!("[\n{}\n{}]", rows.join(",\n"), pad(ind))
        }
        other => value(other),
    }
}

fn actions(v: &Value, ind: usize) -> R<String> {
    let mut out = String::new();
    for a in v.as_array().ok_or("a rule is an array")? {
        let o = a.as_object().ok_or("an action is an object")?;
        let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("");
        let line = if o.contains_key("set") {
            format!("{} = {}", bare(s("set")), value(&o["to"]))
        } else if o.contains_key("toggle") {
            format!("toggle {}", bare(s("toggle")))
        } else if o.contains_key("add") {
            match o.get("by") {
                Some(by) => format!("add {} by {}", bare(s("add")), num(by)),
                None => format!("add {}", bare(s("add"))),
            }
        } else if o.contains_key("append") {
            format!("append {} {}", bare(s("append")), value(&o["record"]))
        } else if o.contains_key("patch") {
            format!("patch {} where {} {}", bare(s("patch")), value(&o["where"]), value(&o["fields"]))
        } else if o.contains_key("remove") {
            format!("remove {} where {}", bare(s("remove")), value(&o["where"]))
        } else if o.contains_key("clear") {
            format!("clear {}", bare(s("clear")))
        } else if o.contains_key("call") {
            let mut l = format!("call {}", bare(s("call")));
            if let Some(args) = o.get("args").filter(|a| a.as_object().is_some_and(|m| !m.is_empty())) {
                l.push(' ');
                l.push_str(&value(args));
            }
            for k in ["then", "error"] {
                if o.contains_key(k) {
                    l.push_str(&format!(" {k} {}", bare(s(k))));
                }
            }
            l
        } else if o.contains_key("toast") {
            format!("toast {}", quote(s("toast")))
        } else if o.contains_key("open") {
            format!("open {}", bare(s("open")))
        } else if o.contains_key("close") {
            format!("close {}", bare(s("close")))
        } else if o.contains_key("quit") {
            "quit".into()
        } else if o.contains_key("when") {
            format!("when {} {{\n{}{}}}", bare(s("when")), actions(&o["do"], ind + 1)?, pad(ind))
        } else {
            return Err(format!("unknown action {a}"));
        };
        out.push_str(&pad(ind));
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

fn surface(name: &str, s: &Value) -> R<String> {
    let o = s.as_object().ok_or("a surface is an object")?;
    let kind = o.get("kind").and_then(Value::as_str).ok_or("surface kind missing")?;
    let sk = vocab::surface_kind(kind).ok_or(format!("unknown surface kind {kind}"))?;
    let named = if name == "main" { String::new() } else { format!(" {}", bare(name)) };
    let mut head = match kind {
        "window" | "dialog" => format!("{kind}{named} {}", quote(o.get("title").and_then(Value::as_str).unwrap_or(""))),
        "panel" => {
            let mut h = format!("panel{named} {}", o.get("anchor").and_then(Value::as_str).unwrap_or("bottom"));
            if let Some(sz) = o.get("size") {
                h.push_str(&format!(" size {}", num(sz)));
            }
            if let Some(m) = o.get("hide_mode").and_then(Value::as_str) {
                h.push_str(&format!(" hide {m}"));
            }
            h
        }
        _ => format!("{kind}{named}"),
    };
    head.push_str(" {\n");
    for p in sk.props {
        match (p.name, o.get(p.name)) {
            ("header" | "actions", Some(Value::Array(items))) => {
                head.push_str(&format!("  {} {{\n", p.name));
                for n in items {
                    node(n, 2, &mut head)?;
                }
                head.push_str("  }\n");
            }
            ("body", Some(b)) => node(b, 1, &mut head)?,
            _ => {}
        }
    }
    head.push_str("}\n");
    Ok(head)
}

fn prop_value(p: &Prop, v: &Value) -> R<String> {
    Ok(match p.kind {
        Kind::Num => num(v),
        Kind::Bool => v.as_bool().ok_or("expected a bool")?.to_string(),
        Kind::Strs => format!(
            "[{}]",
            v.as_array()
                .ok_or("expected a list")?
                .iter()
                .filter_map(Value::as_str)
                .map(quote)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Kind::Str | Kind::Template => quote(v.as_str().ok_or_else(|| format!("{} must be text", p.name))?),
        _ => {
            let s = v.as_str().ok_or_else(|| format!("{} must be text", p.name))?;
            if s.contains('{') { quote(s) } else { bare(s) }
        }
    })
}

fn node(v: &Value, ind: usize, out: &mut String) -> R<()> {
    let o = v.as_object().ok_or("a component is an object")?;
    let ty = o.get("type").and_then(Value::as_str).ok_or("component without a type")?;
    let comp = vocab::component(ty).ok_or(format!("unknown component {ty}"))?;
    for k in o.keys() {
        let known =
            k == "type" || comp.props.iter().any(|p| p.name == k) || COMMON_MODIFIERS.iter().any(|(_, j)| j == k);
        if !known {
            return Err(format!("{ty} has no property `{k}`"));
        }
    }
    let (spelled, skip_direction) = match (ty, o.get("direction").and_then(Value::as_str)) {
        ("Box", Some("row")) => ("HStack", true),
        ("Box", Some("column")) => ("VStack", true),
        _ => (ty, false),
    };
    let closure = closure_prop(ty).filter(|c| o.contains_key(*c));
    let pos = positional(ty).filter(|p| o.contains_key(*p));
    let mut args: Vec<String> = Vec::new();
    if let Some(pn) = pos {
        let p = comp.props.iter().find(|p| p.name == pn).expect("positional prop exists");
        args.push(prop_value(p, &o[pn])?);
    }
    for p in comp.props {
        if Some(p.name) == pos
            || Some(p.name) == closure
            || matches!(p.kind, Kind::Node | Kind::Nodes)
            || (skip_direction && p.name == "direction")
        {
            continue;
        }
        if let Some(val) = o.get(p.name) {
            args.push(format!("{}: {}", text_name(p.name), prop_value(p, val)?));
        }
    }
    let mut line = format!("{}{spelled}", pad(ind));
    if !args.is_empty() {
        let one = format!("({})", args.join(", "));
        if line.len() + one.len() <= 100 {
            line.push_str(&one);
        } else {
            line.push_str(&format!(
                "(\n{}\n{})",
                args.iter().map(|a| format!("{}{a}", pad(ind + 1))).collect::<Vec<_>>().join(",\n"),
                pad(ind)
            ));
        }
    }
    let mut body = String::new();
    if let Some(c) = closure {
        line.push_str(&format!(" {{ {} }}", bare(o[c].as_str().unwrap_or(""))));
    } else {
        for p in comp.props.iter().filter(|p| matches!(p.kind, Kind::Node | Kind::Nodes)) {
            let Some(val) = o.get(p.name) else { continue };
            match (p.name, p.kind, val) {
                ("children", _, Value::Array(items)) => {
                    for n in items {
                        node(n, ind + 1, &mut body)?;
                    }
                }
                (_, Kind::Nodes, Value::Array(items)) => {
                    body.push_str(&format!("{}{} {{\n", pad(ind + 1), text_name(p.name)));
                    for n in items {
                        node(n, ind + 2, &mut body)?;
                    }
                    body.push_str(&format!("{}}}\n", pad(ind + 1)));
                }
                (_, Kind::Node, n) => {
                    body.push_str(&format!("{}{} {{\n", pad(ind + 1), text_name(p.name)));
                    node(n, ind + 2, &mut body)?;
                    body.push_str(&format!("{}}}\n", pad(ind + 1)));
                }
                _ => return Err(format!("{ty}.{} has the wrong shape", p.name)),
            }
        }
    }
    let mods = modifiers(o);
    if body.is_empty() {
        line.push_str(&mods);
        line.push('\n');
        out.push_str(&line);
    } else {
        out.push_str(&format!("{line} {{\n{body}{}}}{mods}\n", pad(ind)));
    }
    Ok(())
}

fn modifiers(o: &Map<String, Value>) -> String {
    let mut s = String::new();
    for (name, prop) in COMMON_MODIFIERS {
        if let Some(v) = o.get(*prop).and_then(Value::as_str) {
            let arg = if *prop == "visible_if" { bare(v) } else { quote(v) };
            s.push_str(&format!(".{name}({arg})"));
        }
    }
    s
}
