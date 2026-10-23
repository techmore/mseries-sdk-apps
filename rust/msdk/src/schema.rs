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

pub fn schema() -> Value {
    let nodes: Vec<Value> = vocab::COMPONENTS.iter().map(|c| object(Some(("type", c.name)), c.props, true)).collect();
    let surfaces: Vec<Value> = vocab::SURFACES.iter().map(|s| object(Some(("kind", s.name)), s.props, false)).collect();
    let state_type = "^(text|bool|number|list<[A-Z][A-Za-z0-9]*>)$";
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
                "properties": { "id": { "type": "string", "pattern": "^[A-Za-z0-9_]+(\\.[A-Za-z0-9_]+){2,}$" }, "name": { "type": "string" }, "icon": { "type": "string" } }
            },
            "capabilities": { "type": "array", "items": { "enum": vocab::CAPABILITIES } },
            "types": { "type": "object", "additionalProperties": { "type": "object", "additionalProperties": { "enum": vocab::SCALARS } } },
            "state": { "type": "object", "additionalProperties": { "type": "string", "pattern": state_type } },
            "messages": { "type": "object", "additionalProperties": { "enum": vocab::PAYLOADS } },
            "surfaces": { "type": "object", "minProperties": 1, "additionalProperties": { "oneOf": surfaces } }
        },
        "$defs": { "node": { "oneOf": nodes } }
    })
}
