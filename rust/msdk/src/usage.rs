//! Candidate usage-monitor data semantics. This module does not collect data,
//! authenticate providers, render a status control, or install a shell host.
//! Callers must use opaque identifiers and keep credentials out of these fields.

/// Stable keys distinguish a provider aggregate from a particular account at
/// a particular source. A remote server is not the same source as a local client.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub provider: String,
    pub source: Source,
    pub account: Option<String>,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("source", &self.source)
            .field("has_account", &self.account.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Source {
    LocalClient,
    ProviderApi,
    Server(String),
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LocalClient => "LocalClient",
            Self::ProviderApi => "ProviderApi",
            Self::Server(_) => "Server(..)",
        })
    }
}

/// A missing value is not an observed zero. A failure may retain a stale value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Loading,
    Empty,
    Unknown,
    Fresh,
    Stale,
    Unavailable,
    Failed,
    Unsupported,
    Expired,
}

/// Percentages use basis points (0 through 10,000) to avoid rounding and
/// ambiguity between used and remaining. Conversion requires explicit units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaUnit {
    Used,
    Remaining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    basis_points: u16,
    unit: QuotaUnit,
    /// Source observation and optional reset in Unix seconds, never receipt time.
    observed_at: i64,
    reset_at: Option<i64>,
}

impl Quota {
    pub fn new(basis_points: u16, unit: QuotaUnit, observed_at: i64, reset_at: Option<i64>) -> Option<Self> {
        (basis_points <= 10_000).then_some(Self { basis_points, unit, observed_at, reset_at })
    }

    pub fn used_basis_points(self) -> u16 {
        match self.unit {
            QuotaUnit::Used => self.basis_points,
            QuotaUnit::Remaining => 10_000 - self.basis_points,
        }
    }

    pub fn remaining_basis_points(self) -> u16 {
        10_000 - self.used_basis_points()
    }

    pub fn observed_at(self) -> i64 {
        self.observed_at
    }

    pub fn reset_at(self) -> Option<i64> {
        self.reset_at
    }

    /// An expired reset is not current capacity, even when the last read was
    /// successful. The caller must obtain a fresh source observation.
    pub fn state_at(self, source_state: State, now: i64) -> State {
        if source_state == State::Fresh && self.reset_at.is_some_and(|reset| reset <= now) {
            State::Expired
        } else {
            source_state
        }
    }
}

/// Source-defined token counts. Cached input and reasoning are subsets of input
/// and output respectively; neither is added again to the total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached_input: Option<u64>,
    pub reasoning: Option<u64>,
}
impl Tokens {
    /// Return no total if either component is unknown or the sum overflows.
    pub fn total(self) -> Option<u64> {
        self.input?.checked_add(self.output?)
    }

    /// Reject contradictions where a declared subset exceeds its known parent.
    pub fn subsets_valid(self) -> bool {
        !matches!((self.cached_input, self.input), (Some(child), Some(parent)) if child > parent)
            && !matches!((self.reasoning, self.output), (Some(child), Some(parent)) if child > parent)
    }
}

/// Coverage describes whether a collector has observed the full scope it
/// promises. Partial is not interchangeable with a complete empty result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    Partial,
    Unknown,
}

/// One provider aggregate or account at one source. Window identifiers are
/// stable within this observation and are not interpreted as account IDs.
#[derive(Clone)]
pub struct Observation {
    pub scope: Scope,
    /// Display labels are provided by the adapter, never inferred from opaque IDs.
    pub provider_label: String,
    pub source_label: String,
    pub account_label: Option<String>,
    pub selected: bool,
    pub state: State,
    pub observed_at: Option<i64>,
    pub windows: Vec<Window>,
    pub tokens: Option<Tokens>,
}

#[derive(Clone)]
pub struct Window {
    pub key: String,
    pub label: String,
    pub quota: Option<Quota>,
}

/// Versioned in-memory boundary, not a wire format or collector protocol.
/// Consumers can distinguish an empty complete collection from incomplete
/// coverage without deduplicating or summing overlapping sources implicitly.
#[derive(Clone)]
pub struct Snapshot {
    pub coverage: Coverage,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    EmptyIdentity,
    InvalidLabel,
    InconsistentProviderLabel,
    DuplicateScope,
    DuplicateWindow,
    FreshWithoutTimestamp,
    InvalidTokenSubset,
}

impl Snapshot {
    pub const VERSION: u32 = 1;

    /// Re-evaluate source age on the state-owner thread immediately before
    /// display. Never promote stale/failed observations to fresh.
    pub fn refresh_freshness(&mut self, now: i64, max_age_seconds: u64) {
        for observation in &mut self.observations {
            if observation.state != State::Fresh {
                continue;
            }
            observation.state = match observation.observed_at {
                None => State::Unknown,
                Some(time) if time > now => State::Unknown,
                Some(time) if now.saturating_sub(time) as u64 > max_age_seconds => State::Stale,
                Some(_) => State::Fresh,
            };
        }
    }

    /// Validate structural facts only. This does not prove provider coverage,
    /// authorize an account, or deduplicate overlapping upstream measurements.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        let mut scopes = std::collections::HashSet::new();
        let mut labels = std::collections::HashMap::new();
        for observation in &self.observations {
            let scope = &observation.scope;
            if scope.provider.trim().is_empty()
                || scope.account.as_ref().is_some_and(|key| key.trim().is_empty())
                || matches!(&scope.source, Source::Server(key) if key.trim().is_empty())
            {
                return Err(SnapshotError::EmptyIdentity);
            }
            if !scopes.insert(scope) {
                return Err(SnapshotError::DuplicateScope);
            }
            if observation.provider_label.trim().is_empty()
                || observation.source_label.trim().is_empty()
                || observation.account_label.as_ref().is_some_and(|label| label.trim().is_empty())
                || scope.account.is_some() != observation.account_label.is_some()
            {
                return Err(SnapshotError::InvalidLabel);
            }
            let group_key = (&scope.provider, &scope.source);
            if let Some(previous) = labels.insert(group_key, (&observation.provider_label, &observation.source_label))
                && previous != (&observation.provider_label, &observation.source_label)
            {
                return Err(SnapshotError::InconsistentProviderLabel);
            }
            if observation.state == State::Fresh && observation.observed_at.is_none() {
                return Err(SnapshotError::FreshWithoutTimestamp);
            }
            if observation.tokens.is_some_and(|tokens| !tokens.subsets_valid()) {
                return Err(SnapshotError::InvalidTokenSubset);
            }
            let mut windows = std::collections::HashSet::new();
            for window in &observation.windows {
                if window.key.trim().is_empty() {
                    return Err(SnapshotError::EmptyIdentity);
                }
                if window.label.trim().is_empty() {
                    return Err(SnapshotError::InvalidLabel);
                }
                if !windows.insert(&window.key) {
                    return Err(SnapshotError::DuplicateWindow);
                }
            }
        }
        Ok(())
    }

    /// Preserve first-seen provider/source order and account order; selected
    /// accounts never filter out other observations. Aggregates stay separate.
    pub fn groups(&self) -> Result<Vec<ProviderGroup<'_>>, SnapshotError> {
        self.validate()?;
        let mut groups: Vec<ProviderGroup<'_>> = Vec::new();
        for observation in &self.observations {
            let group_index = groups.iter().position(|group| {
                group.provider == observation.scope.provider && group.source == &observation.scope.source
            });
            let group = if let Some(index) = group_index {
                &mut groups[index]
            } else {
                groups.push(ProviderGroup {
                    provider: &observation.scope.provider,
                    source: &observation.scope.source,
                    label: &observation.provider_label,
                    source_label: &observation.source_label,
                    aggregate: None,
                    accounts: Vec::new(),
                });
                groups.last_mut().expect("just pushed")
            };
            if observation.scope.account.is_some() {
                group.accounts.push(observation);
            } else {
                group.aggregate = Some(observation);
            }
        }
        Ok(groups)
    }
}

/// A presentation-ready grouping, borrowing the validated snapshot. This is
/// not a widget, a host, or a claim of source deduplication.
pub struct ProviderGroup<'a> {
    pub provider: &'a str,
    pub source: &'a Source,
    pub label: &'a str,
    pub source_label: &'a str,
    pub aggregate: Option<&'a Observation>,
    pub accounts: Vec<&'a Observation>,
}
