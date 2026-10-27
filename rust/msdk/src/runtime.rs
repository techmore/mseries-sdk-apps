//! Versioned extension contract and privacy-safe runtime failures.
//! Diagnostics contain source identifiers and categories, never service arguments,
//! command text, process output, credentials or backend error payloads.

use serde_json::{Value, json};

pub const SERVICES_API_VERSION: u32 = 1;
pub const MAX_HOST_PROCESSES: usize = 4;

pub fn description() -> Value {
    let limits = crate::process::Limits::default();
    json!({"runtime":"msdk", "rust_package_version":env!("CARGO_PKG_VERSION"),
        "app_description_format":crate::FORMAT_VERSION, "service_manifest_format":crate::vocab::SERVICE_MANIFEST_VERSION,
        "extension_api_version":SERVICES_API_VERSION,
        "diagnostic_schema":"msdk.runtime.diagnostic.v1",
        "typed_failure":"msdk::runtime::ServiceFailure",
        "usage_observation":{"rust_api":"msdk::usage", "status":"candidate-data-semantics-only", "status_host":false},
        "request_context":{"owner":"msdk::context::RequestContext", "token":"ContextToken",
            "delivery":"Scoped::into_current on the state-owner thread", "process_api":"msdk::process::spawn_scoped",
            "gtk_adapter_automatic":false, "gtk_typed_reply":"msdk_gtk::host::ServiceReply::in_context", "streaming_automatic":false},
        "failure_codes":FailureKind::ALL.iter().map(|kind| json!({"code":kind.code(), "recovery":kind.message()})).collect::<Vec<_>>(),
        "extension_negotiation":"Services::api_version is checked before calls/feeds; backend OS/network protocols require their own published contract",
        "process":{"rust_api":"msdk::process::{spawn, Limits}", "scoped_program_api":"msdk::process::spawn_program_scoped", "description_service":"process.run",
            "default_timeout_ms":limits.timeout.as_millis(), "default_combined_output_bytes":limits.output_bytes,
            "host_max_concurrent":MAX_HOST_PROCESSES, "owned_handle_drop":"cancel same-group work and reap leader",
            "streaming":false,
            "rust_streaming":{"api":"msdk::process::spawn_streaming", "scoped_api":"msdk::process::spawn_streaming_scoped", "queue_chunks":crate::process::STREAM_QUEUE_CHUNKS,
                "max_chunk_bytes":crate::process::STREAM_CHUNK_BYTES, "backpressure":"stop with process-output-limit",
                "drop_subscription":"cancel owned command", "partial_output":"already received or queued raw bytes survive terminal failure"},
            "privileged_or_daemon_jobs":"use an OS-owned service adapter"},
        "privacy":{"diagnostic_payloads":false, "private_ui_detail":"ServiceFailure::detail()"}})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Unavailable,
    VersionMismatch,
    PermissionDenied,
    InvalidInput,
    MalformedData,
    Io,
    ProcessFailed,
    TimedOut,
    OutputLimit,
    Cancelled,
    Busy,
}

impl FailureKind {
    pub const ALL: &'static [Self] = &[
        Self::Unavailable,
        Self::VersionMismatch,
        Self::PermissionDenied,
        Self::InvalidInput,
        Self::MalformedData,
        Self::Io,
        Self::ProcessFailed,
        Self::TimedOut,
        Self::OutputLimit,
        Self::Cancelled,
        Self::Busy,
    ];
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "service-unavailable",
            Self::VersionMismatch => "service-version-mismatch",
            Self::PermissionDenied => "service-permission-denied",
            Self::InvalidInput => "service-invalid-input",
            Self::MalformedData => "service-malformed-data",
            Self::Io => "service-io-failure",
            Self::ProcessFailed => "process-failed",
            Self::TimedOut => "process-timeout",
            Self::OutputLimit => "process-output-limit",
            Self::Cancelled => "service-cancelled",
            Self::Busy => "service-busy",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "This service is unavailable. Check its installation and retry.",
            Self::VersionMismatch => "The app and service use incompatible APIs. Update them and retry.",
            Self::PermissionDenied => "Access was denied. Check the service permissions and retry.",
            Self::InvalidInput => "The operation has invalid inputs. Correct them and retry.",
            Self::MalformedData => "The service returned invalid data. Retry or update the service.",
            Self::Io => "The operation could not read or write its data. Retry the operation.",
            Self::ProcessFailed => "The command failed. Check its output and retry.",
            Self::TimedOut => "The command exceeded its time limit and was stopped. Retry when ready.",
            Self::OutputLimit => "The command exceeded its output limit and was stopped.",
            Self::Cancelled => "The operation was cancelled.",
            Self::Busy => "Too many operations are running. Wait for one to finish and retry.",
        }
    }
}

#[derive(Clone)]
pub struct ServiceFailure {
    pub kind: FailureKind,
    pub service: String,
    pub source: String,
    pub operation: u64,
    pub expected_version: Option<u32>,
    pub actual_version: Option<u32>,
    // UI-only detail is intentionally omitted from Debug, Display and diagnostics.
    detail: Option<String>,
}

impl ServiceFailure {
    pub fn new(kind: FailureKind, service: &str, source: &str, operation: u64) -> Self {
        Self {
            kind,
            service: service.into(),
            source: source.into(),
            operation,
            expected_version: None,
            actual_version: None,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub fn diagnostic(&self) -> Value {
        json!({"schema":"msdk.runtime.diagnostic.v1", "code":self.kind.code(),
            "service":self.service, "source":self.source, "operation":self.operation,
            "expected_version":self.expected_version, "actual_version":self.actual_version,
            "message":self.kind.message()})
    }
}

impl std::fmt::Debug for ServiceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostic().fmt(f)
    }
}
impl std::fmt::Display for ServiceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind.message())
    }
}
impl std::error::Error for ServiceFailure {}

pub fn negotiate_service(actual: u32, service: &str, source: &str, operation: u64) -> Result<(), ServiceFailure> {
    if actual == SERVICES_API_VERSION {
        return Ok(());
    }
    let mut failure = ServiceFailure::new(FailureKind::VersionMismatch, service, source, operation);
    failure.expected_version = Some(SERVICES_API_VERSION);
    failure.actual_version = Some(actual);
    Err(failure)
}
