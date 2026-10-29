//! Saves a VPN connection through NetworkManager's D-Bus API (`Settings.Connection.Update2`).
//!
//! Secrets travel only over the system bus, never on a command line (where `ps` shows them), and are never
//! printed or logged. A blank secret field keeps the stored value: it is read back with `GetSecrets` and
//! written again, because `Update2` replaces the whole connection.

use gtk::gio;
use gtk::glib::{self, Variant, prelude::*};
use std::collections::HashMap;

const NM: &str = "org.freedesktop.NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";

type Settings = HashMap<String, HashMap<String, Variant>>;

/// What the edit sheet changes. `None` secrets keep what is stored.
pub struct VpnEdit {
    pub server_key: String,
    pub server: String,
    pub user_key: String,
    pub user: String,
    pub password: Option<String>,
    pub psk: Option<String>,
    pub all_traffic: bool,
    /// Store secrets (flag 0) or ask every time (flag 2).
    pub remember: bool,
}

async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&Variant>,
) -> Result<Variant, glib::Error> {
    bus.call_future(Some(NM), path, iface, method, args, None, gio::DBusCallFlags::NONE, 30_000).await
}

fn friendly(e: &glib::Error) -> String {
    let m = e.message();
    if m.contains("NotAuthorized") || m.contains("not authorized") || m.contains("Insufficient privileges") {
        "You didn't approve the change.".into()
    } else {
        // D-Bus errors carry the remote error name first; keep the human part.
        m.rsplit(": ").next().unwrap_or(m).to_string()
    }
}

pub async fn save_vpn(uuid: &str, edit: VpnEdit) -> Result<(), String> {
    let bus = gio::bus_get_future(gio::BusType::System).await.map_err(|e| friendly(&e))?;
    let found = call(
        &bus,
        SETTINGS_PATH,
        "org.freedesktop.NetworkManager.Settings",
        "GetConnectionByUuid",
        Some(&(uuid,).to_variant()),
    )
    .await
    .map_err(|e| friendly(&e))?;
    let path: glib::variant::ObjectPath = found.child_value(0).get().ok_or("NetworkManager gave no connection")?;
    let path = path.as_str().to_string();

    let current = call(&bus, &path, CONNECTION_IFACE, "GetSettings", None).await.map_err(|e| friendly(&e))?;
    let mut settings: Settings = current.child_value(0).get().ok_or("Couldn't read the VPN's settings")?;

    let vpn = settings.entry("vpn".into()).or_default();
    let mut data: HashMap<String, String> = vpn.get("data").and_then(|v| v.get()).unwrap_or_default();
    let flag = if edit.remember { "0" } else { "2" };
    data.insert(edit.server_key.clone(), edit.server.trim().to_string());
    data.insert(edit.user_key.clone(), edit.user.trim().to_string());
    data.insert("password-flags".into(), flag.into());
    let has_psk = data.contains_key("ipsec-psk-flags") || data.get("ipsec-enabled").is_some_and(|v| v == "yes");
    if has_psk {
        data.insert("ipsec-psk-flags".into(), flag.into());
    }

    // Keep stored secrets for blank fields: read them back (they never leave this process).
    let mut secrets: HashMap<String, String> = HashMap::new();
    if edit.remember && (edit.password.is_none() || (has_psk && edit.psk.is_none())) {
        // Never wipe a stored secret by accident: if it can't be read back, stop and ask for it.
        let stored =
            call(&bus, &path, CONNECTION_IFACE, "GetSecrets", Some(&("vpn",).to_variant())).await.map_err(|_| {
                "The saved password couldn't be read back. Type the password and key, then save.".to_string()
            })?;
        let stored: Settings = stored.child_value(0).get().unwrap_or_default();
        if let Some(s) =
            stored.get("vpn").and_then(|v| v.get("secrets")).and_then(|v| v.get::<HashMap<String, String>>())
        {
            secrets = s;
        }
    }
    if edit.remember {
        if let Some(p) = edit.password {
            secrets.insert("password".into(), p);
        }
        if has_psk && let Some(k) = edit.psk {
            secrets.insert("ipsec-psk".into(), k);
        }
    } else {
        secrets.clear();
    }
    vpn.insert("data".into(), data.to_variant());
    vpn.insert("secrets".into(), secrets.to_variant());

    // macOS's "Send all traffic over VPN connection" is never-default = false.
    for family in ["ipv4", "ipv6"] {
        settings.entry(family.into()).or_default().insert("never-default".into(), (!edit.all_traffic).to_variant());
    }

    // Update2(settings, flags = TO_DISK, args = {}).
    let args = (settings.to_variant(), 1u32, HashMap::<String, Variant>::new().to_variant());
    let packed = Variant::tuple_from_iter([args.0, args.1.to_variant(), args.2]);
    call(&bus, &path, CONNECTION_IFACE, "Update2", Some(&packed)).await.map_err(|e| friendly(&e))?;
    Ok(())
}
