//! Parser for the text form. Produces the same JSON the checker reads, plus a pointer-to-line map.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::lex::{Tok, Token, lex};
use super::names::{ALIASES, COMMON_MODIFIERS, closure_prop, positional, prop_by_text, text_name};
use super::{ParseError, Parsed};
use crate::vocab::{self, Kind};

type R<T> = Result<T, ParseError>;

pub fn parse(src: &str) -> R<Parsed> {
    let toks = lex(src).map_err(|(line, col, message)| ParseError { line, col, message })?;
    let mut p = P { toks, i: 0, lines: BTreeMap::new() };
    let doc = p.document()?;
    Ok(Parsed { doc, lines: p.lines })
}

struct P {
    toks: Vec<Token>,
    i: usize,
    lines: BTreeMap<String, usize>,
}

fn show(t: &Tok) -> String {
    match t {
        Tok::Word(w) => format!("`{w}`"),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Num(n) => format!("{n}"),
        Tok::Sym(c) => format!("`{c}`"),
        Tok::Newline => "the end of the line".into(),
        Tok::Eof => "the end of the file".into(),
    }
}

fn nearest(word: &str, candidates: &[String]) -> Option<String> {
    crate::check::suggest(word, &candidates.iter().map(String::as_str).collect::<Vec<_>>()).map(|s| format!(" ({s})"))
}

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }

    fn next(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn line(&self) -> usize {
        self.toks[self.i].line
    }

    fn fail<T>(&self, message: impl Into<String>) -> R<T> {
        let t = &self.toks[self.i];
        Err(ParseError { line: t.line, col: t.col, message: message.into() })
    }

    fn mark(&mut self, path: &str, line: usize) {
        self.lines.entry(path.to_string()).or_insert(line);
    }

    fn skip_nl(&mut self) {
        while *self.peek() == Tok::Newline {
            self.next();
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if *self.peek() == Tok::Sym(c) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> R<()> {
        if self.eat(c) { Ok(()) } else { self.fail(format!("expected `{c}`, found {}", show(self.peek()))) }
    }

    fn word(&mut self, what: &str) -> R<String> {
        match self.peek().clone() {
            Tok::Word(w) => {
                self.next();
                Ok(w)
            }
            other => self.fail(format!("expected {what}, found {}", show(&other))),
        }
    }

    fn string(&mut self, what: &str) -> R<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.next();
                Ok(s)
            }
            other => self.fail(format!("expected {what} in quotes, found {}", show(&other))),
        }
    }

    /// A name that may be written bare or quoted.
    fn name(&mut self, what: &str) -> R<String> {
        match self.peek().clone() {
            Tok::Word(w) | Tok::Str(w) => {
                self.next();
                Ok(w)
            }
            other => self.fail(format!("expected {what}, found {}", show(&other))),
        }
    }

    /// Comma-separated list; a newline after a comma continues it.
    fn list<T>(&mut self, mut item: impl FnMut(&mut P) -> R<T>) -> R<Vec<T>> {
        let mut out = vec![item(self)?];
        while self.eat(',') {
            self.skip_nl();
            out.push(item(self)?);
        }
        Ok(out)
    }

    fn end_of_statement(&mut self) -> R<()> {
        match self.peek() {
            Tok::Newline | Tok::Eof => Ok(()),
            Tok::Sym('}') => Ok(()),
            other => self.fail(format!("unexpected {} at the end of a statement", show(other))),
        }
    }

    // ---- document ----

    fn document(&mut self) -> R<Value> {
        let mut doc = Map::new();
        doc.insert("sdk".into(), json!(crate::FORMAT_VERSION));
        self.lines.insert(String::new(), 1);
        loop {
            self.skip_nl();
            if *self.peek() == Tok::Eof {
                break;
            }
            let line = self.line();
            let kw = self.word("a section keyword (app, needs, type, state, message, on, window ...)")?;
            match kw.as_str() {
                "app" => {
                    let name = self.name("the app's display name")?;
                    let (mut id, mut icon) = (None, None);
                    let mut extra: Vec<(&'static str, String)> = Vec::new();
                    while let Tok::Word(w) = self.peek().clone() {
                        match w.as_str() {
                            "id" => {
                                self.next();
                                id = Some(self.name("an app id")?);
                            }
                            "icon" => {
                                self.next();
                                icon = Some(self.name("an icon name")?);
                            }
                            "version" | "license" | "website" => {
                                self.next();
                                let key: &'static str = match w.as_str() {
                                    "version" => "version",
                                    "license" => "license",
                                    _ => "website",
                                };
                                extra.push((key, self.name("a value")?));
                            }
                            _ => break,
                        }
                    }
                    let (Some(id), Some(icon)) = (id, icon) else {
                        return self.fail("`app` needs `id ...` and `icon ...`");
                    };
                    let mut app = json!({ "id": id, "name": name, "icon": icon });
                    for (k, v) in extra {
                        app[k] = json!(v);
                    }
                    doc.insert("app".into(), app);
                    self.mark("/app", line);
                }
                "needs" => {
                    let caps = self.list(|p| p.name("a capability in quotes, like \"subprocess\""))?;
                    doc.insert("capabilities".into(), json!(caps));
                    self.mark("/capabilities", line);
                }
                "logic" => {
                    let w = self.word("`rules` or `rust`")?;
                    doc.insert("logic".into(), json!(w));
                    self.mark("/logic", line);
                }
                "type" => self.record(&mut doc, line)?,
                "state" => self.state(&mut doc, line)?,
                "initial" => {
                    self.expect('{')?;
                    let mut m = Map::new();
                    loop {
                        self.skip_nl();
                        while self.eat(',') {
                            self.skip_nl();
                        }
                        if self.eat('}') {
                            break;
                        }
                        let l = self.line();
                        let k = self.name("a state key")?;
                        self.expect(':')?;
                        let v = self.json_value()?;
                        self.mark(&format!("/initial/{k}"), l);
                        m.insert(k, v);
                    }
                    doc.insert("initial".into(), Value::Object(m));
                    self.mark("/initial", line);
                }
                "persist" => {
                    let keys = self.list(|p| p.name("a state key"))?;
                    doc.insert("persist".into(), json!(keys));
                    self.mark("/persist", line);
                }
                "feed" => {
                    let key = self.name("a state key")?;
                    self.expect('=')?;
                    let feed = self.name("a feed name, like windows.list")?;
                    doc.entry("feeds")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("object")
                        .insert(key.clone(), json!(feed));
                    self.mark(&format!("/feeds/{key}"), line);
                }
                "message" => {
                    let items = self.list(|p| {
                        let l = p.line();
                        let n = p.name("a message name")?;
                        let payload = if p.eat('(') {
                            let t = p.word("`Text` or `Bool`")?;
                            p.expect(')')?;
                            match t.as_str() {
                                "Text" => "text",
                                "Bool" => "bool",
                                _ => return p.fail(format!("a message payload is `Text` or `Bool`, not `{t}`")),
                            }
                        } else {
                            "none"
                        };
                        Ok((n, payload, l))
                    })?;
                    let m = doc.entry("messages").or_insert_with(|| json!({})).as_object_mut().expect("object");
                    for (n, p, l) in items {
                        m.insert(n.clone(), json!(p));
                        self.lines.entry(format!("/messages/{n}")).or_insert(l);
                    }
                }
                "on" => {
                    let msg = self.name("a message name")?;
                    self.expect('{')?;
                    let path = format!("/update/{msg}");
                    let acts = self.actions(&path)?;
                    self.expect('}')?;
                    doc.entry("update")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("object")
                        .insert(msg, Value::Array(acts));
                    self.mark(&path, line);
                }
                "window" | "dialog" | "popover" | "panel" | "fullscreen" => self.surface(&kw, &mut doc, line)?,
                other => {
                    let kws: Vec<String> = [
                        "app",
                        "needs",
                        "logic",
                        "type",
                        "state",
                        "initial",
                        "persist",
                        "feed",
                        "message",
                        "on",
                        "window",
                        "dialog",
                        "popover",
                        "panel",
                        "fullscreen",
                    ]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                    self.i = self.i.saturating_sub(1);
                    return self.fail(format!("unknown section `{other}`{}", nearest(other, &kws).unwrap_or_default()));
                }
            }
            self.end_of_statement()?;
        }
        Ok(Value::Object(doc))
    }

    fn scalar_type(&mut self) -> R<String> {
        if self.eat('[') {
            let r = self.word("a record type name")?;
            self.expect(']')?;
            return Ok(format!("list<{r}>"));
        }
        let t = self.word("a type (Text, Bool, Number or [Record])")?;
        Ok(match t.as_str() {
            "Text" => "text".into(),
            "Bool" => "bool".into(),
            "Number" => "number".into(),
            other => return self.fail(format!("unknown type `{other}`; use Text, Bool, Number or [RecordName]")),
        })
    }

    fn record(&mut self, doc: &mut Map<String, Value>, line: usize) -> R<()> {
        let name = self.word("a record name")?;
        self.expect('{')?;
        let mut fields = Map::new();
        loop {
            self.skip_nl();
            while self.eat(',') {
                self.skip_nl();
            }
            if self.eat('}') {
                break;
            }
            let k = self.word("a field name")?;
            self.expect(':')?;
            let t = self.scalar_type()?;
            fields.insert(k, json!(t));
        }
        doc.entry("types")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("object")
            .insert(name.clone(), Value::Object(fields));
        self.mark(&format!("/types/{name}"), line);
        Ok(())
    }

    fn state(&mut self, doc: &mut Map<String, Value>, line: usize) -> R<()> {
        self.expect('{')?;
        let mut m = Map::new();
        loop {
            self.skip_nl();
            while self.eat(',') {
                self.skip_nl();
            }
            if self.eat('}') {
                break;
            }
            let l = self.line();
            let k = self.name("a state key")?;
            self.expect(':')?;
            let t = self.scalar_type()?;
            self.mark(&format!("/state/{k}"), l);
            m.insert(k, json!(t));
        }
        doc.insert("state".into(), Value::Object(m));
        self.mark("/state", line);
        Ok(())
    }

    fn json_value(&mut self) -> R<Value> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.next();
                Ok(json!(s))
            }
            Tok::Num(n) => {
                self.next();
                Ok(number(n))
            }
            Tok::Word(w) if w == "true" || w == "false" => {
                self.next();
                Ok(json!(w == "true"))
            }
            Tok::Sym('[') => {
                self.next();
                let mut items = Vec::new();
                loop {
                    self.skip_nl();
                    while self.eat(',') {
                        self.skip_nl();
                    }
                    if self.eat(']') {
                        break;
                    }
                    items.push(self.json_value()?);
                }
                Ok(Value::Array(items))
            }
            Tok::Sym('{') => {
                self.next();
                let mut m = Map::new();
                loop {
                    self.skip_nl();
                    while self.eat(',') {
                        self.skip_nl();
                    }
                    if self.eat('}') {
                        break;
                    }
                    let k = self.name("a field name")?;
                    self.expect(':')?;
                    m.insert(k, self.json_value()?);
                }
                Ok(Value::Object(m))
            }
            other => self.fail(format!("expected a value, found {}", show(&other))),
        }
    }

    // ---- update rules ----

    /// An expression: text in quotes, a number, true/false, or `$payload`, `$id`, `$now`.
    fn expr(&mut self) -> R<Value> {
        match self.peek().clone() {
            Tok::Word(w) if matches!(w.as_str(), "$payload" | "$id" | "$now") => {
                self.next();
                Ok(json!(w))
            }
            Tok::Word(w) if w == "true" || w == "false" => {
                self.next();
                Ok(json!(w == "true"))
            }
            Tok::Str(s) => {
                self.next();
                Ok(json!(s))
            }
            Tok::Num(n) => {
                self.next();
                Ok(number(n))
            }
            other => self.fail(format!(
                "expected a value (text in quotes, a number, true, false, $payload, $id or $now), found {}",
                show(&other)
            )),
        }
    }

    fn fields(&mut self) -> R<Value> {
        self.expect('{')?;
        let mut m = Map::new();
        loop {
            self.skip_nl();
            while self.eat(',') {
                self.skip_nl();
            }
            if self.eat('}') {
                break;
            }
            let k = self.name("a field name")?;
            self.expect(':')?;
            m.insert(k, self.expr()?);
        }
        Ok(Value::Object(m))
    }

    fn actions(&mut self, path: &str) -> R<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            self.skip_nl();
            if *self.peek() == Tok::Sym('}') {
                break;
            }
            let ap = format!("{path}/{}", out.len());
            let line = self.line();
            self.mark(&ap, line);
            let first = self.word("an action (`key = value`, toggle, add, append, patch, remove, clear, call, toast, open, close, quit, when)")?;
            let a = if *self.peek() == Tok::Sym('=') {
                self.next();
                json!({ "set": first, "to": self.expr()? })
            } else {
                match first.as_str() {
                    "toggle" => json!({ "toggle": self.name("a bool state key")? }),
                    "clear" => json!({ "clear": self.name("a list state key")? }),
                    "add" => {
                        let k = self.name("a number state key")?;
                        let by = if matches!(self.peek(), Tok::Word(w) if w == "by") {
                            self.next();
                            match self.next().tok {
                                Tok::Num(n) => number(n),
                                _ => return self.fail("`by` needs a number"),
                            }
                        } else {
                            Value::Null
                        };
                        if by.is_null() { json!({ "add": k }) } else { json!({ "add": k, "by": by }) }
                    }
                    "append" => {
                        let l = self.name("a list state key")?;
                        json!({ "append": l, "record": self.fields()? })
                    }
                    "patch" => {
                        let l = self.name("a list state key")?;
                        if !matches!(self.peek(), Tok::Word(w) if w == "where") {
                            return self.fail("`patch` needs `where <id value>`");
                        }
                        self.next();
                        let w = self.expr()?;
                        json!({ "patch": l, "where": w, "fields": self.fields()? })
                    }
                    "remove" => {
                        let l = self.name("a list state key")?;
                        if !matches!(self.peek(), Tok::Word(w) if w == "where") {
                            return self.fail("`remove` needs `where <id value>`");
                        }
                        self.next();
                        json!({ "remove": l, "where": self.expr()? })
                    }
                    "call" => {
                        let svc = self.name("a service name, like process.run")?;
                        let mut a = Map::new();
                        a.insert("call".into(), json!(svc));
                        if *self.peek() == Tok::Sym('{') {
                            a.insert("args".into(), self.fields()?);
                        }
                        while let Tok::Word(w) = self.peek().clone() {
                            if w == "then" || w == "error" {
                                self.next();
                                let m = self.name("a message name")?;
                                a.insert(w, json!(m));
                            } else {
                                break;
                            }
                        }
                        Value::Object(a)
                    }
                    "toast" => json!({ "toast": self.string("the toast text")? }),
                    "open" => json!({ "open": self.name("a surface name")? }),
                    "close" => json!({ "close": self.name("a surface name")? }),
                    "quit" => json!({ "quit": true }),
                    "when" => {
                        let k = self.name("a bool state key")?;
                        self.expect('{')?;
                        let inner = self.actions(&format!("{ap}/do"))?;
                        self.expect('}')?;
                        json!({ "when": k, "do": inner })
                    }
                    other => {
                        self.i = self.i.saturating_sub(1);
                        let verbs: Vec<String> = vocab::ACTIONS.iter().map(|s| s.to_string()).collect();
                        return self.fail(format!(
                            "unknown action `{other}`{}; to set state write `{other} = value`",
                            nearest(other, &verbs).unwrap_or_default()
                        ));
                    }
                }
            };
            out.push(a);
            self.end_of_statement()?;
        }
        Ok(out)
    }

    // ---- surfaces and components ----

    fn surface(&mut self, kind: &str, doc: &mut Map<String, Value>, line: usize) -> R<()> {
        let mut name = "main".to_string();
        let mut obj = Map::new();
        obj.insert("kind".into(), json!(kind));
        let anchors = vocab::enum_values("anchor");
        match kind {
            "window" | "dialog" => {
                if let Tok::Word(w) = self.peek().clone() {
                    self.next();
                    name = w;
                }
                obj.insert("title".into(), json!(self.string("the window title")?));
            }
            "panel" => {
                if let Tok::Word(w) = self.peek().clone()
                    && !anchors.contains(&w.as_str())
                {
                    self.next();
                    name = w;
                }
                let a = self.word("an anchor (top, bottom, left or right)")?;
                if !anchors.contains(&a.as_str()) {
                    return self.fail(format!("`{a}` is not an anchor; use top, bottom, left or right"));
                }
                obj.insert("anchor".into(), json!(a));
                while let Tok::Word(w) = self.peek().clone() {
                    match w.as_str() {
                        "size" => {
                            self.next();
                            match self.next().tok {
                                Tok::Num(n) => obj.insert("size".into(), number(n)),
                                _ => return self.fail("`size` needs a number"),
                            };
                        }
                        "hide" => {
                            self.next();
                            let m = self.word("a hide mode")?;
                            obj.insert("hide_mode".into(), json!(m));
                        }
                        _ => break,
                    }
                }
            }
            _ => {
                if let Tok::Word(w) = self.peek().clone() {
                    self.next();
                    name = w;
                }
            }
        }
        let path = format!("/surfaces/{name}");
        self.mark(&path, line);
        self.expect('{')?;
        loop {
            self.skip_nl();
            if self.eat('}') {
                break;
            }
            let l = self.line();
            match self.peek().clone() {
                Tok::Word(w) if matches!(w.as_str(), "header" | "actions") && *self.peek_at(1) == Tok::Sym('{') => {
                    self.next();
                    self.next();
                    let items = self.nodes_until_close(&format!("{path}/{w}"))?;
                    obj.insert(w, Value::Array(items));
                }
                Tok::Word(_) => {
                    if obj.contains_key("body") {
                        return self.fail("a surface has one body component; wrap several in a VStack");
                    }
                    let body = self.component(&format!("{path}/body"))?;
                    self.mark(&format!("{path}/body"), l);
                    obj.insert("body".into(), body);
                }
                other => return self.fail(format!("expected a component, found {}", show(&other))),
            }
        }
        doc.entry("surfaces")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("object")
            .insert(name, Value::Object(obj));
        Ok(())
    }

    /// Components until the matching `}` (which it consumes).
    fn nodes_until_close(&mut self, path: &str) -> R<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            self.skip_nl();
            if self.eat('}') {
                return Ok(out);
            }
            let p = format!("{path}/{}", out.len());
            out.push(self.component(&p)?);
        }
    }

    fn component(&mut self, path: &str) -> R<Value> {
        let t = self.next();
        let Tok::Word(raw) = t.tok else {
            self.i = self.i.saturating_sub(1);
            return self.fail(format!("expected a component name, found {}", show(&self.toks[self.i].tok)));
        };
        self.mark(path, t.line);
        let (ty, implied) = match ALIASES.iter().find(|(a, _, _)| *a == raw) {
            Some((_, c, imp)) => (*c, *imp),
            None => (raw.as_str(), None),
        };
        let Some(comp) = vocab::component(ty) else {
            let mut names: Vec<String> = vocab::COMPONENTS.iter().map(|c| c.name.to_string()).collect();
            names.extend(ALIASES.iter().map(|(a, _, _)| a.to_string()));
            return Err(ParseError {
                line: t.line,
                col: t.col,
                message: format!("no component `{raw}`{}", nearest(&raw, &names).unwrap_or_default()),
            });
        };
        let mut obj = Map::new();
        obj.insert("type".into(), json!(ty));
        if let Some((k, v)) = implied {
            obj.insert(k.into(), json!(v));
        }
        if raw == "HStack" || raw == "VStack" {
            // already set through the alias
        }
        if self.eat('(') {
            self.args(ty, comp, &mut obj, path)?;
        }
        // Block: children, slots, or the trailing-closure action.
        if *self.peek() == Tok::Sym('{') {
            self.next();
            if let Some(action) = closure_prop(ty) {
                self.skip_nl();
                let m = self.name("a message name")?;
                self.skip_nl();
                self.expect('}')?;
                obj.insert(action.into(), json!(m));
            } else {
                self.block(ty, comp, &mut obj, path)?;
            }
        }
        while *self.peek() == Tok::Sym('.') && matches!(self.peek_at(1), Tok::Word(_)) {
            self.next();
            let l = self.line();
            let m = self.word("a modifier")?;
            let Some((_, prop)) = COMMON_MODIFIERS.iter().find(|(n, _)| *n == m) else {
                let names: Vec<String> = COMMON_MODIFIERS.iter().map(|(n, _)| n.to_string()).collect();
                return self.fail(format!("unknown modifier `.{m}`{}", nearest(&m, &names).unwrap_or_default()));
            };
            self.expect('(')?;
            let v = if *prop == "visible_if" { self.name("a bool state key")? } else { self.string("text in quotes")? };
            self.expect(')')?;
            self.mark(&format!("{path}/{prop}"), l);
            obj.insert((*prop).into(), json!(v));
        }
        Ok(Value::Object(obj))
    }

    fn args(&mut self, ty: &str, comp: &'static vocab::Component, obj: &mut Map<String, Value>, path: &str) -> R<()> {
        let mut first = true;
        loop {
            self.skip_nl();
            if self.eat(')') {
                return Ok(());
            }
            let l = self.line();
            let named = matches!(self.peek(), Tok::Word(_)) && *self.peek_at(1) == Tok::Sym(':');
            let prop = if named {
                let n = self.word("a property name")?;
                self.next(); // ':'
                match prop_by_text(comp.props, &n) {
                    Some(p) => *p,
                    None => {
                        let names: Vec<String> = comp.props.iter().map(|p| text_name(p.name)).collect();
                        self.i = self.i.saturating_sub(2);
                        return self
                            .fail(format!("{ty} has no property `{n}`{}", nearest(&n, &names).unwrap_or_default()));
                    }
                }
            } else if first && let Some(pos) = positional(ty) {
                *comp.props.iter().find(|p| p.name == pos).expect("positional prop exists")
            } else {
                return self.fail(format!("{ty} takes no unlabelled value here; write `name: value`"));
            };
            first = false;
            if matches!(prop.kind, Kind::Node | Kind::Nodes) {
                return self.fail(format!(
                    "`{}` is a block; write `{} {{ ... }}` inside {ty}'s braces",
                    text_name(prop.name),
                    text_name(prop.name)
                ));
            }
            let v = self.arg_value(prop.kind, prop.name)?;
            if obj.insert(prop.name.into(), v).is_some_and(|_| true) && !matches!(prop.name, "direction" | "entry_kind")
            {
                return self.fail(format!("`{}` is given twice", text_name(prop.name)));
            }
            self.mark(&format!("{path}/{}", prop.name), l);
            self.skip_nl();
            if !self.eat(',') {
                self.skip_nl();
                self.expect(')')?;
                return Ok(());
            }
        }
    }

    fn arg_value(&mut self, kind: Kind, name: &str) -> R<Value> {
        match kind {
            Kind::Num => match self.next().tok {
                Tok::Num(n) => Ok(number(n)),
                _ => {
                    self.i = self.i.saturating_sub(1);
                    self.fail(format!("`{}` needs a number", text_name(name)))
                }
            },
            Kind::Bool => match self.next().tok {
                Tok::Word(w) if w == "true" || w == "false" => Ok(json!(w == "true")),
                _ => {
                    self.i = self.i.saturating_sub(1);
                    self.fail(format!("`{}` needs true or false", text_name(name)))
                }
            },
            Kind::Strs => {
                self.expect('[')?;
                let items = if self.eat(']') {
                    vec![]
                } else {
                    let v = self.list(|p| p.string("an option in quotes"))?;
                    self.expect(']')?;
                    v
                };
                Ok(json!(items))
            }
            _ => Ok(json!(self.name(&format!("a value for `{}`", text_name(name)))?)),
        }
    }

    fn block(&mut self, ty: &str, comp: &'static vocab::Component, obj: &mut Map<String, Value>, path: &str) -> R<()> {
        let mut children: Vec<Value> = Vec::new();
        loop {
            self.skip_nl();
            if self.eat('}') {
                break;
            }
            let l = self.line();
            match self.peek().clone() {
                Tok::Word(w)
                    if w.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                        && *self.peek_at(1) == Tok::Sym('{') =>
                {
                    let Some(prop) =
                        prop_by_text(comp.props, &w).filter(|p| matches!(p.kind, Kind::Node | Kind::Nodes))
                    else {
                        let names: Vec<String> = comp
                            .props
                            .iter()
                            .filter(|p| matches!(p.kind, Kind::Node | Kind::Nodes))
                            .map(|p| text_name(p.name))
                            .collect();
                        return self
                            .fail(format!("{ty} has no block `{w}`{}", nearest(&w, &names).unwrap_or_default()));
                    };
                    self.next();
                    self.next();
                    let sp = format!("{path}/{}", prop.name);
                    self.mark(&sp, l);
                    if matches!(prop.kind, Kind::Node) {
                        self.skip_nl();
                        let node = self.component(&sp)?;
                        self.skip_nl();
                        self.expect('}')?;
                        obj.insert(prop.name.into(), node);
                    } else {
                        let items = self.nodes_until_close(&sp)?;
                        obj.insert(prop.name.into(), Value::Array(items));
                    }
                }
                Tok::Word(_) => {
                    if !comp.props.iter().any(|p| p.name == "children") {
                        return self.fail(format!("{ty} cannot contain components directly; use one of its blocks"));
                    }
                    let p = format!("{path}/children/{}", children.len());
                    children.push(self.component(&p)?);
                }
                other => return self.fail(format!("expected a component or a block name, found {}", show(&other))),
            }
        }
        if !children.is_empty() {
            obj.insert("children".into(), Value::Array(children));
        }
        Ok(())
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 { json!(n as i64) } else { json!(n) }
}
