use crate::DESKTOP_RESERVE_BYTES;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct HostCapabilities {
    pub available_bytes: u64,
    pub has_render_node: bool,
}

impl HostCapabilities {
    pub fn capture() -> Self {
        let available_bytes = fs::read_to_string("/proc/meminfo")
            .map(|s| parse_mem_available(&s))
            .unwrap_or(0);
        let has_render_node = fs::read_dir("/dev/dri")
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
            })
            .unwrap_or(false);
        Self {
            available_bytes,
            has_render_node,
        }
    }

    pub fn available_gib(&self) -> f64 {
        self.available_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn reserve_met(&self) -> bool {
        self.available_bytes >= DESKTOP_RESERVE_BYTES
    }

    pub fn memory_summary(&self) -> String {
        if self.available_bytes == 0 {
            "Available memory could not be measured.".into()
        } else {
            format!("{:.1} GiB available now", self.available_gib())
        }
    }

    pub fn accelerator_summary(&self) -> &'static str {
        if self.has_render_node {
            "A render device is present; model/runtime support is unverified."
        } else {
            "No DRM render device detected; image support is unverified."
        }
    }
}

pub fn parse_mem_available(contents: &str) -> u64 {
    contents
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("MemAvailable:")?;
            let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            kib.checked_mul(1024)
        })
        .unwrap_or(0)
}

pub fn has_render_node_at(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memavailable_and_missing_values() {
        assert_eq!(
            parse_mem_available("MemTotal: 32000000 kB\nMemAvailable: 6400000 kB\n"),
            6_400_000 * 1024
        );
        assert_eq!(parse_mem_available("MemAvailable: unknown kB\n"), 0);
        assert_eq!(parse_mem_available("MemTotal: 1000 kB\n"), 0);
    }
}

/// Recover only locks whose recorded worker PID is definitely no longer alive.
pub fn recover_abandoned_workflows(base: &Path) {
    for (lock_name, pid_name) in [
        ("generation.lock", "generation.lock"),
        ("batch.lock", "batch.pid"),
    ] {
        let lock = base.join(lock_name);
        let pid = fs::read_to_string(base.join(pid_name))
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok());
        if let Some(pid) = pid {
            if pid > 0 && !Path::new("/proc").join(pid.to_string()).exists() {
                let _ = fs::remove_file(&lock);
                if lock_name != pid_name {
                    let _ = fs::remove_file(base.join(pid_name));
                }
                for filename in ["current-job.json", "review-status.json"] {
                    let path = base.join(filename);
                    if let Ok(raw) = fs::read(&path) {
                        if let Ok(mut job) = serde_json::from_slice::<serde_json::Value>(&raw) {
                            if job["running"].as_bool() == Some(true) {
                                job["running"] = false.into();
                                job["status"] = "Interrupted · previous worker ended".into();
                                if let Ok(raw) = serde_json::to_vec(&job) {
                                    let _ = fs::write(path, raw);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
