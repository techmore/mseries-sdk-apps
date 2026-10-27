//! Repos plugin: the pinned-repo and issue page keeps living in `repos.rs` (the runner, webhooks and the
//! thread-to-issue map reach into it); this registers it so it can be switched off and feeds the indicator.

use crate::app::App;
use crate::plugin::{Plugin, StatusLine};
use crate::repos::ReposPage;
use gtk::prelude::*;
use std::rc::Rc;

pub struct Repos {
    page: Rc<ReposPage>,
}

impl Repos {
    pub fn new(page: Rc<ReposPage>) -> Repos {
        Repos { page }
    }
}

impl Plugin for Repos {
    fn id(&self) -> &'static str {
        "repos"
    }
    fn title(&self) -> &'static str {
        "Repos and Issues"
    }
    fn subtitle(&self) -> &'static str {
        "Pinned GitHub repos, their issue backlog, and the runner"
    }
    fn icon(&self) -> &'static str {
        "folder-git-symbolic"
    }
    fn build(&self) -> gtk::Widget {
        self.page.widget().clone().upcast()
    }
    fn open(&self, app: &Rc<App>) -> bool {
        app.show_repos();
        true
    }
    fn status(&self) -> Vec<StatusLine> {
        let n = self.page.issue_count();
        if n == 0 {
            return Vec::new();
        }
        vec![StatusLine { label: "Open issues".into(), detail: n.to_string() }]
    }
}
