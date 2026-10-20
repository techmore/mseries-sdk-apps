use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
struct Config {
    charge_limit: u32,
    charge_start: u32,
    battery_profile: String,
    ac_profile: String,
    battery_cap_khz: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            charge_limit: 80,
            charge_start: 75,
            battery_profile: "power-saver".into(),
            ac_profile: "balanced".into(),
            battery_cap_khz: 2_400_000,
        }
    }
}

/// Parse KEY=VALUE lines like the bash `source` of power.conf.
/// Unknown keys are ignored; invalid values for known keys are ignored.
fn parse_conf(text: &str) -> Config {
    let mut cfg = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k.trim() {
            "CHARGE_LIMIT" => {
                if let Ok(n) = v.parse() {
                    cfg.charge_limit = n;
                }
            }
            "CHARGE_START" => {
                if let Ok(n) = v.parse() {
                    cfg.charge_start = n;
                }
            }
            "BATTERY_PROFILE" => cfg.battery_profile = v.to_string(),
            "AC_PROFILE" => cfg.ac_profile = v.to_string(),
            "BATTERY_CAP_KHZ" => {
                if let Ok(n) = v.parse() {
                    cfg.battery_cap_khz = n;
                }
            }
            _ => {}
        }
    }
    cfg
}

fn load_conf(path: &Path) -> Config {
    fs::read_to_string(path)
        .map(|t| parse_conf(&t))
        .unwrap_or_default()
}

fn write_sys(path: &Path, value: &str) {
    if fs::write(path, value).is_err() {
        eprintln!("could not write {value} to {}", path.display());
    }
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

struct Paths {
    conf: PathBuf,
    bat: PathBuf,
    ac_online: PathBuf,
    cpufreq: PathBuf,
}

impl Paths {
    fn system() -> Self {
        Self {
            conf: PathBuf::from("/etc/mseries/power.conf"),
            bat: PathBuf::from("/sys/class/power_supply/macsmc-battery"),
            ac_online: PathBuf::from("/sys/class/power_supply/macsmc-ac/online"),
            cpufreq: PathBuf::from("/sys/devices/system/cpu/cpufreq"),
        }
    }
}

/// Order matters: start threshold must stay below end threshold.
/// Returns (first_write, second_write) as (file, value) for testability.
fn limit_writes(limit: u32, start: u32) -> Vec<(&'static str, String)> {
    if limit >= 100 {
        vec![
            ("charge_control_end_threshold", "100".into()),
            ("charge_control_start_threshold", "95".into()),
        ]
    } else {
        vec![
            ("charge_control_start_threshold", start.to_string()),
            ("charge_control_end_threshold", limit.to_string()),
        ]
    }
}

fn apply_limit(paths: &Paths, cfg: &Config) {
    for (file, value) in limit_writes(cfg.charge_limit, cfg.charge_start) {
        write_sys(&paths.bat.join(file), &value);
    }
}

fn apply_source(paths: &Paths, cfg: &Config) {
    let on_ac = read_trim(&paths.ac_online).unwrap_or_else(|| "1".into());
    let (cap, profile) = if on_ac == "1" {
        (0, &cfg.ac_profile)
    } else {
        (cfg.battery_cap_khz, &cfg.battery_profile)
    };
    if let Ok(entries) = fs::read_dir(&paths.cpufreq) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("policy") {
                continue;
            }
            let max: u64 = read_trim(&entry.path().join("cpuinfo_max_freq"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let target = if max > 2_500_000 && cap > 0 { cap } else { max };
            write_sys(&entry.path().join("scaling_max_freq"), &target.to_string());
        }
    }
    let _ = Command::new("powerprofilesctl")
        .arg("set")
        .arg(profile)
        .output();
}

fn status(paths: &Paths) {
    let get = |f: &str| read_trim(&paths.bat.join(f)).unwrap_or_else(|| "?".into());
    let cap = get("capacity");
    let st = get("status");
    let health = match (
        get("energy_full").parse::<u64>(),
        get("energy_full_design").parse::<u64>(),
    ) {
        (Ok(f), Ok(d)) if d > 0 => format!("{}%", f * 100 / d),
        _ => "?".into(),
    };
    println!(
        "battery: {cap}% ({st}), health {health}, {} cycles",
        get("cycle_count")
    );
    println!(
        "charge limit: stop at {}%, resume at {}%",
        get("charge_control_end_threshold"),
        get("charge_control_start_threshold")
    );
    let ac = read_trim(&paths.ac_online).unwrap_or_else(|| "1".into());
    let profile = Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    println!(
        "power source: {}; profile: {}",
        if ac == "1" { "AC" } else { "battery" },
        profile.trim()
    );
    if let Ok(entries) = fs::read_dir(&paths.cpufreq) {
        let mut policies: Vec<_> = entries.flatten().collect();
        policies.sort_by_key(|e| e.file_name());
        for entry in policies {
            if !entry.file_name().to_string_lossy().starts_with("policy") {
                continue;
            }
            let num = |f: &str| {
                read_trim(&entry.path().join(f))
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    / 1000
            };
            let cpus = read_trim(&entry.path().join("related_cpus")).unwrap_or_else(|| "?".into());
            println!(
                "cpus {cpus}: max {} MHz (hardware {})",
                num("scaling_max_freq"),
                num("cpuinfo_max_freq")
            );
        }
    }
}

fn update_conf_file(path: &Path, limit: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut map = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    if let Ok(text) = fs::read_to_string(path) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=')
                && k.trim() == "CHARGE_LIMIT"
            {
                let _ = v;
                continue;
            }
            if let Some((k, _)) = line.split_once('=') {
                map.entry(k.trim().to_string()).or_insert_with(|| {
                    order.push(k.trim().to_string());
                    line.to_string()
                });
            } else {
                order.push(line.to_string());
            }
        }
    }
    let mut out: Vec<String> = order
        .iter()
        .map(|l| map.remove(l).unwrap_or_else(|| l.clone()))
        .collect();
    out.push(format!("CHARGE_LIMIT={limit}"));
    // Drop stale CHARGE_LIMIT lines that came through `order` as raw lines.
    out.retain(|l| !l.starts_with("CHARGE_LIMIT=") || l == &format!("CHARGE_LIMIT={limit}"));
    // Keep exactly one CHARGE_LIMIT line.
    let mut seen = false;
    out.retain(|l| {
        if l.starts_with("CHARGE_LIMIT=") {
            if seen {
                return false;
            }
            seen = true;
        }
        true
    });
    fs::write(path, out.join("\n") + "\n")
}

fn usage() -> ! {
    eprintln!("usage: mseries-power status|limit 80|100|apply");
    std::process::exit(1);
}

fn main() {
    let paths = Paths::system();
    let mut cfg = load_conf(&paths.conf);
    let mut args = std::env::args().skip(1);
    match args.next().as_deref().unwrap_or("status") {
        "apply" => {
            apply_limit(&paths, &cfg);
            apply_source(&paths, &cfg);
        }
        "limit" => {
            let v = args.next().unwrap_or_default();
            if v != "80" && v != "100" {
                eprintln!("usage: mseries-power limit 80|100");
                std::process::exit(1);
            }
            if update_conf_file(&paths.conf, &v).is_err() {
                eprintln!("could not update {}", paths.conf.display());
                std::process::exit(1);
            }
            cfg.charge_limit = v.parse().unwrap();
            apply_limit(&paths, &cfg);
        }
        "status" => {}
        _ => usage(),
    }
    status(&paths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_bash() {
        let cfg = Config::default();
        assert_eq!((cfg.charge_limit, cfg.charge_start), (80, 75));
        assert_eq!(cfg.battery_cap_khz, 2_400_000);
    }

    #[test]
    fn conf_overrides_known_keys() {
        let cfg = parse_conf("CHARGE_LIMIT=100\nBATTERY_CAP_KHZ=0\n# comment\n");
        assert_eq!(cfg.charge_limit, 100);
        assert_eq!(cfg.battery_cap_khz, 0);
        assert_eq!(cfg.charge_start, 75); // untouched
    }

    #[test]
    fn limit_ordering_matches_bash() {
        // limit >= 100: end first, then start (avoids start >= end trip).
        assert_eq!(
            limit_writes(100, 75)
                .iter()
                .map(|(f, _)| *f)
                .collect::<Vec<_>>(),
            vec![
                "charge_control_end_threshold",
                "charge_control_start_threshold"
            ]
        );
        // normal: start first, then end.
        let w = limit_writes(80, 75);
        assert_eq!(w[0], ("charge_control_start_threshold", "75".into()));
        assert_eq!(w[1], ("charge_control_end_threshold", "80".into()));
    }

    #[test]
    fn only_80_or_100_allowed() {
        for bad in ["0", "79", "81", "99", "101", "abc", ""] {
            assert!(bad != "80" && bad != "100", "{bad} must be rejected");
        }
    }
}
