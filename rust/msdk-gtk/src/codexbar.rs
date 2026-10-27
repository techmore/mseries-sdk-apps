//! Read-only decoding of CodexBar's v1 provider snapshot. This does not launch
//! CodexBar, poll, authenticate, enumerate accounts or claim full coverage.
use msdk::usage::{
    Coverage, Observation, Quota, QuotaUnit, Scope, Snapshot, Source, State, Window,
};
use serde_json::Value;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDERS: usize = 128;
const MAX_WINDOWS: usize = 128;

/// Deliberately contains no source payload, identifiers, paths or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    TooLarge,
    Malformed,
    UnsupportedVersion,
    InvalidField,
    TooManyItems,
}

/// Public failures contain no command path, stdout, stderr or source payload.
#[derive(Debug)]
pub enum SnapshotReadError {
    Process(msdk::runtime::ServiceFailure),
    Decode(DecodeError),
    Clock,
}

/// Perform one bounded, read-only `--snapshot` request against a trusted
/// absolute executable. No polling or service management is started. The
/// caller retains the returned process and forwards the scoped result to the
/// state-owner thread for `into_current` immediately before applying it.
pub fn read_snapshot_scoped(
    context: msdk::context::ContextToken,
    executable: std::path::PathBuf,
    max_age_seconds: u64,
    operation: u64,
    complete: impl FnOnce(msdk::context::Scoped<Result<Snapshot, SnapshotReadError>>) + Send + 'static,
) -> Result<msdk::process::Process, msdk::runtime::ServiceFailure> {
    let limits = msdk::process::Limits {
        timeout: std::time::Duration::from_secs(5),
        output_bytes: MAX_BYTES,
    };
    msdk::process::spawn_program_scoped(
        context,
        executable,
        vec!["--snapshot".into()],
        limits,
        operation,
        move |reply| {
            complete(reply.map(|result| {
                let output = result.map_err(SnapshotReadError::Process)?;
                // The process API returns lossy text. Reject replacement characters
                // conservatively rather than accepting a potentially corrupted JSON.
                if output.stdout.contains('\u{fffd}') {
                    return Err(SnapshotReadError::Decode(DecodeError::Malformed));
                }
                let elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| SnapshotReadError::Clock)?;
                let now = i64::try_from(elapsed.as_secs()).map_err(|_| SnapshotReadError::Clock)?;
                decode_snapshot(output.stdout.as_bytes(), now, max_age_seconds)
                    .map_err(SnapshotReadError::Decode)
            }))
        },
    )
}

fn optional_text<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, DecodeError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        _ => Err(DecodeError::InvalidField),
    }
}
fn optional_bool(value: &Value, key: &str) -> Result<bool, DecodeError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(DecodeError::InvalidField),
    }
}
fn timestamp(value: Option<&str>) -> Result<Option<i64>, DecodeError> {
    match value {
        None | Some("") => Ok(None),
        Some(text) => gtk::glib::DateTime::from_iso8601(text, None)
            .map(|date| Some(date.to_unix()))
            .map_err(|_| DecodeError::InvalidField),
    }
}
fn provider_label(key: &str) -> &str {
    match key {
        "codex" => "Codex",
        "claude" => "Claude",
        "grok" => "Grok",
        "opencode" | "opencodego" => "OpenCode",
        other => other,
    }
}

/// Decode the provider aggregate view emitted by `codexbar-linux --snapshot`.
/// `now` and `max_age_seconds` describe the consumer's freshness policy; they
/// never replace the source's `updatedAt`. Future timestamps are unknown.
/// An absent/null remaining value stays unknown and is never converted to zero.
pub fn decode_snapshot(
    bytes: &[u8],
    now: i64,
    max_age_seconds: u64,
) -> Result<Snapshot, DecodeError> {
    if bytes.len() > MAX_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| DecodeError::Malformed)?;
    if !root.is_object() {
        return Err(DecodeError::Malformed);
    }
    if root.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err(DecodeError::UnsupportedVersion);
    }
    let entries = root
        .get("entries")
        .and_then(Value::as_array)
        .ok_or(DecodeError::InvalidField)?;
    if entries.len() > MAX_PROVIDERS {
        return Err(DecodeError::TooManyItems);
    }
    let stale = optional_bool(&root, "stale")?;
    let busy = optional_bool(&root, "busy")?;
    let global_error = optional_text(&root, "error")?.is_some_and(|error| !error.is_empty());
    let mut observations = Vec::new();
    for entry in entries {
        if !entry.is_object() {
            return Err(DecodeError::InvalidField);
        }
        let provider = optional_text(entry, "provider")?
            .filter(|key| !key.trim().is_empty())
            .ok_or(DecodeError::InvalidField)?;
        let observed = timestamp(optional_text(entry, "updatedAt")?)?;
        let failed =
            global_error || optional_text(entry, "error")?.is_some_and(|error| !error.is_empty());
        let windows = match entry.get("windows") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(windows)) => windows.as_slice(),
            _ => return Err(DecodeError::InvalidField),
        };
        if windows.len() > MAX_WINDOWS {
            return Err(DecodeError::TooManyItems);
        }
        let state = match observed {
            None if failed => State::Unavailable,
            None if busy => State::Loading,
            None => State::Unknown,
            Some(time) if time > now => State::Unknown,
            Some(_) if failed || stale => State::Stale,
            Some(time) if now.saturating_sub(time) as u64 > max_age_seconds => State::Stale,
            Some(_) if windows.is_empty() => State::Empty,
            Some(_) => State::Fresh,
        };
        let mut decoded_windows = Vec::new();
        for window in windows {
            if !window.is_object() {
                return Err(DecodeError::InvalidField);
            }
            let key = optional_text(window, "key")?
                .filter(|key| !key.trim().is_empty())
                .ok_or(DecodeError::InvalidField)?;
            let label = optional_text(window, "label")?
                .filter(|label| !label.trim().is_empty())
                .ok_or(DecodeError::InvalidField)?;
            let reset = timestamp(optional_text(window, "resetsAt")?)?;
            let remaining = match window.get("remaining") {
                None | Some(Value::Null) => None,
                Some(Value::Number(value)) => {
                    let number = value
                        .as_f64()
                        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
                        .ok_or(DecodeError::InvalidField)?;
                    Some((number * 100.0).round() as u16)
                }
                _ => return Err(DecodeError::InvalidField),
            };
            let quota = observed
                .zip(remaining)
                .and_then(|(time, value)| Quota::new(value, QuotaUnit::Remaining, time, reset));
            decoded_windows.push(Window {
                key: key.into(),
                label: label.into(),
                quota,
            });
        }
        observations.push(Observation {
            scope: Scope {
                provider: provider.into(),
                source: Source::LocalClient,
                account: None,
            },
            provider_label: provider_label(provider).into(),
            source_label: "CodexBar local snapshot".into(),
            account_label: None,
            selected: false,
            state,
            observed_at: observed,
            windows: decoded_windows,
            tokens: None,
        });
    }
    let snapshot = Snapshot {
        coverage: Coverage::Partial,
        observations,
    };
    snapshot.validate().map_err(|_| DecodeError::InvalidField)?;
    Ok(snapshot)
}
