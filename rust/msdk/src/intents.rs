//! The agent surface of an app (design: `mseries/SDK-AGENT-SURFACE.md`).
//!
//! An app description may carry an `agent` block that says what an AI agent is allowed to ask for:
//! ```json
//! "agent": {
//!   "scopes":   { "chat.read": "Read thread titles and status" },
//!   "entities": { "Thread": { "scope": "chat.read", "expose": ["title", "project"] } },
//!   "intents":  { "NewThread": { "scope": "chat.create", "risk": "reversible", "verify": "...", "about": "..." } }
//! }
//! ```
//! An intent triggers one existing message (`message`, default: its own name), so an agent can never do something the app's
//! own UI could not. Messages carry at most one payload, so an intent has at most one parameter and its type is that payload.
//! [`diagnose`] is the checker half, [`manifest`] generates the agent-facing tool list, [`validate_args`] is what a broker runs
//! before an intent is sent. Nothing here grants access: grants live in the broker and are bound to the manifest hash.

use serde_json::{Map, Value, json};

use crate::vocab;

pub const RISKS: &[&str] = &["read", "reversible", "external", "privileged"];
pub const MAX_TEXT_ARG: usize = 4000;
const KEYS: &[&str] = &["scopes", "entities", "intents"];
const SECRET_WORDS: &[&str] =
    &["password", "passcode", "secret", "token", "apikey", "api_key", "credential", "private_key", "cookie"];

pub struct Finding {
    pub error: bool,
    pub path: String,
    pub code: &'static str,
    pub message: String,
    pub fix: Option<String>,
}

fn rank(risk: &str) -> usize {
    RISKS.iter().position(|r| *r == risk).unwrap_or(0)
}

fn scope_name_ok(s: &str) -> bool {
    let mut parts = 0;
    for part in s.split('.') {
        parts += 1;
        let mut cs = part.chars();
        if !cs.next().is_some_and(|c| c.is_ascii_lowercase())
            || !cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return false;
        }
    }
    parts >= 2
}

/// The lowest risk class an update rule's service calls allow, and the call that set it.
fn risk_floor(root: &Map<String, Value>, message: &str) -> Option<(usize, String)> {
    fn calls(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                if let Some(s) = o.get("call").and_then(Value::as_str) {
                    out.push(s.to_string());
                }
                o.values().for_each(|x| calls(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| calls(x, out)),
            _ => {}
        }
    }
    let mut names = vec![];
    calls(root.get("update")?.get(message)?, &mut names);
    names
        .into_iter()
        .filter_map(|n| {
            let cap = vocab::service(&n)?.capability;
            let floor = match cap {
                "subprocess" => 3,
                "network" => 2,
                _ => 1,
            };
            Some((floor, n))
        })
        .max_by_key(|(f, _)| *f)
}

pub fn diagnose(root: &Map<String, Value>) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut err = |path: &str, code: &'static str, message: String, fix: Option<String>| {
        out.push(Finding { error: true, path: path.into(), code, message, fix })
    };
    let mut warnings: Vec<Finding> = vec![];
    let Some(agent) = root.get("agent") else { return out };
    let Some(agent) = agent.as_object() else {
        err("/agent", "wrong-type", "`agent` must be an object with scopes, entities and intents".into(), None);
        return out;
    };
    for k in agent.keys().filter(|k| !KEYS.contains(&k.as_str())) {
        err(
            &format!("/agent/{k}"),
            "unknown-property",
            format!("`agent` has no `{k}`"),
            Some(format!("use one of {}", KEYS.join(", "))),
        );
    }

    let mut scopes: Vec<String> = vec![];
    for (name, desc) in agent.get("scopes").and_then(Value::as_object).into_iter().flatten() {
        let path = format!("/agent/scopes/{name}");
        if !scope_name_ok(name) {
            err(
                &path,
                "agent-bad-scope-name",
                format!("scope `{name}` must be lowercase dotted words like `notes.create`"),
                None,
            );
        }
        if !desc.as_str().is_some_and(|d| !d.trim().is_empty()) {
            err(
                &path,
                "agent-scope-needs-description",
                format!("scope `{name}` needs a one-sentence description for the consent prompt"),
                None,
            );
        }
        scopes.push(name.clone());
    }
    let mut used: Vec<String> = vec![];

    let types = root.get("types").and_then(Value::as_object);
    for (name, e) in agent.get("entities").and_then(Value::as_object).into_iter().flatten() {
        let path = format!("/agent/entities/{name}");
        let Some(e) = e.as_object() else {
            err(&path, "wrong-type", "an entity is {scope, expose}".into(), None);
            continue;
        };
        match e.get("scope").and_then(Value::as_str) {
            Some(s) if scopes.iter().any(|x| x == s) => used.push(s.into()),
            Some(s) => err(
                &format!("{path}/scope"),
                "agent-undeclared-scope",
                format!("scope `{s}` is not declared"),
                Some(format!("add `scope \"{s}\" \"...\"`")),
            ),
            None => err(&path, "agent-undeclared-scope", format!("entity `{name}` needs a scope"), None),
        }
        let Some(fields) = types.and_then(|t| t.get(name)).and_then(Value::as_object) else {
            err(&path, "agent-unknown-entity-type", format!("`{name}` is not a record under `types`"), None);
            continue;
        };
        let expose = e.get("expose").and_then(Value::as_array).cloned().unwrap_or_default();
        if expose.is_empty() {
            err(
                &path,
                "agent-entity-exposes-nothing",
                format!("entity `{name}` lists no fields; list the fields an agent may read"),
                None,
            );
        }
        for f in expose.iter().filter_map(Value::as_str) {
            if !fields.contains_key(f) {
                err(
                    &format!("{path}/expose"),
                    "agent-unknown-field",
                    format!("`{name}` has no field `{f}`"),
                    crate::check::suggest(f, &fields.keys().map(String::as_str).collect::<Vec<_>>()),
                );
            } else if SECRET_WORDS.iter().any(|w| f.to_ascii_lowercase().contains(w)) {
                err(
                    &format!("{path}/expose"),
                    "agent-secret-field",
                    format!("field `{f}` looks like a secret and cannot be exposed to agents"),
                    Some("remove it from `expose`".into()),
                );
            }
        }
    }

    let messages = root.get("messages").and_then(Value::as_object);
    for (name, i) in agent.get("intents").and_then(Value::as_object).into_iter().flatten() {
        let path = format!("/agent/intents/{name}");
        let Some(i) = i.as_object() else {
            err(&path, "wrong-type", "an intent is {scope, risk, about, ...}".into(), None);
            continue;
        };
        let message = i.get("message").and_then(Value::as_str).unwrap_or(name);
        let payload = match messages.and_then(|m| m.get(message)).and_then(Value::as_str) {
            Some(p) => p,
            None => {
                let names: Vec<&str> = messages.map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
                err(
                    &path,
                    "agent-unknown-message",
                    format!("intent `{name}` triggers message `{message}`, which does not exist"),
                    crate::check::suggest(message, &names),
                );
                continue;
            }
        };
        let param = i.get("param").and_then(Value::as_object);
        match (payload, param.and_then(|p| p.get("type")).and_then(Value::as_str)) {
            ("none", None) | ("text", Some("text")) | ("bool", Some("bool")) => {}
            ("none", Some(_)) => err(
                &format!("{path}/param"),
                "agent-param-mismatch",
                format!("message `{message}` takes no payload, so intent `{name}` takes no parameter"),
                None,
            ),
            (p, _) => err(
                &format!("{path}/param"),
                "agent-param-mismatch",
                format!("message `{message}` takes a {p} payload, so intent `{name}` needs one {p} parameter"),
                Some(format!("write `intent {name}(value: {})`", if p == "bool" { "Bool" } else { "Text" })),
            ),
        }
        match i.get("scope").and_then(Value::as_str) {
            Some(s) if scopes.iter().any(|x| x == s) => used.push(s.into()),
            Some(s) => err(
                &format!("{path}/scope"),
                "agent-undeclared-scope",
                format!("scope `{s}` is not declared"),
                Some(format!("add `scope \"{s}\" \"...\"`")),
            ),
            None => err(&path, "agent-undeclared-scope", format!("intent `{name}` needs a scope"), None),
        }
        let risk = i.get("risk").and_then(Value::as_str);
        match risk {
            Some(r) if RISKS.contains(&r) => {
                let confirm = i.get("confirm").and_then(Value::as_bool).unwrap_or(false);
                if rank(r) >= 2 && !confirm {
                    err(
                        &path,
                        "agent-needs-confirm",
                        format!("`{name}` is {r}: it must say `confirm` so the owner sees the concrete action"),
                        Some("add `confirm`".into()),
                    );
                }
                if rank(r) >= 1 && !i.get("verify").and_then(Value::as_str).is_some_and(|v| !v.trim().is_empty()) {
                    err(
                        &path,
                        "agent-needs-verify",
                        format!("`{name}` is {r}: say how the result is checked with `verify \"...\"`"),
                        None,
                    );
                }
                if let Some((floor, call)) = risk_floor(root, message)
                    && rank(r) < floor
                {
                    err(
                        &path,
                        "agent-risk-understated",
                        format!("`{name}` is {r} but its rule calls `{call}`, which needs at least {}", RISKS[floor]),
                        Some(format!("set `risk {}`", RISKS[floor])),
                    );
                }
            }
            _ => err(&path, "agent-bad-risk", format!("`{name}` needs `risk` to be one of {}", RISKS.join(", ")), None),
        }
        let state = root.get("state").and_then(Value::as_object);
        for key in i.get("shows").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !state.is_some_and(|st| st.contains_key(key)) {
                err(
                    &format!("{path}/shows"),
                    "agent-unknown-state",
                    format!("`show {key}`: there is no state `{key}`"),
                    crate::check::suggest(
                        key,
                        &state.map(|st| st.keys().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default(),
                    ),
                );
            }
        }
        if i.get("confirm").and_then(Value::as_bool) == Some(true)
            && param.is_none()
            && i.get("shows").and_then(Value::as_array).is_none_or(|a| a.is_empty())
        {
            warnings.push(Finding {
                error: false,
                path: path.clone(),
                code: "agent-confirm-shows-nothing",
                message: format!("`{name}` asks for confirmation but has no parameter and shows no state, so the owner cannot see what is being confirmed"),
                fix: Some("add `show draft` (the state keys that make up the action)".into()),
            });
        }
        if !i.get("about").and_then(Value::as_str).is_some_and(|a| !a.trim().is_empty()) {
            err(
                &path,
                "agent-needs-about",
                format!("`{name}` needs `about \"...\"`: it becomes the tool description an agent reads"),
                None,
            );
        }
    }

    out.extend(warnings);
    for s in scopes.iter().filter(|s| !used.contains(s)) {
        out.push(Finding {
            error: false,
            path: format!("/agent/scopes/{s}"),
            code: "agent-unused-scope",
            message: format!("scope `{s}` is not used by any entity or intent"),
            fix: Some("remove it".into()),
        });
    }
    out
}

/// Sorted-key JSON, so the hash does not depend on how the description was written.
pub fn canonical(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&o[*k])))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn arg_schema(ty: &str) -> Value {
    match ty {
        "bool" => json!({ "type": "boolean" }),
        _ => {
            json!({ "type": "string", "maxLength": MAX_TEXT_ARG, "pattern": "^[^\\u0000-\\u0008\\u000b-\\u001f\\u007f]*$" })
        }
    }
}

/// The agent-facing description of an app: scopes with the strongest risk they gate, intents with input schemas, entities
/// with the state lists that back them. Fails with the checker's errors if the description is not clean.
pub fn manifest(doc: &Value) -> Result<Value, String> {
    let root = doc.as_object().ok_or("the document is not an object")?;
    let errors: Vec<String> = diagnose(root)
        .into_iter()
        .filter(|f| f.error)
        .map(|f| format!("{} {}: {}", f.code, f.path, f.message))
        .collect();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let agent = root
        .get("agent")
        .and_then(Value::as_object)
        .ok_or("this app declares no `agent` block, so it exposes nothing to agents")?;
    let mut intents = Map::new();
    let mut strongest: Map<String, Value> = Map::new();
    for (name, i) in agent.get("intents").and_then(Value::as_object).into_iter().flatten() {
        let (scope, risk) = (i["scope"].as_str().unwrap_or(""), i["risk"].as_str().unwrap_or("read"));
        let param = i.get("param").and_then(Value::as_object);
        let input = match param {
            Some(p) => {
                let pname = p.get("name").and_then(Value::as_str).unwrap_or("value");
                json!({ "type": "object", "properties": { pname: arg_schema(p["type"].as_str().unwrap_or("text")) }, "required": [pname], "additionalProperties": false })
            }
            None => json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        };
        let mut entry = json!({
            "about": i["about"], "scope": scope, "risk": risk,
            "confirm": i.get("confirm").and_then(Value::as_bool).unwrap_or(false),
            "message": i.get("message").and_then(Value::as_str).unwrap_or(name),
            "input": input,
        });
        if let Some(v) = i.get("verify") {
            entry["verify"] = v.clone();
        }
        if let Some(v) = i.get("shows").filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
            entry["shows"] = v.clone();
        }
        let cur = strongest.get(scope).and_then(Value::as_str).map(rank).unwrap_or(0);
        if rank(risk) >= cur {
            strongest.insert(scope.into(), json!(risk));
        }
        intents.insert(name.clone(), entry);
    }
    let mut entities = Map::new();
    for (name, e) in agent.get("entities").and_then(Value::as_object).into_iter().flatten() {
        let fields: Map<String, Value> = e["expose"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|f| (f.to_string(), root["types"][name][f].clone()))
            .collect();
        let sources: Vec<&str> = root
            .get("state")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, t)| t.as_str() == Some(&format!("list<{name}>")))
            .map(|(k, _)| k.as_str())
            .collect();
        entities.insert(name.clone(), json!({ "scope": e["scope"], "fields": fields, "lists": sources }));
        let cur = strongest.get(e["scope"].as_str().unwrap_or("")).and_then(Value::as_str).map(rank);
        if cur.is_none() {
            strongest.insert(e["scope"].as_str().unwrap_or("").into(), json!("read"));
        }
    }
    let scopes: Map<String, Value> = agent
        .get("scopes")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, d)| {
            (k.clone(), json!({ "about": d, "strongest_risk": strongest.get(k).cloned().unwrap_or(json!("read")) }))
        })
        .collect();
    let app = &root["app"];
    let body = json!({ "app": { "id": app["id"], "name": app["name"] }, "scopes": scopes, "entities": entities, "intents": intents });
    let hash = sha256_hex(canonical(&body).as_bytes());
    let mut out = Map::new();
    out.insert("manifest".into(), json!(1));
    out.insert("hash".into(), json!(hash));
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
        out.insert(k, v);
    }
    Ok(Value::Object(out))
}

/// The same manifest in the MCP `tools/list` shape. Read-only entity queries and intents become tools; risk, scope and the
/// manifest hash ride in `_meta` so a broker can enforce grants without trusting the model.
pub fn mcp_tools(doc: &Value) -> Result<Value, String> {
    let m = manifest(doc)?;
    let slug: String =
        m["app"]["id"].as_str().unwrap_or("app").rsplit('.').next().unwrap_or("app").to_ascii_lowercase();
    let hash = m["hash"].clone();
    let mut tools = vec![];
    for (name, e) in m["entities"].as_object().into_iter().flatten() {
        tools.push(json!({
            "name": format!("{slug}_list_{}", snake(name)),
            "description": format!("List {name} records. Fields: {}.", e["fields"].as_object().map(|f| f.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()),
            "inputSchema": { "type": "object", "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false },
            "_meta": { "x-mseries-scope": e["scope"], "x-mseries-risk": "read", "x-mseries-manifest": hash },
        }));
    }
    for (name, i) in m["intents"].as_object().into_iter().flatten() {
        let risk = i["risk"].as_str().unwrap_or("read");
        tools.push(json!({
            "name": format!("{slug}_{}", snake(name)),
            "description": i["about"],
            "inputSchema": i["input"],
            "annotations": { "readOnlyHint": risk == "read", "destructiveHint": rank(risk) >= 2, "openWorldHint": rank(risk) >= 2 },
            "_meta": { "x-mseries-scope": i["scope"], "x-mseries-risk": risk, "x-mseries-confirm": i["confirm"], "x-mseries-shows": i.get("shows").cloned().unwrap_or(json!([])), "x-mseries-manifest": hash },
        }));
    }
    Ok(json!({ "tools": tools }))
}

/// What a broker runs before sending an intent: the arguments must match the intent's single typed parameter exactly.
/// Returns the payload for the message (`None` for a parameterless intent).
pub fn validate_args(manifest: &Value, intent: &str, args: &Value) -> Result<Option<Value>, String> {
    let i = manifest["intents"].get(intent).ok_or_else(|| format!("no intent `{intent}`"))?;
    let props = i["input"]["properties"].as_object().cloned().unwrap_or_default();
    let obj = args.as_object().ok_or("arguments must be an object")?;
    if let Some(extra) = obj.keys().find(|k| !props.contains_key(*k)) {
        return Err(format!("unexpected argument `{extra}`"));
    }
    let Some((pname, schema)) = props.iter().next() else { return Ok(None) };
    let v = obj.get(pname).ok_or_else(|| format!("missing argument `{pname}`"))?;
    match schema["type"].as_str() {
        Some("boolean") => {
            v.as_bool().map(|b| Some(json!(b))).ok_or_else(|| format!("`{pname}` must be true or false"))
        }
        _ => {
            let s = v.as_str().ok_or_else(|| format!("`{pname}` must be text"))?;
            if s.chars().count() > MAX_TEXT_ARG {
                return Err(format!("`{pname}` is longer than {MAX_TEXT_ARG} characters"));
            }
            if s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) {
                return Err(format!("`{pname}` contains control characters"));
            }
            Ok(Some(json!(s)))
        }
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_matches_the_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
