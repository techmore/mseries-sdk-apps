//! Read-only monitoring effects shared by the declarative T3 client.
//! No model turns, collectors, Git mutations, background polling or credential switching.
use crate::t3::{Rpc, model::ServerConfig};
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

type Push = Rc<dyn Fn(Value)>;

#[derive(Default)]
pub struct Monitoring {
    feeds: RefCell<HashMap<String, Push>>,
    usage_generation: Cell<u64>,
    repository_generation: Cell<u64>,
    links: RefCell<HashMap<String, String>>,
    local_generation: Cell<u64>,
    usage_task: RefCell<Option<glib::JoinHandle<()>>>,
    local_task: RefCell<Option<glib::JoinHandle<()>>>,
    repository_task: RefCell<Option<glib::JoinHandle<()>>>,
    page: RefCell<String>,
}

pub fn notice(id: &str, title: &str, detail: &str, state: &str, source: &str) -> Value {
    json!({"id": id, "title": title, "detail": detail, "state": state,
        "source": source, "observed_at": "", "can_open": false})
}

impl Monitoring {
    pub fn register(&self, feed: &str, push: Push) -> bool {
        if !matches!(feed, "t3.accounts" | "t3.history" | "local.usage" | "github.overview") {
            return false;
        }
        push(json!([notice(feed, "Not read yet", "Choose Refresh to read this source.", "idle", feed)]));
        self.feeds.borrow_mut().insert(feed.into(), push);
        true
    }

    fn push(&self, feed: &str, rows: Vec<Value>) {
        let push = self.feeds.borrow().get(feed).cloned();
        if let Some(push) = push {
            push(Value::Array(rows));
        }
    }

    pub fn disconnect(&self) {
        self.usage_generation.set(self.usage_generation.get().wrapping_add(1));
        if let Some(task) = self.usage_task.borrow_mut().take() {
            task.abort();
        }
        for feed in ["t3.accounts", "t3.history"] {
            self.push(
                feed,
                vec![notice(
                    feed,
                    "T3 server unavailable",
                    "Connect to the paired server, then refresh. No current server observation.",
                    "unavailable",
                    "T3 server",
                )],
            );
        }
    }

    pub fn refresh(self: &Rc<Self>, page: &str, rpc: Option<Rpc>, server: String) {
        self.cancel();
        self.page.replace(page.to_string());
        match page {
            "Usage" => self.refresh_usage(rpc, server),
            "Repositories" => self.refresh_repositories(),
            _ => {}
        }
    }

    pub fn cancel(&self) {
        self.usage_generation.set(self.usage_generation.get().wrapping_add(1));
        self.local_generation.set(self.local_generation.get().wrapping_add(1));
        self.repository_generation.set(self.repository_generation.get().wrapping_add(1));
        for slot in [&self.usage_task, &self.local_task, &self.repository_task] {
            if let Some(task) = slot.borrow_mut().take() {
                task.abort();
            }
        }
        self.links.borrow_mut().clear();
    }

    pub fn connected(self: &Rc<Self>, rpc: Rpc, server: String) {
        let page = self.page.borrow().clone();
        if page.is_empty() || page == "Usage" {
            self.refresh("Usage", Some(rpc), server);
        }
    }

    pub async fn open_repository(&self, id: &str) -> Result<(), String> {
        let url = self.links.borrow().get(id).cloned().ok_or("Select a repository with a verified GitHub link")?;
        gio::AppInfo::launch_default_for_uri_future(&url, None::<&gio::AppLaunchContext>)
            .await
            .map_err(|_| "Could not open the repository in the default browser".to_string())
    }

    fn refresh_usage(self: &Rc<Self>, rpc: Option<Rpc>, server: String) {
        let generation = self.usage_generation.get();
        let local_generation = self.local_generation.get();
        self.push(
            "local.usage",
            vec![notice(
                "local",
                "Reading reports…",
                "Files on this computer only; no collector is executed.",
                "loading",
                "This computer",
            )],
        );
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(|| {
                crate::quota::read(&crate::quota::directory(), crate::look::now_unix(), |s| {
                    glib::DateTime::from_iso8601(s, None).ok().map(|d| d.to_unix())
                })
            })
            .await;
            let Some(this) = weak.upgrade().filter(|m| m.local_generation.get() == local_generation) else { return };
            let rows = match result {
                Ok(snapshot) => local_rows(&snapshot),
                Err(_) => vec![notice(
                    "local",
                    "Reports unavailable",
                    "Could not read local reports. Choose Refresh to retry.",
                    "unavailable",
                    "This computer",
                )],
            };
            this.push("local.usage", rows);
        });
        self.local_task.replace(Some(task));
        let Some(rpc) = rpc else {
            self.disconnect_server_rows();
            return;
        };
        for feed in ["t3.accounts", "t3.history"] {
            self.push(feed, vec![notice(feed, "Reading server usage…", &server, "loading", "T3 server")]);
        }
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            let today = match glib::DateTime::now_local() {
                Ok(d) => d,
                Err(_) => return,
            };
            let since = today.add_days(-30).unwrap();
            let format_day = |d: &glib::DateTime| d.format("%Y-%m-%d").unwrap().to_string();
            let payload = json!({"sinceDay":format_day(&since),"untilDay":format_day(&today),"timeZone":today.timezone().identifier().to_string()});
            let (config, history) = futures_util::future::join(
                rpc.call("server.getConfig", json!({})),
                rpc.call("server.getUsageSummary", payload),
            )
            .await;
            let Some(this) = weak.upgrade().filter(|m| m.usage_generation.get() == generation) else { return };
            let accounts = config.ok().and_then(|v| serde_json::from_value::<ServerConfig>(v).ok());
            let mut rows = accounts
                .map(|c| crate::usage_monitor::account_rows(&c.providers, crate::look::now_unix()))
                .unwrap_or_else(|| {
                    vec![notice(
                        "accounts",
                        "Plan windows unavailable",
                        "Server request failed or returned an unsupported shape. Refresh to retry.",
                        "unavailable",
                        "T3 server",
                    )]
                });
            scope_rows(&mut rows, &server);
            this.push("t3.accounts", rows);
            let summary = history.ok().and_then(|v| crate::usage_monitor::decode_history(v).ok());
            let mut rows = summary
                .as_ref()
                .map(|s| crate::usage_monitor::history_rows_at(s, crate::look::now_unix()))
                .unwrap_or_else(|| {
                    vec![notice(
                        "history",
                        "Token history unavailable",
                        "Server request failed or returned an unsupported shape. Refresh to retry.",
                        "unavailable",
                        "T3 server",
                    )]
                });
            scope_rows(&mut rows, &server);
            this.push("t3.history", rows);
        });
        self.usage_task.replace(Some(task));
    }

    fn disconnect_server_rows(&self) {
        for feed in ["t3.accounts", "t3.history"] {
            self.push(
                feed,
                vec![notice(
                    feed,
                    "No T3 server connected",
                    "Local reports remain independent. Pair/connect in the native client, then reconnect here.",
                    "unavailable",
                    "T3 server",
                )],
            );
        }
    }

    fn refresh_repositories(self: &Rc<Self>) {
        let generation = self.repository_generation.get();
        self.links.borrow_mut().clear();
        self.push(
            "github.overview",
            vec![notice(
                "github",
                "Reading repository overview…",
                "Uses RepoBar's selected GitHub host/account; no global authentication switch.",
                "loading",
                "RepoBar",
            )],
        );
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            let result = crate::repo_monitor::read_snapshot(true).await;
            let Some(this) = weak.upgrade().filter(|m| m.repository_generation.get() == generation) else { return };
            let rows = match result {
                Ok(snapshot) => {
                    let mut rows = crate::repo_monitor::rows(&snapshot);
                    let observed = glib::DateTime::from_unix_utc(snapshot.fetched_at)
                        .ok()
                        .and_then(|d| d.format_iso8601().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    for row in &mut rows {
                        row["source"] = json!(format!("RepoBar · {} @ {}", snapshot.login, snapshot.host));
                        row["observed_at"] = json!(observed);
                        row["can_open"] = json!(false);
                        if let (Some(id), Some(url), Some(host)) =
                            (row["id"].as_str(), row["url"].as_str(), row["host"].as_str())
                            && let Some(url) = crate::repo_monitor::safe_url(url, host)
                        {
                            this.links.borrow_mut().insert(id.to_string(), url);
                            row["can_open"] = json!(true);
                        }
                    }
                    rows
                }
                Err(message) => {
                    vec![notice("github", "Repository overview unavailable", &message, "unavailable", "RepoBar")]
                }
            };
            this.push("github.overview", rows);
        });
        self.repository_task.replace(Some(task));
    }
}

fn scope_rows(rows: &mut Vec<Value>, server: &str) {
    if rows.is_empty() {
        rows.push(notice(
            "empty",
            "No usage reported",
            "The server returned no observations for this source.",
            "empty",
            "T3 server",
        ));
    }
    for row in rows {
        row["source"] = json!(format!("T3 server · {server}"));
    }
}

pub fn local_rows(snapshot: &crate::quota::Snapshot) -> Vec<Value> {
    let mut rows: Vec<Value> = snapshot
        .reports
        .iter()
        .map(|report| {
            let r = &report.record;
            let state = if !r.ready {
                "unavailable"
            } else if report.stale {
                "stale"
            } else {
                "fresh"
            };
            let mut detail = format!("{} · observed {} · {}", state, r.updated_at, r.usage_status_text);
            if r.ready {
                for window in &r.limits {
                    detail.push_str(&format!(
                        "\n{}: {:.0}% used{}",
                        window.title,
                        window.percent * 100.0,
                        window.resets_at.as_deref().map(|s| format!(" · resets {s}")).unwrap_or_default()
                    ));
                }
                if let Some(balance) = &r.balance {
                    detail.push_str(&format!(
                        "\n{} {:.2} remaining{}",
                        balance.currency,
                        balance.remaining,
                        if balance.estimated { " (estimated)" } else { "" }
                    ));
                }
            } else {
                detail.push_str("\nCurrent quota unavailable; saved gauges suppressed.");
            }
            json!({"id":r.id,"title":r.name,"detail":detail,"state":state,
            "source":"This computer · subscription report · display only","observed_at":r.updated_at})
        })
        .collect();
    for (index, _) in snapshot.problems.iter().enumerate() {
        rows.push(notice(
            &format!("problem:{index}"),
            "Some reports unavailable",
            "An invalid, unreadable or over-limit local record was skipped; results may be partial.",
            "partial",
            "This computer",
        ));
    }
    if rows.is_empty() {
        rows.push(notice(
            "local-empty",
            "No local subscription reports",
            "Configure a supported collector separately. Refresh only reads existing local files.",
            "empty",
            "This computer",
        ));
    }
    rows
}
