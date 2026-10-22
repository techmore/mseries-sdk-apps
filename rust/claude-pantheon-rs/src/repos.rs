//! The Repos page (pinned GitHub repos with lane counts and the next issue up) and the Issues panel
//! (live list per repo, filters, "Start on this", runner install). Backed by `workflow.rs`.

use crate::app::{App, new_id};
use crate::chat::label;
use crate::look::{iso_to_unix, now_unix, relative_time};
use crate::workflow::{self, Config, Issue, PinnedRepo};
use gtk::{glib, pango, prelude::*};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

pub struct ReposPage {
    root: gtk::Box,
    repo_list: gtk::Box,
    status: gtk::Label,
    issues_box: gtk::Box,
    issues_title: gtk::Label,
    lane_dd: gtk::DropDown,
    prio_dd: gtk::DropDown,
    agent_dd: gtk::DropDown,
    config: RefCell<Config>,
    issues: RefCell<Vec<Issue>>,
    current_repo: RefCell<Option<PinnedRepo>>,
}

fn pill(text: &str, class: &str) -> gtk::Label {
    let l = label(text, &["caption", "pill", class]);
    l.set_halign(gtk::Align::Start);
    l
}

impl ReposPage {
    pub fn new() -> ReposPage {
        let status = label("", &["dim-label"]);
        status.set_margin_start(24);
        status.set_margin_top(12);
        let repo_list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        repo_list.set_margin_start(24);
        repo_list.set_margin_end(24);
        repo_list.set_margin_top(12);

        let issues_title = label("", &["title-4"]);
        issues_title.set_hexpand(true);
        let lane_dd = gtk::DropDown::from_strings(&["All lanes", "build", "feature", "maintenance"]);
        let prio_dd = gtk::DropDown::from_strings(&["All priorities", "p0", "p1", "p2", "p3"]);
        let agent_dd = gtk::DropDown::from_strings(&["Any agent label", "ready", "running", "done", "blocked", "none"]);
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        filters.set_margin_start(24);
        filters.set_margin_end(24);
        filters.set_margin_top(12);
        filters.append(&issues_title);
        filters.append(&lane_dd);
        filters.append(&prio_dd);
        filters.append(&agent_dd);
        let issues_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        issues_box.set_margin_start(24);
        issues_box.set_margin_end(24);
        issues_box.set_margin_top(8);
        issues_box.set_margin_bottom(18);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&status);
        column.append(&repo_list);
        column.append(&filters);
        column.append(&issues_box);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&column)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroll);
        ReposPage {
            root,
            repo_list,
            status,
            issues_box,
            issues_title,
            lane_dd,
            prio_dd,
            agent_dd,
            config: RefCell::new(workflow::load_config()),
            issues: RefCell::new(Vec::new()),
            current_repo: RefCell::new(None),
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn config(&self) -> Config {
        self.config.borrow().clone()
    }

    pub fn issue_count(&self) -> usize {
        self.issues.borrow().len()
    }

    /// The issues last loaded, with their repo (for the command palette).
    pub fn cached_issues(&self) -> Vec<(String, Issue)> {
        let repo = self.current_repo.borrow().as_ref().map(|r| r.repo.clone());
        match repo {
            Some(r) => self.issues.borrow().iter().map(|i| (r.clone(), i.clone())).collect(),
            None => Vec::new(),
        }
    }

    fn save(&self, f: impl FnOnce(&mut Config)) {
        let mut c = self.config.borrow_mut();
        f(&mut c);
        workflow::save_config(&c);
    }

    /// The thread started for an issue, if the client started one.
    pub fn thread_for_issue(&self, key: &str) -> Option<String> {
        self.config.borrow().started.get(key).cloned()
    }

    /// `owner/name#number` for a thread the client started from an issue.
    pub fn issue_for_thread(&self, thread_id: &str) -> Option<String> {
        self.config.borrow().started.iter().find(|(_, t)| *t == thread_id).map(|(k, _)| k.clone())
    }

    /// Lists the server's projects that are GitHub checkouts, with pin toggles, then the pinned repos' backlogs.
    pub fn load(&self, app: Rc<App>) {
        self.status.set_text("Reading repositories…");
        let projects = app.projects();
        let page = app.repos_page_rc();
        glib::spawn_future_local(async move {
            let mut found: Vec<(String, String)> = Vec::new();
            for p in &projects {
                if let Some(repo) = workflow::remote_repo(&p.workspace_root).await {
                    found.push((repo, p.workspace_root.clone()));
                }
            }
            page.render_repos(&app, found).await;
        });
    }

    async fn render_repos(&self, app: &Rc<App>, found: Vec<(String, String)>) {
        while let Some(c) = self.repo_list.first_child() {
            self.repo_list.remove(&c);
        }
        let pinned: Vec<PinnedRepo> = self.config.borrow().repos.clone();
        let mut all: Vec<PinnedRepo> = pinned.clone();
        for (repo, path) in &found {
            if !all.iter().any(|r| r.repo == *repo) {
                all.push(PinnedRepo { repo: repo.clone(), path: path.clone() });
            }
        }
        if all.is_empty() {
            self.status
                .set_text("No GitHub checkouts among the server's projects. Import sessions or add a project first.");
            return;
        }
        self.status.set_text(&format!("{} pinned of {} GitHub repos known to the server", pinned.len(), all.len()));
        for r in all {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.add_css_class("card");
            card.add_css_class("repo-card");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let pin = gtk::CheckButton::with_label("Pinned");
            pin.set_active(pinned.iter().any(|p| p.repo == r.repo));
            let name = label(&r.repo, &["title-4"]);
            name.set_hexpand(true);
            head.append(&name);
            head.append(&pin);
            let open = gtk::Button::with_label("Issues");
            head.append(&open);
            let runner = gtk::Button::with_label("Install runner");
            runner.set_tooltip_text(Some("A T3 scheduled task every 5 minutes that takes the next agent:ready issue"));
            head.append(&runner);
            card.append(&head);
            let summary = label(&crate::look::tilde(&r.path), &["caption", "dim-label"]);
            card.append(&summary);
            let counts = label("", &["caption"]);
            card.append(&counts);
            self.repo_list.append(&card);

            let a = app.clone();
            let rr = r.clone();
            pin.connect_toggled(move |b| {
                let active = b.is_active();
                let rr = rr.clone();
                a.repos_page().save(move |c| {
                    c.repos.retain(|p| p.repo != rr.repo);
                    if active {
                        c.repos.push(rr);
                    }
                });
            });
            let a = app.clone();
            let rr = r.clone();
            open.connect_clicked(move |_| a.repos_page().show_issues(&a, rr.clone()));
            let a = app.clone();
            let rr = r.clone();
            runner.connect_clicked(move |_| a.install_runner(rr.clone()));

            if pinned.iter().any(|p| p.repo == r.repo) {
                match workflow::issues(&r.repo).await {
                    Ok(issues) => {
                        let lane = |l: &str| issues.iter().filter(|i| i.lane() == Some(l)).count();
                        let ready = issues.iter().filter(|i| i.agent() == Some("ready")).count();
                        let running = issues.iter().filter(|i| i.agent() == Some("running")).count();
                        let next = issues
                            .iter()
                            .find(|i| i.agent() == Some("ready"))
                            .map(|i| format!("next up: #{} {}", i.number, i.title));
                        counts.set_text(&format!(
                            "{} open · build {} · feature {} · maintenance {} · ready {} · running {}{}",
                            issues.len(),
                            lane("build"),
                            lane("feature"),
                            lane("maintenance"),
                            ready,
                            running,
                            next.map(|n| format!(" · {n}")).unwrap_or_default()
                        ));
                    }
                    Err(e) => counts.set_text(&format!("GitHub: {e}")),
                }
            }
        }
    }

    pub fn show_issues(&self, app: &Rc<App>, repo: PinnedRepo) {
        *self.current_repo.borrow_mut() = Some(repo.clone());
        self.issues_title.set_text(&format!("Issues in {}", repo.repo));
        while let Some(c) = self.issues_box.first_child() {
            self.issues_box.remove(&c);
        }
        self.issues_box.append(&label("Loading…", &["dim-label"]));
        let app = app.clone();
        glib::spawn_future_local(async move {
            workflow::ensure_labels(&repo.repo).await;
            match workflow::issues(&repo.repo).await {
                Ok(issues) => {
                    *app.repos_page().issues.borrow_mut() = issues;
                    app.repos_page().render_issues(&app);
                }
                Err(e) => {
                    let page = app.repos_page();
                    while let Some(c) = page.issues_box.first_child() {
                        page.issues_box.remove(&c);
                    }
                    page.issues_box.append(&label(&format!("GitHub: {e}"), &["dim-label"]));
                }
            }
        });
    }

    pub fn wire_filters(&self, app: Rc<App>) {
        for dd in [&self.lane_dd, &self.prio_dd, &self.agent_dd] {
            let a = app.clone();
            dd.connect_selected_notify(move |_| a.repos_page().render_issues(&a));
        }
    }

    fn render_issues(&self, app: &Rc<App>) {
        while let Some(c) = self.issues_box.first_child() {
            self.issues_box.remove(&c);
        }
        let Some(repo) = self.current_repo.borrow().clone() else { return };
        let lane = ["", "build", "feature", "maintenance"][self.lane_dd.selected() as usize];
        let prio = ["", "p0", "p1", "p2", "p3"][self.prio_dd.selected() as usize];
        let agent = ["", "ready", "running", "done", "blocked", "none"][self.agent_dd.selected() as usize];
        let now = now_unix();
        let issues = self.issues.borrow();
        let shown: Vec<&Issue> = issues
            .iter()
            .filter(|i| lane.is_empty() || i.lane() == Some(lane))
            .filter(|i| prio.is_empty() || i.priority() == Some(prio))
            .filter(|i| match agent {
                "" => true,
                "none" => i.agent().is_none(),
                a => i.agent() == Some(a),
            })
            .collect();
        if shown.is_empty() {
            self.issues_box.append(&label(
                "No issues match. Label issues lane:*, p0–p3 and agent:ready to feed the runner.",
                &["dim-label"],
            ));
            return;
        }
        for i in shown {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("issue-row");
            let number = label(&format!("#{}", i.number), &["caption", "dim-label"]);
            number.set_width_chars(6);
            row.append(&number);
            let title = label(&i.title, &[]);
            title.set_hexpand(true);
            title.set_tooltip_text(Some(&i.url));
            row.append(&title);
            if let Some(l) = i.lane() {
                row.append(&pill(l, &format!("lane-{l}")));
            }
            if let Some(p) = i.priority() {
                row.append(&pill(p, &format!("prio-{p}")));
            }
            if let Some(a) = i.agent() {
                row.append(&pill(&format!("agent:{a}"), "agent"));
            }
            row.append(&label(&relative_time(iso_to_unix(&i.updated_at), now), &["caption", "dim-label"]));
            let key = format!("{}#{}", repo.repo, i.number);
            match self.thread_for_issue(&key) {
                Some(thread_id) => {
                    let b = gtk::Button::with_label("Open thread");
                    let a = app.clone();
                    b.connect_clicked(move |_| {
                        if !a.select_id(&thread_id) {
                            a.toast("That thread is no longer in the list.");
                        }
                    });
                    row.append(&b);
                }
                None => {
                    let b = gtk::Button::with_label("Start on this");
                    b.add_css_class("suggested-action");
                    let a = app.clone();
                    let issue = i.clone();
                    let rr = repo.clone();
                    b.connect_clicked(move |_| a.start_issue(rr.clone(), issue.clone()));
                    row.append(&b);
                }
            }
            self.issues_box.append(&row);
        }
    }

    pub fn remember_started(&self, key: &str, thread_id: &str) {
        let key = key.to_string();
        let thread_id = thread_id.to_string();
        self.save(move |c| {
            c.started.insert(key, thread_id);
        });
    }

    pub fn remember_runner(&self, repo: &str, task_id: &str) {
        let repo = repo.to_string();
        let task_id = task_id.to_string();
        self.save(move |c| {
            c.runners.insert(repo, task_id);
        });
    }

    pub fn runner_id(&self, repo: &str) -> Option<String> {
        self.config.borrow().runners.get(repo).cloned()
    }
}

impl Default for ReposPage {
    fn default() -> Self {
        Self::new()
    }
}

/// A collapsed row with an issue's comments, for the top of a thread started from that issue.
pub fn comments_row(key: &str, comments: &[(String, String, String)]) -> gtk::Widget {
    let text = comments
        .iter()
        .map(|(who, when, body)| format!("{who} · {}\n{}", crate::look::clock(when), body.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let shown = if comments.is_empty() { "No comments yet.".to_string() } else { text };
    let row = crate::chat::tool_row("", &format!("Issue {key}: {} comments", comments.len()), shown, None);
    row
}

/// The value the server stores for a started issue thread, kept on the thread as `goal` text.
pub fn issue_goal(repo: &str, issue: &Issue) -> Value {
    json!({"repo": repo, "number": issue.number, "url": issue.url})
}

#[allow(dead_code)]
fn _keep(_: &pango::EllipsizeMode) {}

#[allow(dead_code)]
fn _id() -> String {
    new_id()
}
