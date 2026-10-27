//! Embeddable read-only usage view with explicit open/close ownership.
//! This is not a desktop status-area host or an account-action interface.
use crate::codexbar_monitor::{CodexBarMonitor, RefreshState};
use crate::usage::UsagePanel;
use gtk::prelude::*;
use msdk::runtime::ServiceFailure;
use msdk::usage::{Snapshot, SnapshotError};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

pub struct UsageControl {
    root: gtk::Box,
    refresh: gtk::Button,
    status: gtk::Label,
    monitor: Rc<CodexBarMonitor>,
    opened: Rc<Cell<bool>>,
    preserve_unmap: Rc<Cell<bool>>,
}

impl UsageControl {
    /// The caller supplies a trusted absolute executable and an initial,
    /// validated snapshot. No process runs until the host opens the control.
    pub fn new(
        initial: &Snapshot,
        now: i64,
        executable: PathBuf,
        max_age_seconds: u64,
    ) -> Result<Self, SnapshotError> {
        let panel = Rc::new(UsagePanel::new(initial, now)?);
        panel.widget().set_vexpand(true);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.append(panel.widget());
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let status = gtk::Label::new(Some("Usage not open"));
        status.set_xalign(0.0);
        status.set_hexpand(true);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let refresh = gtk::Button::with_label("Refresh usage");
        refresh.set_tooltip_text(Some("Refresh usage"));
        refresh.update_property(&[gtk::accessible::Property::Label("Refresh usage")]);
        refresh.set_sensitive(false);
        footer.append(&status);
        footer.append(&spinner);
        footer.append(&refresh);
        root.append(&footer);
        let (weak_status, weak_spinner, weak_refresh) =
            (status.downgrade(), spinner.downgrade(), refresh.downgrade());
        let monitor = Rc::new(CodexBarMonitor::new(
            panel,
            executable,
            max_age_seconds,
            move |state| {
                let (Some(status), Some(spinner), Some(refresh)) = (
                    weak_status.upgrade(),
                    weak_spinner.upgrade(),
                    weak_refresh.upgrade(),
                ) else {
                    return;
                };
                let (text, busy, enabled) = match state {
                    RefreshState::Idle => ("Usage not open", false, false),
                    RefreshState::Refreshing => ("Refreshing usage…", true, false),
                    RefreshState::Updated => ("Refresh complete", false, true),
                    RefreshState::Failed => ("Retry when ready", false, true),
                };
                status.set_text(text);
                refresh.set_sensitive(enabled);
                spinner.set_visible(busy);
                if busy {
                    spinner.start();
                } else {
                    spinner.stop();
                }
            },
        ));
        let weak_monitor = Rc::downgrade(&monitor);
        refresh.connect_clicked(move |_| {
            if let Some(monitor) = weak_monitor.upgrade() {
                let _ = monitor.refresh();
            }
        });
        let opened = Rc::new(Cell::new(false));
        let weak_monitor = Rc::downgrade(&monitor);
        let opened_on_unmap = opened.clone();
        let preserve_unmap = Rc::new(Cell::new(false));
        let preserve_on_unmap = preserve_unmap.clone();
        root.connect_unmap(move |_| {
            if preserve_on_unmap.get() {
                return;
            }
            opened_on_unmap.set(false);
            if let Some(monitor) = weak_monitor.upgrade() {
                monitor.close();
            }
        });
        Ok(Self {
            root,
            refresh,
            status,
            monitor,
            opened,
            preserve_unmap,
        })
    }

    // Used only during a synchronous popup mode transition. Ordinary unmaps
    // still close and cancel the monitor.
    pub(crate) fn preserve_during_unmap(&self, preserve: bool) {
        self.preserve_unmap.set(preserve);
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn refresh_button(&self) -> &gtk::Button {
        &self.refresh
    }
    pub fn status_label(&self) -> &gtk::Label {
        &self.status
    }
    /// A new open requests one bounded snapshot; repeated opens do nothing.
    pub fn open(&self) -> Result<(), ServiceFailure> {
        if self.opened.replace(true) {
            return Ok(());
        }
        self.monitor.open();
        self.monitor.refresh()?;
        Ok(())
    }
    pub fn close(&self) {
        if self.opened.replace(false) {
            self.monitor.close();
        }
    }
}
impl Drop for UsageControl {
    fn drop(&mut self) {
        self.close();
    }
}
