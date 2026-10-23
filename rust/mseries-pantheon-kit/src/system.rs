//! Small Linux host CPU and memory snapshots for native system UI.
//!
//! This reads `/proc` on demand and creates no background worker or timer.
//! Use [`poll_system_metrics`] when a view needs periodic updates, and disable
//! it while the view is hidden.

use crate::Polling;
use gtk::gio;
use gtk::gio::prelude::*;
use std::cell::Cell;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

/// CPU usage over the interval since the previous sample, in the range 0–100.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuUsage {
    pub percent: Option<f64>,
}

/// Physical memory use in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryUsage {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

/// One best-effort host metrics snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemSample {
    pub sampled_at: SystemTime,
    pub cpu: CpuUsage,
    pub memory: MemoryUsage,
}

/// On-demand host CPU and memory sampler backed by Linux procfs.
#[derive(Clone, Default)]
pub struct SystemSampler {
    previous_cpu: Rc<Cell<Option<CpuCounters>>>,
}

impl SystemSampler {
    /// Read one CPU and memory snapshot. The first CPU sample has no percentage
    /// because utilization requires a delta; memory is available immediately.
    pub fn sample(&self) -> io::Result<SystemSample> {
        let current_cpu = read_cpu_counters()?;
        let previous_cpu = self.previous_cpu.replace(Some(current_cpu));
        let cpu_percent = previous_cpu.and_then(|previous| {
            let total = current_cpu.total.saturating_sub(previous.total);
            let idle = current_cpu.idle.saturating_sub(previous.idle);
            (total > 0).then(|| {
                (total.saturating_sub(idle) as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
            })
        });

        Ok(SystemSample {
            sampled_at: SystemTime::now(),
            cpu: CpuUsage {
                percent: cpu_percent,
            },
            memory: read_memory_usage()?,
        })
    }
}

/// Create a disabled poller that samples Linux CPU and memory on demand.
pub fn poll_system_metrics(
    interval: Duration,
    mut on_sample: impl FnMut(io::Result<SystemSample>) + 'static,
) -> Polling {
    let sampler = SystemSampler::default();
    Polling::new(interval, move || on_sample(sampler.sample()))
}

struct ResumeWatchInner {
    cancellable: gio::Cancellable,
    connection: RefCell<Option<gio::DBusConnection>>,
    subscription: RefCell<Option<gio::SignalSubscription>>,
    callback: Rc<dyn Fn()>,
}

impl Drop for ResumeWatchInner {
    fn drop(&mut self) {
        self.cancellable.cancel();
        self.subscription.get_mut().take();
        self.connection.get_mut().take();
    }
}

/// Watch logind's `PrepareForSleep` signal and run a callback when the system
/// resumes. The system-bus subscription is asynchronous and best-effort: on
/// hosts without logind or a system bus the watcher stays idle. Dropping it
/// cancels an in-flight connection and removes the signal subscription.
#[derive(Clone)]
pub struct ResumeWatcher {
    _inner: Rc<ResumeWatchInner>,
}

impl ResumeWatcher {
    pub fn new(on_resume: impl Fn() + 'static) -> Self {
        let inner = Rc::new(ResumeWatchInner {
            cancellable: gio::Cancellable::new(),
            connection: RefCell::new(None),
            subscription: RefCell::new(None),
            callback: Rc::new(on_resume),
        });
        let weak = Rc::downgrade(&inner);
        let cancellable = inner.cancellable.clone();
        gio::bus_get(gio::BusType::System, Some(&cancellable), move |result| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let Ok(connection) = result else {
                return;
            };
            let callback = Rc::clone(&inner.callback);
            let subscription = connection.subscribe_to_signal(
                Some("org.freedesktop.login1"),
                Some("org.freedesktop.login1.Manager"),
                Some("PrepareForSleep"),
                Some("/org/freedesktop/login1"),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    if is_resume_signal(signal.parameters) {
                        callback();
                    }
                },
            );
            inner.subscription.replace(Some(subscription));
            inner.connection.replace(Some(connection));
        });
        Self { _inner: inner }
    }
}

fn is_resume_signal(parameters: &gtk::glib::Variant) -> bool {
    parameters
        .get::<(bool,)>()
        .is_some_and(|(preparing_for_sleep,)| !preparing_for_sleep)
}

fn read_cpu_counters() -> io::Result<CpuCounters> {
    let stat = fs::read_to_string("/proc/stat")?;
    let mut fields = stat
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| invalid_data("aggregate CPU counters are unavailable"))?
        .split_whitespace();
    let _label = fields.next();
    let counters: Vec<u64> = fields
        .take(8)
        .map(|field| {
            field
                .parse::<u64>()
                .map_err(|_| invalid_data("invalid CPU counter"))
        })
        .collect::<io::Result<_>>()?;
    if counters.len() < 4 {
        return Err(invalid_data("aggregate CPU counters are incomplete"));
    }

    let total = counters.iter().copied().fold(0_u64, u64::saturating_add);
    let idle = counters[3].saturating_add(counters.get(4).copied().unwrap_or(0));
    Ok(CpuCounters { total, idle })
}

fn read_memory_usage() -> io::Result<MemoryUsage> {
    let meminfo = fs::read_to_string("/proc/meminfo")?;
    let mut total_kib = None;
    let mut available_kib = None;
    for line in meminfo.lines() {
        let (key, value) = line.split_once(':').unwrap_or(("", ""));
        let parsed = value
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok());
        match key {
            "MemTotal" => total_kib = parsed,
            "MemAvailable" => available_kib = parsed,
            _ => {}
        }
    }
    let total_bytes = total_kib
        .ok_or_else(|| invalid_data("total memory is unavailable"))?
        .saturating_mul(1024);
    let available_bytes = available_kib
        .ok_or_else(|| invalid_data("available memory is unavailable"))?
        .saturating_mul(1024)
        .min(total_bytes);

    Ok(MemoryUsage {
        total_bytes,
        used_bytes: total_bytes.saturating_sub(available_bytes),
        available_bytes,
    })
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib::prelude::ToVariant;

    #[test]
    fn resume_watcher_ignores_sleep_and_malformed_signals() {
        assert!(!is_resume_signal(&(true,).to_variant()));
        assert!(!is_resume_signal(&"invalid".to_variant()));
    }

    #[test]
    fn resume_watcher_fires_only_for_the_resume_signal() {
        assert!(is_resume_signal(&(false,).to_variant()));
    }
}
