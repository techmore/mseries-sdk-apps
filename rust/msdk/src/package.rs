//! Linux Debian packaging of an explicitly supplied, already-built SDK executable.
//! Does not build, install, launch, access credentials or execute app services.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

pub fn architecture(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" || bytes[4] != 2 || bytes[5] != 1 {
        return Err("pack: expected a 64-bit little-endian Linux ELF executable".into());
    }
    if !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3) {
        return Err("pack: ELF input is not an executable or PIE".into());
    }
    match u16::from_le_bytes([bytes[18], bytes[19]]) {
        62 => Ok("amd64"),
        183 => Ok("arm64"),
        _ => Err("pack: supported architectures are amd64 and arm64".into()),
    }
}

fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn desktop(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// Metadata is generated independently of external packaging tools for testing.
pub fn desktop_entry(doc: &Value) -> String {
    let id = doc["app"]["id"].as_str().unwrap_or("");
    let name = doc["app"]["name"].as_str().unwrap_or("");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=/usr/bin/{id}\nTryExec=/usr/bin/{id}\nIcon={id}\nTerminal=false\nStartupNotify=true\nStartupWMClass={id}\nCategories=Utility;\n",
        desktop(name)
    )
}

pub fn metainfo(doc: &Value) -> String {
    let id = xml(doc["app"]["id"].as_str().unwrap_or(""));
    let name = xml(doc["app"]["name"].as_str().unwrap_or(""));
    let developer = xml(doc["app"]["id"]
        .as_str()
        .unwrap_or("")
        .rsplit_once('.')
        .map(|(owner, _)| owner)
        .unwrap_or("org.example")
        .to_ascii_lowercase()
        .replace('_', "-")
        .as_str());
    let homepage = xml(doc["app"]["website"].as_str().unwrap_or("https://github.com/techmore/ElementaryOS_MSeries"));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"desktop-application\">\n  <id>{id}</id>\n  <metadata_license>CC0-1.0</metadata_license>\n  <project_license>GPL-3.0-only</project_license>\n  <name>{name}</name>\n  <summary>Native Pantheon application</summary>\n  <description><p>An application created with the M-series app SDK. It follows its declared interface and rules and integrates with native Pantheon desktop controls.</p></description>\n  <launchable type=\"desktop-id\">{id}.desktop</launchable>\n  <provides><binary>{id}</binary></provides>\n  <url type=\"homepage\">{homepage}</url>\n  <developer id=\"{developer}\"><name>Application developer</name></developer>\n  <content_rating type=\"oars-1.1\" />\n</component>\n"
    )
}

pub const DEFAULT_ICON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\" viewBox=\"0 0 128 128\"><rect x=\"12\" y=\"16\" width=\"104\" height=\"96\" rx=\"16\" fill=\"#3689e6\"/><rect x=\"24\" y=\"40\" width=\"80\" height=\"60\" rx=\"6\" fill=\"#fff\"/><circle cx=\"28\" cy=\"28\" r=\"4\" fill=\"#fff\"/><path d=\"M38 60h52M38 76h36\" stroke=\"#3689e6\" stroke-width=\"8\" stroke-linecap=\"round\"/></svg>\n";

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn command(command: &mut Command, operation: &str) -> Result<String, String> {
    let output = command.output().map_err(|e| format!("pack: {operation}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "pack: {operation}: {} {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Create a Debian package. Dependencies come from dpkg-shlibdeps on these exact
/// binary bytes, not a guessed list. Existing output is never overwritten.
pub fn pack(project: &Path, binary: &Path, output: &Path, version: &str) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("pack: run on Linux with dpkg-dev installed".into());
    }
    let doc = crate::text::load(&project.join("app.ui.json"))?.0;
    for entry in std::fs::read_dir(project).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.to_string_lossy().ends_with(".services.json") {
            let manifest = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            crate::vocab::load_manifest(&serde_json::from_str(&manifest).map_err(|e| e.to_string())?)?;
        }
    }
    let errors: Vec<_> = crate::check(&doc)
        .into_iter()
        .filter(|d| d.severity == crate::Severity::Error)
        .map(|d| d.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    command(Command::new("dpkg").args(["--validate-version", version]), "version validation")?;
    let bytes = std::fs::read(binary).map_err(|e| format!("pack: executable: {e}"))?;
    let arch = architecture(&bytes)?;
    let host_arch = command(Command::new("dpkg").arg("--print-architecture"), "architecture detection")?;
    if host_arch.trim() != arch {
        return Err("pack: dependency resolution requires the binary's matching native architecture".into());
    }
    let id = doc["app"]["id"].as_str().ok_or("pack: missing app id")?;
    let name = format!("msdk-{}", id.replace('_', "-").to_ascii_lowercase());
    let output = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(output)
    };
    if output.exists() {
        return Err("pack: output already exists; choose a new package filename".into());
    }
    let parent = output.parent().ok_or("pack: output has no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let stage = Staging(parent.join(format!(".msdk-pack-{}-{stamp}", std::process::id())));
    std::fs::create_dir(&stage.0).map_err(|e| e.to_string())?;
    let root = stage.0.join("root");
    let paths =
        ["DEBIAN", "usr/bin", "usr/share/applications", "usr/share/metainfo", "usr/share/icons/hicolor/scalable/apps"];
    for path in paths {
        std::fs::create_dir_all(root.join(path)).map_err(|e| e.to_string())?;
    }
    let docs = root.join("usr/share/doc").join(&name);
    std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    let license = std::fs::read_to_string(project.join("LICENSE")).map_err(|e| format!("pack: LICENSE: {e}"))?;
    std::fs::write(docs.join("copyright"), format!("SDK source and attribution: https://github.com/techmore/ElementaryOS_MSeries\nLicense: GPL-3.0-only\n\n{license}")).map_err(|e| e.to_string())?;
    let installed_binary = root.join("usr/bin").join(id);
    std::fs::write(&installed_binary, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let desktop_path = root.join("usr/share/applications").join(format!("{id}.desktop"));
    let metainfo_path = root.join("usr/share/metainfo").join(format!("{id}.metainfo.xml"));
    std::fs::write(&desktop_path, desktop_entry(&doc)).map_err(|e| e.to_string())?;
    std::fs::write(&metainfo_path, metainfo(&doc)).map_err(|e| e.to_string())?;
    let icon = project.join("assets/icon.svg");
    let icon = std::fs::read_to_string(icon).map_err(|e| format!("pack: assets/icon.svg: {e}"))?;
    std::fs::write(root.join("usr/share/icons/hicolor/scalable/apps").join(format!("{id}.svg")), icon)
        .map_err(|e| e.to_string())?;
    command(Command::new("desktop-file-validate").arg(&desktop_path), "desktop validation")?;
    command(Command::new("appstreamcli").args(["validate", "--no-net"]).arg(&metainfo_path), "AppStream validation")?;
    let control = format!(
        "Package: {name}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: SDK app developer <developer@example.invalid>\nDescription: Application built with the M-series SDK\n"
    );
    let debian = stage.0.join("debian");
    std::fs::create_dir(&debian).map_err(|e| e.to_string())?;
    std::fs::write(debian.join("control"), format!("Source: {name}\nSection: utils\nPriority: optional\nMaintainer: SDK app developer <developer@example.invalid>\n\nPackage: {name}\nArchitecture: any\nDescription: Application built with the M-series SDK\n")).map_err(|e| e.to_string())?;
    let depends = command(
        Command::new("dpkg-shlibdeps").current_dir(&stage.0).arg("-O").arg("-e").arg(&installed_binary),
        "native library dependency resolution",
    )?;
    let depends = depends
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends="))
        .ok_or("pack: no native dependency result")?;
    std::fs::write(root.join("DEBIAN/control"), format!("{control}Depends: {depends}\n")).map_err(|e| e.to_string())?;
    let archive = stage.0.join("application.deb");
    command(
        Command::new("dpkg-deb").args(["--root-owner-group", "--build"]).arg(&root).arg(&archive),
        "Debian archive assembly",
    )?;
    // Exclusive publication prevents a race from replacing an existing output.
    std::fs::hard_link(archive, output).map_err(|e| format!("pack: publish output: {e}"))?;
    Ok(())
}
