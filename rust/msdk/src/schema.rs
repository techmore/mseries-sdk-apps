//! JSON Schema (draft 2020-12) for `*.ui.json`, generated from [`crate::vocab`]. Feeds constrained decoding for small
//! models: a grammar built from this schema can only produce structurally valid documents. Cross references (state
//! keys, messages, row fields) cannot be expressed in a schema; `msdk check` covers those.

use serde_json::{Map, Value, json};

use crate::vocab::{self, Kind, Prop};

fn prop_schema(p: &Prop) -> Value {
    match p.kind {
        Kind::Str | Kind::Template | Kind::Bind(_) | Kind::Msg(_) | Kind::Field => json!({ "type": "string" }),
        Kind::Num => json!({ "type": "number" }),
        Kind::Bool => json!({ "type": "boolean" }),
        Kind::Enum(set) => json!({ "enum": vocab::enum_values(set) }),
        Kind::EnumOrTemplate(set) => {
            json!({ "anyOf": [{ "enum": vocab::enum_values(set) }, { "type": "string", "pattern": "\\{" }] })
        }
        Kind::Node => json!({ "$ref": "#/$defs/node" }),
        Kind::Nodes => json!({ "type": "array", "items": { "$ref": "#/$defs/node" } }),
        Kind::Strs => json!({ "type": "array", "items": { "type": "string" }, "minItems": 1 }),
    }
}

fn object(tag: Option<(&str, &str)>, props: &[Prop], common: bool) -> Value {
    let mut properties = Map::new();
    let mut required: Vec<&str> = vec![];
    if let Some((k, v)) = tag {
        properties.insert(k.into(), json!({ "const": v }));
        required.push(k);
    }
    let all = props.iter().chain(if common { vocab::COMMON.iter() } else { [].iter() });
    for p in all {
        if p.name == "type" {
            continue;
        }
        properties.insert(p.name.into(), prop_schema(p));
        if p.required {
            required.push(p.name);
        }
    }
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

fn action_schema(verb: &str) -> Value {
    let text = json!({"type":"string"});
    let expression = json!({"$ref":"#/$defs/expression"});
    let fields = json!({"type":"object", "additionalProperties": expression});
    let mut properties = Map::new();
    properties.insert(verb.into(), if verb == "quit" { json!({"const":true}) } else { text.clone() });
    let mut required = vec![verb];
    match verb {
        "set" => {
            properties.insert("to".into(), expression);
            required.push("to");
        }
        "add" => {
            properties.insert("by".into(), json!({"type":"number"}));
        }
        "append" => {
            properties.insert("record".into(), fields);
            required.push("record");
        }
        "patch" => {
            properties.insert("where".into(), expression);
            properties.insert("fields".into(), fields);
            required.extend(["where", "fields"]);
        }
        "remove" => {
            properties.insert("where".into(), expression);
            required.push("where");
        }
        "call" => {
            properties.insert("args".into(), fields);
            properties.insert("then".into(), text.clone());
            properties.insert("error".into(), text);
        }
        "when" => {
            properties.insert("do".into(), json!({"type":"array", "items":{"$ref":"#/$defs/action"}}));
            required.push("do");
        }
        "toggle" | "clear" | "toast" | "open" | "close" | "quit" => {}
        _ => panic!("action {verb} needs a schema"),
    }
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}

pub fn schema() -> Value {
    let nodes: Vec<Value> = vocab::COMPONENTS.iter().map(|c| object(Some(("type", c.name)), c.props, true)).collect();
    let surfaces: Vec<Value> = vocab::SURFACES.iter().map(|s| object(Some(("kind", s.name)), s.props, false)).collect();
    let state_type = "^(text|bool|number|list<[^<>]+>)$";
    let actions: Vec<Value> = vocab::ACTIONS.iter().map(|verb| action_schema(verb)).collect();
    let agent = json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "scopes": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "entities": { "type": "object", "additionalProperties": { "type": "object", "required": ["scope", "expose"], "additionalProperties": false,
                "properties": { "scope": { "type": "string" }, "expose": { "type": "array", "minItems": 1, "items": { "type": "string" } } } } },
            "intents": { "type": "object", "additionalProperties": { "type": "object", "required": ["scope", "risk", "about"], "additionalProperties": false,
                "properties": {
                    "message": { "type": "string" },
                    "param": { "type": "object", "required": ["name", "type"], "additionalProperties": false, "properties": { "name": { "type": "string" }, "type": { "enum": ["text", "bool"] } } },
                    "scope": { "type": "string" }, "risk": { "enum": crate::intents::RISKS },
                    "confirm": { "type": "boolean" }, "shows": { "type": "array", "items": { "type": "string" } }, "verify": { "type": "string" }, "about": { "type": "string" }
                } } }
        }
    });
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "M-series SDK app description",
        "type": "object",
        "required": ["sdk", "app", "surfaces"],
        "additionalProperties": false,
        "properties": {
            "sdk": { "const": crate::FORMAT_VERSION },
            "app": {
                "type": "object", "required": ["id", "name", "icon"], "additionalProperties": false,
                "properties": { "id": { "type": "string", "pattern": "^[A-Za-z0-9_]+(\\.[A-Za-z0-9_]+){2,}$" }, "name": { "type": "string" }, "icon": { "type": "string" }, "version":{"type":"string"}, "license":{"type":"string"}, "website":{"type":"string"} }
            },
            "capabilities": { "type": "array", "items": { "enum": vocab::CAPABILITIES } },
            "types": { "type": "object", "additionalProperties": { "type": "object", "additionalProperties": { "enum": vocab::SCALARS } } },
            "state": { "type": "object", "additionalProperties": { "type": "string", "pattern": state_type } },
            "initial": { "type":"object" },
            "persist": { "type":"array", "items":{"type":"string"} },
            "feeds": { "type":"object", "additionalProperties":{"type":"string"} },
            "logic": { "const":"rust" },
            "update": { "type":"object", "additionalProperties":{"type":"array", "items":{"$ref":"#/$defs/action"}} },
            "messages": { "type": "object", "additionalProperties": { "enum": vocab::PAYLOADS } },
            "agent": agent,
            "surfaces": { "type": "object", "minProperties": 1, "additionalProperties": { "oneOf": surfaces } }
        },
        "$defs": { "node": { "oneOf": nodes }, "expression":{"type":["string","number","boolean"]}, "action":{"oneOf":actions} }
    })
}
