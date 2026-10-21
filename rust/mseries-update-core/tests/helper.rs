//! The root helper must refuse everything outside the allowlist *before* it runs anything.
//! Only rejections are exercised here: accepted operations would exec the real /usr/sbin/mseries-update.

use std::process::Command;

fn helper(args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mseries-update-helper")).args(args).env_clear().output().expect("run helper");
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn refuses_what_the_ui_never_sends() {
    for args in [
        vec![],
        vec!["apply", "--now"],
        vec!["init", "--uri", "https://example.invalid", "--keyring", "/tmp/k"],
        vec!["config", "auto_stage", "true"],
        vec!["boot"],
        vec!["channel", "nightly"],
        vec!["check", "--include-skipped"],
        vec!["stage", "extra"],
        vec!["-h"],
        vec!["; id"],
    ] {
        let (code, stderr) = helper(&args);
        assert_eq!(code, Some(2), "{args:?} must be refused, got {code:?}: {stderr}");
        assert!(stderr.contains("mseries-update-helper:"), "{args:?}: {stderr}");
    }
}
