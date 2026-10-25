//! The Activity page stays in T3's native window. Collection is explicit and
//! read-only; GitHub is opt-in. Copy exports factual records to the clipboard,
//! never to an agent, Omi, a repository or a remote service automatically.

use crate::activity::{Item, Kind, Report, Source, State, Window};
use crate::activity_collect::{self, Repo, Request};
use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct ActivityPage {
    root: gtk::Box,
    status: gtk::Label,
    sources: gtk::Box,
    rows: gtk::Box,
    refresh_button: gtk::Button,
    copy_button: gtk::Button,
    github: gtk::CheckButton,
    report: RefCell<Option<Report>>,
    generation: Cell<u64>,
    fixture: Cell<bool>,
}

/// Local calendar arithmetic handles DST days (23/25 hours), not fixed seconds.
fn today() -> Option<Window> {
    let now = glib::DateTime::now_local().ok()?;
    let zone = now.timezone();
    let start = glib::DateTime::new(&zone, now.year(), now.month(), now.day_of_month(), 0, 0, 0.0).ok()?;
    let end = start.add_days(1).ok()?;
    Some(Window {
        key: start.format("%Y-%m-%d").ok()?.to_string(),
        start: start.to_unix(),
        end: end.to_unix(),
        search_date: start.to_utc().ok()?.format("%Y-%m-%d").ok()?.to_string(),
    })
}

impl ActivityPage {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_start(24);
        root.set_margin_end(24);
        root.set_margin_top(18);
        root.set_margin_bottom(18);

        let title = label("Activity today", &["title-2"]);
        title.set_hexpand(true);
        let refresh_button = gtk::Button::with_label("Refresh");
        let copy_button = gtk::Button::with_label("Copy recap");
        copy_button.set_sensitive(false);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.append(&title);
        header.append(&refresh_button);
        header.append(&copy_button);
        root.append(&header);
        root.append(&label(
            "Read-only records from your pinned repositories. Local commits and GitHub records can overlap; activity does not prove a release shipped.",
            &["dim-label"],
        ));
        let github = gtk::CheckButton::with_label("Include GitHub");
        github.set_tooltip_text(Some(
            "Use gh's signed-in account to read GitHub activity in pinned repositories. Credentials remain in gh.",
        ));
        root.append(&github);
        let status = label("Choose Refresh to read local commits in your pinned checkouts.", &["dim-label"]);
        root.append(&status);
        let sources = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        column.append(&sources);
        column.append(&rows);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&column)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();
        root.append(&scroll);
        let page = Rc::new(Self {
            root,
            status,
            sources,
            rows,
            refresh_button,
            copy_button,
            github,
            report: RefCell::new(None),
            generation: Cell::new(0),
            fixture: Cell::new(false),
        });
        let weak = Rc::downgrade(&page);
        page.refresh_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.refresh();
            }
        });
        let weak = Rc::downgrade(&page);
        page.github.connect_toggled(move |_| {
            if let Some(page) = weak.upgrade() {
                page.refresh();
            }
        });
        let weak = Rc::downgrade(&page);
        page.copy_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade()
                && let Some(report) = page.report.borrow().as_ref()
            {
                page.root.clipboard().set_text(&report.recap());
                page.status.set_text(
                    "Recap copied. Source status is included; review private repository details before sharing.",
                );
            }
        });
        page
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Called by the existing window when the owner chooses Activity. No timer,
    /// background indexer, automatic write or service activation is installed.
    pub fn refresh(self: &Rc<Self>) {
        if self.fixture.get() || !self.refresh_button.is_sensitive() {
            return;
        }
        if std::env::var_os("CLAUDE_PANTHEON_OFFLINE").is_some() {
            self.status.set_text("Offline preview: activity collection is disabled.");
            return;
        }
        let Some(window) = today() else {
            self.status.set_text("Could not determine today's local calendar window.");
            return;
        };
        let repos = crate::workflow::load_config()
            .repos
            .into_iter()
            .map(|repo| Repo { name: repo.repo, path: repo.path.into() })
            .collect();
        let request = Request { window, repos, github: self.github.is_active() };
        self.refresh_button.set_sensitive(false);
        self.github.set_sensitive(false);
        self.copy_button.set_sensitive(false);
        self.status.set_text("Reading activity… Previous records remain visible while sources refresh.");
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result =
                gio::spawn_blocking(move || activity_collect::collect(request, crate::look::iso_to_unix)).await;
            let Some(page) = weak.upgrade() else { return };
            if page.generation.get() != generation {
                return;
            }
            page.refresh_button.set_sensitive(true);
            page.github.set_sensitive(true);
            match result {
                Ok(mut report) => {
                    if let Some(previous) = page.report.borrow().as_ref() {
                        report.retain_last_good(previous);
                    }
                    page.display(report);
                }
                Err(_) => {
                    page.status.set_text("Activity collection stopped unexpectedly. Choose Refresh to retry.");
                    page.copy_button.set_sensitive(page.report.borrow().is_some());
                }
            }
        });
    }

    fn display(&self, report: Report) {
        clear(&self.sources);
        clear(&self.rows);
        let items = report.items();
        let count = |kind| {
            let sources: Vec<_> = report.sources.iter().filter(|source| source.kind == kind).collect();
            if sources.is_empty() {
                return "off".to_string();
            }
            if sources.iter().all(|source| source.state == State::Failed) {
                return "unavailable".to_string();
            }
            let count = items.iter().filter(|item| item.kind == kind).count();
            if sources.iter().any(|source| source.state != State::Fresh) {
                format!("{count} shown (partial or stale)")
            } else {
                count.to_string()
            }
        };
        self.status.set_text(&format!(
            "{} · Local commits: {} · GitHub commits: {} · Merged PRs: {} · Closed authored issues: {}",
            report.window.key,
            count(Kind::LocalCommit),
            count(Kind::GithubCommit),
            count(Kind::MergedPr),
            count(Kind::ClosedIssue)
        ));
        self.sources.append(&label("Sources", &["title-4"]));
        if report.sources.is_empty() {
            self.sources.append(&label("No pinned repositories. Add a checkout in Repositories, then Refresh.", &[]));
        }
        for source in &report.sources {
            let time = glib::DateTime::from_unix_local(source.observed_at)
                .and_then(|date| date.format("%H:%M"))
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "unknown time".into());
            self.sources.append(&label(
                &format!("{} — {} ({time})\n{}", source.label, source.state.label(), source.detail),
                &[],
            ));
        }
        if items.is_empty() {
            self.rows.append(&label(
                if report.sources.iter().any(|source| source.state == State::Failed) {
                    "No activity records available. Check the source messages above."
                } else {
                    "No matching activity today."
                },
                &["dim-label"],
            ));
        }
        let mut groups: std::collections::BTreeMap<&str, Vec<&Item>> = std::collections::BTreeMap::new();
        for item in &items {
            groups.entry(&item.repo).or_default().push(item);
        }
        for (repo, items) in groups {
            self.rows.append(&label(repo, &["title-4"]));
            for item in items {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                text.set_hexpand(true);
                text.append(&label(&item.title, &[]));
                let time = glib::DateTime::from_unix_local(item.timestamp)
                    .and_then(|date| date.format("%H:%M"))
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                text.append(&label(&format!("{} · {time}", item.kind.label()), &["caption", "dim-label"]));
                row.append(&text);
                if let Some(url) = item.url.as_deref().and_then(crate::activity::web_url) {
                    let open = gtk::Button::with_label("Open");
                    open.set_valign(gtk::Align::Center);
                    open.set_tooltip_text(Some(&format!("Open {} in your browser", item.title)));
                    open.connect_clicked(move |_| {
                        // Only validated web links reach the system's URI launcher.
                        gio::AppInfo::launch_default_for_uri_async(
                            &url,
                            gio::AppLaunchContext::NONE,
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                    });
                    row.append(&open);
                }
                self.rows.append(&row);
                self.rows.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
        }
        self.copy_button.set_sensitive(true);
        *self.report.borrow_mut() = Some(report);
    }

    /// Deterministic isolated render path; disables every collection entry point.
    pub fn show_fixture(&self, report: Report) {
        self.fixture.set(true);
        self.generation.set(self.generation.get().wrapping_add(1));
        self.refresh_button.set_sensitive(false);
        self.github.set_sensitive(false);
        self.display(report);
    }
}

fn clear(column: &gtk::Box) {
    while let Some(child) = column.first_child() {
        column.remove(&child);
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .ellipsize(gtk::pango::EllipsizeMode::None)
        .max_width_chars(70)
        .hexpand(true)
        .css_classes(classes)
        .build()
}

pub fn fixture_report() -> Report {
    let window =
        Window { key: "2026-10-25".into(), start: 1_792_886_400, end: 1_792_972_800, search_date: "2026-10-25".into() };
    let item = |kind, title: &str, id: &str| Item {
        kind,
        repo: "techmore/ElementaryOS_MSeries".into(),
        title: title.into(),
        url: if kind == Kind::LocalCommit {
            None
        } else {
            Some("https://github.com/techmore/ElementaryOS_MSeries/pull/587".into())
        },
        timestamp: window.start + 3600,
        id: id.into(),
    };
    Report {
        window: window.clone(),
        sources: vec![
            Source {
                kind: Kind::LocalCommit,
                id: "local:fixture".into(),
                label: "Local: techmore/ElementaryOS_MSeries".into(),
                identity: "fixture@example.invalid".into(),
                items: vec![item(
                    Kind::LocalCommit,
                    "Keep SDK readiness tied to the selected toolkit; local commit only",
                    "abc",
                )],
                state: State::Fresh,
                detail: "Matching author email across reachable branches; author time, not proof of push".into(),
                observed_at: window.start + 4000,
            },
            Source {
                kind: Kind::MergedPr,
                id: "github:fixture".into(),
                label: "Merged PRs".into(),
                identity: "fixture-account".into(),
                items: vec![item(
                    Kind::MergedPr,
                    "Fixture merged pull request with a deliberately long title to check wrapping in a narrow native window",
                    "42",
                )],
                state: State::Partial,
                detail: "Fixture account · pinned repositories; GitHub results incomplete or capped at three pages"
                    .into(),
                observed_at: window.start + 4000,
            },
            Source {
                kind: Kind::GithubCommit,
                id: "github:offline".into(),
                label: "GitHub commits".into(),
                identity: "fixture-account".into(),
                items: vec![],
                state: State::Failed,
                detail: "GitHub unavailable; sign in with gh auth login or disable Include GitHub".into(),
                observed_at: window.start + 4000,
            },
        ],
    }
}
