//! Read-only collectors for Activity. Run off the GTK main thread. Every command
//! has a deadline and a stdout cap; commands never pass through a shell.

use crate::activity::{self, Kind, Report, Source, State, Window};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_PAGES: usize = 3;
const MAX_REPOS: usize = 20;
const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const MAX_LOCAL: usize = 500;

#[derive(Clone, Debug)]
pub struct Repo {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub window: Window,
    pub repos: Vec<Repo>,
    pub github: bool,
}

pub trait Runner {
    fn run(&mut self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<String, String>;
}

/// One refresh has a 90-second total budget and a 15-second per-command limit.
/// Reader threads are bounded and are never joined after a timed-out child.
pub struct BoundedRunner {
    deadline: Instant,
}

impl Default for BoundedRunner {
    fn default() -> Self {
        Self { deadline: Instant::now() + Duration::from_secs(90) }
    }
}

impl Runner for BoundedRunner {
    fn run(&mut self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<String, String> {
        let deadline = self.deadline.min(Instant::now() + Duration::from_secs(15));
        if Instant::now() >= deadline {
            return Err("Refresh time budget reached; retry individual sources later".into());
        }
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0").env("GH_PROMPT_DISABLED", "1");
        if let Some(path) = cwd {
            command.current_dir(path);
        }
        let mut child =
            command.spawn().map_err(|_| format!("Could not start {program}; check that it is installed"))?;
        let stdout = child.stdout.take().ok_or("Could not capture command output")?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
            let output = if result.is_err() {
                Err("Could not read command output".to_string())
            } else if bytes.len() as u64 > MAX_OUTPUT {
                Err("Command output exceeded the activity safety limit".to_string())
            } else {
                String::from_utf8(bytes).map_err(|_| "Command output was not UTF-8".to_string())
            };
            let _ = send.send(output);
        });
        let mut output = None;
        loop {
            if let Ok(result) = receive.try_recv() {
                if result.is_err() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return result;
                }
                output = Some(result);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return Err(format!(
                            "{program} did not complete successfully; check access and authentication"
                        ));
                    }
                    return output.unwrap_or_else(|| {
                        receive
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                            .unwrap_or_else(|_| Err("Command output timed out".into()))
                    });
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{program} timed out; last known data may be available"));
                }
            }
        }
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn source(id: &str, label: &str, identity: &str, kind: Kind) -> Source {
    Source {
        kind,
        id: id.into(),
        label: label.into(),
        identity: identity.into(),
        items: Vec::new(),
        state: State::Failed,
        detail: String::new(),
        observed_at: now(),
    }
}

pub fn collect(request: Request, parse_time: fn(&str) -> i64) -> Report {
    collect_with(request, parse_time, &mut BoundedRunner::default())
}

pub fn collect_with(request: Request, parse_time: fn(&str) -> i64, runner: &mut impl Runner) -> Report {
    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for repo in request.repos.iter().take(MAX_REPOS) {
        // Exact explicitly pinned paths only; never recursively discover repositories.
        let key = repo.path.to_string_lossy().into_owned();
        if !seen.insert(key.clone()) {
            continue;
        }
        let mut src = source(&format!("local:{key}"), &format!("Local: {}", repo.name), "", Kind::LocalCommit);
        if !repo.path.is_absolute() || !repo.path.is_dir() {
            src.detail = "Pinned checkout is unavailable; choose an existing absolute path in Repositories".into();
            sources.push(src);
            continue;
        }
        let email = runner.run("git", &["config".into(), "--get".into(), "user.email".into()], Some(&repo.path));
        let email = match email {
            Ok(email) if !email.trim().is_empty() => email.trim().to_string(),
            _ => {
                src.detail = "No readable Git author email in this pinned checkout".into();
                sources.push(src);
                continue;
            }
        };
        src.identity = email.clone();
        let args = [
            "--no-pager".into(),
            "log".into(),
            "--all".into(),
            "--no-merges".into(),
            format!("--max-count={}", MAX_LOCAL + 1),
            "-z".into(),
            "--format=%H%x00%at%x00%ae%x00%s".into(),
        ];
        match runner.run("git", &args, Some(&repo.path)) {
            Ok(raw) => match activity::parse_local(&raw, &repo.name, &email, &request.window) {
                Ok(items) => {
                    let records = raw.split_terminator('\0').count() / 4;
                    src.items = items;
                    src.state = if records > MAX_LOCAL { State::Partial } else { State::Fresh };
                    src.detail = if records > MAX_LOCAL {
                        "Scanned the newest 501 reachable non-merge commits; older history was not scanned".into()
                    } else {
                        "Matching author email across reachable branches; author time, not proof of push".into()
                    };
                }
                Err(error) => src.detail = error,
            },
            Err(error) => src.detail = error,
        }
        sources.push(src);
    }
    if request.repos.len() > MAX_REPOS {
        let mut src = source("local:limit", "Additional pinned checkouts", "", Kind::LocalCommit);
        src.state = State::Partial;
        src.detail = "Only the first 20 pinned checkouts were scanned".into();
        sources.push(src);
    }
    if request.github {
        let args: Vec<String> = ["api", "--hostname", "github.com", "user"].map(str::to_string).to_vec();
        let login = runner.run("gh", &args, None).and_then(|raw| {
            let user: Value = serde_json::from_str(&raw).map_err(|_| "GitHub identity response was invalid")?;
            let login = user["login"].as_str().ok_or("GitHub identity was unavailable")?;
            if login.is_empty() || login.len() > 39 || !login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err("GitHub identity was invalid".into());
            }
            Ok(login.to_string())
        });
        let names: std::collections::BTreeSet<&str> = request
            .repos
            .iter()
            .take(MAX_REPOS)
            .map(|repo| repo.name.as_str())
            .filter(|repo| activity::valid_repo(repo))
            .collect();
        for (kind, label, endpoint, filter) in [
            (Kind::GithubCommit, "GitHub commits", "commits", "author-date"),
            (Kind::MergedPr, "Merged PRs", "issues", "merged"),
            (Kind::ClosedIssue, "Closed authored issues", "issues", "closed"),
        ] {
            let mut src = source(&format!("github:{endpoint}:{filter}"), label, login.as_deref().unwrap_or(""), kind);
            if names.is_empty() {
                src.detail = "Pin a GitHub owner/repository in Repositories to enable this source".into();
            } else if let Ok(login) = &login {
                let repos = names.iter().map(|repo| format!("repo:{repo}")).collect::<Vec<_>>().join(" ");
                src.identity = format!("{login}|{repos}");
                let subtype = match kind {
                    Kind::MergedPr => " is:pr is:merged",
                    Kind::ClosedIssue => " is:issue is:closed",
                    _ => "",
                };
                let query = format!("author:{login} {repos}{subtype} {filter}:>={}", request.window.search_date);
                match fetch_search(runner, endpoint, &query, kind, &request.window, parse_time) {
                    Ok((items, partial)) => {
                        src.items = items;
                        src.state = if partial { State::Partial } else { State::Fresh };
                        src.detail = format!(
                            "{login} · pinned repositories · {}{}",
                            if kind == Kind::GithubCommit {
                                "author time; indexed default branches"
                            } else {
                                "authored by this account"
                            },
                            if partial { "; GitHub results incomplete or capped at three pages" } else { "" }
                        );
                    }
                    Err(error) => src.detail = error,
                }
            } else {
                src.detail = "GitHub unavailable; sign in with gh auth login or disable Include GitHub".into();
            }
            sources.push(src);
        }
    }
    Report { window: request.window, sources }
}

fn fetch_search(
    runner: &mut impl Runner,
    endpoint: &str,
    query: &str,
    kind: Kind,
    window: &Window,
    parse_time: fn(&str) -> i64,
) -> Result<(Vec<activity::Item>, bool), String> {
    let mut items = Vec::new();
    let mut partial = false;
    for page in 1..=MAX_PAGES {
        let args: Vec<String> = [
            "api".into(),
            "--hostname".into(),
            "github.com".into(),
            "--method".into(),
            "GET".into(),
            format!("search/{endpoint}"),
            "-f".into(),
            format!("q={query}"),
            "-f".into(),
            "per_page=100".into(),
            "-f".into(),
            format!("page={page}"),
        ]
        .to_vec();
        let raw = runner.run("gh", &args, None)?;
        let result = activity::parse_search(&raw, kind, window, parse_time)?;
        partial |= result.incomplete;
        items.extend(result.items);
        if result.total <= page * 100 {
            return Ok((items, partial));
        }
    }
    Ok((items, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: Vec<Vec<String>>,
        replies: std::collections::VecDeque<Result<String, String>>,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[String], _: Option<&Path>) -> Result<String, String> {
            self.calls.push(std::iter::once(program.to_string()).chain(args.iter().cloned()).collect());
            self.replies.pop_front().expect("unexpected command")
        }
    }

    fn page(total: usize, incomplete: bool) -> String {
        serde_json::json!({"total_count":total,"incomplete_results":incomplete,"items":[]}).to_string()
    }

    fn window() -> Window {
        Window { key: "fixture".into(), start: 1, end: 200, search_date: "2026-10-25".into() }
    }

    #[test]
    fn search_paginates_and_marks_budget_and_upstream_partial() {
        let mut fake =
            Fake { calls: vec![], replies: [Ok(page(400, false)), Ok(page(400, false)), Ok(page(400, false))].into() };
        assert!(
            fetch_search(&mut fake, "commits", "author:fixture", Kind::GithubCommit, &window(), |_| 100).unwrap().1
        );
        assert_eq!(fake.calls.len(), 3);
        assert!(fake.calls[2].contains(&"page=3".into()));
        assert!(!fake.calls.iter().flatten().any(|a| a == "sh" || a == "bash"));
        let mut fake = Fake { calls: vec![], replies: [Ok(page(0, true))].into() };
        assert!(fetch_search(&mut fake, "commits", "fixture", Kind::GithubCommit, &window(), |_| 100).unwrap().1);
        assert_eq!(fake.calls.len(), 1);
    }

    #[test]
    fn failed_later_page_does_not_claim_complete_or_replace_last_good() {
        let mut fake = Fake { calls: vec![], replies: [Ok(page(150, false)), Err("offline".into())].into() };
        assert!(fetch_search(&mut fake, "commits", "fixture", Kind::GithubCommit, &window(), |_| 100).is_err());
    }

    #[test]
    fn github_is_opt_in_and_empty_configuration_runs_no_command() {
        let mut fake = Fake { calls: vec![], replies: [].into() };
        let report = collect_with(Request { window: window(), repos: vec![], github: false }, |_| 100, &mut fake);
        assert!(report.sources.is_empty());
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn real_fixture_git_round_trips_nul_records_and_exact_author() {
        let root = std::env::temp_dir().join(format!(
            "t3-activity-fixture-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let git = |args: &[&str]| {
            let output = Command::new("git").args(args).current_dir(&root).output().unwrap();
            assert!(output.status.success(), "fixture git failed: {}", String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "--quiet", "--template="]);
        git(&["config", "user.email", "a.b@example.invalid"]);
        git(&["config", "user.name", "Activity Fixture"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("fixture.txt"), "fixture\n").unwrap();
        git(&["add", "fixture.txt"]);
        git(&["-c", "core.hooksPath=/dev/null", "commit", "--quiet", "-m", "one\ttabbed commit"]);
        let mut window = window();
        window.start = now() - 60;
        window.end = now() + 60;
        let report = collect(
            Request { window, repos: vec![Repo { name: "fixture/repo".into(), path: root }], github: false },
            |_| 0,
        );
        assert_eq!(report.sources[0].state, State::Fresh, "{:?}", report.sources[0]);
        assert_eq!(report.items().len(), 1);
        assert_eq!(report.items()[0].title, "one tabbed commit");
        assert!(report.items()[0].url.is_none());
    }

    #[test]
    fn bounded_runner_rejects_exit_failure_even_with_stdout_and_enforces_deadline() {
        // Constant fixture commands only; production collectors invoke git/gh directly.
        let mut runner = BoundedRunner::default();
        assert!(runner.run("sh", &["-c".into(), "printf 'not success'; exit 7".into()], None).is_err());
        let mut runner = BoundedRunner { deadline: Instant::now() + Duration::from_millis(80) };
        let start = Instant::now();
        assert!(runner.run("sleep", &["2".into()], None).unwrap_err().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn source_errors_do_not_make_an_auth_failure_a_proven_account() {
        let mut fake = Fake { calls: vec![], replies: [Err("auth unavailable".into())].into() };
        let report = collect_with(Request { window: window(), repos: vec![], github: true }, |_| 100, &mut fake);
        assert_eq!(report.sources.len(), 3);
        assert!(report.sources.iter().all(|source| source.state == State::Failed && source.identity.is_empty()));
        assert_eq!(fake.calls.len(), 1);
    }
}
