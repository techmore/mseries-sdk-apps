//! Factual daily development activity, inspired by Shiplog (Joshua Warren, MIT).
//! Pure data layer: no network, commands, credentials, UI or automatic exports.
//! Local and GitHub commits stay separate; neither proves that a release shipped.

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    LocalCommit,
    GithubCommit,
    MergedPr,
    ClosedIssue,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalCommit => "Local commit",
            Self::GithubCommit => "GitHub commit",
            Self::MergedPr => "Merged PR",
            Self::ClosedIssue => "Closed authored issue",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub repo: String,
    pub title: String,
    pub url: Option<String>,
    pub timestamp: i64,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub key: String,
    pub start: i64,
    /// Exclusive next local midnight; generated with calendar arithmetic by the UI.
    pub end: i64,
    /// UTC calendar date at/before the start, for a broad GitHub query.
    pub search_date: String,
}

impl Window {
    pub fn contains(&self, timestamp: i64) -> bool {
        timestamp >= self.start && timestamp < self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Fresh,
    Partial,
    Failed,
    Stale,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "Updated",
            Self::Partial => "Partial",
            Self::Failed => "Unavailable",
            Self::Stale => "Last known data",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    pub kind: Kind,
    pub id: String,
    pub label: String,
    pub identity: String,
    pub items: Vec<Item>,
    pub state: State,
    pub detail: String,
    pub observed_at: i64,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub window: Window,
    pub sources: Vec<Source>,
}

impl Report {
    /// Preserve useful failures only within the same day, source and proven identity.
    /// Auth failures use an empty identity, so cached private GitHub rows never carry
    /// across an unverified account. No history is persisted to disk.
    pub fn retain_last_good(&mut self, previous: &Self) {
        if self.window != previous.window {
            return;
        }
        for source in &mut self.sources {
            if source.state != State::Failed || source.identity.is_empty() {
                continue;
            }
            if let Some(old) = previous
                .sources
                .iter()
                .find(|old| old.id == source.id && old.identity == source.identity && old.state != State::Failed)
            {
                source.items = old.items.clone();
                source.observed_at = old.observed_at;
                source.state = State::Stale;
            }
        }
    }

    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.sources.iter().flat_map(|source| source.items.iter().cloned()).collect();
        items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(a.repo.cmp(&b.repo)).then(a.id.cmp(&b.id)));
        let mut seen = HashSet::new();
        items.retain(|item| seen.insert((item.kind, item.repo.clone(), item.id.clone())));
        items
    }

    pub fn recap(&self) -> String {
        let mut text = format!("# Development activity — {}\n\n", markdown(&self.window.key));
        text.push_str(
            "Local commits and GitHub activity can overlap. These records do not establish a shipped release.\n",
        );
        for source in &self.sources {
            text.push_str(&format!(
                "\n- {}: {}. {}\n",
                markdown(&source.label),
                source.state.label(),
                markdown(&source.detail)
            ));
        }
        let mut groups: BTreeMap<String, Vec<Item>> = BTreeMap::new();
        for item in self.items() {
            groups.entry(item.repo.clone()).or_default().push(item);
        }
        if groups.is_empty() {
            text.push_str("\nNo activity records available. Check source status above.\n");
        }
        for (repo, items) in groups {
            text.push_str(&format!("\n## {}\n", markdown(&repo)));
            for item in items {
                let title = markdown(&item.title);
                let description = format!("{}: {title}", item.kind.label());
                if let Some(url) = item.url.as_deref().and_then(web_url) {
                    text.push_str(&format!("- [{description}](<{url}>)\n"));
                } else {
                    text.push_str(&format!("- {description}\n"));
                }
            }
        }
        text
    }
}

pub fn single_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn markdown(raw: &str) -> String {
    let mut out = String::new();
    for c in single_line(raw).chars() {
        if matches!(c, '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '#' | '!' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Conservative web-link validation. Prevent control characters, Markdown angle
/// delimiter escapes, credentials and scheme confusion; links are never executed.
pub fn web_url(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://"))?;
    let host = rest.split('/').next()?;
    if host.is_empty()
        || host.starts_with('.')
        || host.contains('@')
        || raw.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '\\' | '"' | '\''))
    {
        return None;
    }
    Some(raw.to_string())
}

pub fn valid_repo(repo: &str) -> bool {
    let parts: Vec<&str> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        })
}

pub struct SearchPage {
    pub items: Vec<Item>,
    pub total: usize,
    pub incomplete: bool,
}

/// Parse one GitHub search page; structural errors remain errors, never zeroes.
/// Timestamp parsing is injected so the production collector uses GLib's existing
/// ISO8601 parser and the model remains independent of GTK.
pub fn parse_search(
    raw: &str,
    kind: Kind,
    window: &Window,
    parse_time: impl Fn(&str) -> i64,
) -> Result<SearchPage, String> {
    let value: Value = serde_json::from_str(raw).map_err(|_| "GitHub returned invalid JSON")?;
    let rows = value["items"].as_array().ok_or("GitHub returned no item list")?;
    let total = value["total_count"].as_u64().ok_or("GitHub returned no total count")? as usize;
    let incomplete = value["incomplete_results"].as_bool().ok_or("GitHub returned no completeness status")?;
    let mut items = Vec::new();
    for row in rows {
        let (repo, timestamp, title, id) = match kind {
            Kind::GithubCommit => (
                row["repository"]["full_name"].as_str(),
                row["commit"]["author"]["date"].as_str(),
                row["commit"]["message"].as_str(),
                row["sha"].as_str().map(str::to_string),
            ),
            Kind::MergedPr | Kind::ClosedIssue => (
                row["repository_url"].as_str().and_then(|url| url.strip_prefix("https://api.github.com/repos/")),
                if kind == Kind::MergedPr {
                    row["pull_request"]["merged_at"].as_str()
                } else {
                    row["closed_at"].as_str()
                },
                row["title"].as_str(),
                row["number"].as_u64().map(|n| n.to_string()),
            ),
            Kind::LocalCommit => return Err("Local commits are not a GitHub search".into()),
        };
        let (Some(repo), Some(timestamp), Some(title), Some(id)) = (repo, timestamp, title, id) else {
            return Err("GitHub returned an incomplete activity record".into());
        };
        if !valid_repo(repo) || id.is_empty() {
            return Err("GitHub returned an invalid repository or item identity".into());
        }
        let timestamp = parse_time(timestamp);
        if timestamp <= 0 {
            return Err("GitHub returned an invalid activity time".into());
        }
        if window.contains(timestamp) {
            items.push(Item {
                kind,
                repo: repo.to_string(),
                title: single_line(title.lines().next().unwrap_or_default()),
                url: row["html_url"].as_str().and_then(web_url),
                timestamp,
                id,
            });
        }
    }
    Ok(SearchPage { items, total, incomplete })
}

/// NUL-delimited git records: sha, author epoch, exact author email, subject.
/// Matching the email as data avoids git --author's regex semantics.
pub fn parse_local(raw: &str, repo: &str, email: &str, window: &Window) -> Result<Vec<Item>, String> {
    let fields: Vec<&str> = raw.split_terminator('\0').collect();
    if !fields.len().is_multiple_of(4) {
        return Err("Git returned an incomplete commit record".into());
    }
    let mut items = Vec::new();
    for row in fields.as_chunks::<4>().0 {
        let timestamp = row[1].parse::<i64>().map_err(|_| "Git returned an invalid commit time")?;
        if row[0].is_empty() || !row[0].bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Git returned an invalid commit identity".into());
        }
        if row[2].eq_ignore_ascii_case(email) && window.contains(timestamp) {
            items.push(Item {
                kind: Kind::LocalCommit,
                repo: repo.to_string(),
                title: single_line(row[3]),
                url: None,
                timestamp,
                id: row[0].to_string(),
            });
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        Window { key: "2026-10-25".into(), start: 100, end: 200, search_date: "2026-10-24".into() }
    }

    fn source(state: State, identity: &str) -> Source {
        Source {
            kind: Kind::GithubCommit,
            id: "github:commits".into(),
            label: "GitHub commits".into(),
            identity: identity.into(),
            items: vec![Item {
                kind: Kind::GithubCommit,
                repo: "owner/repo".into(),
                title: "first\n## untrusted".into(),
                url: Some("https://github.com/owner/repo/commit/abc".into()),
                timestamp: 150,
                id: "abc".into(),
            }],
            state,
            detail: "Fixture data".into(),
            observed_at: 160,
        }
    }

    #[test]
    fn local_identity_is_literal_and_day_has_both_bounds() {
        let raw = "abc\0 100\0a.b@example.com\0first\0".replace(" 100", "100");
        assert_eq!(parse_local(&raw, "owner/repo", "a.b@example.com", &window()).unwrap().len(), 1);
        assert!(parse_local(&raw, "owner/repo", "axb@example.com", &window()).unwrap().is_empty());
        assert!(
            parse_local(&raw.replace("100", "200"), "owner/repo", "a.b@example.com", &window()).unwrap().is_empty()
        );
        assert!(parse_local(&raw.replace("100", "99"), "owner/repo", "a.b@example.com", &window()).unwrap().is_empty());
        assert!(parse_local("broken", "owner/repo", "x", &window()).is_err());
    }

    #[test]
    fn search_requires_structural_completeness_and_exact_event_time() {
        let raw = serde_json::json!({"total_count":1,"incomplete_results":false,"items":[{
            "number":2,"title":"PR", "repository_url":"https://api.github.com/repos/a/b",
            "html_url":"https://github.com/a/b/pull/2", "pull_request":{"merged_at":"merged"},
            "closed_at":"closed", "updated_at":"updated"
        }]})
        .to_string();
        let page = parse_search(&raw, Kind::MergedPr, &window(), |s| if s == "merged" { 150 } else { 250 }).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total, 1);
        assert!(!page.incomplete);
        assert!(parse_search(&raw, Kind::ClosedIssue, &window(), |_| 250).unwrap().items.is_empty());
        for bad in ["{}", "{", r#"{"items":[],"total_count":0}"#] {
            assert!(parse_search(bad, Kind::MergedPr, &window(), |_| 150).is_err());
        }
    }

    #[test]
    fn failures_retain_only_same_day_and_proven_account() {
        let old = Report { window: window(), sources: vec![source(State::Fresh, "account-a")] };
        for (identity, state) in [("account-a", State::Stale), ("account-b", State::Failed), ("", State::Failed)] {
            let mut failed = source(State::Failed, identity);
            failed.items.clear();
            let mut new = Report { window: window(), sources: vec![failed] };
            new.retain_last_good(&old);
            assert_eq!(new.sources[0].state, state);
            assert_eq!(new.items().len(), usize::from(state == State::Stale));
        }
        let mut next = old.clone();
        next.window.start += 1;
        next.sources[0].state = State::Failed;
        next.sources[0].items.clear();
        next.retain_last_good(&old);
        assert!(next.items().is_empty());
    }

    #[test]
    fn recap_escapes_untrusted_text_and_destinations_without_merging_sources() {
        let mut src = source(State::Partial, "a");
        src.items[0].url = Some("https://example.org/)\n- injected".into());
        let mut local = src.items[0].clone();
        local.kind = Kind::LocalCommit;
        src.items.push(local);
        let report = Report { window: window(), sources: vec![src] };
        let recap = report.recap();
        assert_eq!(report.items().len(), 2);
        assert!(recap.contains("Partial"));
        assert!(!recap.contains("\n- injected"));
        assert!(!recap.contains("\n## untrusted"));
        assert!(recap.contains("do not establish a shipped release"));
        assert!(web_url("file:///tmp/x").is_none());
        assert!(web_url("https://user:password@host/path").is_none());
        assert!(web_url("https://github.com/a/b/pull/2").is_some());
    }
}
