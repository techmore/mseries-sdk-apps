//! Explicit, on-demand ownership of one CodexBar snapshot request. No polling,
//! authentication or desktop status-host installation is performed here.
use crate::codexbar::{SnapshotReadError, read_snapshot_scoped};
use crate::usage::{ScopedUpdate, UsagePanel};
use futures_channel::oneshot;
use msdk::context::RequestContext;
use msdk::runtime::ServiceFailure;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshState {
    Idle,
    Refreshing,
    Updated,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStart {
    Started,
    Coalesced,
    Closed,
}

struct Inner {
    panel: Rc<UsagePanel>,
    executable: PathBuf,
    max_age_seconds: u64,
    context: RequestContext,
    process: RefCell<Option<msdk::process::Process>>,
    active: Cell<bool>,
    operation: Cell<u64>,
    changed: Box<dyn Fn(RefreshState)>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.context.close();
        self.process.get_mut().take();
    }
}

/// All methods and state notifications run on the GTK/state-owner thread.
/// Keep the monitor alive while the host is open; dropping it cancels owned work.
pub struct CodexBarMonitor {
    inner: Rc<Inner>,
}
impl CodexBarMonitor {
    pub fn new(
        panel: Rc<UsagePanel>,
        executable: PathBuf,
        max_age_seconds: u64,
        changed: impl Fn(RefreshState) + 'static,
    ) -> Self {
        Self {
            inner: Rc::new(Inner {
                panel,
                executable,
                max_age_seconds,
                context: RequestContext::new(),
                process: RefCell::new(None),
                active: Cell::new(false),
                operation: Cell::new(0),
                changed: Box::new(changed),
            }),
        }
    }
    /// Opening does not itself read or schedule work.
    pub fn open(&self) {
        self.inner.active.set(true);
    }
    pub fn is_busy(&self) -> bool {
        self.inner.process.borrow().is_some()
    }
    /// Invalidates queued replies, cancels the owned read and marks retained
    /// values stale. Closing or reopening does not start a read.
    pub fn close(&self) {
        if !self.inner.active.replace(false) {
            return;
        }
        self.inner.context.advance();
        self.inner.process.borrow_mut().take();
        let _ = self.inner.panel.mark_stale(now().unwrap_or(0));
        (self.inner.changed)(RefreshState::Idle);
    }
    /// Explicit single refresh. Duplicate requests coalesce until final delivery.
    pub fn refresh(&self) -> Result<RefreshStart, ServiceFailure> {
        if !self.inner.active.get() {
            return Ok(RefreshStart::Closed);
        }
        if self.is_busy() {
            return Ok(RefreshStart::Coalesced);
        }
        let token = self
            .inner
            .context
            .advance()
            .expect("active monitor owns an open context");
        let operation = self.inner.operation.get().wrapping_add(1);
        self.inner.operation.set(operation);
        let (sender, receiver) = oneshot::channel();
        let process = match read_snapshot_scoped(
            token.clone(),
            self.inner.executable.clone(),
            self.inner.max_age_seconds,
            operation,
            move |reply| {
                let _ = sender.send(reply);
            },
        ) {
            Ok(process) => process,
            Err(error) => {
                let reply = token.wrap(Err(SnapshotReadError::Process(error.clone())));
                let _ = self.inner.panel.update_scoped(reply, now().unwrap_or(0));
                (self.inner.changed)(RefreshState::Failed);
                return Err(error);
            }
        };
        *self.inner.process.borrow_mut() = Some(process);
        let weak = Rc::downgrade(&self.inner);
        let delivery_token = token.clone();
        gtk::glib::spawn_future_local(async move {
            let delivery = receiver.await;
            let Some(inner) = weak.upgrade() else { return };
            if !inner.active.get() || !delivery_token.is_current() {
                return;
            }
            let reply = match delivery {
                Ok(reply) => reply,
                Err(_) => {
                    delivery_token.wrap(Err(SnapshotReadError::Process(ServiceFailure::new(
                        msdk::runtime::FailureKind::Unavailable,
                        "codexbar.snapshot",
                        "/runtime/codexbar.snapshot",
                        operation,
                    ))))
                }
            };
            let time = now();
            let reply = match time {
                Some(now) => reply.map(|result| {
                    result.map(|mut snapshot| {
                        snapshot.refresh_freshness(now, inner.max_age_seconds);
                        snapshot
                    })
                }),
                None => reply.map(|_| Err(SnapshotReadError::Clock)),
            };
            match inner.panel.update_scoped(reply, time.unwrap_or(0)) {
                Ok(ScopedUpdate::Expired) => (),
                Ok(ScopedUpdate::Applied) => {
                    inner.process.borrow_mut().take();
                    (inner.changed)(RefreshState::Updated);
                }
                Ok(ScopedUpdate::ReadFailed(_)) | Err(_) => {
                    inner.process.borrow_mut().take();
                    (inner.changed)(RefreshState::Failed);
                }
            }
        });
        (self.inner.changed)(RefreshState::Refreshing);
        Ok(RefreshStart::Started)
    }
}
fn now() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}
