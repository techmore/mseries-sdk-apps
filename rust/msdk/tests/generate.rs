use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use msdk::{generate, templates};
use serde_json::{Value, json};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        loop {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("msdk-gen-{}-{stamp}-{serial}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test workspace: {error}"),
            }
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn projects_are_complete_deterministic_and_never_overwrite_existing_files() {
    let scratch = Scratch::new();
    // The generator inspects dependency manifests but does not compile or run them.
    for name in ["msdk", "msdk-gtk"] {
        std::fs::create_dir(scratch.0.join(name)).unwrap();
        std::fs::write(scratch.0.join(name).join("Cargo.toml"), "").unwrap();
    }
    for template in templates::TEMPLATES {
        let doc: Value = serde_json::from_str(template.ui).unwrap();
        let files = generate::project(&doc, &scratch.0).unwrap();
        assert_eq!(files, generate::project(&doc, &scratch.0).unwrap());
        for name in ["Cargo.toml", "app.ui.json", "src/lib.rs", "src/main.rs", "tests/model.rs", "README.md"] {
            assert!(files.contains_key(name), "{} missing {name}", template.name);
        }
        assert!(files["src/main.rs"].contains(doc["app"]["id"].as_str().unwrap()));
        let dir = scratch.0.join(template.name);
        generate::write_project(&dir, &files).unwrap();
        assert!(generate::write_project(&dir, &BTreeMap::new()).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("app.ui.json")).unwrap(), files["app.ui.json"]);
    }
}

#[test]
fn invalid_documents_and_missing_rust_logic_fail_before_generation() {
    assert!(generate::models(&json!({})).is_err());
    let mut doc: Value = serde_json::from_str(templates::TEMPLATES[0].ui).unwrap();
    doc.as_object_mut().unwrap().remove("update");
    doc["logic"] = json!("rust");
    assert!(generate::project(&doc, Path::new(".")).unwrap_err().contains("LogicFactory"));
}

/// Compile the emitted Rust, not just its source string. The fixture is a
/// standalone model crate so core validation also works without GTK installed.
/// Linux qualification separately builds/runs the complete native projects.
#[test]
fn generated_models_compile_and_preserve_state_and_events() {
    let scratch = Scratch::new();
    let msdk = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap();
    let manifest = format!(
        "[package]\nname = \"msdk-generated-model-check\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[lib]\npath = \"lib.rs\"\n[dependencies]\nmsdk = {{ path = {} }}\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n",
        serde_json::to_string(msdk.to_str().unwrap()).unwrap(),
    );
    std::fs::write(scratch.0.join("Cargo.toml"), manifest).unwrap();
    let local_lock = msdk.join("Cargo.lock");
    let workspace_lock = msdk.parent().unwrap().join("Cargo.lock");
    let lock = if local_lock.is_file() { local_lock } else { workspace_lock };
    std::fs::copy(lock, scratch.0.join("Cargo.lock")).unwrap();
    let mut root = String::from("#![deny(warnings)]\n");
    let mut docs: Vec<Value> = templates::TEMPLATES.iter().map(|t| serde_json::from_str(t.ui).unwrap()).collect();
    // Keywords, escaped identifiers and a >2^53 integer must survive codegen.
    let mut unusual = docs[0].clone();
    unusual["state"] = json!({"type": "text", "self": "number", "msdk_encoded_73656c66": "bool"});
    unusual["initial"] = json!({"type": "literal", "self": 9007199254740993_u64});
    unusual["messages"] = json!({"type": "text", "self": "bool", "crate": "none"});
    unusual["update"] = json!({"type": [], "self": [], "crate": []});
    unusual["persist"] = json!([]);
    unusual["surfaces"]["main"]["body"] = json!({"type":"Text", "text":"model check"});
    docs.push(unusual);
    for (index, doc) in docs.iter().enumerate() {
        let dir = scratch.0.join(format!("app{index}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("app.ui.json"), doc.to_string()).unwrap();
        std::fs::write(dir.join("src/lib.rs"), generate::models(doc).unwrap()).unwrap();
        root.push_str(&format!("#[path = \"app{index}/src/lib.rs\"] pub mod app{index};\n#[test] fn initial_{index}() {{\n let state = app{index}::State::initial().unwrap();\n let store = state.clone().into_store().unwrap();\n assert_eq!(serde_json::Value::Object(store.clone()), serde_json::Value::Object(msdk::logic::initial_store(&app{index}::document())));\n assert_eq!(app{index}::State::from_store(store).unwrap(), state);\n}}\n"));
    }
    root.push_str("#[test] fn typed_events_and_large_numbers() {\n assert_eq!(app0::Message::SetEnabled(true).into_event(), (\"SetEnabled\".into(), Some(serde_json::json!(true))));\n assert_eq!(app0::Message::Reset.into_event(), (\"Reset\".into(), None));\n assert_eq!(app0::Message::SetSchedule(\"quoted \\\"text\\\"\".into()).into_event().1, Some(serde_json::json!(\"quoted \\\"text\\\"\")));\n let state = app6::State::initial().unwrap();\n assert_eq!(state.msdk_encoded_73656c66.as_u64(), Some(9007199254740993));\n}\n");
    std::fs::write(scratch.0.join("lib.rs"), root).unwrap();
    let result = std::process::Command::new("cargo")
        .args(["test", "--offline", "--manifest-path"])
        .arg(scratch.0.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", msdk.join("target/generated-model-check"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
