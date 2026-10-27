//! Which GitHub repo and issue (or pull request) a thread belongs to, so the sidebar can group
//! threads by repo, then by issue, and the thread header can show the issue it works on.
//!
//! Sources, strongest first: the server's linked pull request (`linkedPullRequest`,
//! `branchPullRequest`, `pullRequests`), then a number in the thread's branch name
//! (`fix/123-crash`, `issue-123`), then `#123` in the title. The repo comes from the linked pull
//! request or from the project's `origin` remote. Issue titles and states are read with
//! `gh api` (read-only) and cached on disk so the sidebar shows them at once on the next start.

use crate::t3::model::ThreadShell;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Issue,
    PullRequest,
}

/// The issue or pull request a thread is linked to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadLink {
    pub kind: LinkKind,
    pub repo: String,
    pub number: u64,
    pub url: Option<String>,
}

impl ThreadLink {
    pub fn key(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    pub fn web_url(&self) -> String {
        self.url.clone().unwrap_or_else(|| {
            let path = if self.kind == LinkKind::PullRequest { "pull" } else { "issues" };
            format!("https://github.com/{}/{path}/{}", self.repo, self.number)
        })
    }
}

/// `owner/name` from a repository field that may be a slug or a GitHub URL.
fn repo_slug(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(r) = crate::workflow::repo_from_remote(raw) {
        return Some(r);
    }
    let mut parts = raw.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (parts.next().is_none() && ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

fn pull_request(v: &Value) -> Option<ThreadLink> {
    let number = v.get("number").and_then(Value::as_u64).filter(|n| *n > 0)?;
    let url = v.get("url").and_then(Value::as_str).map(str::to_string);
    let repo = v
        .get("repository")
        .and_then(Value::as_str)
        .and_then(repo_slug)
        .or_else(|| url.as_deref().and_then(repo_from_pr_url))?;
    Some(ThreadLink { kind: LinkKind::PullRequest, repo, number, url })
}

fn repo_from_pr_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty()).then(|| format!("{owner}/{name}"))
}

/// A number the branch name points at: `123-fix`, `fix/123-crash`, `issue-123`, `gh_123`, `issues/123`.
pub fn number_from_branch(branch: &str) -> Option<u64> {
    let segments: Vec<&str> = branch.split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        let lower = seg.to_ascii_lowercase();
        if (lower == "issue" || lower == "issues")
            && let Some(n) = segments.get(i + 1).and_then(|s| leading_number(s))
        {
            return Some(n);
        }
        for prefix in ["issue-", "issue_", "issues-", "gh-", "gh_", "i-"] {
            if let Some(rest) = lower.strip_prefix(prefix)
                && let Some(n) = leading_number(rest)
            {
                return Some(n);
            }
        }
        // A segment that is or starts with a number followed by a separator: `123` or `123-fix`.
        if let Some(n) = leading_number(seg) {
            let digits = n.to_string().len();
            let tail = &seg[digits..];
            let dated = digits == 4 && tail.len() >= 3 && tail[1..3].chars().all(|c| c.is_ascii_digit());
            if !dated && (tail.is_empty() || tail.starts_with(['-', '_'])) {
                return Some(n);
            }
        }
    }
    None
}

fn leading_number(s: &str) -> Option<u64> {
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/// `#123` in a title (not part of a word or a colour like `#fff`).
pub fn number_from_title(title: &str) -> Option<u64> {
    let bytes = title.as_bytes();
    for (i, _) in title.match_indices('#') {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let rest = &title[i + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let next = rest[digits.len()..].chars().next();
        if !digits.is_empty() && digits.len() <= 7 && !next.is_some_and(|c| c.is_ascii_alphanumeric()) {
            return digits.parse().ok().filter(|n| *n > 0);
        }
    }
    None
}

pub fn branch(t: &ThreadShell) -> Option<&str> {
    t.rest.get("branch").and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The thread's issue or pull request, given the repo of its project (if that is a GitHub checkout).
pub fn link_for(t: &ThreadShell, project_repo: Option<&str>) -> Option<ThreadLink> {
    for key in ["linkedPullRequest", "branchPullRequest"] {
        if let Some(link) = t.rest.get(key).and_then(pull_request) {
            return Some(link);
        }
    }
    let listed = t.rest.get("pullRequests").and_then(Value::as_array).into_iter().flatten();
    if let Some(link) =
        listed.filter(|p| p.get("source").and_then(Value::as_str) != Some("stack-dismissed")).find_map(pull_request)
    {
        return Some(link);
    }
    let repo = project_repo?.to_string();
    let number = branch(t).and_then(number_from_branch).or_else(|| number_from_title(&t.title))?;
    Some(ThreadLink { kind: LinkKind::Issue, repo, number, url: None })
}

/// The `origin` remote of a local checkout as `owner/name`, read from `.git/config` (worktrees
/// included) without starting a process. `None` when the folder is not a local GitHub checkout.
pub fn repo_of_root(root: &str) -> Option<String> {
    let root = Path::new(root);
    let dot_git = root.join(".git");
    let config = if dot_git.is_dir() {
        dot_git.join("config")
    } else {
        // A worktree: `.git` is a file `gitdir: <main>/.git/worktrees/<name>`, whose `commondir` is the main `.git`.
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        let gitdir = if gitdir.is_absolute() { gitdir } else { root.join(gitdir) };
        let common = std::fs::read_to_string(gitdir.join("commondir"))
            .ok()
            .map(|c| {
                let c = PathBuf::from(c.trim());
                if c.is_absolute() { c } else { gitdir.join(c) }
            })
            .unwrap_or(gitdir);
        common.join("config")
    };
    origin_from_config(&std::fs::read_to_string(config).ok()?)
}

fn origin_from_config(text: &str) -> Option<String> {
    let mut in_origin = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
        } else if in_origin
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "url"
        {
            return crate::workflow::repo_from_remote(v.trim());
        }
    }
    None
}

/// What GitHub says about an issue or pull request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IssueInfo {
    pub title: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub is_pr: bool,
    pub url: String,
    pub fetched_at: i64,
}

impl IssueInfo {
    pub fn from_api(v: &Value, now: i64) -> Option<IssueInfo> {
        let title = v.get("title").and_then(Value::as_str)?.to_string();
        let pr = v.get("pull_request");
        let is_pr = pr.is_some_and(|p| !p.is_null());
        let merged = pr.and_then(|p| p.get("merged_at")).is_some_and(|m| !m.is_null());
        let state = if merged { "merged" } else { v.get("state").and_then(Value::as_str).unwrap_or("open") };
        Some(IssueInfo {
            title,
            state: state.to_string(),
            is_pr,
            url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
            fetched_at: now,
        })
    }

    pub fn state_label(&self) -> &'static str {
        match self.state.as_str() {
            "merged" => "Merged",
            "closed" => "Closed",
            _ => "Open",
        }
    }
}

/// Cached issue details, keyed `owner/name#123`.
#[derive(Default, Serialize, Deserialize)]
pub struct IssueCache {
    #[serde(default)]
    pub issues: HashMap<String, IssueInfo>,
}

/// Re-read an issue after this many seconds when its thread is opened.
pub const ISSUE_TTL: i64 = 10 * 60;

fn cache_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".cache"));
    base.join("claude-pantheon-rs").join("issues.json")
}

impl IssueCache {
    pub fn load() -> IssueCache {
        std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = cache_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = std::fs::write(p, bytes);
        }
    }
}

/// Read one issue or pull request with `gh api` (GitHub's issues endpoint covers both).
pub async fn fetch(link: &ThreadLink, now: i64) -> Result<IssueInfo, String> {
    let path = format!("repos/{}/issues/{}", link.repo, link.number);
    let v = crate::workflow::gh(&[&path]).await?;
    IssueInfo::from_api(&v, now).ok_or_else(|| "unexpected GitHub response".to_string())
}

/// One sidebar group: a repo (or a project without a GitHub remote) and its threads, with the
/// threads that share an issue gathered under it. Thread indices point into the shell's list.
#[derive(Debug, Default, PartialEq)]
pub struct Group {
    /// `repo:owner/name`, or the project id for a folder without a GitHub remote.
    pub key: String,
    pub title: String,
    pub tooltip: String,
    pub project_ids: Vec<String>,
    pub issues: Vec<(ThreadLink, Vec<usize>)>,
    pub loose: Vec<usize>,
}

impl Group {
    pub fn len(&self) -> usize {
        self.loose.len() + self.issues.iter().map(|(_, v)| v.len()).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Groups threads (already sorted and filtered, as indices into `threads`) by repo, then issue.
/// `project` gives a project's (title, workspace root, GitHub repo). Groups keep the order in which
/// their first thread appears; issues likewise.
pub fn group(
    threads: &[ThreadShell],
    order: &[usize],
    project: impl Fn(&str) -> (String, String, Option<String>),
) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for &i in order {
        let t = &threads[i];
        let (title, root, repo) = project(&t.project_id);
        let link = link_for(t, repo.as_deref());
        let repo = link.as_ref().map(|l| l.repo.clone()).or(repo);
        let (key, group_title, tooltip) = match &repo {
            Some(r) => (format!("repo:{r}"), r.clone(), format!("{r}\n{root}")),
            None => (t.project_id.clone(), title, root),
        };
        let at = match groups.iter().position(|g| g.key == key) {
            Some(at) => at,
            None => {
                groups.push(Group { key, title: group_title, tooltip, ..Default::default() });
                groups.len() - 1
            }
        };
        let g = &mut groups[at];
        if !g.project_ids.contains(&t.project_id) {
            g.project_ids.push(t.project_id.clone());
        }
        match link {
            Some(link) => match g.issues.iter_mut().find(|(l, _)| l.key() == link.key()) {
                Some((_, v)) => v.push(i),
                None => g.issues.push((link, vec![i])),
            },
            None => g.loose.push(i),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thread(id: &str, project: &str, title: &str, rest: Value) -> ThreadShell {
        ThreadShell { id: id.into(), project_id: project.into(), title: title.into(), rest, ..Default::default() }
    }

    #[test]
    fn branch_numbers() {
        assert_eq!(number_from_branch("fix/123-crash"), Some(123));
        assert_eq!(number_from_branch("issue-42"), Some(42));
        assert_eq!(number_from_branch("claude/issues/7"), Some(7));
        assert_eq!(number_from_branch("gh_9-x"), Some(9));
        assert_eq!(number_from_branch("88"), Some(88));
        assert_eq!(number_from_branch("claude/t3-native-one-app"), None);
        assert_eq!(number_from_branch("v2-release"), None);
        assert_eq!(number_from_branch("2026-10-27-notes"), None);
    }

    #[test]
    fn title_numbers() {
        assert_eq!(number_from_title("Work GitHub issue #12 in o/r"), Some(12));
        assert_eq!(number_from_title("Fix (#5)"), Some(5));
        assert_eq!(number_from_title("colour #fff and a#3"), None);
        assert_eq!(number_from_title("no number"), None);
    }

    #[test]
    fn links_prefer_the_server_pull_request() {
        let t = thread(
            "t",
            "p",
            "Fix #4",
            json!({"branch": "fix/9-x", "linkedPullRequest": {"projectId": "p", "repository": "o/r", "number": 31, "url": "https://github.com/o/r/pull/31"}}),
        );
        let l = link_for(&t, Some("o/r")).unwrap();
        assert_eq!((l.kind, l.number, l.repo.as_str()), (LinkKind::PullRequest, 31, "o/r"));
        let t = thread("t", "p", "Fix #4", json!({"branch": "fix/9-x"}));
        assert_eq!(link_for(&t, Some("o/r")).unwrap().number, 9);
        let t = thread("t", "p", "Fix #4", json!({"branch": null}));
        assert_eq!(link_for(&t, Some("o/r")).unwrap().number, 4);
        assert_eq!(link_for(&t, None), None);
        let t = thread(
            "t",
            "p",
            "x",
            json!({"pullRequests": [{"repository": "o/r", "number": 2, "url": "u", "source": "stack-dismissed"}, {"url": "https://github.com/a/b/pull/3", "number": 3}]}),
        );
        assert_eq!(link_for(&t, None).unwrap().key(), "a/b#3");
    }

    #[test]
    fn groups_by_repo_then_issue() {
        let threads = vec![
            thread("a", "p1", "Fix #5 header", json!({})),
            thread("b", "p2", "More on #5", json!({})),
            thread("c", "p1", "Loose idea", json!({})),
            thread("d", "p3", "Scratch", json!({})),
        ];
        // p1 and p2 are two checkouts (main and a worktree) of the same repo.
        let groups = group(&threads, &[0, 1, 2, 3], |p| match p {
            "p1" => ("app".into(), "/a".into(), Some("o/app".into())),
            "p2" => ("app-wt".into(), "/b".into(), Some("o/app".into())),
            _ => ("scratch".into(), "/s".into(), None),
        });
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, "o/app");
        assert_eq!(groups[0].project_ids, vec!["p1", "p2"]);
        assert_eq!(groups[0].issues.len(), 1);
        assert_eq!(groups[0].issues[0].1, vec![0, 1]);
        assert_eq!(groups[0].loose, vec![2]);
        assert_eq!(groups[1].key, "p3");
        assert_eq!(groups[1].len(), 1);
    }

    #[test]
    fn origin_parsing_and_issue_state() {
        let cfg = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = git@github.com:x/y.git\n[remote \"origin\"]\n\turl = https://github.com/techmore/mseries-sdk-apps.git\n";
        assert_eq!(origin_from_config(cfg).as_deref(), Some("techmore/mseries-sdk-apps"));
        let v = json!({"title": "T", "state": "closed", "html_url": "h", "pull_request": {"merged_at": "2026-10-27T00:00:00Z"}});
        let i = IssueInfo::from_api(&v, 1).unwrap();
        assert_eq!((i.state.as_str(), i.is_pr, i.state_label()), ("merged", true, "Merged"));
        assert_eq!(IssueInfo::from_api(&json!({"title": "x", "state": "open"}), 1).unwrap().state_label(), "Open");
    }
}
