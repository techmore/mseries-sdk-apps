//! Environments (INTEGRATIONS.md 4d): the servers this client can reach, each with its own route and
//! credential, switched from the sidebar footer. Direct (a URL), Tailscale (a peer from
//! `tailscale status --json`, the server listening on its tailnet address), SSH (a local port forward
//! `ssh -N -L` kept as a child process), and T3 Connect (a pairing URL minted on the host; the relay
//! sign-in itself happens in the browser). Saved in `~/.config/claude-pantheon-rs/environments.json`;
//! credentials are stored per server URL.

use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Route {
    /// `http://host:port`
    Direct { url: String },
    /// A tailnet peer; the server must listen on it (`t3 serve --host <ip>` or `t3 pair --tailscale`).
    Tailscale { host: String, port: u16 },
    /// `ssh -N -L <local>:127.0.0.1:<remote> <target>`; the client talks to the local port.
    Ssh { target: String, remote_port: u16, local_port: u16 },
    /// A relay (app.t3.codes) environment, reached through the URL its pairing link names.
    T3Connect { url: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub route: Route,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Environments {
    #[serde(default)]
    pub list: Vec<Environment>,
    #[serde(default)]
    pub current: Option<String>,
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs").join("environments.json")
}

impl Environments {
    pub fn load() -> Environments {
        let mut e: Environments =
            std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if e.list.is_empty() {
            e.list.push(Environment {
                id: "local".into(),
                name: "This computer".into(),
                route: Route::Direct { url: crate::app::DEFAULT_SERVER.into() },
            });
            e.current = Some("local".into());
        }
        e
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(p, bytes);
        }
    }

    pub fn current(&self) -> Option<&Environment> {
        let id = self.current.as_deref()?;
        self.list.iter().find(|e| e.id == id)
    }

    pub fn upsert(&mut self, env: Environment) {
        match self.list.iter_mut().find(|e| e.id == env.id) {
            Some(slot) => *slot = env,
            None => self.list.push(env),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.list.retain(|e| e.id != id);
        if self.current.as_deref() == Some(id) {
            self.current = self.list.first().map(|e| e.id.clone());
        }
    }
}

impl Route {
    /// The HTTP origin the client connects to once the route is up.
    pub fn url(&self) -> String {
        match self {
            Route::Direct { url } | Route::T3Connect { url } => url.trim_end_matches('/').to_string(),
            Route::Tailscale { host, port } => format!("http://{host}:{port}"),
            Route::Ssh { local_port, .. } => format!("http://127.0.0.1:{local_port}"),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Route::Direct { .. } => "direct",
            Route::Tailscale { .. } => "Tailscale",
            Route::Ssh { .. } => "SSH",
            Route::T3Connect { .. } => "T3 Connect",
        }
    }
}

/// A Tailscale peer as `tailscale status --json` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub host: String,
    pub ip: String,
    pub online: bool,
}

pub fn parse_tailscale_status(v: &Value) -> Vec<Peer> {
    let mut out = Vec::new();
    if let Some(peers) = v.get("Peer").and_then(Value::as_object) {
        for p in peers.values() {
            let host = p.get("HostName").and_then(Value::as_str).unwrap_or("").to_string();
            let ip = p
                .get("TailscaleIPs")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !host.is_empty() && !ip.is_empty() {
                out.push(Peer { host, ip, online: p.get("Online").and_then(Value::as_bool).unwrap_or(false) });
            }
        }
    }
    out.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.host.cmp(&b.host)));
    out
}

/// `tailscale status --json`, when the CLI is present.
pub async fn tailscale_peers() -> Vec<Peer> {
    let launcher =
        gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE);
    let argv: Vec<&std::ffi::OsStr> =
        ["timeout", "5", "tailscale", "status", "--json"].iter().map(std::ffi::OsStr::new).collect();
    let Ok(proc) = launcher.spawn(&argv) else { return Vec::new() };
    let Ok((out, _)) = proc.communicate_utf8_future(None).await else { return Vec::new() };
    let text = out.map(|s| s.to_string()).unwrap_or_default();
    serde_json::from_str::<Value>(&text).map(|v| parse_tailscale_status(&v)).unwrap_or_default()
}

/// An SSH port forward kept alive while the environment is current.
pub struct Tunnel {
    proc: gio::Subprocess,
    pub local_port: u16,
}

impl Tunnel {
    pub fn open(target: &str, remote_port: u16, local_port: u16) -> Result<Tunnel, String> {
        let launcher = gio::SubprocessLauncher::new(
            gio::SubprocessFlags::STDIN_PIPE | gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE,
        );
        let forward = format!("{local_port}:127.0.0.1:{remote_port}");
        let argv = [
            "ssh",
            "-N",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ServerAliveInterval=30",
            "-L",
            &forward,
            target,
        ];
        let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        let proc = launcher.spawn(&argv).map_err(|e| e.to_string())?;
        Ok(Tunnel { proc, local_port })
    }

    /// Waits until the local port answers (up to ~8 s) so the first connect does not race the tunnel.
    pub async fn wait_ready(&self) -> bool {
        for _ in 0..40 {
            if !self.is_alive() {
                return false;
            }
            let client = gio::SocketClient::new();
            client.set_timeout(1);
            if client.connect_to_host_future(&format!("127.0.0.1:{}", self.local_port), self.local_port).await.is_ok() {
                return true;
            }
            glib::timeout_future(std::time::Duration::from_millis(200)).await;
        }
        false
    }

    pub fn is_alive(&self) -> bool {
        !self.proc.has_exited()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.proc.force_exit();
    }
}

/// Live state of every saved environment, for the switcher's dots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Connected,
    Connecting,
    Offline,
    Idle,
}

pub type Shared = Rc<RefCell<Environments>>;

// ---- the add / edit dialog ----------------------------------------------------------------------

/// Called with the saved environment.
pub type OnSave = Rc<dyn Fn(Environment)>;

/// A small modal window: name, kind, the kind's fields (Tailscale lists the peers from the CLI).
pub fn open_dialog(parent: &impl IsA<gtk::Window>, existing: Option<Environment>, on_save: OnSave) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(if existing.is_some() { "Edit Environment" } else { "Add Environment" })
        .default_width(460)
        .resizable(false)
        .build();
    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(if existing.is_some() { "Save" } else { "Add" });
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);
    window.set_titlebar(Some(&header));

    let grid = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let name = gtk::Entry::builder().hexpand(true).placeholder_text("Name").build();
    let kinds =
        gtk::StringList::new(&["Direct (URL)", "Tailscale peer", "SSH port forward", "T3 Connect (pairing URL)"]);
    let kind = gtk::DropDown::new(Some(kinds), gtk::Expression::NONE);
    let url = gtk::Entry::builder().hexpand(true).placeholder_text("http://host:3773").build();
    let peers = gtk::StringList::new(&["(reading `tailscale status`…)"]);
    let peer = gtk::DropDown::new(Some(peers.clone()), gtk::Expression::NONE);
    peer.set_hexpand(true);
    let ts_port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
    ts_port.set_value(3773.0);
    let ssh_target =
        gtk::Entry::builder().hexpand(true).placeholder_text("user@host (an ssh config alias works)").build();
    let ssh_remote = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
    ssh_remote.set_value(3773.0);
    let ssh_local = gtk::SpinButton::with_range(1024.0, 65535.0, 1.0);
    ssh_local.set_value(3791.0);
    let connect_url = gtk::Entry::builder()
        .hexpand(true)
        .placeholder_text("https://…/pair#token=… from `t3 pair` on the host")
        .build();
    let hint = gtk::Label::new(None);
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");

    let fields = gtk::Stack::new();
    let direct = gtk::Box::new(gtk::Orientation::Vertical, 6);
    direct.append(&url);
    let tailscale = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tailscale.append(&peer);
    let port_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    port_row.append(&gtk::Label::new(Some("Port")));
    port_row.append(&ts_port);
    tailscale.append(&port_row);
    let ssh = gtk::Box::new(gtk::Orientation::Vertical, 6);
    ssh.append(&ssh_target);
    let ssh_ports = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ssh_ports.append(&gtk::Label::new(Some("Remote port")));
    ssh_ports.append(&ssh_remote);
    ssh_ports.append(&gtk::Label::new(Some("Local port")));
    ssh_ports.append(&ssh_local);
    ssh.append(&ssh_ports);
    let t3c = gtk::Box::new(gtk::Orientation::Vertical, 6);
    t3c.append(&connect_url);
    fields.add_named(&direct, Some("direct"));
    fields.add_named(&tailscale, Some("tailscale"));
    fields.add_named(&ssh, Some("ssh"));
    fields.add_named(&t3c, Some("t3connect"));

    fn row(grid: &gtk::Grid, r: i32, text: &str, w: &impl IsA<gtk::Widget>) {
        let l = gtk::Label::new(Some(text));
        l.set_xalign(1.0);
        grid.attach(&l, 0, r, 1, 1);
        grid.attach(w, 1, r, 1, 1);
    }
    row(&grid, 0, "Name", &name);
    row(&grid, 1, "Route", &kind);
    grid.attach(&fields, 1, 2, 1, 1);
    grid.attach(&hint, 1, 3, 1, 1);
    window.set_child(Some(&grid));

    let hints = [
        "The server's own address, reachable as is (`t3 serve` on this machine or on the LAN).",
        "A tailnet peer; the server there must listen on its Tailscale address (`t3 serve --host <ip>`).",
        "`ssh -N -L local:127.0.0.1:remote target` is kept open while this environment is current; the client talks to the local port.",
        "The relay (app.t3.codes) environment from a pairing URL; the relay sign-in happens in the browser, the URL names the server.",
    ];
    let (f, h) = (fields.clone(), hint.clone());
    kind.connect_selected_notify(move |k| {
        let i = k.selected() as usize;
        f.set_visible_child_name(["direct", "tailscale", "ssh", "t3connect"][i.min(3)]);
        h.set_text(hints[i.min(3)]);
    });
    hint.set_text(hints[0]);

    // Peers arrive async; keep their ips beside the names.
    let peer_ips: Rc<RefCell<Vec<Peer>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let (peers, peer, peer_ips) = (peers.clone(), peer.clone(), peer_ips.clone());
        let preselect = match &existing {
            Some(Environment { route: Route::Tailscale { host, .. }, .. }) => Some(host.clone()),
            _ => None,
        };
        glib::spawn_future_local(async move {
            let list = tailscale_peers().await;
            peers.splice(0, peers.n_items(), &[]);
            if list.is_empty() {
                peers.append("(no Tailscale peers: is `tailscale` installed and up?)");
            }
            for p in &list {
                peers.append(&format!("{} ({}){}", p.host, p.ip, if p.online { "" } else { " — offline" }));
            }
            if let Some(h) = preselect
                && let Some(i) = list.iter().position(|p| p.ip == h || p.host == h)
            {
                peer.set_selected(i as u32);
            }
            *peer_ips.borrow_mut() = list;
        });
    }

    if let Some(e) = &existing {
        name.set_text(&e.name);
        match &e.route {
            Route::Direct { url: u } => {
                kind.set_selected(0);
                url.set_text(u);
            }
            Route::Tailscale { port, .. } => {
                kind.set_selected(1);
                ts_port.set_value(*port as f64);
            }
            Route::Ssh { target, remote_port, local_port } => {
                kind.set_selected(2);
                ssh_target.set_text(target);
                ssh_remote.set_value(*remote_port as f64);
                ssh_local.set_value(*local_port as f64);
            }
            Route::T3Connect { url: u } => {
                kind.set_selected(3);
                connect_url.set_text(u);
            }
        }
    }

    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = window.clone();
    let id = existing.as_ref().map(|e| e.id.clone());
    save.connect_clicked(move |_| {
        let label = name.text().trim().to_string();
        let route = match kind.selected() {
            0 => Route::Direct { url: url.text().trim().trim_end_matches('/').to_string() },
            1 => {
                let i = peer.selected() as usize;
                let Some(p) = peer_ips.borrow().get(i).cloned() else {
                    hint.set_text("Pick a Tailscale peer first.");
                    return;
                };
                Route::Tailscale { host: p.ip, port: ts_port.value() as u16 }
            }
            2 => Route::Ssh {
                target: ssh_target.text().trim().to_string(),
                remote_port: ssh_remote.value() as u16,
                local_port: ssh_local.value() as u16,
            },
            _ => {
                let text = connect_url.text().trim().to_string();
                let (server, _) = crate::t3::auth::parse_pairing(&text, &text);
                Route::T3Connect { url: server }
            }
        };
        let ok = match &route {
            Route::Direct { url } | Route::T3Connect { url } => {
                url.starts_with("http://") || url.starts_with("https://")
            }
            Route::Ssh { target, .. } => !target.is_empty(),
            Route::Tailscale { host, .. } => !host.is_empty(),
        };
        if label.is_empty() || !ok {
            hint.set_text("A name and a reachable address are needed.");
            return;
        }
        let id = id.clone().unwrap_or_else(|| {
            let slug: String =
                label.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
            format!("{}-{}", slug.trim_matches('-'), glib::real_time() / 1_000_000 % 100_000)
        });
        on_save(Environment { id, name: label, route });
        w.close();
    });
    window.present();
}

impl State {
    pub fn icon(self) -> &'static str {
        match self {
            State::Connected => "emblem-default-symbolic",
            State::Connecting => "content-loading-symbolic",
            State::Offline => "dialog-warning-symbolic",
            State::Idle => "network-server-symbolic",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            State::Connected => "connected",
            State::Connecting => "connecting",
            State::Offline => "offline",
            State::Idle => "not connected",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_peers() {
        assert_eq!(Route::Tailscale { host: "100.1.2.3".into(), port: 3773 }.url(), "http://100.1.2.3:3773");
        assert_eq!(
            Route::Ssh { target: "ser8".into(), remote_port: 3773, local_port: 3791 }.url(),
            "http://127.0.0.1:3791"
        );
        let v = serde_json::json!({"Peer": {"a": {"HostName": "ser8", "TailscaleIPs": ["100.1.1.1"], "Online": true}, "b": {"HostName": "mini", "TailscaleIPs": ["100.1.1.2"], "Online": false}}});
        let peers = parse_tailscale_status(&v);
        assert_eq!(peers[0].host, "ser8");
        assert!(peers[0].online && !peers[1].online);
        let mut envs = Environments::default();
        assert!(envs.current().is_none());
        envs.upsert(Environment {
            id: "x".into(),
            name: "X".into(),
            route: Route::Direct { url: "http://h:1/".into() },
        });
        assert_eq!(envs.list[0].route.url(), "http://h:1");
        let json = serde_json::to_string(&envs).unwrap();
        assert!(json.contains("\"kind\":\"direct\""));
    }
}
