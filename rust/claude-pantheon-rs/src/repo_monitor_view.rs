//! On-demand native read-only overview, independent of the issue automation page and paired server.
use crate::{chat::label as base_label, look, repo_monitor::{self, Snapshot}};
use gtk::{glib, prelude::*};
use std::{cell::{Cell, RefCell}, rc::Rc};

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let widget = base_label(text, classes);
    widget.set_wrap(true);
    widget.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    widget.set_ellipsize(gtk::pango::EllipsizeMode::None);
    widget.set_selectable(true);
    widget
}

pub struct RepoMonitorView {
    root: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    refresh_button: gtk::Button,
    search: gtk::SearchEntry,
    scope: gtk::DropDown,
    snapshot: RefCell<Option<Snapshot>>,
    generation: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}

impl RepoMonitorView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_start(24);
        root.set_margin_end(24);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = label("GitHub overview", &["title-3"]);
        title.set_hexpand(true);
        header.append(&title);
        let refresh_button = gtk::Button::with_label("Refresh");
        refresh_button.set_tooltip_text(Some("Ask the installed RepoBar helper for a fresh read-only GitHub snapshot"));
        header.append(&refresh_button);
        root.append(&header);
        root.append(&label("Read-only · RepoBar account on this computer · separate from T3 issue automation", &["caption", "dim-label"]));
        let status = label("Open this overview to read RepoBar. No background polling.", &["caption"]);
        root.append(&status);
        let search = gtk::SearchEntry::builder().placeholder_text("Search repositories").hexpand(true).build();
        root.append(&search);
        let scope = gtk::DropDown::from_strings(&["Visible repositories", "Pinned repositories"]);
        scope.set_tooltip_text(Some("Filter using existing RepoBar visibility and pins; never changes preferences"));
        scope.set_halign(gtk::Align::Start);
        root.append(&scope);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let scroll = gtk::ScrolledWindow::builder().child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        root.append(&scroll);
        let view = Rc::new(Self { root, content, status, refresh_button, search, scope,
            snapshot: RefCell::new(None), generation: Cell::new(0), task: RefCell::new(None) });
        let weak = Rc::downgrade(&view);
        view.refresh_button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() { view.refresh(); }
        });
        let weak = Rc::downgrade(&view);
        view.search.connect_search_changed(move |_| {
            if let Some(view) = weak.upgrade() { view.render_cards(); }
        });
        let weak = Rc::downgrade(&view);
        view.scope.connect_selected_notify(move |_| {
            if let Some(view) = weak.upgrade() { view.render_cards(); }
        });
        let weak = Rc::downgrade(&view);
        view.root.connect_map(move |_| {
            if let Some(view) = weak.upgrade() {
                // A fixture render also counts as loaded, and must never trigger a live helper.
                if view.snapshot.borrow().is_none() { view.load(false); }
            }
        });
        let weak = Rc::downgrade(&view);
        view.root.connect_unmap(move |_| {
            if let Some(view) = weak.upgrade() {
                let was_loading = view.task.borrow().is_some();
                view.cancel();
                if was_loading {
                    view.status.set_text("Refresh cancelled while overview was hidden. Previous data is last-known; choose Refresh to read again.");
                }
            }
        });
        view
    }

    pub fn widget(&self) -> &gtk::Box { &self.root }

    /// User-triggered refresh; no timer or hidden poller.
    pub fn refresh(self: &Rc<Self>) { self.load(true); }

    fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() { task.abort(); }
        self.refresh_button.set_sensitive(true);
    }

    fn load(self: &Rc<Self>, force: bool) {
        self.cancel();
        let generation = self.generation.get();
        self.refresh_button.set_sensitive(false);
        self.status.set_text("Reading GitHub through RepoBar… Existing data below is last-known until this finishes.");
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            let result = repo_monitor::read_snapshot(force).await;
            let Some(view) = weak.upgrade() else { return };
            if view.generation.get() != generation { return; }
            view.task.borrow_mut().take();
            view.refresh_button.set_sensitive(true);
            match result {
                Ok(snapshot) => view.apply_snapshot(&snapshot),
                Err(error) => view.status.set_text(&format!("{error}{}", if view.snapshot.borrow().is_some() {
                    " Previous snapshot below is last-known, not a successful refresh."
                } else { "" })),
            }
        });
        self.task.replace(Some(task));
    }

    /// Fixture/native rendering does not execute the helper and invalidates any pending result.
    pub fn render(&self, snapshot: &Snapshot) {
        self.cancel();
        self.apply_snapshot(snapshot);
    }

    fn apply_snapshot(&self, snapshot: &Snapshot) {
        let fetched = glib::DateTime::from_unix_utc(snapshot.fetched_at).ok()
            .and_then(|date| date.format("%Y-%m-%d %H:%M UTC").ok())
            .map(|date| date.to_string()).unwrap_or_else(|| snapshot.fetched_at.to_string());
        let freshness = if snapshot.stale { "Stale · last-known data" }
            else if snapshot.error.is_some() { "Refresh/identity verification failed · last-known data" }
            else { "Snapshot received" };
        self.status.set_text(&format!("{} @ {} · fetched {} ({})\n{}{}", snapshot.login, snapshot.host,
            fetched, look::relative_time(snapshot.fetched_at, look::now_unix()), freshness,
            snapshot.error.as_ref().map(|e| format!("\n{e}")).unwrap_or_default()));
        self.snapshot.replace(Some(snapshot.clone()));
        self.render_cards();
    }

    fn render_cards(&self) {
        while let Some(child) = self.content.first_child() { self.content.remove(&child); }
        let snapshot = self.snapshot.borrow();
        let Some(snapshot) = snapshot.as_ref() else { return };
        self.content.append(&label(&repo_monitor::actions_detail(snapshot), &["caption", "dim-label"]));
        let query = self.search.text().trim().to_lowercase();
        let pinned_only = self.scope.selected() == 1;
        let mut repos: Vec<_> = snapshot.repos.iter().filter(|repo| {
            repo.visibility != "hidden" && (!pinned_only || repo.visibility == "pinned")
                && (query.is_empty() || repo.name.to_lowercase().contains(&query) || repo.description.to_lowercase().contains(&query))
        }).collect();
        repos.sort_by(|a, b| (a.visibility != "pinned", &a.name).cmp(&(b.visibility != "pinned", &b.name)));
        if repos.is_empty() {
            self.content.append(&label(if query.is_empty() {
                if pinned_only { "No pinned repositories in this snapshot. Pins are managed in RepoBar." }
                else { "No visible repositories in this snapshot. Check this account's repository access and RepoBar visibility settings." }
            } else { "No repositories match this search." }, &["dim-label"]));
            return;
        }
        let mut previous_group = "";
        for repo in repos {
            let group = if repo.visibility == "pinned" { "Pinned" } else { "Visible repositories" };
            if group != previous_group {
                self.content.append(&label(group, &["title-4"]));
                previous_group = group;
            }
            let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
            for class in ["card", "pantheon-surface", "repo-card"] { card.add_css_class(class); }
            card.append(&label(&repo.name, &["title-4"]));
            if !repo.description.is_empty() { card.append(&label(&repo.description, &[])); }
            card.append(&label(&format!("Open issues: {} · Open pull requests: {}",
                repo_monitor::count_text(repo.open_issues), repo_monitor::count_text(repo.open_pulls)), &[]));
            card.append(&label(&format!("Default branch: {} · CI: {}", repo.branch.as_deref().unwrap_or("unavailable"),
                if repo.ci == "UNKNOWN" { "unavailable / no status published" } else { &repo.ci }), &[]));
            card.append(&label("Default-branch CI is not the status of every branch or pull request.", &["caption", "dim-label"]));
            let release = match (&repo.release, &repo.release_name) {
                (Some(tag), Some(name)) if tag != name => format!("Latest release: {name} ({tag})"),
                (Some(tag), _) => format!("Latest release: {tag}"),
                _ => "Latest release: unavailable / none published".into(),
            };
            card.append(&label(&format!("{release}{}",repo.release_date.as_ref()
                .map(|date|format!(" · published {date}")).unwrap_or_default()), &[]));
            self.append_link(&card, snapshot, repo.url.as_deref(), "Open repository on GitHub");
            if repo.release.is_some() { self.append_link(&card, snapshot, repo.release_url.as_deref(), "Open latest release"); }
            let runs: Vec<_> = snapshot.actions.runs.iter().filter(|run| run.repo.eq_ignore_ascii_case(&repo.name)).collect();
            card.append(&label("Recent Actions · branches and pull requests", &["title-4"]));
            let observation = snapshot.actions.observations.iter().find(|o| o.repo.eq_ignore_ascii_case(&repo.name));
            card.append(&label(&observation.map(|o| o.detail()).unwrap_or_else(||
                "Actions observation unknown: older helper or missing metadata. Update RepoBar and Refresh; no healthy/zero-run inference.".into()),
                &["caption", "dim-label"]));
            for run in runs {
                card.append(&label(&format!("{} · {}{}\nBranch: {} · Event: {}{}", run.name, run.status,
                    run.conclusion.as_ref().map(|c|format!(" / {c}")).unwrap_or_default(),
                    run.branch.as_deref().unwrap_or("unavailable"), run.event.as_deref().unwrap_or("unavailable"),
                    run.updated_at.as_ref().map(|date|format!("\nUpdated {date}")).unwrap_or_default()), &["caption"]));
                self.append_link(&card, snapshot, run.url.as_deref(), "Open workflow run");
            }
            self.content.append(&card);
        }
    }

    fn append_link(&self, card: &gtk::Box, snapshot: &Snapshot, url: Option<&str>, title: &str) {
        if let Some(url) = url.and_then(|url| repo_monitor::safe_url(url, &snapshot.host)) {
            let link = gtk::LinkButton::with_label(&url, title);
            link.set_halign(gtk::Align::Start);
            card.append(&link);
        }
    }
}

impl Drop for RepoMonitorView {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() { task.abort(); }
    }
}
