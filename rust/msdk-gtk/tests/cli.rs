//! Run in the same isolated Xvfb/private D-Bus environment as lifecycle/runtime.
use std::process::Command;

#[test]
fn preview_failures_have_nonzero_exit_status() {
    assert_eq!(std::env::var("MSDK_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("msdk-preview-cli-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_msdk-run"))
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap()
    };
    std::fs::write(
        root.join("app.ui.json"),
        include_str!("../../msdk/examples/home-storage.ui.json"),
    )
    .unwrap();
    std::fs::write(root.join("bad.services.json"), r#"{"manifest_version":2}"#).unwrap();
    let result = run(&["app.ui.json"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("unsupported service manifest version")
    );
    std::fs::remove_file(root.join("bad.services.json")).unwrap();
    let result = run(&["app.ui.json", "--services"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("requires a manifest path"));
    let result = run(&["app.ui.json", "--surface", "absent"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no surface"));
    let result = run(&["app.ui.json", "--screenshot", "missing-parent/capture.png"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("screenshot:"));
    assert!(!root.join("missing-parent/capture.png").exists());
}
