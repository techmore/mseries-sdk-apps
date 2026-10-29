//! What the kernel says about tunnels: which interface carries internet traffic, which interface owns an
//! address, and per-interface counters. NetworkManager's "activated" is not trusted on its own.
//!
//! `MSERIES_TAILSCALE_FIXTURES=DIR` reads `net.json` (a serialized `Snapshot`) instead.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Iface {
    pub name: String,
    pub index: u32,
    pub up: bool,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub ipv4: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Interface the kernel would use to reach the internet (`ip route get 1.1.1.1`, policy routing included).
    pub internet_iface: Option<String>,
    pub ifaces: Vec<Iface>,
}

impl Snapshot {
    pub fn iface(&self, name: &str) -> Option<&Iface> {
        self.ifaces.iter().find(|i| i.name == name)
    }

    /// The interface that holds `address` (a VPN's own IP, as NetworkManager reports it).
    pub fn iface_with_address(&self, address: &str) -> Option<&Iface> {
        let address = address.split('/').next().unwrap_or(address);
        self.ifaces.iter().find(|i| i.ipv4.iter().any(|a| a == address))
    }

    /// Tunnel-like interfaces (tun, WireGuard, PPP) other than Tailscale's.
    pub fn tunnels(&self) -> impl Iterator<Item = &Iface> {
        self.ifaces.iter().filter(|i| {
            let n = i.name.as_str();
            n != "tailscale0"
                && (n.starts_with("ppp") || n.starts_with("tun") || n.starts_with("utun") || n.starts_with("wg"))
        })
    }
}

fn read_u64(path: &str) -> u64 {
    std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

#[derive(Deserialize)]
struct IpAddr {
    ifname: String,
    #[serde(default)]
    addr_info: Vec<IpAddrInfo>,
}

#[derive(Deserialize)]
struct IpAddrInfo {
    #[serde(default)]
    local: String,
}

#[derive(Deserialize)]
struct IpRoute {
    #[serde(default)]
    dev: String,
}

/// Reads the snapshot. Cheap: two `ip` calls and a few small sysfs files.
pub async fn probe() -> Snapshot {
    if let Some(dir) = std::env::var_os("MSERIES_TAILSCALE_FIXTURES") {
        let path = std::path::Path::new(&dir).join("net.json");
        return std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    }
    let route = crate::cli::run_program(&["ip", "-j", "route", "get", "1.1.1.1"], 5).await;
    let internet_iface = route
        .ok()
        .and_then(|(ok, out, _)| ok.then_some(out))
        .and_then(|out| serde_json::from_str::<Vec<IpRoute>>(&out).ok())
        .and_then(|r| r.into_iter().next())
        .map(|r| r.dev)
        .filter(|d| !d.is_empty());
    let mut addrs: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok((true, out, _)) = crate::cli::run_program(&["ip", "-j", "-4", "addr"], 5).await {
        for entry in serde_json::from_str::<Vec<IpAddr>>(&out).unwrap_or_default() {
            addrs.entry(entry.ifname).or_default().extend(entry.addr_info.into_iter().map(|a| a.local));
        }
    }
    let mut ifaces = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/sys/class/net") {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let base = format!("/sys/class/net/{name}");
            let oper = std::fs::read_to_string(format!("{base}/operstate")).unwrap_or_default();
            ifaces.push(Iface {
                index: read_u64(&format!("{base}/ifindex")) as u32,
                up: oper.trim() != "down",
                rx_bytes: read_u64(&format!("{base}/statistics/rx_bytes")),
                tx_bytes: read_u64(&format!("{base}/statistics/tx_bytes")),
                ipv4: addrs.remove(&name).unwrap_or_default(),
                name,
            });
        }
    }
    Snapshot { internet_iface, ifaces }
}

/// Counters for one interface, read fresh (for the one-second timer).
pub fn counters(name: &str) -> Option<(u64, u64)> {
    if std::env::var_os("MSERIES_TAILSCALE_FIXTURES").is_some() {
        return Some((48_300_000, 6_100_000));
    }
    let base = format!("/sys/class/net/{name}/statistics");
    std::path::Path::new(&base)
        .exists()
        .then(|| (read_u64(&format!("{base}/rx_bytes")), read_u64(&format!("{base}/tx_bytes"))))
}

/// "1.2 GB", "340 KB".
pub fn bytes_text(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.0} {}", UNITS[u])
    }
}

/// "00:04:09", "12:30:00".
pub fn duration_text(secs: i64) -> String {
    let s = secs.max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// A friendly name for a tunnel interface that is not one of the listed VPNs.
pub fn tunnel_owner(iface: &str) -> String {
    match iface {
        "utun" => "WiFiman (Ubiquiti Teleport)".into(),
        n if n.starts_with("ppp") => format!("A PPP/L2TP VPN ({n})"),
        n if n.starts_with("wg") => format!("A WireGuard VPN ({n})"),
        n => format!("Another VPN ({n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        assert_eq!(bytes_text(999), "999 B");
        assert_eq!(bytes_text(4_307_842), "4.3 MB");
        assert_eq!(bytes_text(48_300_000), "48 MB");
        assert_eq!(duration_text(249), "00:04:09");
        assert_eq!(duration_text(45_000), "12:30:00");
        assert_eq!(tunnel_owner("utun"), "WiFiman (Ubiquiti Teleport)");
    }

    #[test]
    fn maps_addresses_to_tunnels() {
        let snap = Snapshot {
            internet_iface: Some("utun".into()),
            ifaces: vec![
                Iface { name: "ppp0".into(), ipv4: vec!["192.168.200.20".into()], up: true, ..Default::default() },
                Iface { name: "utun".into(), ipv4: vec!["192.168.2.3".into()], up: true, ..Default::default() },
                Iface { name: "tailscale0".into(), up: true, ..Default::default() },
                Iface { name: "wld0".into(), up: true, ..Default::default() },
            ],
        };
        assert_eq!(snap.iface_with_address("192.168.200.20/32").map(|i| i.name.as_str()), Some("ppp0"));
        let tunnels: Vec<_> = snap.tunnels().map(|i| i.name.as_str()).collect();
        assert_eq!(tunnels, ["ppp0", "utun"]);
    }
}
