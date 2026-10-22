//! The workflow from docs/WORKFLOW.md: pinned GitHub repos, their issue backlog (lane, priority and
//! agent labels), "Start on this" threads, and the runner as T3 scheduled tasks. GitHub is reached
//! through `gh api` (the user is logged in with `gh`); the server owns no GitHub token we can read.

use crate::t3::model::{Provider, pick_provider};
use gtk::{gio, glib};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const LANES: [&str; 3] = ["build", "feature", "maintenance"];
pub const PRIORITIES: [&str; 4] = ["p0", "p1", "p2", "p3"];
pub const RUNNER_INTERVAL_MS: u64 = 5 * 60 * 1000;

/// Labels the workflow uses, with the colours they get when created.
pub const LABELS: [(&str, &str, &str); 11] = [
    ("lane:build", "0e8a16", "Production-grade builds, release pipelines, packaging"),
    ("lane:feature", "1d76db", "Roadmap features"),
    ("lane:maintenance", "fbca04", "Repo maintenance: branches, bumps, hygiene, flaky tests"),
    ("p0", "b60205", "Drop everything"),
    ("p1", "d93f0b", "This week"),
    ("p2", "e99695", "Soon"),
    ("p3", "f9d0c4", "Whenever"),
    ("agent:ready", "5319e7", "Specified well enough for an agent to take unattended"),
    ("agent:running", "0052cc", "An agent thread is working on it"),
    ("agent:done", "006b75", "The agent opened a PR"),
    ("agent:blocked", "000000", "Failed twice; needs a human"),
];

pub fn runner_prompt(repo: &str) -> String {
    format!(
        "List open issues labeled agent:ready in {repo}, highest priority first (p0 before p3), lanes in the order \
build > feature > maintenance. Take the first one not labeled agent:running, label it agent:running, work it to a \
PR that closes it, run the repo's checks, then label it agent:done. If it fails twice, label it agent:blocked and \
comment with the error. Never push to main. One running agent per repo unless the issue is labeled parallel-ok."
    )
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PinnedRepo {
    /// `owner/name`
    pub repo: String,
    /// Local checkout (a T3 project's workspace root).
    pub path: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub repos: Vec<PinnedRepo>,
    /// `owner/name#number` → thread id for issues started from the client.
    #[serde(default)]
    pub started: HashMap<String, String>,
    /// repo → scheduled task id of its runner.
    #[serde(default)]
    pub runners: HashMap<String, String>,
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs").join("workflow.json")
}

pub fn load_config() -> Config {
    std::fs::read(config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_config(c: &Config) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(c) {
        let _ = std::fs::write(path, bytes);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub labels: Vec<String>,
    pub updated_at: String,
    pub comments: u64,
}

impl Issue {
    pub fn lane(&self) -> Option<&str> {
        self.labels.iter().find_map(|l| l.strip_prefix("lane:"))
    }
    pub fn priority(&self) -> Option<&str> {
        self.labels.iter().find(|l| PRIORITIES.contains(&l.as_str())).map(String::as_str)
    }
    pub fn agent(&self) -> Option<&str> {
        self.labels.iter().find_map(|l| l.strip_prefix("agent:"))
    }
    /// Sort key: priority first (unlabelled last), then lane order, then number.
    pub fn rank(&self) -> (usize, usize, u64) {
        let p = self.priority().and_then(|p| PRIORITIES.iter().position(|x| *x == p)).unwrap_or(PRIORITIES.len());
        let l = self.lane().and_then(|l| LANES.iter().position(|x| *x == l)).unwrap_or(LANES.len());
        (p, l, self.number)
    }
}

/// Parses `gh api .../issues` output; pull requests (which the issues endpoint also lists) are dropped.
pub fn parse_issues(v: &Value) -> Vec<Issue> {
    let mut out: Vec<Issue> = v
        .as_array()
        .map(|a| a.iter())
        .into_iter()
        .flatten()
        .filter(|i| i.get("pull_request").is_none())
        .map(|i| Issue {
            number: i.get("number").and_then(Value::as_u64).unwrap_or(0),
            title: i.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            body: i.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
            url: i.get("html_url").and_then(Value::as_str).unwrap_or("").to_string(),
            labels: i
                .get("labels")
                .and_then(Value::as_array)
                .map(|ls| ls.iter().filter_map(|l| l.get("name").and_then(Value::as_str).map(str::to_string)).collect())
                .unwrap_or_default(),
            updated_at: i.get("updated_at").and_then(Value::as_str).unwrap_or("").to_string(),
            comments: i.get("comments").and_then(Value::as_u64).unwrap_or(0),
        })
        .collect();
    out.sort_by_key(Issue::rank);
    out
}

/// `owner/name` from a git remote URL.
pub fn repo_from_remote(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(".git");
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    (!owner.is_empty() && !name.is_empty()).then(|| format!("{owner}/{name}"))
}

async fn run(argv: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE);
    if let Some(d) = cwd {
        launcher.set_cwd(d);
    }
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let proc = launcher.spawn(&argv).map_err(|e| e.to_string())?;
    let (out, err) = proc.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    if proc.has_exited() && proc.exit_status() == 0 {
        Ok(out.map(|s| s.to_string()).unwrap_or_default())
    } else {
        Err(err.map(|s| s.to_string()).unwrap_or_default().trim().to_string())
    }
}

/// The GitHub repo of a local checkout, from its `origin` remote.
pub async fn remote_repo(path: &str) -> Option<String> {
    let out = run(&["git", "remote", "get-url", "origin"], Some(Path::new(path))).await.ok()?;
    repo_from_remote(&out)
}

pub async fn gh(args: &[&str]) -> Result<Value, String> {
    let mut argv = vec!["gh", "api"];
    argv.extend_from_slice(args);
    let out = run(&argv, None).await?;
    if out.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&out).map_err(|e| format!("gh api returned something other than JSON: {e}"))
}

pub async fn issues(repo: &str) -> Result<Vec<Issue>, String> {
    let v = gh(&[&format!("repos/{repo}/issues?state=open&per_page=100")]).await?;
    Ok(parse_issues(&v))
}

pub async fn comments(repo: &str, number: u64) -> Result<Vec<(String, String, String)>, String> {
    let v = gh(&[&format!("repos/{repo}/issues/{number}/comments?per_page=50")]).await?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    (
                        c.get("user").and_then(|u| u.get("login")).and_then(Value::as_str).unwrap_or("").to_string(),
                        c.get("created_at").and_then(Value::as_str).unwrap_or("").to_string(),
                        c.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Creates the workflow labels that are missing in the repo (errors for existing ones are ignored).
pub async fn ensure_labels(repo: &str) {
    let existing: Vec<String> = gh(&[&format!("repos/{repo}/labels?per_page=100")])
        .await
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(|l| l.get("name").and_then(Value::as_str).map(str::to_string)).collect())
        })
        .unwrap_or_default();
    for (name, color, description) in LABELS {
        if existing.iter().any(|e| e == name) {
            continue;
        }
        let _ = gh(&[
            "-X",
            "POST",
            &format!("repos/{repo}/labels"),
            "-f",
            &format!("name={name}"),
            "-f",
            &format!("color={color}"),
            "-f",
            &format!("description={description}"),
        ])
        .await;
    }
}

pub async fn add_label(repo: &str, number: u64, label: &str) -> Result<(), String> {
    gh(&["-X", "POST", &format!("repos/{repo}/issues/{number}/labels"), "-f", &format!("labels[]={label}")])
        .await
        .map(|_| ())
}

pub async fn remove_label(repo: &str, number: u64, label: &str) -> Result<(), String> {
    gh(&["-X", "DELETE", &format!("repos/{repo}/issues/{number}/labels/{label}")]).await.map(|_| ())
}

pub async fn comment(repo: &str, number: u64, body: &str) -> Result<(), String> {
    gh(&["-X", "POST", &format!("repos/{repo}/issues/{number}/comments"), "-f", &format!("body={body}")])
        .await
        .map(|_| ())
}

/// The first message of a thread started from an issue.
pub fn issue_prompt(repo: &str, issue: &Issue) -> String {
    let mut p = format!(
        "Work GitHub issue #{} in {repo}: {}\n\n{}\n\n",
        issue.number,
        issue.title,
        if issue.body.trim().is_empty() { "(no description)" } else { issue.body.trim() }
    );
    p.push_str(&format!(
        "When done, open a pull request against the repo's development branch whose description closes #{} and \
carries the handoff summary (what was done, what is open, files touched, decisions). Never push to main.",
        issue.number
    ));
    p
}

/// The provider instance and default model for an unattended task.
pub fn runner_selection(providers: &[Provider]) -> Option<(Provider, String, String)> {
    let (p, reason) = pick_provider(providers)?;
    let model =
        p.models.iter().find(|m| m.is_default == Some(true)).or(p.models.first()).map(|m| m.key().to_string())?;
    Some((p.clone(), model, reason))
}

/// The scheduled-task payload for a repo's runner. `runtimeMode` is the provider's supervised mode:
/// full access stays an explicit per-thread choice.
pub fn runner_task(repo: &str, project_id: &str, provider: &Provider, model: &str, existing_id: Option<&str>) -> Value {
    let mut v = json!({
        "title": format!("Runner: {repo}"),
        "prompt": runner_prompt(repo),
        "enabled": true,
        "schedule": {"type": "interval", "everyMs": RUNNER_INTERVAL_MS},
        "projectId": project_id,
        "workspaceStrategy": {"type": "root"},
        "modelSelection": {"instanceId": provider.instance_id, "model": model},
        "runtimeMode": "auto",
        "interactionMode": "default",
        "createdBy": "user",
        "creationSource": "web",
    });
    if let Some(id) = existing_id {
        v["id"] = json!(id);
    }
    v
}

pub fn now_iso() -> String {
    glib::DateTime::now_utc().and_then(|d| d.format_iso8601()).map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_parse() {
        assert_eq!(
            repo_from_remote("git@github.com:techmore/ElementaryOS_MSeries.git").as_deref(),
            Some("techmore/ElementaryOS_MSeries")
        );
        assert_eq!(repo_from_remote("https://github.com/techmore/rupert").as_deref(), Some("techmore/rupert"));
        assert_eq!(repo_from_remote("https://gitlab.com/x/y"), None);
    }

    #[test]
    fn issues_sort_by_priority_then_lane() {
        let v = json!([
            {"number": 1, "title": "a", "labels": [{"name": "lane:maintenance"}, {"name": "p1"}]},
            {"number": 2, "title": "b", "labels": [{"name": "lane:build"}, {"name": "p1"}]},
            {"number": 3, "title": "pr", "pull_request": {}, "labels": []},
            {"number": 4, "title": "c", "labels": [{"name": "p0"}, {"name": "agent:ready"}]},
            {"number": 5, "title": "d", "labels": []},
        ]);
        let issues = parse_issues(&v);
        assert_eq!(issues.iter().map(|i| i.number).collect::<Vec<_>>(), vec![4, 2, 1, 5]);
        assert_eq!(issues[0].agent(), Some("ready"));
        assert_eq!(issues[1].lane(), Some("build"));
    }

    #[test]
    fn runner_task_is_supervised() {
        let p = Provider { instance_id: "claudeAgent".into(), driver: "claudeAgent".into(), ..Default::default() };
        let t = runner_task("o/r", "pid", &p, "claude-opus-5-5", None);
        assert_eq!(t["runtimeMode"], "auto");
        assert_eq!(t["schedule"]["everyMs"], 300000);
        assert!(t["prompt"].as_str().unwrap().contains("agent:ready"));
    }
}
