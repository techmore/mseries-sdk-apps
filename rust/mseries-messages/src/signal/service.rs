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

#[cfg(test)]
mod tests {
    use super::*;

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
