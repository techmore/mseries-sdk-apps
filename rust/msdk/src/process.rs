//! Owned subprocess work for SDK clients. A private Unix process group, bounded
//! nonblocking pipes and monotonic deadline bound inherited-pipe handling. The
//! runner reaps its leader and cancels same-group descendants. Dropping its handle
//! requests cancellation; daemon/privileged jobs need an OS-owned service adapter.

use crate::runtime::{FailureKind, ServiceFailure};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub const STREAM_QUEUE_CHUNKS: usize = 32;
pub const STREAM_CHUNK_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Raw bytes: chunks need not align with UTF-8 or line boundaries. Private data
/// is deliberately absent from Debug. Keep a decoder per stream in the client.
pub struct Chunk {
    pub stream: Stream,
    pub bytes: Vec<u8>,
}
impl std::fmt::Debug for Chunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chunk").field("stream", &self.stream).field("byte_count", &self.bytes.len()).finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    /// Combined stdout and stderr bytes; at most 16 MiB is accepted.
    pub output_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(120), output_bytes: 1024 * 1024 }
    }
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
}

/// An owned bounded output subscription. Dropping it cancels the task even
/// while the command is silent. Retain it until the desired output is consumed.
pub struct OutputStream {
    receiver: Receiver<Chunk>,
    _process: Process,
}
impl OutputStream {
    pub fn recv(&self) -> Result<Chunk, mpsc::RecvError> {
        self.receiver.recv()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Chunk, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
    pub fn try_recv(&self) -> Result<Chunk, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

/// A context-bound output subscription. Preserve each returned envelope until
/// the state owner applies it, then call `into_current` on that same thread.
pub struct ScopedOutputStream {
    output: OutputStream,
    context: crate::context::ContextToken,
}
impl ScopedOutputStream {
    pub fn recv(&self) -> Result<crate::context::Scoped<Chunk>, mpsc::RecvError> {
        self.output.recv().map(|chunk| self.context.wrap(chunk))
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<crate::context::Scoped<Chunk>, mpsc::RecvTimeoutError> {
        self.output.recv_timeout(timeout).map(|chunk| self.context.wrap(chunk))
    }
    pub fn try_recv(&self) -> Result<crate::context::Scoped<Chunk>, mpsc::TryRecvError> {
        self.output.try_recv().map(|chunk| self.context.wrap(chunk))
    }
}

struct Control {
    cancelled: AtomicBool,
    finished: AtomicBool,
    group: Mutex<Option<i32>>,
    context: Option<crate::context::ContextToken>,
}

impl Control {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.context.as_ref().is_some_and(|token| !token.is_current())
    }
}

pub struct Process {
    control: Arc<Control>,
}
impl Process {
    pub fn cancel(&self) {
        self.control.cancelled.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.control.group.lock().unwrap_or_else(|e| e.into_inner()) {
            kill_group(pid);
        }
    }
    pub fn is_finished(&self) -> bool {
        self.control.finished.load(Ordering::SeqCst)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if !self.is_finished() {
            self.cancel();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn kill_group(pid: i32) {
    if pid > 0 {
        // SAFETY: only the dedicated group created by this operation is targeted.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn kill_group(_: i32) {}

pub fn spawn(
    command: String,
    limits: Limits,
    operation: u64,
    complete: impl FnOnce(Result<Output, ServiceFailure>) + Send + 'static,
) -> Result<Process, ServiceFailure> {
    spawn_inner(CommandSpec::Shell(command), limits, operation, None, None, complete)
}

/// Bounded live output. Drain the receiver while retaining the Process handle.
/// A full queue stops the owned command with OutputLimit; dropping the stream
/// cancels the command, including silent work. Already received/queued chunks
/// remain available after failure. Output delivery never invokes client code;
/// only the terminal completion callback runs on the reader worker.
/// Per-stream byte order is preserved; cross-stream ordering is not guaranteed.
pub fn spawn_streaming(
    command: String,
    limits: Limits,
    operation: u64,
    complete: impl FnOnce(Result<Output, ServiceFailure>) + Send + 'static,
) -> Result<(Process, OutputStream), ServiceFailure> {
    let (tx, rx) = mpsc::sync_channel(STREAM_QUEUE_CHUNKS);
    let process = spawn_inner(CommandSpec::Shell(command), limits, operation, Some(tx), None, complete)?;
    let subscription = OutputStream { receiver: rx, _process: Process { control: process.control.clone() } };
    Ok((process, subscription))
}

/// Stream output under an app-owned context. Expiration cancels owned work;
/// queued chunks and completion remain scoped until final state-thread delivery.
/// As with `spawn_streaming`, drain the bounded queue while work is active.
pub fn spawn_streaming_scoped(
    context: crate::context::ContextToken,
    command: String,
    limits: Limits,
    operation: u64,
    complete: impl FnOnce(crate::context::Scoped<Result<Output, ServiceFailure>>) + Send + 'static,
) -> Result<(Process, ScopedOutputStream), ServiceFailure> {
    let (tx, rx) = mpsc::sync_channel(STREAM_QUEUE_CHUNKS);
    let reply = context.clone();
    let process =
        spawn_inner(CommandSpec::Shell(command), limits, operation, Some(tx), Some(context.clone()), move |result| {
            complete(reply.wrap(result))
        })?;
    let output = OutputStream { receiver: rx, _process: Process { control: process.control.clone() } };
    Ok((process, ScopedOutputStream { output, context }))
}

/// Bind a task to the current account/page generation. Switching/closing the
/// context cancels it. Forward the Scoped completion to the state owner's queue
/// and call into_current there; stale results never need to enter app state.
pub fn spawn_scoped(
    context: crate::context::ContextToken,
    command: String,
    limits: Limits,
    operation: u64,
    complete: impl FnOnce(crate::context::Scoped<Result<Output, ServiceFailure>>) + Send + 'static,
) -> Result<Process, ServiceFailure> {
    let reply = context.clone();
    spawn_inner(CommandSpec::Shell(command), limits, operation, None, Some(context), move |result| {
        complete(reply.wrap(result))
    })
}

/// Run a trusted absolute executable with literal arguments, without a shell.
/// Context expiry cancels the owned process and scopes its completion.
pub fn spawn_program_scoped(
    context: crate::context::ContextToken,
    executable: std::path::PathBuf,
    arguments: Vec<std::ffi::OsString>,
    limits: Limits,
    operation: u64,
    complete: impl FnOnce(crate::context::Scoped<Result<Output, ServiceFailure>>) + Send + 'static,
) -> Result<Process, ServiceFailure> {
    if !executable.is_absolute() {
        return Err(failure(FailureKind::InvalidInput, operation));
    }
    let reply = context.clone();
    spawn_inner(CommandSpec::Program(executable, arguments), limits, operation, None, Some(context), move |result| {
        complete(reply.wrap(result))
    })
}

enum CommandSpec {
    Shell(String),
    Program(std::path::PathBuf, Vec<std::ffi::OsString>),
}

fn spawn_inner(
    command: CommandSpec,
    limits: Limits,
    operation: u64,
    stream: Option<SyncSender<Chunk>>,
    context: Option<crate::context::ContextToken>,
    complete: impl FnOnce(Result<Output, ServiceFailure>) + Send + 'static,
) -> Result<Process, ServiceFailure> {
    let control = Arc::new(Control {
        cancelled: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        group: Mutex::new(None),
        context,
    });
    let worker = control.clone();
    std::thread::Builder::new()
        .name("msdk-process".into())
        .spawn(move || {
            let result = run(command, limits, operation, &worker, stream.as_ref());
            // Close the stream before reporting completion: clients can drain
            // all queued bytes and then observe disconnection deterministically.
            drop(stream);
            worker.finished.store(true, Ordering::SeqCst);
            complete(result);
        })
        .map_err(|_| failure(FailureKind::Unavailable, operation))?;
    Ok(Process { control })
}

fn failure(kind: FailureKind, operation: u64) -> ServiceFailure {
    ServiceFailure::new(kind, "process.run", "/runtime/process.run", operation)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn run(
    _: CommandSpec,
    _: Limits,
    operation: u64,
    _: &Control,
    _: Option<&SyncSender<Chunk>>,
) -> Result<Output, ServiceFailure> {
    Err(failure(FailureKind::Unavailable, operation))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run(
    command: CommandSpec,
    limits: Limits,
    operation: u64,
    control: &Control,
    stream: Option<&SyncSender<Chunk>>,
) -> Result<Output, ServiceFailure> {
    use std::io::Read;
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    use std::process::{Command, Stdio};
    use std::time::Instant;
    if limits.timeout.is_zero() || limits.output_bytes == 0 || limits.output_bytes > 16 * 1024 * 1024 {
        return Err(failure(FailureKind::InvalidInput, operation));
    }
    if control.is_cancelled() {
        return Err(failure(FailureKind::Cancelled, operation));
    }
    let mut program = match command {
        CommandSpec::Shell(command) => {
            let mut shell = Command::new("/bin/sh");
            shell.arg("-c").arg(command);
            shell
        }
        CommandSpec::Program(executable, arguments) => {
            let mut program = Command::new(executable);
            program.args(arguments);
            program
        }
    };
    let mut child =
        program.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).spawn().map_err(
            |error| {
                failure(
                    match error.kind() {
                        std::io::ErrorKind::PermissionDenied => FailureKind::PermissionDenied,
                        std::io::ErrorKind::InvalidInput => FailureKind::InvalidInput,
                        std::io::ErrorKind::NotFound => FailureKind::Unavailable,
                        _ => FailureKind::Io,
                    },
                    operation,
                )
            },
        )?;
    let pid = child.id() as i32;
    *control.group.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let result = (|| {
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            // SAFETY: owned live pipe descriptors; retain existing status flags.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
                return Err(failure(FailureKind::Io, operation));
            }
        }
        let start = Instant::now();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut total = 0usize;
        loop {
            if control.is_cancelled() {
                return Err(failure(FailureKind::Cancelled, operation));
            }
            if start.elapsed() >= limits.timeout {
                return Err(failure(FailureKind::TimedOut, operation));
            }
            let mut buffer = [0_u8; STREAM_CHUNK_BYTES];
            for (pipe, target, channel) in [
                (&mut stdout as &mut dyn Read, &mut out, Stream::Stdout),
                (&mut stderr as &mut dyn Read, &mut err, Stream::Stderr),
            ] {
                loop {
                    if control.is_cancelled() {
                        return Err(failure(FailureKind::Cancelled, operation));
                    }
                    if start.elapsed() >= limits.timeout {
                        return Err(failure(FailureKind::TimedOut, operation));
                    }
                    match pipe.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if total.saturating_add(count) > limits.output_bytes {
                                return Err(failure(FailureKind::OutputLimit, operation));
                            }
                            deliver(stream, channel, &buffer[..count], operation)?;
                            target.extend_from_slice(&buffer[..count]);
                            total += count;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => return Err(failure(FailureKind::Io, operation)),
                    }
                }
            }
            if out.len().saturating_add(err.len()) > limits.output_bytes {
                return Err(failure(FailureKind::OutputLimit, operation));
            }
            if exited_without_reaping(pid).map_err(|_| failure(FailureKind::Io, operation))? {
                // Descendants must not keep pipe ownership alive after the leader
                // exits. Clean the group and drain its finite buffered bytes next.
                kill_group(pid);
                for (pipe, target, channel) in [
                    (&mut stdout as &mut dyn Read, &mut out, Stream::Stdout),
                    (&mut stderr as &mut dyn Read, &mut err, Stream::Stderr),
                ] {
                    loop {
                        match pipe.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(count) => {
                                if total.saturating_add(count) > limits.output_bytes {
                                    return Err(failure(FailureKind::OutputLimit, operation));
                                }
                                deliver(stream, channel, &buffer[..count], operation)?;
                                target.extend_from_slice(&buffer[..count]);
                                total += count;
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(_) => return Err(failure(FailureKind::Io, operation)),
                        }
                    }
                }
                if out.len().saturating_add(err.len()) > limits.output_bytes {
                    return Err(failure(FailureKind::OutputLimit, operation));
                }
                let output = Output {
                    stdout: String::from_utf8_lossy(&out).into_owned(),
                    stderr: String::from_utf8_lossy(&err).into_owned(),
                };
                // Keep the leader unreaped until all group-directed signals are
                // finished, preventing PID reuse from targeting another process.
                let mut group = control.group.lock().unwrap_or_else(|e| e.into_inner());
                kill_group(pid);
                *group = None;
                let status = child.wait().map_err(|_| failure(FailureKind::Io, operation))?;
                return if status.success() {
                    Ok(output)
                } else {
                    Err(failure(FailureKind::ProcessFailed, operation).with_detail(output.stderr))
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    let mut group = control.group.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pid) = group.take() {
        kill_group(pid);
        let _ = child.wait();
    }
    result
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exited_without_reaping(pid: i32) -> std::io::Result<bool> {
    // SAFETY: initialized siginfo output, live child PID, WNOWAIT preserves its
    // identity until cancellation/group cleanup and wait() are serialized.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT)
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    let child = unsafe { info.si_pid() };
    #[cfg(target_os = "macos")]
    let child = info.si_pid;
    Ok(child == pid)
}

fn deliver(
    stream: Option<&SyncSender<Chunk>>,
    channel: Stream,
    bytes: &[u8],
    operation: u64,
) -> Result<(), ServiceFailure> {
    if let Some(tx) = stream {
        tx.try_send(Chunk { stream: channel, bytes: bytes.to_vec() }).map_err(|error| {
            failure(
                match error {
                    mpsc::TrySendError::Full(_) => FailureKind::OutputLimit,
                    mpsc::TrySendError::Disconnected(_) => FailureKind::Cancelled,
                },
                operation,
            )
        })?;
    }
    Ok(())
}
