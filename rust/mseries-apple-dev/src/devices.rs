//! Connected iPhones and iPads.
//!
//! USB presence comes from sysfs (Apple vendor 05ac with an iPhone/iPad/iPod product string), so a
//! phone shows up even before any toolchain is installed. When pymobiledevice3 or libimobiledevice
//! is present, names and iOS versions are filled in from `usbmux list` / `ideviceinfo`. `scan`
//! blocks briefly; run it off the UI thread.

use crate::toolkit::run_with_timeout;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The UDID as the toolkit passes it to `--udid` (dashed form for newer phones).
    pub udid: String,
    pub name: String,
    /// `iPhone16,2` style hardware id, or the USB product string.
    pub model: String,
    /// iOS version, empty until a tool reports it.
    pub os: String,
    pub connection: String,
}

impl Device {
    pub fn summary(&self) -> String {
        let mut parts = vec![pretty_model(&self.model)];
        if !self.os.is_empty() {
            parts.push(format!("iOS {}", self.os));
        }
        parts.push(self.connection.clone());
        parts.join(" · ")
    }
}

/// A short readable name for common hardware ids; unknown ids are shown as they are.
pub fn pretty_model(model: &str) -> String {
    let known = [
        ("iPhone14,7", "iPhone 14"),
        ("iPhone14,8", "iPhone 14 Plus"),
        ("iPhone15,2", "iPhone 14 Pro"),
        ("iPhone15,3", "iPhone 14 Pro Max"),
        ("iPhone15,4", "iPhone 15"),
        ("iPhone15,5", "iPhone 15 Plus"),
        ("iPhone16,1", "iPhone 15 Pro"),
        ("iPhone16,2", "iPhone 15 Pro Max"),
        ("iPhone17,1", "iPhone 16 Pro"),
        ("iPhone17,2", "iPhone 16 Pro Max"),
        ("iPhone17,3", "iPhone 16"),
        ("iPhone17,4", "iPhone 16 Plus"),
        ("iPhone17,5", "iPhone 16e"),
        ("iPhone18,1", "iPhone 17 Pro"),
        ("iPhone18,2", "iPhone 17 Pro Max"),
        ("iPhone18,3", "iPhone 17"),
        ("iPhone18,4", "iPhone Air"),
    ];
    known.iter().find(|(id, _)| *id == model).map(|(_, n)| n.to_string()).unwrap_or_else(|| model.to_string())
}

fn compact(udid: &str) -> String {
    udid.chars().filter(|c| *c != '-').collect::<String>().to_ascii_uppercase()
}

/// Apple mobile devices on USB, from a sysfs tree (`/sys/bus/usb/devices`).
pub fn scan_sysfs(root: &Path) -> Vec<Device> {
    let read = |dir: &Path, f: &str| std::fs::read_to_string(dir.join(f)).map(|s| s.trim().to_string()).ok();
    let mut found: Vec<Device> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|dir| read(dir, "idVendor").as_deref() == Some("05ac"))
        .filter_map(|dir| {
            let product = read(&dir, "product")?;
            if !["iPhone", "iPad", "iPod"].iter().any(|p| product.contains(p)) {
                return None;
            }
            let serial = read(&dir, "serial")?;
            // Newer phones report a 24-digit serial; the UDID is the same with a dash after 8.
            let udid = if serial.len() == 24 && !serial.contains('-') {
                format!("{}-{}", &serial[..8], &serial[8..])
            } else {
                serial
            };
            Some(Device { udid, name: product.clone(), model: product, os: String::new(), connection: "USB".into() })
        })
        .collect();
    found.sort_by(|a, b| a.udid.cmp(&b.udid));
    found.dedup_by(|a, b| a.udid == b.udid);
    found
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsbmuxEntry {
    #[serde(default)]
    identifier: String,
    #[serde(default, rename = "UniqueDeviceID")]
    unique_device_id: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    product_type: String,
    #[serde(default)]
    product_version: String,
    #[serde(default)]
    connection_type: String,
}

/// `pymobiledevice3 usbmux list` JSON.
pub fn parse_usbmux_list(json: &str) -> Vec<Device> {
    let entries: Vec<UsbmuxEntry> = serde_json::from_str(json).unwrap_or_default();
    entries
        .into_iter()
        .map(|e| Device {
            udid: if e.identifier.is_empty() { e.unique_device_id } else { e.identifier },
            name: e.device_name,
            model: e.product_type,
            os: e.product_version,
            connection: match e.connection_type.as_str() {
                "USB" | "" => "USB".into(),
                "Network" => "Wi-Fi".into(),
                other => other.into(),
            },
        })
        .filter(|d| !d.udid.is_empty())
        .collect()
}

/// Merge tool-reported details into the sysfs list; devices only a tool sees (Wi-Fi) are added.
pub fn merge(usb: Vec<Device>, reported: Vec<Device>) -> Vec<Device> {
    let mut out = usb;
    for r in reported {
        if let Some(d) = out.iter_mut().find(|d| compact(&d.udid) == compact(&r.udid)) {
            d.udid = r.udid;
            if !r.name.is_empty() {
                d.name = r.name;
            }
            if !r.model.is_empty() {
                d.model = r.model;
            }
            d.os = r.os;
        } else {
            out.push(r);
        }
    }
    out
}

/// Fixture list for screenshots and demos: `MSERIES_APPLE_DEV_FIXTURE_DEVICES=<usbmux list JSON file>`.
fn fixture() -> Option<Vec<Device>> {
    let path = std::env::var_os("MSERIES_APPLE_DEV_FIXTURE_DEVICES")?;
    Some(parse_usbmux_list(&std::fs::read_to_string(path).unwrap_or_default()))
}

pub fn scan() -> Vec<Device> {
    if let Some(devices) = fixture() {
        return devices;
    }
    let usb = scan_sysfs(Path::new("/sys/bus/usb/devices"));
    let home = crate::settings::home();
    let pmd3 = home.join("pymobile3-venv/bin/pymobiledevice3");
    let reported = if pmd3.is_file() {
        run_with_timeout(&pmd3.to_string_lossy(), &["usbmux", "list"], None, Duration::from_secs(6))
            .map(|json| parse_usbmux_list(&json))
            .unwrap_or_default()
    } else {
        usb.iter().filter_map(|d| ideviceinfo(&d.udid)).collect()
    };
    merge(usb, reported)
}

fn ideviceinfo(udid: &str) -> Option<Device> {
    let get = |key: &str| {
        run_with_timeout("ideviceinfo", &["-u", udid, "-k", key], None, Duration::from_secs(3))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let name = get("DeviceName")?;
    Some(Device {
        udid: udid.into(),
        name,
        model: get("ProductType").unwrap_or_default(),
        os: get("ProductVersion").unwrap_or_default(),
        connection: "USB".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_phones_in_sysfs_and_ignores_other_apple_usb() {
        let root = std::env::temp_dir().join(format!("apple-dev-sysfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, vendor, product, serial) in [
            ("1-1", "05ac", "iPhone", "00008110001A2B3C4D5E801E"),
            ("1-2", "05ac", "Apple Internal Keyboard / Trackpad", "X"),
            ("1-3", "046d", "iPhone lookalike", "Y"),
            ("1-4", "05ac", "iPad", "0123456789abcdef0123456789abcdef01234567"),
        ] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("idVendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(d.join("product"), product).unwrap();
            std::fs::write(d.join("serial"), serial).unwrap();
        }
        let found = scan_sysfs(&root);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].udid, "00008110-001A2B3C4D5E801E");
        assert_eq!(found[1].name, "iPad");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn merges_usbmux_details() {
        let usb = vec![Device {
            udid: "00008110-001A2B3C4D5E801E".into(),
            name: "iPhone".into(),
            model: "iPhone".into(),
            os: String::new(),
            connection: "USB".into(),
        }];
        let json = r#"[{"BuildVersion":"23A","ConnectionType":"USB","DeviceClass":"iPhone","DeviceName":"Sean's iPhone",
            "Identifier":"00008110-001A2B3C4D5E801E","ProductType":"iPhone16,2","ProductVersion":"27.0.1",
            "UniqueDeviceID":"00008110-001A2B3C4D5E801E"},
            {"ConnectionType":"Network","DeviceName":"iPad","Identifier":"abc","ProductType":"iPad14,1","ProductVersion":"27.0"}]"#;
        let merged = merge(usb, parse_usbmux_list(json));
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "Sean's iPhone");
        assert_eq!(merged[0].summary(), "iPhone 15 Pro Max · iOS 27.0.1 · USB");
        assert_eq!(merged[1].connection, "Wi-Fi");
        assert!(parse_usbmux_list("not json").is_empty());
    }
}
