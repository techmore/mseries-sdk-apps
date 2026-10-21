//! Mounted filesystems and free space, straight from the kernel (no extra services).

use std::ffi::CString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Mount {
    pub path: PathBuf,
    pub device: String,
    pub fstype: String,
    pub total: u64,
    pub free: u64,
}

/// Block-backed and network filesystems a person would want to analyse. Pseudo filesystems
/// (proc, tmpfs, cgroup, snap squashfs loops) are left out.
pub fn mounts() -> Vec<Mount> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return Vec::new();
    };
    let mut out: Vec<Mount> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let fields: Vec<&str> = left.split(' ').collect();
        let mut tail = right.split(' ');
        let (Some(fstype), Some(device)) = (tail.next(), tail.next()) else { continue };
        let Some(raw_path) = fields.get(4) else { continue };
        if !is_real(fstype, device) {
            continue;
        }
        let path = PathBuf::from(unescape(raw_path));
        if out.iter().any(|m| m.path == path) {
            continue;
        }
        if let Some((total, free)) = space(&path)
            && total > 0
        {
            out.push(Mount { path, device: device.to_string(), fstype: fstype.to_string(), total, free });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn is_real(fstype: &str, device: &str) -> bool {
    const NETWORK: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "fuse.sshfs"];
    if NETWORK.contains(&fstype) {
        return true;
    }
    if matches!(fstype, "squashfs" | "overlay" | "tmpfs" | "devtmpfs" | "iso9660" | "erofs") {
        return false;
    }
    device.starts_with("/dev/") && !device.starts_with("/dev/loop")
}

/// mountinfo escapes space, tab, newline and backslash as three-digit octal.
fn unescape(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            let value = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(total, available-to-you)` bytes for the filesystem holding `path`.
pub fn space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized, writable statvfs.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some((st.f_blocks as u64 * unit, st.f_bavail as u64 * unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_octal_sequences() {
        assert_eq!(unescape("/mnt/My\\040Disk"), "/mnt/My Disk");
        assert_eq!(unescape("/plain"), "/plain");
        assert_eq!(unescape("/a\\134b"), "/a\\b");
    }

    #[test]
    fn root_has_space() {
        let (total, free) = space(Path::new("/")).unwrap();
        assert!(total > 0 && free <= total);
    }

    #[test]
    fn filters_pseudo_filesystems() {
        assert!(is_real("btrfs", "/dev/mapper/root"));
        assert!(is_real("nfs4", "server:/share"));
        assert!(!is_real("tmpfs", "tmpfs"));
        assert!(!is_real("squashfs", "/dev/loop3"));
        assert!(!is_real("proc", "proc"));
    }
}
