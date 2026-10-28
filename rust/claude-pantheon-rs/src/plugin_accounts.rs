//! Accounts plugin: the saved Codex accounts and a one-click switch, through the same helper the
//! CodexBar Wingpanel indicator uses (`codexbar-accounts.py`), so there is one source of truth.

use crate::plugin::{Plugin, StatusLine};
use crate::strings::tr;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub plan: String,
    pub active: bool,
}

pub fn parse_accounts(v: &Value) -> Vec<Account> {
    v.get("accounts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Account {
                        id: x.get("id")?.as_str()?.to_string(),
                        email: x.get("email")?.as_str()?.to_string(),
                        plan: x.get("plan").and_then(Value::as_str).unwrap_or("").to_string(),
                        active: x.get("active").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn helper() -> Option<PathBuf> {
    let env = std::env::var_os("CODEXBAR_ACCOUNTS").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    env.into_iter()
        .chain([
            home.join(".local/lib/codexbar-cli/codexbar-accounts.py"),
            PathBuf::from("/usr/lib/mseries/codexbar-accounts.py"),
        ])
        .find(|p| p.exists())
}

fn run(args: &[&str]) -> Result<Value, String> {
    let script = helper().ok_or("codexbar-accounts.py not installed")?;
    let out = Command::new("python3").arg(script).args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

pub struct Accounts {
    list: gtk::ListBox,
    status: gtk::Label,
    cache: Rc<RefCell<Vec<Account>>>,
}

impl Default for Accounts {
    fn default() -> Self {
        Self::new()
    }
}

impl Accounts {
    pub fn new() -> Accounts {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list.add_css_class("boxed-list");
        Accounts { list, status: gtk::Label::new(None), cache: Rc::new(RefCell::new(Vec::new())) }
    }

    fn fill(self_list: &gtk::ListBox, status: &gtk::Label, accounts: &[Account]) {
        while let Some(row) = self_list.first_child() {
            self_list.remove(&row);
        }
        status.set_visible(accounts.is_empty());
        for a in accounts {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.set_margin_top(8);
            row.set_margin_bottom(8);
            row.set_margin_start(12);
            row.set_margin_end(12);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            text.set_hexpand(true);
            let email = gtk::Label::builder().label(&a.email).xalign(0.0).build();
            let plan = gtk::Label::builder().label(&a.plan).xalign(0.0).build();
            plan.add_css_class("dim-label");
            text.append(&email);
            text.append(&plan);
            row.append(&text);
            if a.active {
                let badge = gtk::Label::new(Some(&tr("Active")));
                badge.add_css_class("accent");
                row.append(&badge);
            } else {
                let btn = gtk::Button::with_label("Switch");
                let (id, list, status) = (a.id.clone(), self_list.clone(), status.clone());
                btn.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let (id, list, status) = (id.clone(), list.clone(), status.clone());
                    let (tx, rx) = futures_channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(run(&["switch", &id]).and_then(|_| run(&["list", "--json"])));
                    });
                    glib::spawn_future_local(async move {
                        match rx.await {
                            Ok(Ok(v)) => Accounts::fill(&list, &status, &parse_accounts(&v)),
                            Ok(Err(e)) => status.set_text(&format!("Switch failed: {e}")),
                            Err(_) => {}
                        }
                    });
                });
                row.append(&btn);
            }
            self_list.append(&row);
        }
    }
}

impl Plugin for Accounts {
    fn id(&self) -> &'static str {
        "accounts"
    }
    fn title(&self) -> &'static str {
        "Accounts"
    }
    fn subtitle(&self) -> &'static str {
        "Saved Codex accounts; running sessions keep the account they started with"
    }
    fn icon(&self) -> &'static str {
        "system-users-symbolic"
    }
    fn build(&self) -> gtk::Widget {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        page.set_margin_top(18);
        page.set_margin_bottom(18);
        page.set_margin_start(24);
        page.set_margin_end(24);
        self.status.add_css_class("dim-label");
        self.status.set_visible(false);
        page.append(&self.list);
        page.append(&self.status);
        let scroll = gtk::ScrolledWindow::builder().child(&page).vexpand(true).build();
        scroll.upcast()
    }
    fn refresh(&self) {
        let (list, status, cache) = (self.list.clone(), self.status.clone(), self.cache.clone());
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run(&["list", "--json"]));
        });
        glib::spawn_future_local(async move {
            match rx.await {
                Ok(Ok(v)) => {
                    let accounts = parse_accounts(&v);
                    *cache.borrow_mut() = accounts.clone();
                    if accounts.is_empty() {
                        status
                            .set_text(&tr("No saved accounts. Sign in to Codex, then run codexbar-accounts.py save."));
                    }
                    Accounts::fill(&list, &status, &accounts);
                }
                Ok(Err(e)) => {
                    status.set_text(&e);
                    status.set_visible(true);
                }
                Err(_) => {}
            }
        });
    }
    fn status(&self) -> Vec<StatusLine> {
        self.cache
            .borrow()
            .iter()
            .filter(|a| a.active)
            .map(|a| StatusLine { label: a.email.clone(), detail: a.plan.clone() })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_helper_output() {
        let v = json!({"accounts": [
            {"id": "a|1", "email": "a@x.org", "plan": "prolite", "active": true},
            {"id": "b|2", "email": "b@x.org", "plan": "free"},
            {"email": "no-id"}
        ]});
        let a = parse_accounts(&v);
        assert_eq!(a.len(), 2);
        assert!(a[0].active && !a[1].active);
    }
}
