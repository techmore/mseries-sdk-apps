//! Request generations for a single app-owned account/page/selection context.
//! Context identity remains the app's responsibility; no account data is stored.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Inner(Mutex<Option<ContextToken>>);
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(token) = self.0.get_mut().unwrap_or_else(|error| error.into_inner()) {
            token.active.store(false, Ordering::SeqCst);
        }
    }
}

/// Clones share ownership. The last owner dropping invalidates pending replies;
/// tokens and queued replies do not retain that ownership.
#[derive(Clone)]
pub struct RequestContext(Arc<Inner>);
impl Default for RequestContext {
    fn default() -> Self {
        Self(Arc::new(Inner(Mutex::new(Some(ContextToken::new())))))
    }
}
impl RequestContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn token(&self) -> Option<ContextToken> {
        self.0.0.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    /// Invalidate every pending reply in this context and start a fresh
    /// generation. A closed context cannot reopen; create a new owner instead.
    pub fn advance(&self) -> Option<ContextToken> {
        let mut current = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
        let previous = current.as_ref()?;
        previous.active.store(false, Ordering::SeqCst);
        let next = ContextToken::new();
        *current = Some(next.clone());
        Some(next)
    }

    /// Permanently invalidate the context, including tokens held by workers.
    pub fn close(&self) {
        if let Some(token) = self.0.0.lock().unwrap_or_else(|error| error.into_inner()).take() {
            token.active.store(false, Ordering::SeqCst);
        }
    }
}

#[derive(Clone)]
pub struct ContextToken {
    active: Arc<AtomicBool>,
}
impl ContextToken {
    fn new() -> Self {
        Self { active: Arc::new(AtomicBool::new(true)) }
    }
    pub fn is_current(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    /// Preserve the token until delivery, including time spent in a UI queue.
    pub fn wrap<T>(&self, value: T) -> Scoped<T> {
        Scoped { token: self.clone(), value }
    }
}
impl std::fmt::Debug for ContextToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextToken").field("current", &self.is_current()).finish()
    }
}

/// A reply that must be checked at its final consumer, not on the worker before
/// enqueueing. Debug omits its private value, without requiring T: Debug.
pub struct Scoped<T> {
    token: ContextToken,
    value: T,
}
impl<T> Scoped<T> {
    /// Transform a reply without discarding its final-delivery freshness check.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Scoped<U> {
        Scoped { token: self.token, value: f(self.value) }
    }
    /// Consume on the UI/state owner's thread immediately before applying.
    /// This is a freshness snapshot, not a lock around arbitrary app mutation:
    /// serialize context switches and state application on that same thread.
    pub fn into_current(self) -> Option<T> {
        self.token.is_current().then_some(self.value)
    }
}
impl<T> std::fmt::Debug for Scoped<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scoped").field("current", &self.token.is_current()).finish_non_exhaustive()
    }
}
