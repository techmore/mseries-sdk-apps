//! Who may write the Signal history, and who owns the Signal daemon.
//!
//! The Signal Pantheon client rewrites whole conversation snapshots from its
//! own memory, so a write from another process would be lost, or worse, would
//! be overwritten with stale state. The rule is therefore single-writer:
//!
//! * while the client is running this app never writes `message-history.db`;
//! * while this app writes, it holds an exclusive advisory lock, so two
//!   instances of this app cannot interleave their writes either.
//!
//! The client is recognised by the executable of a running process owned by
//! the same user (never by killing or signalling anything).

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const CLIENT_EXECUTABLE: &str = "signal-pantheon-client";

/// True when a Signal Pantheon client process of this user is running.
pub fn client_running() -> bool {
    // The name can be overridden so isolated tests ignore a real client on the machine.
    let name = std::env::var("MSERIES_SIGNAL_CLIENT_NAME")
        .unwrap_or_else(|_| CLIENT_EXECUTABLE.to_string());
    client_running_in(Path::new("/proc"), &name)
}

pub(crate) fn client_running_in(proc_root: &Path, executable: &str) -> bool {
    let Ok(own) = std::fs::metadata(proc_root.join("self")).map(|meta| meta.uid()) else {
        // Without /proc the client cannot be ruled out: assume it is running.
        return true;
    };
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return true;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        if !name
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            return false;
        }
        let path = entry.path();
        std::fs::metadata(&path).is_ok_and(|meta| meta.uid() == own)
            && std::fs::read_link(path.join("exe"))
                .ok()
                .and_then(|target| {
                    target.file_name().map(|file| {
                        file.to_string_lossy().trim_end_matches(" (deleted)") == executable
                    })
                })
                .unwrap_or(false)
    })
}

/// Exclusive right to write the history; released on drop.
pub struct WriterLock {
    _file: File,
}

impl WriterLock {
    pub fn path(data_directory: &Path) -> PathBuf {
        data_directory.join(".messages-writer.lock")
    }

    /// `Ok(None)` when another writer holds the lock.
    pub fn try_acquire(data_directory: &Path) -> Result<Option<Self>, String> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(Self::path(data_directory))
            .map_err(|error| format!("could not open the Signal writer lock: {error}"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => {
                Err(format!("could not lock the Signal history: {error}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("signal-owner-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_process_is_found_by_its_executable_not_its_name_text() {
        let proc_root = scratch("proc");
        std::os::unix::fs::symlink(std::process::id().to_string(), proc_root.join("self")).ok();
        let own = proc_root.join(std::process::id().to_string());
        std::fs::create_dir_all(&own).unwrap();
        std::os::unix::fs::symlink("/usr/bin/signal-pantheon-client", own.join("exe")).unwrap();
        assert!(client_running_in(&proc_root, "signal-pantheon-client"));
        assert!(!client_running_in(&proc_root, "something-else"));
        std::fs::remove_dir_all(&proc_root).ok();
    }

    #[test]
    fn the_real_proc_does_not_list_this_test_as_the_client() {
        assert!(!client_running_in(
            Path::new("/proc"),
            "no-such-signal-client"
        ));
    }

    #[test]
    fn only_one_writer_holds_the_lock() {
        let directory = scratch("lock");
        let first = WriterLock::try_acquire(&directory).unwrap();
        assert!(first.is_some());
        assert!(WriterLock::try_acquire(&directory).unwrap().is_none());
        drop(first);
        assert!(WriterLock::try_acquire(&directory).unwrap().is_some());
        std::fs::remove_dir_all(&directory).ok();
    }
}
