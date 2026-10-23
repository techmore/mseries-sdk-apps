use std::{env, path::PathBuf, process::Command};
fn main() {
    let xml = "../pantheon-compositor/protocol/pantheon-desktop-shell-v1.xml";
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for (mode, file) in [
        ("client-header", "pantheon.h"),
        ("private-code", "pantheon.c"),
    ] {
        assert!(
            Command::new("wayland-scanner")
                .args([mode, xml])
                .arg(out.join(file))
                .status()
                .unwrap()
                .success()
        );
    }
    let gtk = pkg_config::probe_library("gtk4-wayland").unwrap();
    let wl = pkg_config::probe_library("wayland-client").unwrap();
    let mut b = cc::Build::new();
    b.file("src/panel.c")
        .file(out.join("pantheon.c"))
        .include(&out);
    for p in gtk.include_paths.iter().chain(wl.include_paths.iter()) {
        b.include(p);
    }
    b.compile("msdk-panel");
    println!("cargo:rerun-if-changed={xml}");
    println!("cargo:rerun-if-changed=src/panel.c");
}
