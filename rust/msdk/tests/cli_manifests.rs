use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        loop {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("msdk-cli-{}-{stamp}-{serial}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    std::fs::write(path.join("app.ui.json"), include_str!("../examples/home-storage.ui.json")).unwrap();
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create CLI fixture: {error}"),
            }
        }
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_msdk")).current_dir(&self.0).args(args).output().unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn invalid_adjacent_manifest_fails_every_consumer_instead_of_printing_success() {
    let project = Project::new();
    std::fs::write(project.0.join("bad.services.json"), r#"{"manifest_version":2}"#).unwrap();
    for args in [
        vec!["check", "app.ui.json"],
        vec!["render", "app.ui.json"],
        vec!["gate", "app.ui.json"],
        vec!["complete", "app.ui.json", "/"],
        vec!["agent-manifest", "app.ui.json"],
        vec!["gen", "app.ui.json", "generated"],
    ] {
        let result = project.run(&args);
        assert!(!result.status.success(), "{args:?}");
        let output = format!("{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        assert!(output.contains("unsupported service manifest version"), "{args:?}: {output}");
        assert!(!output.contains(": ok (") && !output.contains("created standalone"));
        assert!(!project.0.join("generated").exists());
    }
    let result = project.run(&["check", "--json", "app.ui.json"]);
    let diagnostics: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(!result.status.success());
    assert_eq!(diagnostics[0]["code"], "service-manifest");
    assert_eq!(diagnostics[0]["severity"], "error");
}

#[test]
fn missing_explicit_manifest_and_missing_flag_value_are_errors() {
    let project = Project::new();
    for args in [vec!["check", "app.ui.json", "--services", "missing.json"], vec!["check", "app.ui.json", "--services"]]
    {
        assert!(!project.run(&args).status.success(), "{args:?}");
    }
}

#[test]
fn checking_no_descriptions_is_not_a_successful_validation() {
    let project = Project::new();
    let result = project.run(&["check", "--json"]);
    assert!(!result.status.success());
    let diagnostic: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(diagnostic[0]["code"], "missing-input");
}

#[test]
fn valid_explicit_manifest_supports_check_and_gate_without_positional_confusion() {
    let project = Project::new();
    std::fs::write(project.0.join("explicit.json"), json!({"manifest_version":1,"services":[]}).to_string()).unwrap();
    for args in [
        vec!["check", "app.ui.json", "--services", "explicit.json"],
        vec!["gate", "app.ui.json", "--services", "explicit.json"],
    ] {
        let result = project.run(&args);
        assert!(result.status.success(), "{args:?}: {}", String::from_utf8_lossy(&result.stderr));
    }
}
