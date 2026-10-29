//! `tailscale status --json` and `tailscale debug prefs`, reduced to what the window shows.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Running,
    Starting,
    Stopped,
    NeedsLogin,
    NeedsMachineAuth,
    Other(String),
}

impl Backend {
    fn from_str(s: &str) -> Self {
        match s {
            "Running" => Self::Running,
            "Starting" => Self::Starting,
            "Stopped" => Self::Stopped,
            "NeedsLogin" | "NoState" => Self::NeedsLogin,
            "NeedsMachineAuth" => Self::NeedsMachineAuth,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// First MagicDNS label, what `ssh` takes ("sdolbec-ser8").
    pub ssh_host: String,
    pub ipv4: String,
    pub ipv6: String,
    pub os: String,
    pub online: bool,
    /// RFC 3339; `None` when Tailscale has no time (online devices report year 1).
    pub last_seen: Option<String>,
    pub exit_node: bool,
    pub exit_node_option: bool,
}

impl Device {
    /// The address people type: IPv4 when there is one.
    pub fn ip(&self) -> &str {
        if self.ipv4.is_empty() { &self.ipv6 } else { &self.ipv4 }
    }

    pub fn ssh_command(&self) -> String {
        format!("ssh {}", if self.ssh_host.is_empty() { self.ip() } else { &self.ssh_host })
    }

    pub fn os_name(&self) -> &str {
        match self.os.as_str() {
            "linux" => "Linux",
            "windows" => "Windows",
            "android" => "Android",
            "" => "Unknown",
            other => other, // macOS, iOS, tvOS already read well
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub backend: Backend,
    pub auth_url: String,
    pub tailnet: String,
    pub account: String,
    pub version: String,
    pub this_device: Option<Device>,
    /// Online first, then by name.
    pub peers: Vec<Device>,
    pub health: Vec<String>,
}

impl Status {
    pub fn exit_node_options(&self) -> Vec<&Device> {
        self.peers.iter().filter(|p| p.exit_node_option).collect()
    }

    pub fn current_exit_node(&self) -> Option<&Device> {
        self.peers.iter().find(|p| p.exit_node)
    }

    pub fn online_count(&self) -> usize {
        self.peers.iter().filter(|p| p.online).count()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prefs {
    pub allow_lan_access: bool,
    pub accept_dns: bool,
    pub accept_routes: bool,
    pub shields_up: bool,
    pub run_ssh: bool,
    pub operator: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawStatus {
    #[serde(default)]
    backend_state: String,
    #[serde(default, rename = "AuthURL")]
    auth_url: String,
    #[serde(default)]
    version: String,
    #[serde(rename = "Self")]
    this: Option<RawPeer>,
    #[serde(default)]
    peer: Option<HashMap<String, RawPeer>>,
    #[serde(default)]
    current_tailnet: Option<RawTailnet>,
    #[serde(default)]
    health: Option<Vec<String>>,
    #[serde(default)]
    user: Option<HashMap<String, RawUser>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawTailnet {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawUser {
    #[serde(default)]
    login_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPeer {
    #[serde(default, rename = "ID")]
    id: String,
    #[serde(default)]
    host_name: String,
    #[serde(default, rename = "DNSName")]
    dns_name: String,
    #[serde(default, rename = "OS")]
    os: String,
    #[serde(default, rename = "TailscaleIPs")]
    tailscale_ips: Option<Vec<String>>,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    last_seen: String,
    #[serde(default)]
    exit_node: bool,
    #[serde(default)]
    exit_node_option: bool,
    #[serde(default, rename = "UserID")]
    user_id: i64,
}

impl RawPeer {
    fn into_device(self) -> Device {
        let ips = self.tailscale_ips.unwrap_or_default();
        let ipv4 = ips.iter().find(|ip| !ip.contains(':')).cloned().unwrap_or_default();
        let ipv6 = ips.iter().find(|ip| ip.contains(':')).cloned().unwrap_or_default();
        let ssh_host = self.dns_name.split('.').next().unwrap_or_default().to_string();
        // Phones often call themselves "localhost"; the MagicDNS name is more useful.
        let name =
            if self.host_name.is_empty() || self.host_name == "localhost" { ssh_host.clone() } else { self.host_name };
        let last_seen = (!self.last_seen.is_empty() && !self.last_seen.starts_with("0001-")).then_some(self.last_seen);
        Device {
            id: self.id,
            name,
            ssh_host,
            ipv4,
            ipv6,
            os: self.os,
            online: self.online,
            last_seen,
            exit_node: self.exit_node,
            exit_node_option: self.exit_node_option,
        }
    }
}

pub fn parse_status(json: &str) -> Result<Status, serde_json::Error> {
    let raw: RawStatus = serde_json::from_str(json)?;
    let account = raw
        .this
        .as_ref()
        .and_then(|me| raw.user.as_ref()?.get(&me.user_id.to_string()))
        .map(|u| u.login_name.clone())
        .unwrap_or_default();
    let mut peers: Vec<Device> =
        raw.peer.unwrap_or_default().into_values().map(RawPeer::into_device).filter(|d| !d.ip().is_empty()).collect();
    peers.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(Status {
        backend: Backend::from_str(&raw.backend_state),
        auth_url: raw.auth_url,
        tailnet: raw.current_tailnet.map(|t| t.name).unwrap_or_default(),
        account,
        version: raw.version.split('-').next().unwrap_or_default().to_string(),
        this_device: raw.this.map(RawPeer::into_device),
        peers,
        health: raw.health.unwrap_or_default(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPrefs {
    #[serde(default)]
    exit_node_allow_lan_access: bool,
    #[serde(default, rename = "CorpDNS")]
    corp_dns: bool,
    #[serde(default)]
    route_all: bool,
    #[serde(default)]
    shields_up: bool,
    #[serde(default, rename = "RunSSH")]
    run_ssh: bool,
    #[serde(default)]
    operator_user: Option<String>,
}

pub fn parse_prefs(json: &str) -> Result<Prefs, serde_json::Error> {
    let raw: RawPrefs = serde_json::from_str(json)?;
    Ok(Prefs {
        allow_lan_access: raw.exit_node_allow_lan_access,
        accept_dns: raw.corp_dns,
        accept_routes: raw.route_all,
        shields_up: raw.shields_up,
        run_ssh: raw.run_ssh,
        operator: raw.operator_user.unwrap_or_default(),
    })
}

/// "Online", "Last seen 5 minutes ago", "Last seen 3 days ago"; `now` and `seen` in Unix seconds.
pub fn presence_text(online: bool, seen_unix: Option<i64>, now_unix: i64) -> String {
    if online {
        return "Online".into();
    }
    let Some(seen) = seen_unix else { return "Offline".into() };
    let secs = (now_unix - seen).max(0);
    let (n, unit) = match secs {
        s if s < 60 => return "Last seen just now".into(),
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 86_400 * 60 => (s / 86_400, "day"),
        s if s < 86_400 * 730 => (s / (86_400 * 30), "month"),
        s => (s / (86_400 * 365), "year"),
    };
    format!("Last seen {n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = include_str!("../fixtures/status.json");

    #[test]
    fn parses_status_and_orders_devices() {
        let s = parse_status(STATUS).unwrap();
        assert_eq!(s.backend, Backend::Running);
        assert_eq!(s.tailnet, "example@gmail.com");
        assert_eq!(s.account, "example@gmail.com");
        assert_eq!(s.version, "1.104.4");
        let me = s.this_device.as_ref().unwrap();
        assert_eq!((me.name.as_str(), me.ip()), ("eos", "100.115.140.33"));
        assert!(s.peers.iter().take(s.online_count()).all(|p| p.online));
        let phone = s.peers.iter().find(|p| p.ssh_host == "iphone-14-pro").unwrap();
        assert_eq!(phone.name, "iphone-14-pro");
        assert_eq!(phone.ip(), "100.96.99.44");
        assert_eq!(phone.last_seen, None);
        let ser8 = s.peers.iter().find(|p| p.name == "sdolbec-SER8").unwrap();
        assert_eq!(ser8.ssh_command(), "ssh sdolbec-ser8");
        assert_eq!(s.exit_node_options().len(), 1);
        assert_eq!(s.current_exit_node().map(|d| d.name.as_str()), Some("sdolbec-SER8"));
    }

    #[test]
    fn parses_stopped_and_logged_out() {
        let s = parse_status(r#"{"BackendState":"Stopped","Self":null,"Peer":null}"#).unwrap();
        assert_eq!(s.backend, Backend::Stopped);
        assert!(s.peers.is_empty());
        let s = parse_status(r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/x"}"#).unwrap();
        assert_eq!(s.backend, Backend::NeedsLogin);
        assert_eq!(s.auth_url, "https://login.tailscale.com/a/x");
    }

    #[test]
    fn parses_prefs() {
        let p = parse_prefs(include_str!("../fixtures/prefs.json")).unwrap();
        assert!(p.accept_dns && !p.accept_routes && !p.shields_up && !p.run_ssh);
        assert_eq!(p.operator, "");
    }

    #[test]
    fn presence() {
        assert_eq!(presence_text(true, None, 0), "Online");
        assert_eq!(presence_text(false, None, 0), "Offline");
        assert_eq!(presence_text(false, Some(0), 30), "Last seen just now");
        assert_eq!(presence_text(false, Some(0), 3600), "Last seen 1 hour ago");
        assert_eq!(presence_text(false, Some(0), 86_400 * 3), "Last seen 3 days ago");
        assert_eq!(presence_text(false, Some(0), 86_400 * 90), "Last seen 3 months ago");
    }
}

/// A NetworkManager VPN connection (`nmcli`), with macOS's "Send all traffic over VPN" setting
/// (NetworkManager's `never-default` set to no).
#[derive(Debug, Clone, PartialEq)]
pub struct Vpn {
    pub name: String,
    pub uuid: String,
    pub active: bool,
    pub all_traffic: bool,
    /// False when a secret is "always ask" (flag 2) and so is never stored.
    pub secrets_saved: bool,
    /// "vpn" (NetworkManager VPN plugin) or "wireguard".
    pub kind: String,
}

impl Vpn {
    /// Only plugin VPNs get the "Send all traffic" switch; WireGuard routes by its allowed IPs.
    pub fn kind_has_routing(&self) -> bool {
        self.kind == "vpn"
    }
}

/// A friendlier name for connections other apps create.
pub fn display_name(name: &str) -> String {
    match name {
        "utun" => "WiFiman (utun)".into(),
        other => other.into(),
    }
}

/// `nmcli -t -f UUID,STATE,DEVICE,TYPE connection show --active` → uuid → (state, device, type).
pub fn parse_active(text: &str) -> std::collections::HashMap<String, (String, String, String)> {
    text.lines()
        .map(nmcli_fields)
        .filter(|f| f.len() >= 4)
        .map(|f| (f[0].clone(), (f[1].clone(), f[2].clone(), f[3].clone())))
        .collect()
}

/// Why a VPN dropped, from recent NetworkManager / pppd / IPsec log lines, in plain words.
/// `others` are tunnels from other apps that are up now.
pub fn classify_vpn_failure(log: &str, others: &[String]) -> String {
    if let Some(other) = others.first() {
        return format!("{other} took over the connection");
    }
    let l = log.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&["authentication failed", "auth failed"]) {
        "the server rejected the username or password".into()
    } else if has(&["no agents were available", "secrets were required"]) {
        "passwords aren't saved".into()
    } else if has(&["modem hangup", "hung up", "lcp terminated", "peer closed"]) {
        "the server hung up".into()
    } else if has(&["timed out", "timeout"]) {
        "the server didn't answer".into()
    } else if has(&["ipsec"]) && has(&["shut", "terminat", "deleting ike", "closing"]) {
        "the IPsec tunnel closed".into()
    } else {
        "the connection stopped".into()
    }
}

/// Reads the `*-flags` entries from `nmcli -g vpn.data` (flags only; secrets are not in vpn.data).
/// Flag 2 means "not saved, ask every time".
pub fn secrets_saved(vpn_data: &str) -> bool {
    !vpn_data
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .any(|(k, v)| k.trim().ends_with("-flags") && v.trim().parse::<u32>().is_ok_and(|f| f & 2 != 0))
}

/// Whether an `nmcli connection up` failure means the VPN's passwords are missing.
pub fn needs_secrets(stderr: &str) -> bool {
    let l = stderr.to_lowercase();
    l.contains("secrets were required") || l.contains("no agents were available") || l.contains("no secrets")
}

/// Splits one `nmcli -t` line on unescaped colons and unescapes `\:` and `\\`.
pub fn nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// VPNs from `nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show` (WireGuard counts; Tailscale's own does not).
pub fn parse_vpn_list(text: &str) -> Vec<Vpn> {
    text.lines()
        .map(nmcli_fields)
        .filter(|f| f.len() >= 4 && (f[2] == "vpn" || f[2] == "wireguard") && f[0] != "tailscale0")
        .map(|f| Vpn {
            name: f[0].clone(),
            uuid: f[1].clone(),
            active: f[3] == "yes",
            all_traffic: false,
            secrets_saved: true,
            kind: f[2].clone(),
        })
        .collect()
}

/// `nmcli -g vpn.data` ("key = value, key = value") as a map. Values with ", " inside are not expected here.
pub fn parse_vpn_data(text: &str) -> std::collections::HashMap<String, String> {
    text.trim()
        .split(", ")
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[cfg(test)]
mod vpn_tests {
    use super::*;

    #[test]
    fn parses_nmcli() {
        let text = "Home Wi\\:Fi:aaa:802-11-wireless:yes\nBFS:4a1d:vpn:no\nWork\\\\VPN:bbb:wireguard:yes\nlo:ccc:loopback:yes\n";
        let vpns = parse_vpn_list(text);
        assert_eq!(vpns.len(), 2);
        assert_eq!((vpns[0].name.as_str(), vpns[0].uuid.as_str(), vpns[0].active), ("BFS", "4a1d", false));
        assert_eq!((vpns[1].name.as_str(), vpns[1].active), ("Work\\VPN", true));
        assert_eq!(nmcli_fields("a\\:b:c"), vec!["a:b", "c"]);
        assert!(!secrets_saved("gateway = x, ipsec-psk-flags = 2, password-flags = 2, user = a"));
        assert!(!secrets_saved("password-flags = 0, ipsec-psk-flags = 2"));
        assert!(secrets_saved("password-flags = 0, ipsec-psk-flags = 0, user = a"));
        assert!(secrets_saved(""));
        let d = parse_vpn_data("gateway = vpn.example.com, ipsec-enabled = yes, password-flags = 0, user = a@b.org\n");
        assert_eq!(d.get("gateway").map(String::as_str), Some("vpn.example.com"));
        assert_eq!(d.get("user").map(String::as_str), Some("a@b.org"));
        let a = parse_active("4a1d:activated:enx60:vpn\nutun-1:activated:utun:wireguard\n");
        assert_eq!(a.get("4a1d").map(|t| t.0.as_str()), Some("activated"));
        assert_eq!(a.get("utun-1").map(|t| t.1.as_str()), Some("utun"));
        assert_eq!(display_name("utun"), "WiFiman (utun)");
        assert_eq!(
            classify_vpn_failure("", &["WiFiman (Ubiquiti Teleport)".into()]),
            "WiFiman (Ubiquiti Teleport) took over the connection"
        );
        assert_eq!(classify_vpn_failure("pppd: Modem hangup", &[]), "the server hung up");
        assert_eq!(
            classify_vpn_failure("PAP authentication failed", &[]),
            "the server rejected the username or password"
        );
        assert!(needs_secrets("Error: Connection activation failed: Secrets were required, but not provided"));
    }
}
