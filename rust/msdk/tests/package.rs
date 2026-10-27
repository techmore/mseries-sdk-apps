use msdk::package;
use serde_json::json;

fn elf(machine: u16) -> Vec<u8> {
    let mut data = vec![0; 64];
    data[..4].copy_from_slice(b"\x7fELF");
    data[4] = 2;
    data[5] = 1;
    data[16] = 3; // position-independent executable
    data[18..20].copy_from_slice(&machine.to_le_bytes());
    data
}

#[test]
fn package_architecture_comes_from_the_binary_not_the_build_host() {
    assert_eq!(package::architecture(&elf(183)).unwrap(), "arm64");
    assert_eq!(package::architecture(&elf(62)).unwrap(), "amd64");
    assert!(package::architecture(&elf(40)).is_err());
    let mut wrong = elf(62);
    wrong[5] = 2;
    assert!(package::architecture(&wrong).is_err());
    wrong[5] = 1;
    wrong[16] = 1;
    assert!(package::architecture(&wrong).is_err());
    assert!(package::architecture(b"not an executable").is_err());
}

#[test]
fn desktop_fields_cannot_inject_an_extra_launcher_action() {
    let doc = json!({"app":{"id":"com.example.Notes", "name":"Notes\nExec=bad\\value\ttext"}});
    let entry = package::desktop_entry(&doc);
    assert!(entry.contains("Name=Notes\\nExec=bad\\\\value\\ttext\n"));
    assert_eq!(entry.lines().filter(|line| line.starts_with("Exec=")).count(), 1);
    assert!(entry.contains("Exec=/usr/bin/com.example.Notes\n"));
    assert!(entry.contains("Icon=com.example.Notes\n"));
    assert!(entry.contains("StartupWMClass=com.example.Notes\n"));
}

#[test]
fn metadata_escapes_markup_and_uses_the_same_desktop_identity() {
    let doc = json!({"app":{"id":"com.example.Notes", "name":"Notes & <draft> \"quoted\""}});
    let metadata = package::metainfo(&doc);
    assert!(metadata.contains("<name>Notes &amp; &lt;draft&gt; &quot;quoted&quot;</name>"));
    assert!(metadata.contains("<launchable type=\"desktop-id\">com.example.Notes.desktop</launchable>"));
    assert!(!metadata.contains("<draft>"));
}
