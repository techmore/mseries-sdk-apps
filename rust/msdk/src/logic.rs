//! State, templates and the rules engine. No GTK types, so it is testable without a display.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

pub type Store = Map<String, Value>;

/// Something the host must do after a message was handled.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Call { service: String, args: BTreeMap<String, Value>, then: Option<String>, error: Option<String> },
    Toast(String),
    Open(String),
    Close(String),
    Quit,
}

/// App logic: turns a message (and payload) into changes to the store plus effects.
/// `RulesLogic` interprets the document's `update` rules; a Rust app implements this trait itself.
pub trait Logic {
    fn update(&mut self, store: &mut Store, msg: &str, payload: Option<Value>) -> Vec<Effect>;
}

/// Display form of a value inside a template: strings as they are, whole numbers without a fraction.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Replaces `{name}` with the row field of that name, else the state value.
pub fn interpolate(tpl: &str, store: &Store, row: Option<&Value>) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) if !after[..j].is_empty() && after[..j].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                let name = &after[..j];
                let v = row.and_then(|r| r.get(name)).or_else(|| store.get(name));
                out.push_str(&v.map(display).unwrap_or_default());
                rest = &after[j + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn truthy(store: &Store, key: &str) -> bool {
    store.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Default value for a state type string.
pub fn default_for(ty: &str) -> Value {
    match ty {
        "bool" => json!(false),
        "number" => json!(0),
        t if t.starts_with("list<") => json!([]),
        _ => json!(""),
    }
}

pub struct RulesLogic {
    rules: Map<String, Value>,
    counter: u64,
}

impl RulesLogic {
    pub fn new(doc: &Value) -> RulesLogic {
        RulesLogic { rules: doc.get("update").and_then(Value::as_object).cloned().unwrap_or_default(), counter: 0 }
    }

    fn eval(&mut self, e: &Value, store: &Store, payload: &Option<Value>) -> Value {
        match e {
            Value::String(s) if s == "$payload" => payload.clone().unwrap_or(Value::Null),
            Value::String(s) if s == "$id" => {
                self.counter += 1;
                let ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                json!(format!("{ns:x}-{}", self.counter))
            }
            Value::String(s) if s == "$now" => json!(now_text()),
            Value::String(s) => json!(interpolate(s, store, None)),
            other => other.clone(),
        }
    }

    fn run(&mut self, actions: &Value, store: &mut Store, payload: &Option<Value>, out: &mut Vec<Effect>) {
        let Some(list) = actions.as_array() else { return };
        for a in list {
            let Some(obj) = a.as_object() else { continue };
            let text = |k: &str| obj.get(k).and_then(Value::as_str).map(String::from);
            if let Some(key) = text("set") {
                let v = self.eval(obj.get("to").unwrap_or(&Value::Null), store, payload);
                store.insert(key, v);
            } else if let Some(key) = text("toggle") {
                let cur = truthy(store, &key);
                store.insert(key, json!(!cur));
            } else if let Some(key) = text("append") {
                let mut rec = Map::new();
                if let Some(r) = obj.get("record").and_then(Value::as_object) {
                    for (f, e) in r {
                        rec.insert(f.clone(), self.eval(e, store, payload));
                    }
                }
                list_mut(store, &key).push(Value::Object(rec));
            } else if let Some(key) = text("patch") {
                let id = self.eval(obj.get("where").unwrap_or(&Value::Null), store, payload);
                let mut fields = Map::new();
                if let Some(r) = obj.get("fields").and_then(Value::as_object) {
                    for (f, e) in r {
                        fields.insert(f.clone(), self.eval(e, store, payload));
                    }
                }
                for item in list_mut(store, &key).iter_mut() {
                    if item.get("id") == Some(&id)
                        && let Some(o) = item.as_object_mut()
                    {
                        o.extend(fields.clone());
                    }
                }
            } else if let Some(key) = text("remove") {
                let id = self.eval(obj.get("where").unwrap_or(&Value::Null), store, payload);
                list_mut(store, &key).retain(|item| item.get("id") != Some(&id));
            } else if let Some(key) = text("clear") {
                list_mut(store, &key).clear();
            } else if let Some(service) = text("call") {
                let mut args = BTreeMap::new();
                if let Some(a) = obj.get("args").and_then(Value::as_object) {
                    for (k, e) in a {
                        args.insert(k.clone(), self.eval(e, store, payload));
                    }
                }
                out.push(Effect::Call { service, args, then: text("then"), error: text("error") });
            } else if let Some(t) = text("toast") {
                out.push(Effect::Toast(interpolate(&t, store, None)));
            } else if let Some(s) = text("open") {
                out.push(Effect::Open(s));
            } else if let Some(s) = text("close") {
                out.push(Effect::Close(s));
            } else if obj.contains_key("quit") {
                out.push(Effect::Quit);
            } else if let Some(key) = text("when")
                && truthy(store, &key)
                && let Some(d) = obj.get("do")
            {
                self.run(d, store, payload, out);
            }
        }
    }
}

fn list_mut<'a>(store: &'a mut Store, key: &str) -> &'a mut Vec<Value> {
    let v = store.entry(key.to_string()).or_insert_with(|| json!([]));
    if !v.is_array() {
        *v = json!([]);
    }
    v.as_array_mut().expect("just ensured array")
}

fn now_text() -> String {
    // Local wall clock without a time zone database: "2026-10-23 14:05" in UTC is enough for note stamps in dev mode.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Logic for RulesLogic {
    fn update(&mut self, store: &mut Store, msg: &str, payload: Option<Value>) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(actions) = self.rules.get(msg).cloned() {
            self.run(&actions, store, &payload, &mut out);
        }
        out
    }
}
