//! What a non-Arch host needs that the toolkit's installer leaves to the user: the Ubuntu packages
//! and a swift.org Swift that matches the Xcode the iOS SDK came from. Verified on elementary OS 9
//! (Ubuntu 26.04 "resolute", aarch64) on 2026-10-23, where libimobiledevice 1.4.0 is new enough.

use std::path::Path;

/// The apt line that builds xtool and runs every toolkit script on Ubuntu 26.04.
pub const APT_PACKAGES: &[&str] = &[
    "gnupg2",
    "libcurl4-openssl-dev",
    "libheif-examples",
    "libimobiledevice-dev",
    "libimobiledevice-utils",
    "libz3-dev",
    "llvm",
    "libplist-dev",
    "libusbmuxd-dev",
];

/// Packages a libimobiledevice built from source into [`crate::toolkit::LIMD_PREFIX`] stands in
/// for, with the file in that prefix that shows it is there.
pub fn prefix_provides(package: &str) -> Option<&'static str> {
    match package {
        "libimobiledevice-dev" => Some("lib/pkgconfig/libimobiledevice-1.0.pc"),
        "libimobiledevice-utils" => Some("bin/ideviceinfo"),
        "libplist-dev" => Some("lib/pkgconfig/libplist-2.0.pc"),
        "libusbmuxd-dev" => Some("lib/pkgconfig/libusbmuxd-2.0.pc"),
        _ => None,
    }
}

/// The packages still to install: not installed per dpkg and not covered by the prefix.
pub fn still_missing(not_installed: &[String], prefix: Option<&Path>) -> Vec<String> {
    not_installed
        .iter()
        .filter(|pkg| match (prefix, prefix_provides(pkg)) {
            (Some(dir), Some(file)) => !dir.join(file).exists(),
            _ => true,
        })
        .cloned()
        .collect()
}

pub fn apt_command(packages: &[impl AsRef<str>]) -> String {
    let list: Vec<&str> = packages.iter().map(AsRef::as_ref).collect();
    format!("sudo apt install {}", list.join(" "))
}

/// Swift that ships inside each Xcode the toolkit supports, newest first. The iOS SDK's Swift
/// modules only load in the same compiler version (toolkit FINDINGS.md items 16 and 22).
const XCODE_SWIFT: &[(&str, &str)] = &[("27.1", "6.4.1"), ("27.0", "6.4.0")];

/// The Xcode the newest supported Swift belongs to; used before any SDK is installed.
pub fn default_xcode() -> &'static str {
    XCODE_SWIFT[0].0
}

/// Swift for an Xcode version (`27.1`, `27`, `27.0.1`); `None` for one the table does not know.
pub fn swift_for_xcode(xcode: &str) -> Option<&'static str> {
    let mut parts = xcode.trim().split('.');
    let major = parts.next().unwrap_or("");
    let minor = parts.next().unwrap_or("0");
    let key = format!("{major}.{minor}");
    XCODE_SWIFT.iter().find(|(x, _)| *x == key).map(|(_, s)| *s)
}

/// True when a `swift --version` line is exactly this release (6.4.0 also prints as 6.4).
pub fn swift_matches(version_line: &str, swift: &str) -> bool {
    let short = swift.strip_suffix(".0").unwrap_or(swift);
    [swift, short].iter().any(|v| {
        let needle = format!("Swift version {v}");
        version_line
            .match_indices(&needle)
            .any(|(i, _)| !version_line[i + needle.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '.'))
    })
}

/// The newest Xcode whose SDK xtool has cached, from `~/.cache/xtool` entry names
/// (`darwin-iPhoneOS27.1.xtoolsdk.version.plist`).
pub fn xcode_from_cache<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    names
        .into_iter()
        .filter_map(|n| n.strip_prefix("darwin-iPhoneOS")?.strip_suffix(".xtoolsdk.version.plist"))
        .filter(|v| !v.is_empty() && v.split('.').all(|p| p.parse::<u32>().is_ok()))
        .max_by_key(|v| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>())
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tarball {
    pub swift: String,
    /// `swift-6.4.1-RELEASE-ubuntu26.04-aarch64`: the tarball's top folder too.
    pub name: String,
    pub url: String,
}

/// The swift.org Ubuntu tarball for a Swift release, an Ubuntu version (`26.04`) and a CPU
/// (`aarch64`, `x86_64`).
pub fn swift_tarball(swift: &str, ubuntu: &str, arch: &str) -> Option<Tarball> {
    let compact = ubuntu.replace('.', "");
    if compact.len() != 4 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (platform, suffix) = match arch {
        "aarch64" => (format!("ubuntu{compact}-aarch64"), format!("ubuntu{ubuntu}-aarch64")),
        "x86_64" => (format!("ubuntu{compact}"), format!("ubuntu{ubuntu}")),
        _ => return None,
    };
    let name = format!("swift-{swift}-RELEASE-{suffix}");
    let url =
        format!("https://download.swift.org/swift-{swift}-release/{platform}/swift-{swift}-RELEASE/{name}.tar.gz");
    Some(Tarball { swift: swift.to_string(), name, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xcode_maps_to_its_swift() {
        assert_eq!(swift_for_xcode("27.1"), Some("6.4.1"));
        assert_eq!(swift_for_xcode("27"), Some("6.4.0"));
        assert_eq!(swift_for_xcode("27.0.1"), Some("6.4.0"));
        assert_eq!(swift_for_xcode("26.4"), None);
        assert_eq!(swift_for_xcode(default_xcode()), Some("6.4.1"));
    }

    #[test]
    fn swift_version_lines_match_exactly() {
        assert!(swift_matches("Swift version 6.4.1 (swift-6.4.1-RELEASE)", "6.4.1"));
        assert!(swift_matches("Swift version 6.4 (swift-6.4-RELEASE)", "6.4.0"));
        assert!(swift_matches("Swift version 6.4.0 (swift-6.4.0-RELEASE)", "6.4.0"));
        assert!(!swift_matches("Swift version 6.4.1 (swift-6.4.1-RELEASE)", "6.4.0"));
        assert!(!swift_matches("Swift version 6.4.10", "6.4.1"));
        assert!(!swift_matches("", "6.4.1"));
    }

    #[test]
    fn tarball_urls_follow_swift_org() {
        let t = swift_tarball("6.4.1", "26.04", "aarch64").unwrap();
        assert_eq!(t.name, "swift-6.4.1-RELEASE-ubuntu26.04-aarch64");
        assert_eq!(
            t.url,
            "https://download.swift.org/swift-6.4.1-release/ubuntu2604-aarch64/swift-6.4.1-RELEASE/swift-6.4.1-RELEASE-ubuntu26.04-aarch64.tar.gz"
        );
        let x = swift_tarball("6.4.0", "24.04", "x86_64").unwrap();
        assert!(x.url.ends_with("/ubuntu2404/swift-6.4.0-RELEASE/swift-6.4.0-RELEASE-ubuntu24.04.tar.gz"), "{}", x.url);
        assert!(swift_tarball("6.4.1", "9", "aarch64").is_none());
        assert!(swift_tarball("6.4.1", "26.04", "riscv64").is_none());
    }

    #[test]
    fn newest_cached_sdk_names_the_xcode() {
        let names = [
            "darwin-iPhoneOS27.1.xtoolsdk",
            "darwin-iPhoneOS27.1.xtoolsdk.version.plist",
            "darwin-iPhoneOS27.0.xtoolsdk.version.plist",
            "darwin-iPhoneOS9.9.xtoolsdk.version.plist",
            "pid-1-0",
        ];
        assert_eq!(xcode_from_cache(names).as_deref(), Some("27.1"));
        assert_eq!(xcode_from_cache(["x"]), None);
    }

    #[test]
    fn the_user_prefix_covers_libimobiledevice_packages() {
        let dir = std::env::temp_dir().join(format!("apple-dev-prefix-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib/pkgconfig")).unwrap();
        std::fs::write(dir.join("lib/pkgconfig/libimobiledevice-1.0.pc"), "").unwrap();
        let missing: Vec<String> = ["llvm", "libimobiledevice-dev", "libplist-dev"].map(String::from).into();
        assert_eq!(still_missing(&missing, Some(&dir)), vec!["llvm".to_string(), "libplist-dev".to_string()]);
        assert_eq!(still_missing(&missing, None).len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(apt_command(APT_PACKAGES).starts_with("sudo apt install gnupg2 libcurl4-openssl-dev "));
    }
}
