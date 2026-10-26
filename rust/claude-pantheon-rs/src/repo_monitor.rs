//! Read-only, identity-scoped RepoBar snapshots. Never reads settings, credentials or workspace paths.
use futures_util::future::{Either, select};
use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{collections::HashSet, ffi::OsStr, time::Duration};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const TIMEOUT_SECONDS: u64 = 90;
const FRESH_SECONDS: i64 = 300;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub host: String,
    pub login: String,
    pub fetched_at: i64,
    pub stale: bool,
    /// Deliberately does not contain raw helper diagnostics, which can include sensitive material.
    pub error: Option<String>,
    pub repos: Vec<Repository>,
    pub actions: Actions,
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub name: String,
    pub description: String,
    pub visibility: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    pub open_issues: Option<u64>,
    pub open_pulls: Option<u64>,
    pub ci: String,
    pub release: Option<String>,
    pub release_name: Option<String>,
    pub release_url: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Actions {
    pub checked_repos: Option<u64>,
    pub runs: Vec<Run>,
    pub available: bool,
    pub partial: bool,
    pub observations: Vec<ActionObservation>,
}

#[derive(Clone, Debug)]
pub struct ActionObservation {
    pub repo: String,
    pub state: String,
    pub reason: String,
    pub fetched_at: Option<i64>,
    pub total_count: Option<u64>,
    pub returned_count: Option<u64>,
    pub displayed_count: Option<u64>,
    pub query_limit: Option<u64>,
    pub query_truncated: Option<bool>,
    pub snapshot_truncated: Option<bool>,
}

impl ActionObservation {
    fn unknown(repo: &str) -> Self {
        Self { repo: repo.into(), state: "unknown".into(), reason: "metadata_missing_or_invalid".into(),
            fetched_at: None, total_count: None, returned_count: None, displayed_count: None,
            query_limit: None, query_truncated: None, snapshot_truncated: None }
    }

    fn partial(&self) -> bool {
        !matches!(self.state.as_str(), "success" | "empty")
            || self.total_count.is_none() || self.query_truncated != Some(false)
            || self.snapshot_truncated != Some(false)
    }

    pub fn detail(&self) -> String {
        let status = match self.state.as_str() {
            "empty" => "Actions query succeeded: no workflow runs exist in the response",
            "success" => "Actions query succeeded",
            "error" => "Actions unavailable: query failed or access restricted; check RepoBar/GitHub access and Refresh",
            "not_checked" => "Actions not checked",
            _ => "Actions observation unknown: older helper or missing/invalid metadata; update RepoBar and Refresh",
        };
        let reason = match self.reason.as_str() {
            "hidden" => " · hidden in RepoBar", "archived" => " · archived repository",
            "fork" => " · fork excluded", "inactive" => " · not recently active",
            "repository_limit" => " · beyond the 30-repository query bound",
            "invalid_response" => " · malformed API response", _ => "",
        };
        if !matches!(self.state.as_str(), "success" | "empty") { return format!("{status}{reason}"); }
        format!("{status} · {} fetched / {} total · {} recent runs shown · query limit {}{}{}{}. Recent means running within 6 hours or completed within 2 hours; not all historical runs.",
            count_text(self.returned_count), count_text(self.total_count), count_text(self.displayed_count),
            count_text(self.query_limit),
            if self.query_truncated == Some(true) { " · query truncated" } else { "" },
            if self.query_truncated.is_none() { " · query completeness unknown" } else { "" },
            if self.snapshot_truncated == Some(true) { " · snapshot run cap truncated this repository" } else { "" })
    }
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: u64,
    pub repo: String,
    pub name: String,
    pub branch: Option<String>,
    pub event: Option<String>,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).filter(|s| {
        !s.trim().is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
    }).map(str::to_owned)
}

fn valid_host(host: &str) -> bool {
    host.len() <= 253 && host.contains('.') && host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    })
}

fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 100
        && login.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn valid_repo(name: &str) -> bool {
    let Some((owner, repo)) = name.split_once('/') else { return false };
    valid_login(owner) && !repo.is_empty() && repo.len() <= 100 && repo != "." && repo != ".."
        && repo.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

/// Links must be HTTPS on the snapshot's exact host; never file:, credential URLs, or external sites.
pub fn safe_url(raw: &str, host: &str) -> Option<String> {
    if !valid_host(host) || raw.len() > 4096 || !raw.is_ascii()
        || raw.bytes().any(|c| c.is_ascii_control() || c.is_ascii_whitespace() || b"\\?#".contains(&c))
    {
        return None;
    }
    raw.strip_prefix(&format!("https://{host}/")).map(|_| raw.to_owned())
}

fn link(value: &Value, key: &str, host: &str, repo: &str) -> Option<String> {
    let raw = text(value, key)?;
    let safe = safe_url(&raw, host)?;
    let base = format!("https://{host}/{repo}");
    (safe == base || safe.strip_prefix(&base).is_some_and(|suffix| suffix.starts_with('/'))).then_some(safe)
}

fn timestamp(value: &Value, key: &str) -> Option<String> {
    let raw = text(value, key)?;
    glib::DateTime::from_iso8601(&raw, None).ok().map(|_| raw)
}

fn parse_observation(value: &Value, repo: &str, fetched_at: i64) -> Option<ActionObservation> {
    let state = text(value, "state")?;
    if !["success", "empty", "error", "not_checked"].contains(&state.as_str()) { return None; }
    let reason = text(value, "reason").unwrap_or_default();
    let query_limit = value.get("queryLimit").and_then(Value::as_u64).filter(|n| (1..=100).contains(n))?;
    let snapshot_truncated = value.get("snapshotTruncated").and_then(Value::as_bool)?;
    let fetched = value.get("fetchedAt").and_then(Value::as_i64).filter(|t| *t > 0 && *t <= fetched_at.saturating_add(60));
    let total = value.get("totalCount").and_then(Value::as_u64);
    let returned = value.get("returnedCount").and_then(Value::as_u64);
    let displayed = value.get("displayedCount").and_then(Value::as_u64);
    let truncated = value.get("queryTruncated").and_then(Value::as_bool);
    match state.as_str() {
        "success" | "empty" => {
            let returned = returned?;
            if fetched.is_none() || displayed? > returned || returned > query_limit
                || total.is_some_and(|total| total < returned) { return None; }
            if let Some(total) = total {
                if truncated != Some(total > returned) { return None; }
            } else if truncated.is_some() { return None; }
            if state == "empty" && (total != Some(0) || returned != 0 || snapshot_truncated) { return None; }
            if state == "success" && total == Some(0) { return None; }
        }
        "error" => {
            if fetched.is_none() || !["fetch_failed", "invalid_response"].contains(&reason.as_str()) { return None; }
        }
        "not_checked" => {
            if value.get("fetchedAt").is_some_and(|v| !v.is_null())
                || !["hidden", "archived", "fork", "inactive", "repository_limit"].contains(&reason.as_str()) { return None; }
        }
        _ => return None,
    }
    Some(ActionObservation { repo: repo.into(), state, reason, fetched_at: fetched,
        total_count: total, returned_count: returned, displayed_count: displayed,
        query_limit: Some(query_limit), query_truncated: truncated, snapshot_truncated: Some(snapshot_truncated) })
}

/// Strict envelope and identity validation; absent/malformed optional measurements remain unknown.
pub fn parse(raw: &str, now: i64) -> Result<Snapshot, String> {
    if raw.len() > MAX_BYTES { return Err("RepoBar snapshot exceeds the 8 MiB safety limit".into()); }
    let value: Value = serde_json::from_str(raw).map_err(|_| "RepoBar returned invalid JSON")?;
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported RepoBar snapshot schema; update mseries-repobar".into());
    }
    let host = text(&value, "host").filter(|s| valid_host(s)).ok_or("RepoBar snapshot has no valid host")?;
    let login = text(&value, "account").filter(|s| valid_login(s)).ok_or("RepoBar snapshot has no valid account")?;
    let fetched_at = value.get("fetchedAt").and_then(Value::as_i64)
        .filter(|t| *t > 0 && *t <= now.saturating_add(60)).ok_or("RepoBar snapshot has an invalid fetched time")?;
    let stale = value.get("stale").and_then(Value::as_bool).ok_or("RepoBar snapshot has no valid freshness state")?
        || now.saturating_sub(fetched_at) > FRESH_SECONDS;
    let error = match value.get("error") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(_)) => Some("RepoBar could not refresh or verify this account. Check the RepoBar connection and GitHub sign-in, then Refresh. Last-known data is shown.".into()),
        Some(_) => return Err("RepoBar snapshot has an invalid error state".into()),
    };
    let raw_repos = value.get("repos").and_then(Value::as_array).filter(|r| r.len() <= 1000)
        .ok_or("RepoBar snapshot has an invalid repository list")?;
    let mut repos = Vec::with_capacity(raw_repos.len());
    let mut names = HashSet::with_capacity(raw_repos.len());
    for repo in raw_repos {
        let name = text(repo, "name").filter(|s| valid_repo(s)).ok_or("RepoBar snapshot has an invalid repository identity")?;
        if !names.insert(name.to_ascii_lowercase()) { return Err("RepoBar snapshot has duplicate repository identities".into()); }
        let visibility = match repo.get("repoState") {
            None => "visible".to_owned(),
            Some(Value::String(s)) if ["pinned", "visible", "hidden"].contains(&s.as_str()) => s.clone(),
            _ => return Err("RepoBar snapshot has an invalid repository visibility".into()),
        };
        let ci = text(repo, "ci").filter(|s| ["SUCCESS", "FAILURE", "ERROR", "PENDING", "EXPECTED", "UNKNOWN"].contains(&s.as_str()))
            .unwrap_or_else(|| "UNKNOWN".into());
        repos.push(Repository {
            url: link(repo, "url", &host, &name),
            branch: text(repo, "branch"),
            description: text(repo, "description").unwrap_or_default(),
            visibility, open_issues: repo.get("openIssues").and_then(Value::as_u64),
            open_pulls: repo.get("openPullRequests").and_then(Value::as_u64), ci,
            release: text(repo, "release"), release_name: text(repo, "releaseName"),
            release_url: link(repo, "releaseUrl", &host, &name), release_date: timestamp(repo, "releaseDate"), name,
        });
    }
    let mut actions = Actions::default();
    if let Some(a) = value.get("actions").filter(|a| a.is_object()) {
        actions.available = true;
        actions.checked_repos = a.get("checkedRepos").and_then(Value::as_u64).filter(|n| *n <= 1000);
        actions.partial = actions.checked_repos.is_none();
        let mut ids = HashSet::new();
        for key in ["running", "finished"] {
            let Some(runs) = a.get(key).and_then(Value::as_array).filter(|r| r.len() <= 50) else {
                actions.partial = true;
                continue;
            };
            for run in runs {
                let id = run.get("id").and_then(Value::as_u64).filter(|n| *n > 0);
                let repo = text(run, "repo").filter(|s| names.contains(&s.to_ascii_lowercase()));
                let status = text(run, "status").filter(|s| ["queued", "in_progress", "waiting", "requested", "pending", "completed"].contains(&s.as_str()));
                let (Some(id), Some(repo), Some(status)) = (id, repo, status) else { actions.partial = true; continue };
                if !ids.insert((repo.to_ascii_lowercase(), id)) { actions.partial = true; continue; }
                let branch = text(run, "branch");
                let event = text(run, "event");
                let conclusion = text(run, "conclusion");
                let updated_at = timestamp(run, "updatedAt");
                if branch.is_none() || event.is_none() || updated_at.is_none() || (status == "completed" && conclusion.is_none()) {
                    actions.partial = true;
                }
                actions.runs.push(Run {
                    id, url: link(run, "url", &host, &repo), repo,
                    name: text(run, "name").unwrap_or_else(|| "Workflow".into()), branch, event,
                    status, conclusion, updated_at,
                });
            }
        }
    }
    let metadata = value.get("actions").and_then(|a| a.get("observations"))
        .and_then(Value::as_array).filter(|items| items.len() <= 1000);
    for repo in &repos {
        let mut candidates = metadata.into_iter().flatten().filter(|observation| {
            observation.get("repo").and_then(Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(&repo.name))
        });
        let observation = candidates.next().and_then(|value| {
            if candidates.next().is_some() { None } else { parse_observation(value, &repo.name, fetched_at) }
        }).unwrap_or_else(|| ActionObservation::unknown(&repo.name));
        actions.partial |= observation.partial();
        actions.observations.push(observation);
    }
    Ok(Snapshot { host, login, fetched_at, stale, error, repos, actions })
}

/// Killing the direct child also happens when the async caller cancels/drops this future.
struct ChildGuard { child: gio::Subprocess, completed: bool }
impl Drop for ChildGuard {
    fn drop(&mut self) { if !self.completed { self.child.force_exit(); } }
}

/// Installed helper only, fixed argv, no shell; stderr is intentionally discarded.
pub async fn read_snapshot(force: bool) -> Result<Snapshot, String> {
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE);
    let args: &[&str] = if force { &["mseries-repobar", "--json", "--refresh", "snapshot"] }
        else { &["mseries-repobar", "--json", "snapshot"] };
    let argv: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let child = launcher.spawn(&argv).map_err(|_| "Could not start mseries-repobar. Install the optional RepoBar helper with mseries/install-repobar-wingpanel.sh, then Refresh.".to_string())?;
    let mut guard = ChildGuard { child, completed: false };
    let operation = async {
        let stdout = guard.child.stdout_pipe().ok_or("RepoBar has no output pipe")?;
        let mut bytes = Vec::new();
        loop {
            let chunk = stdout.read_bytes_future(16 * 1024, glib::Priority::DEFAULT).await
                .map_err(|_| "Could not read RepoBar snapshot")?;
            if chunk.is_empty() { break; }
            if bytes.len() + chunk.len() > MAX_BYTES { return Err("RepoBar snapshot exceeds the 8 MiB safety limit".to_owned()); }
            bytes.extend_from_slice(chunk.as_ref());
        }
        guard.child.wait_future().await.map_err(|_| "RepoBar did not finish successfully")?;
        if !guard.child.has_exited() || guard.child.exit_status() != 0 {
            return Err("RepoBar could not read GitHub. Check the helper installation and GitHub sign-in in RepoBar, then Refresh.".into());
        }
        let raw = std::str::from_utf8(&bytes).map_err(|_| "RepoBar returned invalid UTF-8")?;
        parse(raw, crate::look::now_unix())
    };
    let result = match select(Box::pin(operation), Box::pin(glib::timeout_future(Duration::from_secs(TIMEOUT_SECONDS)))).await {
        Either::Left((result, _)) => result,
        Either::Right((_, operation)) => { drop(operation); Err("RepoBar timed out after 90 seconds. Check the connection and Refresh.".into()) }
    };
    // wait_future succeeded only on a successful result; failures intentionally terminate the child too.
    if result.is_ok() { guard.completed = true; }
    result
}

pub fn count_text(count: Option<u64>) -> String {
    count.map(|n| n.to_string()).unwrap_or_else(|| "unavailable".into())
}

pub fn actions_detail(snapshot: &Snapshot) -> String {
    if !snapshot.actions.available {
        "Actions unavailable: missing, restricted, disabled, or failed; no healthy/zero-run inference.".into()
    } else {
        let observations = &snapshot.actions.observations;
        let count = |state: &str| observations.iter().filter(|o| o.state == state).count();
        format!("Actions {} · {} repositories queried · {} successful responses · {} confirmed empty · {} failed/unavailable · {} not checked · {} unknown observations. Queries are limited to 30 recently active repositories and 15 runs each; running/finished lists are capped at 50 each. Default-branch CI is separate. Older-helper unknown observations cannot distinguish quiet from fetch failure.",
            if snapshot.actions.partial { "partial" } else { "snapshot" }, count_text(snapshot.actions.checked_repos),
            count("success"), count("empty"), count("error"), count("not_checked"), count("unknown"))
    }
}

/// Stable identity includes both host and login. Hidden repository preferences are respected.
pub fn rows(snapshot: &Snapshot) -> Vec<Value> {
    let identity = format!("repobar:{}:{}", snapshot.host.to_ascii_lowercase(), snapshot.login.to_ascii_lowercase());
    let state = if snapshot.stale { "stale" } else if snapshot.error.is_some() { "error" } else { "ready" };
    let mut result = vec![json!({"id":format!("{identity}:snapshot"),"title":"GitHub repository overview",
        "detail":format!("{} @ {} · fetched {}{}",snapshot.login,snapshot.host,snapshot.fetched_at,
            snapshot.error.as_ref().map(|e|format!(" · {e}")).unwrap_or_default()),
        "state":state,"source":"mseries-repobar","observed_at":snapshot.fetched_at,
        "host":snapshot.host,"login":snapshot.login,"stale":snapshot.stale,"error":snapshot.error})];
    result.push(json!({"id":format!("{identity}:actions"),"title":"GitHub Actions · branch and PR runs",
        "detail":actions_detail(snapshot),"state":if snapshot.stale {"stale"} else if snapshot.error.is_some() {"error"} else if !snapshot.actions.available {"unavailable"} else if snapshot.actions.partial {"partial"} else {"ready"},
        "source":"mseries-repobar","observed_at":snapshot.fetched_at,"host":snapshot.host,"login":snapshot.login,
        "checked_repos":snapshot.actions.checked_repos,"limited":true,"partial":snapshot.actions.partial}));
    for repo in snapshot.repos.iter().filter(|r| r.visibility != "hidden") {
        let id = format!("{identity}:repo:{}", repo.name.to_ascii_lowercase());
        let unknown = ActionObservation::unknown(&repo.name);
        let observation = snapshot.actions.observations.iter().find(|o| o.repo.eq_ignore_ascii_case(&repo.name)).unwrap_or(&unknown);
        result.push(json!({"id":id,"title":repo.name,
            "detail":format!("{} · issues {} · PRs {} · default branch {} · CI {} · release {} · {}",
                repo.visibility,count_text(repo.open_issues),count_text(repo.open_pulls),repo.branch.as_deref().unwrap_or("unavailable"),repo.ci,repo.release.as_deref().unwrap_or("unavailable / none published"),observation.detail()),
            "state":if state != "ready" {state} else if observation.state == "error" {"error"} else if repo.open_issues.is_none() || repo.open_pulls.is_none() || repo.branch.is_none() || repo.ci == "UNKNOWN" || observation.partial() {"partial"} else {"ready"},
            "source":"mseries-repobar","observed_at":snapshot.fetched_at,"host":snapshot.host,"login":snapshot.login,
            "repo":repo.name,"visibility":repo.visibility,"description":repo.description,"url":repo.url,
            "default_branch":repo.branch,"open_issues":repo.open_issues,"open_pull_requests":repo.open_pulls,
            "default_branch_ci":repo.ci,"release_tag":repo.release,"release_name":repo.release_name,
            "release_url":repo.release_url,"release_date":repo.release_date,
            "actions_state":observation.state,"actions_reason":observation.reason,"actions_observed_at":observation.fetched_at,
            "actions_total_count":observation.total_count,"actions_returned_count":observation.returned_count,
            "actions_displayed_count":observation.displayed_count,"actions_query_limit":observation.query_limit,
            "actions_query_truncated":observation.query_truncated,"actions_snapshot_truncated":observation.snapshot_truncated}));
        for run in snapshot.actions.runs.iter().filter(|run| run.repo.eq_ignore_ascii_case(&repo.name)) {
            result.push(json!({"id":format!("{id}:run:{}",run.id),"title":run.name,
                "detail":format!("{} · branch {} · event {} · {}{}",run.repo,run.branch.as_deref().unwrap_or("unavailable"),run.event.as_deref().unwrap_or("unavailable"),run.status,
                    run.conclusion.as_ref().map(|c|format!(" / {c}")).unwrap_or_default()),
                "state":if state != "ready" {state} else {run.conclusion.as_deref().unwrap_or(&run.status)},
                "source":"mseries-repobar","observed_at":snapshot.fetched_at,"host":snapshot.host,"login":snapshot.login,
                "repo":run.repo,"run_id":run.id,"branch":run.branch,"event":run.event,"status":run.status,"conclusion":run.conclusion,
                "url":run.url,"updated_at":run.updated_at,"scope":"branch_or_pr"}));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        json!({"schemaVersion":1,"host":"github.com","account":"alice","fetchedAt":1000,"stale":false,
            "repos":[{"name":"team/app","repoState":"pinned","url":"https://github.com/team/app",
                "branch":"dev","openIssues":3,"openPullRequests":2,"ci":"SUCCESS"}]})
    }
    fn decode(value: Value) -> Snapshot { parse(&value.to_string(), 1000).unwrap() }
    fn with_observation(observation: Value) -> Value {
        let mut value = fixture();
        value["actions"] = json!({"checkedRepos":1,"running":[],"finished":[],"observations":[observation]});
        value
    }
    fn empty_observation() -> Value {
        json!({"repo":"team/app","state":"empty","reason":"","fetchedAt":1000,"totalCount":0,
            "returnedCount":0,"displayedCount":0,"queryLimit":15,"queryTruncated":false,"snapshotTruncated":false})
    }
    #[test]
    fn observation_metadata_distinguishes_confirmed_empty_failed_skipped_and_legacy_unknown() {
        let empty = decode(with_observation(empty_observation()));
        assert_eq!(empty.actions.observations[0].state, "empty");
        assert!(!empty.actions.partial);
        assert_eq!(rows(&empty)[2]["actions_total_count"], 0);
        assert_eq!(rows(&empty)[2]["actions_state"], "empty");
        assert!(empty.actions.observations[0].detail().contains("query succeeded"));
        let mut error = empty_observation();
        error["state"] = json!("error"); error["reason"] = json!("fetch_failed");
        error["totalCount"] = Value::Null; error["returnedCount"] = Value::Null;
        error["queryTruncated"] = Value::Null;
        error["rawError"] = json!("secret-token-never-display");
        let failed = decode(with_observation(error));
        assert_eq!(rows(&failed)[2]["state"], "error");
        assert_eq!(rows(&failed)[2]["actions_total_count"], Value::Null);
        assert!(!serde_json::to_string(&rows(&failed)).unwrap().contains("secret-token"));
        let mut skipped = empty_observation();
        skipped["state"] = json!("not_checked"); skipped["reason"] = json!("repository_limit");
        skipped["fetchedAt"] = Value::Null; skipped["totalCount"] = Value::Null;
        skipped["returnedCount"] = Value::Null; skipped["queryTruncated"] = Value::Null;
        let skipped = decode(with_observation(skipped));
        assert_eq!(rows(&skipped)[2]["actions_state"], "not_checked");
        assert_eq!(rows(&skipped)[2]["actions_observed_at"], Value::Null);
        assert!(skipped.actions.observations[0].detail().contains("30-repository"));
        let legacy = decode(fixture());
        assert_eq!(rows(&legacy)[2]["actions_state"], "unknown");
        assert_eq!(rows(&legacy)[2]["actions_total_count"], Value::Null);
    }
    #[test]
    fn successful_bounded_queries_preserve_truncation_and_reject_invalid_metadata() {
        let mut observation = empty_observation();
        observation["state"] = json!("success"); observation["totalCount"] = json!(200);
        observation["returnedCount"] = json!(15); observation["displayedCount"] = json!(10);
        observation["queryTruncated"] = json!(true); observation["snapshotTruncated"] = json!(true);
        let snapshot = decode(with_observation(observation.clone()));
        assert!(snapshot.actions.partial);
        assert_eq!(rows(&snapshot)[2]["actions_query_truncated"], true);
        assert_eq!(rows(&snapshot)[2]["actions_snapshot_truncated"], true);
        assert!(snapshot.actions.observations[0].detail().contains("query truncated"));
        for (key, bad) in [("displayedCount",json!(16)),("returnedCount",json!(99)),
            ("queryTruncated",json!(false)),("fetchedAt",json!(99999)),("queryLimit",json!("15"))] {
            let mut invalid = observation.clone(); invalid[key] = bad;
            let snapshot = decode(with_observation(invalid));
            assert_eq!(snapshot.actions.observations[0].state, "unknown");
            assert_eq!(rows(&snapshot)[2]["actions_total_count"], Value::Null);
        }
        let mut stale = with_observation(observation); stale["stale"] = json!(true);
        assert_eq!(rows(&decode(stale))[2]["state"], "stale");
    }
    #[test]
    fn absent_actions_never_means_zero_or_healthy() {
        let s = decode(fixture());
        assert!(!s.actions.available);
        assert_eq!(s.actions.checked_repos, None);
        assert_eq!(rows(&s)[1]["state"], "unavailable");
        assert!(actions_detail(&s).contains("restricted"));
        let mut v = fixture();
        v["actions"] = json!({"checkedRepos":1,"running":[],"finished":[]});
        assert!(actions_detail(&decode(v)).contains("fetch failure"));
    }
    #[test]
    fn stale_and_error_preserve_observed_counts_without_leaking_diagnostics() {
        let mut v = fixture(); v["stale"] = json!(true); v["error"] = json!("secret-token-do-not-display");
        let s = decode(v);
        assert!(s.stale && s.error.is_some());
        assert_eq!(s.repos[0].open_issues, Some(3));
        assert_eq!(rows(&s)[2]["state"], "stale");
        assert!(!serde_json::to_string(&rows(&s)).unwrap().contains("secret-token"));
        assert!(parse(&fixture().to_string(), 1400).unwrap().stale);
    }
    #[test]
    fn host_and_account_are_part_of_every_stable_identity() {
        let a = rows(&decode(fixture()));
        let mut v = fixture(); v["account"] = json!("bob");
        let b = rows(&decode(v.clone())); v["host"] = json!("git.example.com");
        let c = rows(&decode(v));
        for i in 0..a.len() { assert_ne!(a[i]["id"], b[i]["id"]); assert_ne!(b[i]["id"], c[i]["id"]); }
    }
    #[test]
    fn rejects_unsafe_links_and_never_interprets_local_paths() {
        for url in ["file:///etc/passwd", "javascript:alert(1)", "https://github.com@evil.com/team/app", "https://github.com.evil.com/team/app", "https://github.com/team/app\\evil", "https://github.com/team/app\n", "https://github.com/team/app?access_token=secret", "https://github.com:443/team/app"] {
            assert!(safe_url(url, "github.com").is_none());
        }
        let mut v = fixture(); v["repos"][0]["url"] = json!("https://github.com/other/app");
        v["repos"][0]["local"] = json!({"path":"/server/workspace"});
        assert!(decode(v).repos[0].url.is_none());
        assert!(safe_url("https://git.example.com/team/app", "git.example.com").is_some());
    }
    #[test]
    fn partial_or_malformed_counts_remain_unknown_not_zero() {
        let mut v = fixture(); v["repos"][0]["openIssues"] = json!(-1);
        v["repos"][0].as_object_mut().unwrap().remove("openPullRequests");
        v["actions"] = json!({"checkedRepos":"1","running":[{"id":1,"repo":"team/app","status":"in_progress"}]});
        let s = decode(v); assert_eq!(s.repos[0].open_issues, None); assert_eq!(s.repos[0].open_pulls, None);
        assert!(s.actions.partial); assert_eq!(s.actions.checked_repos, None);
        assert_eq!(rows(&s)[2]["open_issues"], Value::Null);
        assert_eq!(rows(&s)[2]["state"], "partial");
        for bad in [json!("3"), json!(2.5), Value::Null, json!(-3)] {
            let mut v = fixture(); v["repos"][0]["openIssues"] = bad;
            assert_eq!(decode(v).repos[0].open_issues, None);
        }
        let mut zero = fixture(); zero["repos"][0]["openIssues"] = json!(0);
        assert_eq!(decode(zero).repos[0].open_issues, Some(0));
    }
    #[test]
    fn rejects_invalid_envelopes_and_preserves_branch_event_runs_separately() {
        for (key, value) in [("schemaVersion",json!(2)),("host",json!("evil.com/path")),("account",json!("")),("fetchedAt",json!(999999)),("repos",json!({})),("stale",json!("false"))] {
            let mut v = fixture(); v[key] = value; assert!(parse(&v.to_string(),1000).is_err());
        }
        let mut v = fixture();
        v["actions"] = json!({"checkedRepos":1,"running":[{"id":42,"repo":"team/app","name":"Build","branch":"feature","event":"pull_request","status":"in_progress","updatedAt":"2026-10-25T10:00:00Z","url":"https://github.com/team/app/actions/runs/42"}],"finished":[]});
        let s = decode(v); assert_eq!(s.repos[0].ci,"SUCCESS"); assert_eq!(s.actions.runs[0].branch.as_deref(),Some("feature"));
        assert_eq!(rows(&s)[3]["event"],"pull_request");
    }
}
