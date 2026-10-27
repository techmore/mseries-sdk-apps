//! Built-in pages that keep their own code in `app.rs` (they need the live connection), registered as
//! plugins so each can be switched off and gets one menu item and palette entry.

use crate::app::App;
use crate::plugin::Plugin;
use gtk::prelude::*;
use std::rc::Rc;

pub struct Page {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub icon: &'static str,
    pub open: fn(&Rc<App>),
}

impl Plugin for Page {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        self.title
    }
    fn subtitle(&self) -> &'static str {
        self.subtitle
    }
    fn icon(&self) -> &'static str {
        self.icon
    }
    fn build(&self) -> gtk::Widget {
        gtk::Box::new(gtk::Orientation::Vertical, 0).upcast()
    }
    fn open(&self, app: &Rc<App>) -> bool {
        (self.open)(app);
        true
    }
}

pub fn builtin() -> Vec<Rc<dyn Plugin>> {
    vec![
        Rc::new(Page {
            id: "activity",
            title: "Activity",
            subtitle: "What the agents did",
            icon: "document-open-recent-symbolic",
            open: |a| a.show_activity(),
        }),
        Rc::new(Page {
            id: "fleet",
            title: "Remote Machines",
            subtitle: "T3 Fleet",
            icon: "network-workgroup-symbolic",
            open: |a| a.show_fleet(),
        }),
        Rc::new(Page {
            id: "tstack",
            title: "Tstack Workflows",
            subtitle: "Native tstack workflows",
            icon: "view-list-symbolic",
            open: |a| a.show_tstack(),
        }),
        Rc::new(Page {
            id: "repository-overview",
            title: "Repository Overview",
            subtitle: "Read-only RepoBar GitHub status",
            icon: "folder-git-symbolic",
            open: |a| a.show_repository_overview(),
        }),
        Rc::new(Page {
            id: "usage",
            title: "Usage",
            subtitle: "Tokens and cost across providers",
            icon: "utilities-system-monitor-symbolic",
            open: |a| a.show_usage(),
        }),
        Rc::new(Page {
            id: "changes",
            title: "Changes in This Thread",
            subtitle: "The open thread's diff, per file",
            icon: "document-edit-symbolic",
            open: |a| a.show_changes(),
        }),
        Rc::new(Page {
            id: "diagnostics",
            title: "Diagnostics",
            subtitle: "The server's resource telemetry",
            icon: "utilities-system-monitor-symbolic",
            open: |a| a.show_diagnostics(),
        }),
    ]
}
