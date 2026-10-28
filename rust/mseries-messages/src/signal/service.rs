//! Reaching signal-cli without ever running two daemons on one account.
//!
//! * If the Signal Pantheon client's daemon is up, connect to its socket.
//! * If the client process is running but its socket is not ready yet, wait:
//!   starting a second daemon on the same data directory is the one thing this
//!   module must never do.
//! * Otherwise start the bundled signal-cli exactly as the `signal-pantheon`
//!   launcher does, on a socket of our own, and stop only that child (by the
//!   PID we started) when it has been idle or the app quits.

use super::owner;
use super::rpc::{RpcConnection, RpcError};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_GRACE: Duration = Duration::from_secs(8);
/// How long our own daemon may sit unused before it is stopped.
pub const IDLE_LIMIT: Duration = Duration::from_secs(90);

/// Where everything lives. Built from the environment in the app and from a
/// scratch directory in tests.
#[derive(Clone, Debug)]
pub struct Layout {
    pub data_directory: PathBuf,
    /// The Pantheon client's socket (we only ever connect to it).
    pub client_socket: PathBuf,
    /// The socket of a daemon this app started.
    pub own_socket: PathBuf,
    pub signal_cli: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
}

impl Layout {
    pub fn detect() -> Self {
        let data_directory = super::data_directory();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(gtk::glib::user_cache_dir)
            .join("signal-pantheon");
        let bundled = data_directory.join("runtime");
        let signal_cli = std::env::var_os("SIGNAL_CLI_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                Some(bundled.join("signal-cli-arm64/bin/signal-cli")).filter(|path| path.exists())
            });
        let java_home = std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(bundled.join("temurin25")).filter(|path| path.exists()));
        Self {
            data_directory,
            client_socket: runtime.join("signal-cli.sock"),
            own_socket: runtime.join("signal-cli-messages.sock"),
            signal_cli,
            java_home,
        }
    }
}

struct Daemon {
    child: Child,
    socket: PathBuf,
    last_used: Instant,
}

impl Daemon {
    fn start(layout: &Layout) -> Result<Self, RpcError> {
        let Some(executable) = layout.signal_cli.as_ref() else {
            return Err(RpcError::Unavailable(
                "Signal is not set up on this computer. Open Signal Pantheon once to link it."
                    .into(),
            ));
        };
        if let Some(parent) = layout.own_socket.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::remove_file(&layout.own_socket).ok();
        let mut command = Command::new(executable);
        command
            .arg("--data-dir")
            .arg(&layout.data_directory)
            .arg("daemon")
            .arg(format!("--socket={}", layout.own_socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(java) = layout.java_home.as_ref() {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![java.join("bin")];
            paths.extend(std::env::split_paths(&path));
            command.env("JAVA_HOME", java);
            if let Ok(joined) = std::env::join_paths(paths) {
                command.env("PATH", joined);
            }
        }
        let child = command
            .spawn()
            .map_err(|error| RpcError::Unavailable(format!("could not start Signal: {error}")))?;
        let mut daemon = Self {
            child,
            socket: layout.own_socket.clone(),
            last_used: Instant::now(),
        };
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if daemon.socket.exists() && RpcConnection::connect(&daemon.socket).is_ok() {
                return Ok(daemon);
            }
            if let Ok(Some(status)) = daemon.child.try_wait() {
                return Err(RpcError::Unavailable(format!(
                    "The Signal service stopped while starting ({status}). Is Signal Pantheon already using this account?"
                )));
            }
            if Instant::now() >= deadline {
                return Err(RpcError::Unavailable(
                    "Timed out waiting for the local Signal service".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(160));
        }
    }

    /// Ask politely first, then force: only ever the PID we started.
    fn stop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        std::fs::remove_file(&self.socket).ok();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

static DAEMON: Mutex<Option<Daemon>> = Mutex::new(None);

/// A connection to whichever daemon owns the account right now.
pub fn connect(layout: &Layout) -> Result<RpcConnection, RpcError> {
    let mut guard = DAEMON
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(daemon) = guard.as_mut() {
        if matches!(daemon.child.try_wait(), Ok(None))
            && let Ok(connection) = RpcConnection::connect(&daemon.socket)
        {
            daemon.last_used = Instant::now();
            return Ok(connection);
        }
        *guard = None;
    }
    if layout.client_socket.exists()
        && let Ok(connection) = RpcConnection::connect(&layout.client_socket)
    {
        return Ok(connection);
    }
    if owner::client_running() {
        return Err(RpcError::Unavailable(
            "Signal Pantheon is starting. Try again in a moment.".into(),
        ));
    }
    // A new daemon would wait the whole start timeout for a lock it cannot get.
    // (Our own daemon is gone at this point: the guard above was cleared.)
    if stuck_daemon_in(std::path::Path::new("/proc"), layout, None).is_some() {
        return Err(RpcError::Unavailable(STUCK_MESSAGE.into()));
    }
    let daemon = Daemon::start(layout)?;
    let connection = RpcConnection::connect(&daemon.socket).map_err(RpcError::Unavailable)?;
    *guard = Some(daemon);
    Ok(connection)
}

/// Stop our own daemon when it has not been used for a while.
pub fn stop_if_idle(limit: Duration) {
    let mut guard = DAEMON
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard
        .as_ref()
        .is_some_and(|daemon| daemon.last_used.elapsed() >= limit)
    {
        *guard = None;
    }
}

/// Stop our own daemon now (app quit). The Pantheon client's is never touched.
pub fn shutdown() {
    *DAEMON
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// True when this app started the daemon it is talking to.
pub fn owns_daemon() -> bool {
    // Never blocks the caller (the UI thread) behind a daemon that is starting.
    match DAEMON.try_lock() {
        Ok(guard) => guard.is_some(),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner().is_some(),
        Err(std::sync::TryLockError::WouldBlock) => true,
    }
}

/// A signal-cli daemon of this user, on this Signal data directory, that no app
/// can reach: nobody started it (its parent is gone) and its socket file is
/// missing or refuses connections. It still holds the account lock, so no other
/// daemon can start, and what it receives reaches no app.
///
/// This happens when the Signal Pantheon client is closed abruptly (its daemon
/// lives on) and then reopened: the client deletes the old socket file and its
/// new daemon waits forever for the lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckDaemon {
    pub pid: u32,
}

/// The stuck daemon on this machine, if there is one.
pub fn stuck_daemon(layout: &Layout) -> Option<StuckDaemon> {
    let our_child = match DAEMON.try_lock() {
        Ok(guard) => guard.as_ref().map(|daemon| daemon.child.id()),
        // A daemon of ours is starting or stopping: nothing is stuck yet.
        Err(_) => return None,
    };
    stuck_daemon_in(std::path::Path::new("/proc"), layout, our_child)
}

pub(crate) fn stuck_daemon_in(
    proc_root: &std::path::Path,
    layout: &Layout,
    our_child: Option<u32>,
) -> Option<StuckDaemon> {
    use std::os::unix::fs::MetadataExt;
    let own_uid = std::fs::metadata(proc_root.join("self")).ok()?.uid();
    let own_pid = std::process::id();
    for entry in std::fs::read_dir(proc_root).ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid || Some(pid) == our_child {
            continue;
        }
        let path = entry.path();
        if !std::fs::metadata(&path).is_ok_and(|meta| meta.uid() == own_uid) {
            continue;
        }
        let Ok(raw) = std::fs::read(path.join("cmdline")) else {
            continue;
        };
        let arguments: Vec<String> = raw
            .split(|byte| *byte == 0)
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect();
        let Some(socket) = daemon_socket_for(&arguments, &layout.data_directory) else {
            continue;
        };
        if parent_is_alive_owner(proc_root, &path, own_pid) {
            continue;
        }
        let reachable = socket.exists() && std::os::unix::net::UnixStream::connect(&socket).is_ok();
        if !reachable {
            return Some(StuckDaemon { pid });
        }
    }
    None
}

/// The socket of a `signal-cli --data-dir <directory> daemon --socket=<path>`
/// command line (the launcher script or the Java process it became).
fn daemon_socket_for(arguments: &[String], directory: &std::path::Path) -> Option<PathBuf> {
    if !arguments.iter().any(|argument| argument == "daemon") {
        return None;
    }
    let mut data_directory = None;
    let mut socket = None;
    let mut iter = arguments.iter().peekable();
    while let Some(argument) = iter.next() {
        if argument == "--data-dir" {
            data_directory = iter.peek().map(|value| PathBuf::from(value.as_str()));
        } else if let Some(value) = argument.strip_prefix("--data-dir=") {
            data_directory = Some(PathBuf::from(value));
        } else if argument == "--socket" {
            socket = iter.peek().map(|value| PathBuf::from(value.as_str()));
        } else if let Some(value) = argument.strip_prefix("--socket=") {
            socket = Some(PathBuf::from(value));
        }
    }
    (data_directory.as_deref() == Some(directory)).then_some(socket?)
}

/// True when the daemon's parent is an app that manages it: this app, or a
/// running Signal Pantheon client (whose daemon may still be starting).
fn parent_is_alive_owner(
    proc_root: &std::path::Path,
    process: &std::path::Path,
    own_pid: u32,
) -> bool {
    let Ok(stat) = std::fs::read_to_string(process.join("stat")) else {
        return true;
    };
    // The command name may contain spaces; the parent PID follows the closing ")".
    let Some(parent) = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return true;
    };
    if parent == own_pid {
        return true;
    }
    let name = std::fs::read_to_string(proc_root.join(parent.to_string()).join("comm"))
        .unwrap_or_default();
    // `comm` is cut to 15 bytes: "signal-pantheon-client" reads "signal-pantheon".
    let name = name.trim();
    name.starts_with("signal-pantheon") || name == "mseries-messages"
}

/// Stop a stuck daemon, at the owner's request. It is checked again first, so
/// only a process that is still the same stuck signal-cli daemon is signalled.
pub fn restart_stuck_daemon(pid: u32) -> Result<(), String> {
    let layout = Layout::detect();
    if stuck_daemon(&layout) != Some(StuckDaemon { pid }) {
        return Ok(());
    }
    let signal = |name: &str| {
        Command::new("kill")
            .args([name, &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    if !signal("-TERM") {
        return Err("Could not stop the stuck Signal service.".into());
    }
    let gone = || !std::path::Path::new("/proc").join(pid.to_string()).exists();
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if gone() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    signal("-KILL");
    std::thread::sleep(Duration::from_millis(300));
    if gone() {
        Ok(())
    } else {
        Err("The stuck Signal service did not stop.".into())
    }
}

/// What to tell the owner when the account is held by a stuck daemon.
pub const STUCK_MESSAGE: &str = "Signal's background service is stuck and no app can reach it. Choose Restart Signal in the sidebar.";

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_process(root: &std::path::Path, pid: u32, parent: u32, comm: &str, args: &[&str]) {
        let directory = root.join(pid.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("cmdline"), args.join("\0") + "\0").unwrap();
        std::fs::write(directory.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            directory.join("stat"),
            format!("{pid} ({comm}) S {parent} 1 1 0 -1"),
        )
        .unwrap();
    }

    fn scratch_proc(name: &str) -> (std::path::PathBuf, Layout) {
        let root = std::env::temp_dir().join(format!("signal-stuck-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(format!("/proc/{}", std::process::id()), root.join("self"))
            .unwrap();
        let layout = Layout {
            data_directory: root.join("data"),
            client_socket: root.join("runtime/signal-cli.sock"),
            own_socket: root.join("runtime/own.sock"),
            signal_cli: None,
            java_home: None,
        };
        (root, layout)
    }

    #[test]
    fn an_orphaned_daemon_with_its_socket_deleted_is_stuck() {
        let (root, layout) = scratch_proc("orphan");
        let data = layout.data_directory.display().to_string();
        let socket = format!("--socket={}", layout.client_socket.display());
        fake_process(&root, 1, 0, "systemd", &["/sbin/init"]);
        fake_process(
            &root,
            4242,
            1,
            "java",
            &[
                "java",
                "-cp",
                "x.jar",
                "org.asamk.signal.Main",
                "--data-dir",
                &data,
                "daemon",
                &socket,
            ],
        );
        assert_eq!(
            stuck_daemon_in(&root, &layout, None),
            Some(StuckDaemon { pid: 4242 })
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_reachable_daemon_or_a_clients_starting_daemon_is_not_stuck() {
        let (root, layout) = scratch_proc("healthy");
        let data = layout.data_directory.display().to_string();
        let socket = format!("--socket={}", layout.client_socket.display());
        fake_process(&root, 1, 0, "systemd", &["/sbin/init"]);
        fake_process(
            &root,
            900,
            1,
            "signal-pantheon",
            &["signal-pantheon-client"],
        );
        // A client's daemon whose socket is not there yet is starting, not stuck.
        fake_process(
            &root,
            901,
            900,
            "java",
            &["java", "--data-dir", &data, "daemon", &socket],
        );
        assert_eq!(stuck_daemon_in(&root, &layout, None), None);
        // An orphan that still answers on its socket is usable, not stuck.
        std::fs::remove_dir_all(root.join("900")).unwrap();
        std::fs::create_dir_all(layout.client_socket.parent().unwrap()).unwrap();
        let _listener = std::os::unix::net::UnixListener::bind(&layout.client_socket).unwrap();
        fake_process(
            &root,
            901,
            1,
            "java",
            &["java", "--data-dir", &data, "daemon", &socket],
        );
        assert_eq!(stuck_daemon_in(&root, &layout, None), None);
        // Another account's daemon is none of our business.
        fake_process(
            &root,
            902,
            1,
            "java",
            &[
                "java",
                "--data-dir",
                "/elsewhere",
                "daemon",
                "--socket=/nowhere",
            ],
        );
        assert_eq!(stuck_daemon_in(&root, &layout, None), None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_clients_daemon_is_used_and_no_second_one_is_started() {
        let directory = std::env::temp_dir().join(format!("signal-service-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let client_socket = directory.join("signal-cli.sock");
        let _client = std::os::unix::net::UnixListener::bind(&client_socket).unwrap();
        let layout = Layout {
            data_directory: directory.clone(),
            client_socket,
            own_socket: directory.join("own.sock"),
            // Starting a daemon would fail loudly: there is nothing to start.
            signal_cli: None,
            java_home: None,
        };
        assert!(connect(&layout).is_ok());
        assert!(!owns_daemon());
        assert!(!layout.own_socket.exists());
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn without_a_client_or_a_runtime_there_is_a_clear_message() {
        let directory =
            std::env::temp_dir().join(format!("signal-service-none-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let layout = Layout {
            data_directory: directory.clone(),
            client_socket: directory.join("missing.sock"),
            own_socket: directory.join("own.sock"),
            signal_cli: None,
            java_home: None,
        };
        // Only meaningful when the real client is not running on this machine.
        if !owner::client_running() {
            let error = connect(&layout).err().expect("no service to reach");
            assert!(error.message().contains("not set up"));
        }
        std::fs::remove_dir_all(&directory).ok();
    }
}
