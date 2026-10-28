//! Shiplog plugin: what shipped, newest first, grouped by day. Source: merged pull requests of the pinned
//! repos, read with `gh pr list` (the same authentication the Repos page uses). Nothing is stored.

use crate::plugin::{Plugin, StatusLine};
use crate::repos::ReposPage;
use crate::strings::tr;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Shipped {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub day: String,
    pub url: String,
}

/// `gh pr list --json number,title,mergedAt,url` output for one repo.
pub fn parse_prs(repo: &str, v: &Value) -> Vec<Shipped> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let merged = x.get("mergedAt")?.as_str()?;
                    Some(Shipped {
                        repo: repo.to_string(),
                        number: x.get("number")?.as_u64()?,
                        title: x.get("title")?.as_str()?.to_string(),
                        day: merged.get(..10)?.to_string(),
                        url: x.get("url").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Newest day first; within a day, by repo then number.
pub fn by_day(mut all: Vec<Shipped>) -> Vec<(String, Vec<Shipped>)> {
    all.sort_by(|a, b| b.day.cmp(&a.day).then(a.repo.cmp(&b.repo)).then(b.number.cmp(&a.number)));
    let mut out: Vec<(String, Vec<Shipped>)> = Vec::new();
    for s in all {
        match out.last_mut() {
            Some((d, items)) if *d == s.day => items.push(s),
            _ => out.push((s.day.clone(), vec![s])),
        }
    }
    out
}

fn fetch(repos: &[String]) -> Result<Vec<Shipped>, String> {
    let mut all = Vec::new();
    let mut last_err = None;
    for repo in repos {
        let out = Command::new("gh")
            .args(["pr", "list", "-R", repo, "--state", "merged", "--limit", "30"])
            .args(["--json", "number,title,mergedAt,url"])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            last_err = Some(format!("{repo}: {}", String::from_utf8_lossy(&out.stderr).trim()));
            continue;
        }
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        all.extend(parse_prs(repo, &v));
    }
    match (all.is_empty(), last_err) {
        (true, Some(e)) => Err(e),
        _ => Ok(all),
    }
}

pub struct Shiplog {
    repos: Rc<ReposPage>,
    list: gtk::Box,
    status: gtk::Label,
    cache: Rc<RefCell<Vec<Shipped>>>,
}

impl Shiplog {
    pub fn new(repos: Rc<ReposPage>) -> Shiplog {
        Shiplog {
            repos,
            list: gtk::Box::new(gtk::Orientation::Vertical, 6),
            status: gtk::Label::new(None),
            cache: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn render(list: &gtk::Box, shipped: Vec<Shipped>) {
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        for (day, items) in by_day(shipped) {
            let head = gtk::Label::builder().label(&day).xalign(0.0).margin_top(12).build();
            head.add_css_class("title-4");
            list.append(&head);
            for s in items {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let title = gtk::Label::builder().label(&s.title).xalign(0.0).hexpand(true).wrap(true).build();
                let meta = gtk::Label::new(Some(&format!("{} #{}", s.repo, s.number)));
                meta.add_css_class("dim-label");
                meta.add_css_class("caption");
                row.append(&title);
                row.append(&meta);
                if !s.url.is_empty() {
                    let open = gtk::Button::from_icon_name("web-browser-symbolic");
                    open.add_css_class("flat");
                    open.set_tooltip_text(Some(&tr("Open on GitHub")));
                    let url = s.url.clone();
                    open.connect_clicked(move |_| {
                        let _ = gtk::gio::AppInfo::launch_default_for_uri(&url, None::<&gtk::gio::AppLaunchContext>);
                    });
                    row.append(&open);
                }
                list.append(&row);
            }
        }
    }
}

impl Plugin for Shiplog {
    fn id(&self) -> &'static str {
        "shiplog"
    }
    fn title(&self) -> &'static str {
        "Shiplog"
    }
    fn subtitle(&self) -> &'static str {
        "What shipped: merged pull requests of the pinned repos, by day"
    }
    fn icon(&self) -> &'static str {
        "emblem-default-symbolic"
    }
    fn build(&self) -> gtk::Widget {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        page.set_margin_top(18);
        page.set_margin_bottom(18);
        page.set_margin_start(24);
        page.set_margin_end(24);
        self.status.add_css_class("dim-label");
        self.status.set_xalign(0.0);
        page.append(&self.status);
        page.append(&self.list);
        gtk::ScrolledWindow::builder().child(&page).vexpand(true).build().upcast()
    }
    fn refresh(&self) {
        let repos: Vec<String> = self.repos.config().repos.into_iter().map(|r| r.repo).collect();
        if repos.is_empty() {
            self.status.set_text(&tr("Pin a repo on the Repos page to see what ships there."));
            return;
        }
        self.status.set_text(&tr("Reading merged pull requests…"));
        let (list, status, cache) = (self.list.clone(), self.status.clone(), self.cache.clone());
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&repos));
        });
        glib::spawn_future_local(async move {
            match rx.await {
                Ok(Ok(shipped)) => {
                    status.set_text(&format!("{} merged pull requests", shipped.len()));
                    *cache.borrow_mut() = shipped.clone();
                    Shiplog::render(&list, shipped);
                }
                Ok(Err(e)) => status.set_text(&format!("GitHub: {e}")),
                Err(_) => {}
            }
        });
    }
    fn status(&self) -> Vec<StatusLine> {
        let n = self.cache.borrow().len();
        if n == 0 {
            return Vec::new();
        }
        vec![StatusLine { label: "Merged PRs".into(), detail: n.to_string() }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_groups_newest_first() {
        let a = parse_prs(
            "o/a",
            &json!([
                {"number": 1, "title": "one", "mergedAt": "2026-10-25T10:00:00Z", "url": "u1"},
                {"number": 2, "title": "two", "mergedAt": "2026-10-26T09:00:00Z", "url": "u2"},
                {"number": 3, "title": "open", "mergedAt": null}
            ]),
        );
        let b = parse_prs("o/b", &json!([{"number": 9, "title": "nine", "mergedAt": "2026-10-26T01:00:00Z"}]));
        assert_eq!(a.len(), 2);
        let days = by_day([a, b].concat());
        assert_eq!(days[0].0, "2026-10-26");
        assert_eq!(days[0].1.iter().map(|s| s.number).collect::<Vec<_>>(), vec![2, 9]);
        assert_eq!(days[1].0, "2026-10-25");
    }
}
