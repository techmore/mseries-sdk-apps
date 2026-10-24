//! What kind of app lives in a folder, and what the toolkit last built from it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An xtool project (`xtool.yml`): SwiftUI/UIKit iPhone app.
    Xtool,
    /// A Flutter app (`pubspec.yaml`), built with `flutter/build.sh`.
    Flutter,
    /// A SwiftPM package without `xtool.yml`: a Mac app for `ship-mac.sh`.
    MacPackage,
    /// An Xcode project that still needs `tools/xcodeproj2xtool.py`.
    XcodeProject,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Xtool => "iPhone app",
            Kind::Flutter => "Flutter app",
            Kind::MacPackage => "Mac app",
            Kind::XcodeProject => "Xcode project",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Xtool | Kind::Flutter => "phone-symbolic",
            Kind::MacPackage => "computer-symbolic",
            Kind::XcodeProject => "folder-documents-symbolic",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The folder the toolkit runs in (for an Xcode project: the `.xcodeproj` itself).
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    pub bundle_id: Option<String>,
    pub product: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// A `key: value` line of a flat YAML file (xtool.yml, pubspec.yaml top level).
fn yaml_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('\t'))
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.split(" #").next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string())
        .filter(|v| !v.is_empty())
}

/// `name: "Hello"` from a Package.swift's `Package(` call.
fn package_name(text: &str) -> Option<String> {
    let after = &text[text.find("Package(")?..];
    let after = &after[after.find("name:")? + 5..];
    let start = after.find('"')? + 1;
    let end = start + after[start..].find('"')?;
    Some(after[start..end].to_string()).filter(|s| !s.is_empty())
}

fn dir_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn find_xcodeproj(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xcodeproj") && p.is_dir())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Recognise a folder. Returns `None` when the toolkit has nothing to run there.
pub fn inspect(path: &Path) -> Option<Project> {
    if path.extension().is_some_and(|e| e == "xcodeproj") && path.is_dir() {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        return Some(Project { path: path.into(), name, kind: Kind::XcodeProject, bundle_id: None, product: None });
    }
    let read = |f: &str| std::fs::read_to_string(path.join(f)).ok();
    if let Some(yml) = read("xtool.yml") {
        let product = yaml_value(&yml, "product");
        let name = product
            .clone()
            .or_else(|| read("Package.swift").and_then(|p| package_name(&p)))
            .unwrap_or_else(|| dir_name(path));
        return Some(Project {
            path: path.into(),
            name,
            kind: Kind::Xtool,
            bundle_id: yaml_value(&yml, "bundleID"),
            product,
        });
    }
    if let Some(pubspec) = read("pubspec.yaml") {
        let name = yaml_value(&pubspec, "name").unwrap_or_else(|| dir_name(path));
        return Some(Project { path: path.into(), name, kind: Kind::Flutter, bundle_id: None, product: None });
    }
    if let Some(package) = read("Package.swift") {
        let name = package_name(&package).unwrap_or_else(|| dir_name(path));
        return Some(Project { path: path.into(), name, kind: Kind::MacPackage, bundle_id: None, product: None });
    }
    find_xcodeproj(path).and_then(|p| inspect(&p))
}

/// Where `tools/xcodeproj2xtool.py` writes its adapter for an Xcode project.
pub fn xtool_adapter_dir(xcodeproj: &Path) -> PathBuf {
    xcodeproj.parent().unwrap_or(Path::new("/")).join("omarchy-xtool")
}

/// The newest build output the toolkit leaves in a project, if any.
pub fn latest_artifact(project: &Project) -> Option<Artifact> {
    let dirs: &[&str] = match project.kind {
        Kind::Xtool => &["xtool"],
        Kind::Flutter => &["build/ios-linux"],
        Kind::MacPackage => &["build"],
        Kind::XcodeProject => &[],
    };
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(project.path.join(d)).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "app" || x == "ipa" || x == "zip"))
        .filter_map(|e| Some(Artifact { modified: e.metadata().ok()?.modified().ok()?, path: e.path() }))
        .max_by_key(|a| a.modified)
}

/// The newest signed `.ipa` in an xtool project (for an install over Tailscale).
pub fn latest_ipa(project: &Project) -> Option<PathBuf> {
    latest_artifact(project).map(|a| a.path).filter(|p| p.extension().is_some_and(|e| e == "ipa"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apple-dev-project-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_an_xtool_project() {
        let dir = scratch("xtool");
        std::fs::write(
            dir.join("xtool.yml"),
            "version: 1\nbundleID: net.omarchy.fixture # comment\nproduct: FixtureApp\n",
        )
        .unwrap();
        let p = inspect(&dir).unwrap();
        assert_eq!(p.kind, Kind::Xtool);
        assert_eq!(p.name, "FixtureApp");
        assert_eq!(p.bundle_id.as_deref(), Some("net.omarchy.fixture"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn xtool_name_falls_back_to_package_swift() {
        let dir = scratch("xtool-pkg");
        std::fs::write(dir.join("xtool.yml"), "version: 1\nbundleID: com.example.Hello\n").unwrap();
        std::fs::write(dir.join("Package.swift"), "let package = Package(\n    name: \"HelloOmarchy\",\n)").unwrap();
        assert_eq!(inspect(&dir).unwrap().name, "HelloOmarchy");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recognises_flutter_mac_and_xcode() {
        let flutter = scratch("flutter");
        std::fs::write(flutter.join("pubspec.yaml"), "name: counter\nenvironment:\n  name: nested\n").unwrap();
        let f = inspect(&flutter).unwrap();
        assert_eq!((f.kind, f.name.as_str()), (Kind::Flutter, "counter"));

        let mac = scratch("mac");
        std::fs::write(mac.join("Package.swift"), "Package(name: \"Notes\")").unwrap();
        assert_eq!(inspect(&mac).unwrap().kind, Kind::MacPackage);

        let xc = scratch("xcode");
        std::fs::create_dir_all(xc.join("Demo.xcodeproj")).unwrap();
        let x = inspect(&xc).unwrap();
        assert_eq!((x.kind, x.name.as_str()), (Kind::XcodeProject, "Demo"));
        assert_eq!(xtool_adapter_dir(&x.path), xc.join("omarchy-xtool"));

        let empty = scratch("empty");
        assert!(inspect(&empty).is_none());
        for d in [flutter, mac, xc, empty] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn finds_the_newest_artifact() {
        let dir = scratch("artifact");
        std::fs::write(dir.join("xtool.yml"), "bundleID: a.b\n").unwrap();
        let p = inspect(&dir).unwrap();
        assert!(latest_artifact(&p).is_none());
        std::fs::create_dir_all(dir.join("xtool/Hello.app")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("xtool/Hello.ipa"), b"x").unwrap();
        assert_eq!(latest_ipa(&p), Some(dir.join("xtool/Hello.ipa")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
