//! The parts of the T3 contracts this client reads, decoded leniently: unknown fields and variants are
//! kept as `Value` or skipped so a newer server does not break the client.

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct ProviderModel {
    pub slug: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub label: Option<String>,
    pub aliases: Option<Vec<String>>,
    #[serde(rename = "isDefault")]
    pub is_default: Option<bool>,
}

impl ProviderModel {
    /// What `modelSelection.model` takes.
    pub fn key(&self) -> &str {
        if !self.slug.is_empty() { &self.slug } else { self.id.as_deref().unwrap_or("") }
    }

    pub fn display(&self) -> &str {
        self.label.as_deref().or(self.name.as_deref()).unwrap_or_else(|| self.key())
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Provider {
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    pub driver: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "accentColor")]
    pub accent_color: Option<String>,
    pub enabled: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub status: Value,
    pub auth: Value,
    pub availability: Option<String>,
    pub message: Option<String>,
    pub models: Vec<ProviderModel>,
    #[serde(rename = "supportedRuntimeModes")]
    pub supported_runtime_modes: Option<Vec<String>>,
    #[serde(rename = "nativeSessions")]
    pub native_sessions: Option<Value>,
    #[serde(rename = "usageLimits")]
    pub usage_limits: Option<UsageLimits>,
}

/// One rolling window of a provider's plan: the 5-hour session, the week, or the month.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageWindow {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(rename = "usedPercent")]
    pub used_percent: f64,
    #[serde(rename = "resetsAt")]
    pub resets_at: Option<String>,
    #[serde(rename = "windowDurationMins")]
    pub window_duration_mins: Option<u64>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageLimits {
    #[serde(rename = "checkedAt")]
    pub checked_at: String,
    pub windows: Vec<UsageWindow>,
    pub unavailable: Option<Value>,
    #[serde(rename = "resetCredits")]
    pub reset_credits: Option<Value>,
}

impl Provider {
    pub fn window(&self, kind: &str) -> Option<&UsageWindow> {
        self.usage_limits.as_ref()?.windows.iter().find(|w| w.kind == kind)
    }
}

/// Which provider the next unattended task should use: the lowest session use among the ready
/// providers whose windows are under 90 %, weekly as the tie-break, then the owner's preference
/// (Claude, Codex, Grok, OpenCode) when all are open.
pub fn pick_provider(providers: &[Provider]) -> Option<(&Provider, String)> {
    const PREFERENCE: [&str; 4] = ["claudeAgent", "codex", "grok", "opencode"];
    let rank = |p: &Provider| PREFERENCE.iter().position(|d| *d == p.driver).unwrap_or(PREFERENCE.len());
    let mut open: Vec<&Provider> = providers
        .iter()
        .filter(|p| p.ready())
        .filter(|p| p.usage_limits.as_ref().is_none_or(|u| u.windows.iter().all(|w| w.used_percent < 90.0)))
        .collect();
    if open.is_empty() {
        return None;
    }
    let session = |p: &Provider| p.window("session").map(|w| w.used_percent).unwrap_or(0.0);
    let weekly = |p: &Provider| p.window("weekly").map(|w| w.used_percent).unwrap_or(0.0);
    open.sort_by(|a, b| {
        session(a)
            .partial_cmp(&session(b))
            .unwrap()
            .then(weekly(a).partial_cmp(&weekly(b)).unwrap())
            .then(rank(a).cmp(&rank(b)))
    });
    let best = open[0];
    let busiest = providers
        .iter()
        .filter(|p| p.ready() && p.instance_id != best.instance_id)
        .filter_map(|p| p.window("session").or(p.window("weekly")).map(|w| (p, w)))
        .max_by(|a, b| a.1.used_percent.partial_cmp(&b.1.used_percent).unwrap());
    let reason = match busiest {
        Some((p, w)) => format!(
            "next task → {} ({} {:.0}% session); {} at {:.0}% until {}",
            best.name(),
            if session(best) > 0.0 { "" } else { "no" },
            session(best),
            p.name(),
            w.used_percent,
            crate::look::clock(w.resets_at.as_deref().unwrap_or(""))
        ),
        None => format!("next task → {} ({:.0}% session)", best.name(), session(best)),
    };
    Some((best, reason.replace("( ", "(")))
}

impl Provider {
    pub fn name(&self) -> String {
        self.display_name.clone().unwrap_or_else(|| {
            if self.driver == "claudeAgent" {
                return "Claude".into();
            }
            let mut c = self.driver.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
    }

    pub fn status_text(&self) -> String {
        match &self.status {
            Value::String(s) => s.clone(),
            Value::Object(o) => o.get("status").and_then(Value::as_str).unwrap_or("").to_string(),
            _ => String::new(),
        }
    }

    pub fn auth_text(&self) -> String {
        match &self.auth {
            Value::String(s) => s.clone(),
            Value::Object(o) => o
                .get("status")
                .or_else(|| o.get("state"))
                .or_else(|| o.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        }
    }

    /// Authenticated and usable now.
    pub fn ready(&self) -> bool {
        let auth = self.auth_text();
        self.enabled
            && self.installed
            && self.availability.as_deref() != Some("unavailable")
            && (auth == "authenticated" || auth == "ready" || auth == "ok" || auth == "signed-in")
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct ServerConfig {
    pub cwd: String,
    pub providers: Vec<Provider>,
    pub environment: Value,
    pub settings: Value,
}

#[derive(Deserialize, serde::Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Project {
    pub id: String,
    pub title: String,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    #[serde(rename = "deletedAt")]
    pub deleted_at: Option<String>,
    #[serde(rename = "defaultModelSelection")]
    pub default_model_selection: Option<Value>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct ThreadShell {
    pub id: String,
    #[serde(rename = "projectId")]
    pub project_id: String,
    pub title: String,
    #[serde(rename = "providerInstanceId")]
    pub provider_instance_id: String,
    #[serde(rename = "modelSelection")]
    pub model_selection: Value,
    #[serde(rename = "runtimeMode")]
    pub runtime_mode: String,
    #[serde(rename = "interactionMode")]
    pub interaction_mode: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(rename = "archivedAt")]
    pub archived_at: Option<String>,
    #[serde(rename = "historyOrigin")]
    pub history_origin: Option<Value>,
    #[serde(rename = "activeProviderThreadId")]
    pub active_provider_thread_id: Option<String>,
    /// Everything else the shell row carries (status, pending counts, …).
    #[serde(flatten)]
    pub rest: Value,
}

impl ThreadShell {
    pub fn is_running(&self) -> bool {
        let r = &self.rest;
        r.get("activeRunId").is_some_and(|v| !v.is_null())
            || r.get("status").and_then(Value::as_str).is_some_and(|s| s == "running")
            || r.get("latestRun").and_then(|v| v.get("status")).and_then(Value::as_str).is_some_and(|s| s == "running")
    }

    pub fn pending_requests(&self) -> u64 {
        let r = &self.rest;
        r.get("pendingRuntimeRequests")
            .map(|v| v.as_array().map_or(v.as_u64().unwrap_or(0), |a| a.len() as u64))
            .or_else(|| r.get("pendingRuntimeRequestCount").and_then(Value::as_u64))
            .unwrap_or(0)
    }

    /// The native session uuid this thread was imported from, when it came from Claude Code's history.
    pub fn native_session_id(&self) -> Option<String> {
        let h = self.history_origin.as_ref()?;
        h.get("nativeId")
            .or_else(|| h.get("sessionId"))
            .or_else(|| h.get("ref").and_then(|r| r.get("nativeId")))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

/// `ThreadId` → `ThreadShell` plus the projects, as the shell snapshot carries them.
#[derive(Default, Debug, Clone)]
pub struct Shell {
    pub projects: Vec<Project>,
    pub threads: Vec<ThreadShell>,
}

impl Shell {
    pub fn from_snapshot(snapshot: &Value) -> Shell {
        let mut shell = Shell::default();
        if let Some(projects) = snapshot.get("projects").and_then(Value::as_array) {
            for p in projects {
                let project = p.get("project").unwrap_or(p);
                if let Ok(mut project) = serde_json::from_value::<Project>(project.clone()) {
                    if project.title.is_empty() {
                        project.title = project.workspace_root.rsplit('/').next().unwrap_or("Project").to_string();
                    }
                    shell.projects.push(project);
                }
            }
        }
        for key in ["threads", "activeThreads", "archivedThreads"] {
            if let Some(threads) = snapshot.get(key).and_then(Value::as_array) {
                for t in threads {
                    let thread = t.get("thread").unwrap_or(t);
                    if let Ok(thread) = serde_json::from_value::<ThreadShell>(thread.clone()) {
                        shell.threads.push(thread);
                    }
                }
            }
        }
        shell
    }

    pub fn upsert_thread(&mut self, thread: ThreadShell) {
        match self.threads.iter_mut().find(|t| t.id == thread.id) {
            Some(slot) => *slot = thread,
            None => self.threads.push(thread),
        }
    }

    pub fn remove_thread(&mut self, id: &str) {
        self.threads.retain(|t| t.id != id);
    }

    pub fn upsert_project(&mut self, project: Project) {
        match self.projects.iter_mut().find(|p| p.id == project.id) {
            Some(slot) => *slot = project,
            None => self.projects.push(project),
        }
    }
}

/// One visible item of a thread, flattened from `visibleTurnItems[].item`.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct TurnItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub text: String,
    pub streaming: bool,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "requestKind")]
    pub request_kind: Option<String>,
    pub prompt: Option<String>,
    pub reason: Option<String>,
    pub options: Option<Vec<Value>>,
    pub input: Option<String>,
    pub output: Option<String>,
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i64>,
    #[serde(rename = "fileName")]
    pub file_name: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    #[serde(rename = "diffStr")]
    pub diff_str: Option<String>,
    pub markdown: Option<String>,
    pub pattern: Option<String>,
    pub patterns: Option<Vec<String>>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub steps: Option<Vec<Value>>,
    #[serde(flatten)]
    pub rest: Value,
}

impl TurnItem {
    pub fn is_pending_request(&self) -> bool {
        self.kind == "approval_request" && self.status != "completed" && self.status != "cancelled"
    }
}

#[derive(Default, Debug, Clone)]
pub struct ThreadView {
    pub items: Vec<TurnItem>,
    pub active_run_id: Option<String>,
    pub runs: Vec<Value>,
    /// The thread's checkpoints (restore points), as the projection carries them.
    pub checkpoints: Vec<Value>,
}

impl ThreadView {
    pub fn from_projection(p: &Value) -> ThreadView {
        let mut view = ThreadView::default();
        let visible = p.get("visibleTurnItems").and_then(Value::as_array);
        let raw: Vec<&Value> = match visible {
            Some(v) if !v.is_empty() => v.iter().map(|w| w.get("item").unwrap_or(w)).collect(),
            _ => p.get("turnItems").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default(),
        };
        for r in raw {
            if let Ok(item) = serde_json::from_value::<TurnItem>(r.clone()) {
                view.items.push(item);
            }
        }
        if let Some(c) = p.get("checkpoints").and_then(Value::as_array) {
            view.checkpoints = c.clone();
        }
        if let Some(runs) = p.get("runs").and_then(Value::as_array) {
            view.runs = runs.clone();
            view.active_run_id = runs
                .iter()
                .rev()
                .find(|r| r.get("status").and_then(Value::as_str).is_some_and(|s| s == "running" || s == "pending"))
                .and_then(|r| r.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        view
    }

    pub fn upsert(&mut self, item: TurnItem) {
        match self.items.iter_mut().find(|i| i.id == item.id) {
            Some(slot) => *slot = item,
            None => self.items.push(item),
        }
    }
}

// ---- usage ------------------------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageTotals {
    #[serde(rename = "uncachedInputTokens")]
    pub uncached_input: u64,
    #[serde(rename = "cachedInputTokens")]
    pub cached_input: u64,
    #[serde(rename = "cacheCreationTokens")]
    pub cache_creation: u64,
    #[serde(rename = "outputTokens")]
    pub output: u64,
    #[serde(rename = "reasoningTokens")]
    pub reasoning: u64,
}

impl UsageTotals {
    pub fn add(&mut self, o: &UsageTotals) {
        self.uncached_input += o.uncached_input;
        self.cached_input += o.cached_input;
        self.cache_creation += o.cache_creation;
        self.output += o.output;
        self.reasoning += o.reasoning;
    }
    pub fn input(&self) -> u64 {
        self.uncached_input + self.cached_input + self.cache_creation
    }
    pub fn total(&self) -> u64 {
        self.input() + self.output
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageBucket {
    pub day: String,
    pub provider: String,
    pub model: String,
    pub totals: UsageTotals,
    #[serde(rename = "costUsd")]
    pub cost_usd: f64,
    #[serde(rename = "cacheSavingsUsd")]
    pub cache_savings_usd: f64,
    #[serde(rename = "costSource")]
    pub cost_source: String,
    pub records: u64,
    pub sessions: u64,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageSource {
    pub fingerprint: Value,
    pub status: String,
    #[serde(rename = "scannedFiles")]
    pub scanned_files: u64,
    #[serde(rename = "distinctSessions")]
    pub distinct_sessions: u64,
    pub message: Option<String>,
}

impl UsageSource {
    pub fn provider(&self) -> String {
        self.fingerprint.get("provider").and_then(Value::as_str).unwrap_or("").to_string()
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct UsageSummary {
    #[serde(rename = "sinceDay")]
    pub since_day: String,
    #[serde(rename = "untilDay")]
    pub until_day: String,
    pub buckets: Vec<UsageBucket>,
    pub sources: Vec<UsageSource>,
    pub pricing: Value,
    #[serde(rename = "scanDurationMs")]
    pub scan_duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_provider_decode() {
        let p: Provider = serde_json::from_value(serde_json::json!({
            "instanceId": "claude", "driver": "claude", "enabled": true, "installed": true, "version": "2.1",
            "status": "ready", "auth": {"status": "authenticated"}, "checkedAt": "x", "models": [{"id": "opus", "label": "Opus"}],
            "slashCommands": [], "skills": [], "somethingNew": {"a": 1}
        }))
        .unwrap();
        assert!(p.ready());
        assert_eq!(p.name(), "Claude");
        assert_eq!(p.models[0].display(), "Opus");
        assert_eq!(p.models[0].key(), "opus");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    #[test]
    fn shell_snapshot_decodes() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/shell-snapshot.json")).unwrap();
        let shell = Shell::from_snapshot(&v["snapshot"]);
        assert_eq!(shell.projects.len(), 1);
        assert_eq!(shell.projects[0].title, "Projects");
        assert_eq!(shell.threads.len(), 3);
        let claude = shell.threads.iter().find(|t| t.id.starts_with("import:claudeAgent:")).unwrap();
        assert_eq!(claude.provider_instance_id, "claudeAgent");
        assert!(!claude.is_running());
        assert_eq!(claude.pending_requests(), 0);
        assert!(!claude.title.is_empty());
    }

    #[test]
    fn enrichment_snapshot_keeps_threads() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/shell-snapshot.json")).unwrap();
        let mut shell = Shell::from_snapshot(&v["snapshot"]);
        let enrichment = serde_json::json!({"projects": shell.projects.clone().into_iter().map(|p| serde_json::to_value(p).unwrap()).collect::<Vec<_>>(), "threads": [], "archivedThreads": []});
        let fresh = Shell::from_snapshot(&enrichment);
        assert!(fresh.threads.is_empty());
        for p in fresh.projects {
            shell.upsert_project(p);
        }
        assert_eq!(shell.threads.len(), 3);
    }

    #[test]
    fn thread_snapshot_decodes() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/thread-snapshot.json")).unwrap();
        let view = ThreadView::from_projection(&v["projection"]);
        assert_eq!(view.items.len(), 12);
        assert!(view.items.iter().any(|i| i.kind == "assistant_message"));
        assert!(view.items.iter().all(|i| !i.id.is_empty()));
        assert!(view.active_run_id.is_none());
    }

    #[test]
    fn rotation_prefers_the_emptiest_window() {
        let mk = |driver: &str, session: f64, weekly: f64| Provider {
            instance_id: driver.into(),
            driver: driver.into(),
            enabled: true,
            installed: true,
            auth: serde_json::json!({"status": "authenticated"}),
            usage_limits: Some(UsageLimits {
                windows: vec![
                    UsageWindow {
                        kind: "session".into(),
                        used_percent: session,
                        resets_at: Some("2026-10-22T02:00:00Z".into()),
                        ..Default::default()
                    },
                    UsageWindow { kind: "weekly".into(), used_percent: weekly, ..Default::default() },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let providers = vec![mk("claudeAgent", 94.0, 40.0), mk("codex", 20.0, 45.0), mk("grok", 20.0, 10.0)];
        let (best, reason) = pick_provider(&providers).unwrap();
        assert_eq!(best.driver, "grok", "lowest session, then lowest weekly: {reason}");
        assert!(reason.contains("Claude at 94%"), "{reason}");
        let tied = vec![mk("codex", 10.0, 10.0), mk("claudeAgent", 10.0, 10.0)];
        assert_eq!(pick_provider(&tied).unwrap().0.driver, "claudeAgent", "preference order breaks ties");
        let full = vec![mk("codex", 95.0, 10.0)];
        assert!(pick_provider(&full).is_none());
    }

    #[test]
    fn server_config_decodes() {
        let config: ServerConfig =
            serde_json::from_str(include_str!("../../tests/fixtures/server-config.json")).unwrap();
        assert!(config.providers.len() >= 5);
        let claude = config.providers.iter().find(|p| p.driver == "claudeAgent").unwrap();
        assert_eq!(claude.name(), "Claude");
        assert!(claude.ready());
        assert!(claude.models.iter().any(|m| m.key() == "claude-opus-5-5"));
        let codex = config.providers.iter().find(|p| p.driver == "codex").unwrap();
        assert!(!codex.ready());
        assert_eq!(codex.name(), "Codex");
    }
}
