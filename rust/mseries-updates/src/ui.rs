//! The Software Updates window. It renders a [`Summary`] derived from `status --json` and turns a
//! click into one [`Command`]; nothing here decides what to install, stage or roll back.

use crate::backend::{Backend, Failure};
use crate::strings::{tr, trn};
use granite::prelude::{PlaceholderExt, ToastExt};
use gtk::prelude::*;
use gtk::{gio, glib};
use mseries_update_core::command::{Channel, Command};
use mseries_update_core::model::{Age, Change, Notice, Phase, Summary, age};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

/// A question asked before something with side effects the user may not expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub title: String,
    pub detail: String,
    pub accept: String,
}

pub type Confirmer = Box<dyn Fn(&gtk::Window, Confirmation) -> Pin<Box<dyn Future<Output = bool>>>>;

fn ask_with_dialog(
    parent: &gtk::Window,
    question: Confirmation,
) -> Pin<Box<dyn Future<Output = bool>>> {
    let parent = parent.clone();
    Box::pin(async move {
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(question.title)
            .detail(question.detail)
            .build();
        dialog.set_buttons(&[&tr("Cancel"), &question.accept]);
        dialog.set_cancel_button(0);
        dialog.set_default_button(0);
        matches!(dialog.choose_future(Some(&parent)).await, Ok(1))
    })
}

struct Inner {
    backend: Arc<dyn Backend>,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    icon: gtk::Image,
    spinner: gtk::Spinner,
    headline: gtk::Label,
    subtitle: gtk::Label,
    meta: gtk::Label,
    banner: gtk::Label,
    banner_row: gtk::Box,
    banner_icon: gtk::Image,
    changes: gtk::Box,
    changes_title: granite::HeaderLabel,
    changes_list: gtk::ListBox,
    install: gtk::Button,
    check: gtk::Button,
    rollback_section: gtk::Box,
    rollback: gtk::Button,
    channel: gtk::DropDown,
    channel_note: gtk::Label,
    unavailable: granite::Placeholder,
    retry: gtk::Button,
    toast: granite::Toast,
    summary: RefCell<Option<Summary>>,
    busy: Cell<bool>,
    /// True while the dropdown is being set from status, so that does not look like a user choice.
    syncing_channel: Cell<bool>,
    confirmer: RefCell<Confirmer>,
    now: Cell<fn() -> i64>,
}

#[derive(Clone)]
pub struct UpdatesWindow {
    inner: Rc<Inner>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .xalign(0.0)
        .wrap(true)
        .build();
    for class in classes {
        l.add_css_class(class);
    }
    l
}

fn channel_name(channel: &str) -> String {
    match channel {
        "dev" => tr("Development"),
        _ => tr("Stable"),
    }
}

fn describe_age(a: Age) -> String {
    match a {
        Age::JustNow => tr("just now"),
        Age::Minutes(n) => trn("{n} minute ago", "{n} minutes ago", n as usize),
        Age::Hours(n) => trn("{n} hour ago", "{n} hours ago", n as usize),
        Age::Days(n) => trn("{n} day ago", "{n} days ago", n as usize),
    }
}

impl UpdatesWindow {
    pub fn new(app: &gtk::Application, backend: Arc<dyn Backend>) -> UpdatesWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("Software Updates"))
            .default_width(560)
            .default_height(680)
            .build();
        window.set_titlebar(Some(&gtk::HeaderBar::new()));

        let icon = gtk::Image::builder()
            .pixel_size(64)
            .valign(gtk::Align::Start)
            .build();
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let spinner = gtk::Spinner::builder()
            .visible(false)
            .valign(gtk::Align::Start)
            .width_request(48)
            .height_request(48)
            .build();
        let headline = label("", &["title-2"]);
        headline.set_accessible_role(gtk::AccessibleRole::Heading);
        let subtitle = label("", &["dim-label"]);
        let meta = label("", &["dim-label"]);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 4);
        titles.set_hexpand(true);
        titles.append(&headline);
        titles.append(&subtitle);
        titles.append(&meta);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        mseries_pantheon_kit::set_box_spacing_text_units(&status_row, 0.75);
        status_row.append(&icon);
        status_row.append(&spinner);
        status_row.append(&titles);

        // Status is carried by the icon and the words, not by text colour alone.
        let banner = label("", &[]);
        banner.set_hexpand(true);
        let banner_icon = gtk::Image::builder()
            .pixel_size(16)
            .valign(gtk::Align::Start)
            .margin_top(2)
            .build();
        banner_icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let banner_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        mseries_pantheon_kit::set_box_spacing_text_units(&banner_row, 0.5);
        banner_row.append(&banner_icon);
        banner_row.append(&banner);
        banner_row.set_visible(false);

        let changes_title = granite::HeaderLabel::new(&tr("Software"));
        let changes_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        changes_list.add_css_class("boxed-list");
        mseries_pantheon_kit::set_accessible_label(&changes_list, &tr("Packages in this update"));
        let changes = gtk::Box::new(gtk::Orientation::Vertical, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&changes, 0.4);
        changes.append(&changes_title);
        changes.append(&changes_list);

        let install = gtk::Button::with_label(&tr("Install at Next Restart"));
        install.add_css_class("suggested-action");
        let check = gtk::Button::with_label(&tr("Check for Updates"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&actions, 0.6);
        actions.set_halign(gtk::Align::End);
        actions.append(&check);
        actions.append(&install);

        let rollback = gtk::Button::with_label(&tr("Roll Back…"));
        rollback.set_valign(gtk::Align::Center);
        let rollback_text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&rollback_text, 0.2);
        rollback_text.set_hexpand(true);
        rollback_text.append(&label(
            &tr("Go back to the previous versions"),
            &["heading"],
        ));
        rollback_text.append(&label(
            &tr("Restores the software from before the last update. It takes effect the next time you log in."),
            &["dim-label"],
        ));
        let rollback_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&rollback_row, 0.75);
        rollback_row.append(&rollback_text);
        rollback_row.append(&rollback);
        let rollback_section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&rollback_section, 0.4);
        rollback_section.append(&granite::HeaderLabel::new(&tr("Previous Version")));
        rollback_section.append(&rollback_row);

        let channel =
            gtk::DropDown::from_strings(&[&tr("Stable (recommended)"), &tr("Development")]);
        channel.set_valign(gtk::Align::Center);
        mseries_pantheon_kit::set_accessible_label(&channel, &tr("Update channel"));
        let channel_note = label("", &["dim-label"]);
        let channel_text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&channel_text, 0.2);
        channel_text.set_hexpand(true);
        channel_text.append(&label(&tr("Update channel"), &["heading"]));
        channel_text.append(&label(
            &tr("Development receives every change before it is released, and is less tested."),
            &["dim-label"],
        ));
        channel_text.append(&channel_note);
        let channel_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&channel_row, 0.75);
        channel_row.append(&channel_text);
        channel_row.append(&channel);
        let advanced = gtk::Expander::builder()
            .label(tr("Advanced"))
            .child(&channel_row)
            .build();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        mseries_pantheon_kit::set_box_spacing_text_units(&content, 1.0);
        mseries_pantheon_kit::set_margins_text_units(&content, 1.2, 1.2, 1.2, 1.2);
        content.append(&status_row);
        content.append(&banner_row);
        content.append(&changes);
        content.append(&actions);
        content.append(&rollback_section);
        content.append(&advanced);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();

        let unavailable = granite::Placeholder::new(&tr("Updates aren't available"));
        unavailable.set_description(&tr("Software updates are not set up on this computer."));
        let retry = unavailable.append_button(
            &gio::ThemedIcon::new("view-refresh-symbolic"),
            &tr("Try Again"),
            &tr("Read the update status again"),
        );

        // Measure only the page on show: the placeholder page must not set the window's minimum height.
        let stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        stack.add_named(&scroller, Some("content"));
        stack.add_named(&unavailable, Some("unavailable"));

        let toast = granite::Toast::new("");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        overlay.add_overlay(&toast);
        window.set_child(Some(&overlay));

        // Keep paired actions readable when the window or desktop text scale grows.
        let status_for_width = status_row.clone();
        let actions_for_width = actions.clone();
        let rollback_for_width = rollback_row.clone();
        let channel_for_width = channel_row.clone();
        let check_for_width = check.clone();
        let install_for_width = install.clone();
        mseries_pantheon_kit::watch_responsive_width(&window, 32.0, move |class| {
            let compact = class == mseries_pantheon_kit::LayoutClass::Compact;
            let orientation = if compact {
                gtk::Orientation::Vertical
            } else {
                gtk::Orientation::Horizontal
            };
            status_for_width.set_orientation(orientation);
            actions_for_width.set_orientation(orientation);
            rollback_for_width.set_orientation(orientation);
            channel_for_width.set_orientation(orientation);
            actions_for_width.set_halign(if compact {
                gtk::Align::Fill
            } else {
                gtk::Align::End
            });
            check_for_width.set_hexpand(compact);
            install_for_width.set_hexpand(compact);
        });

        let inner = Rc::new(Inner {
            backend,
            window,
            stack,
            icon,
            spinner,
            headline,
            subtitle,
            meta,
            banner,
            banner_row,
            banner_icon,
            changes,
            changes_title,
            changes_list,
            install,
            check,
            rollback_section,
            rollback,
            channel,
            channel_note,
            unavailable,
            retry,
            toast,
            summary: RefCell::new(None),
            busy: Cell::new(false),
            syncing_channel: Cell::new(false),
            confirmer: RefCell::new(Box::new(ask_with_dialog)),
            now: Cell::new(unix_now),
        });
        let this = UpdatesWindow { inner };
        this.connect();
        this
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.inner.window
    }

    /// Replace the confirmation dialog (tests answer without a user).
    pub fn set_confirmer(&self, confirmer: Confirmer) {
        *self.inner.confirmer.borrow_mut() = confirmer;
    }

    /// Replace the clock used for "checked 5 minutes ago" (tests need a fixed one).
    pub fn set_clock(&self, now: fn() -> i64) {
        self.inner.now.set(now);
        self.render();
    }

    fn connect(&self) {
        let this = self.clone();
        self.inner
            .check
            .connect_clicked(move |_| this.run(Command::Check));
        let this = self.clone();
        self.inner
            .install
            .connect_clicked(move |_| this.run(Command::Stage));
        let this = self.clone();
        self.inner.rollback.connect_clicked(move |_| {
            let this = this.clone();
            glib::spawn_future_local(async move {
                let question = Confirmation {
                    title: tr("Roll back the last update?"),
                    detail: tr("Your software returns to the versions from before the last update. This takes effect the next time you log in, and those versions are not offered again."),
                    accept: tr("Roll Back"),
                };
                if this.confirm(question).await {
                    this.run(Command::Rollback);
                }
            });
        });
        let this = self.clone();
        self.inner.channel.connect_selected_notify(move |dropdown| {
            if this.inner.syncing_channel.get() {
                return;
            }
            let wanted = if dropdown.selected() == 1 { Channel::Dev } else { Channel::Stable };
            let this = this.clone();
            glib::spawn_future_local(async move {
                let detail = match wanted {
                    Channel::Dev => tr("Development updates are newer but less tested. Any update you have downloaded is discarded and rechecked on the new channel. You keep your current versions."),
                    Channel::Stable => tr("Any update you have downloaded is discarded and rechecked on the new channel. You keep your current versions until a stable release is newer than them."),
                };
                let question = Confirmation {
                    title: format!("{} {}", tr("Switch to"), channel_name(wanted.as_arg())).trim().to_string() + "?",
                    detail,
                    accept: tr("Switch Channel"),
                };
                if this.confirm(question).await {
                    this.run(Command::SwitchChannel(wanted));
                } else {
                    this.render();
                }
            });
        });
        let this = self.clone();
        self.inner.retry.connect_clicked(move |_| this.refresh());
    }

    async fn confirm(&self, question: Confirmation) -> bool {
        let future = (self.inner.confirmer.borrow())(self.inner.window.upcast_ref(), question);
        future.await
    }

    /// Read the status (off the main thread) and show it.
    pub fn refresh(&self) {
        let this = self.clone();
        self.set_busy(Some(tr("Reading update status…")));
        glib::spawn_future_local(async move {
            let backend = this.inner.backend.clone();
            let result = gio::spawn_blocking(move || backend.status()).await;
            this.inner.busy.set(false);
            match result {
                Ok(Ok(status)) => {
                    *this.inner.summary.borrow_mut() = Some(Summary::from_status(&status));
                    this.inner.stack.set_visible_child_name("content");
                }
                Ok(Err(Failure::NotInstalled)) => this.show_unavailable(
                    &tr("Updates aren't available"),
                    &tr("Software updates are not set up on this computer."),
                ),
                Ok(Err(other)) => this.show_unavailable(
                    &tr("Couldn't read the update status"),
                    &failure_text(&other),
                ),
                Err(_) => this.show_unavailable(
                    &tr("Couldn't read the update status"),
                    &tr("The status reader stopped unexpectedly."),
                ),
            }
            this.render();
        });
    }

    fn show_unavailable(&self, title: &str, detail: &str) {
        *self.inner.summary.borrow_mut() = None;
        self.inner.unavailable.set_title(title);
        self.inner.unavailable.set_description(detail);
        self.inner.stack.set_visible_child_name("unavailable");
    }

    fn run(&self, command: Command) {
        if self.inner.busy.get() {
            return;
        }
        let progress = match command {
            Command::Check => tr("Checking for updates…"),
            Command::Stage => tr("Downloading and verifying updates…"),
            Command::Rollback => tr("Restoring the previous versions…"),
            Command::SwitchChannel(_) => tr("Switching channel…"),
        };
        self.inner.banner_row.set_visible(false);
        self.set_busy(Some(progress));
        let this = self.clone();
        glib::spawn_future_local(async move {
            let backend = this.inner.backend.clone();
            let result = gio::spawn_blocking(move || backend.run(command)).await;
            let result = result.unwrap_or_else(|_| {
                Err(Failure::Failed(tr("The update task stopped unexpectedly.")))
            });
            this.inner.busy.set(false);
            match &result {
                Ok(()) => match command {
                    Command::Check => {}
                    Command::Stage => this.notify(&tr("Updates will install at your next restart")),
                    Command::Rollback => this.notify(&tr(
                        "Previous versions restored. They take effect at your next login",
                    )),
                    Command::SwitchChannel(c) => this.notify(&format!(
                        "{} {}",
                        tr("Switched to"),
                        channel_name(c.as_arg())
                    )),
                },
                Err(Failure::Cancelled) => {}
                Err(failure) => this.show_failure(failure),
            }
            this.refresh();
        });
    }

    fn show_failure(&self, failure: &Failure) {
        let text = failure_text(failure);
        self.inner.banner.set_label(&text);
        self.inner
            .banner_icon
            .set_icon_name(Some("dialog-error-symbolic"));
        self.inner.banner_row.set_visible(true);
        self.notify(&text);
    }

    fn notify(&self, text: &str) {
        self.inner.toast.set_title(text);
        self.inner.toast.send_notification();
    }

    fn set_busy(&self, progress: Option<String>) {
        self.inner.busy.set(progress.is_some());
        if let Some(text) = progress {
            self.inner.spinner.set_visible(true);
            self.inner.spinner.start();
            self.inner.icon.set_visible(false);
            self.inner.subtitle.set_label(&text);
            self.inner.subtitle.set_visible(true);
        }
        self.update_sensitivity();
    }

    fn update_sensitivity(&self) {
        let i = &self.inner;
        let busy = i.busy.get();
        let summary = i.summary.borrow();
        let s = summary.as_ref();
        let recovering = s.is_some_and(|s| s.phase == Phase::Recovering);
        i.check.set_sensitive(!busy && !recovering);
        i.install.set_sensitive(!busy);
        i.rollback
            .set_sensitive(!busy && s.is_some_and(|s| s.can_roll_back));
        i.channel
            .set_sensitive(!busy && s.is_some_and(|s| s.can_switch_channel));
    }

    /// Show the current summary; safe to call at any time.
    pub fn render(&self) {
        let i = &self.inner;
        i.spinner.stop();
        i.spinner.set_visible(false);
        i.icon.set_visible(true);
        let summary = i.summary.borrow().clone();
        let Some(summary) = summary else {
            self.update_sensitivity();
            return;
        };
        let Described {
            icon,
            headline,
            subtitle,
            meta,
            changes,
            install: show_install,
        } = describe_phase(&summary, i.now.get()());
        i.icon.set_icon_name(Some(icon));
        i.headline.set_label(&headline);
        i.subtitle.set_label(&subtitle);
        i.subtitle.set_visible(!subtitle.is_empty());
        i.meta.set_label(&meta);
        i.install.set_visible(show_install);
        i.check.set_label(&if show_install
            || summary.phase == Phase::NeverChecked
            || matches!(summary.phase, Phase::UpToDate)
        {
            tr("Check for Updates")
        } else {
            tr("Check Again")
        });
        if i.check.has_css_class("suggested-action") {
            i.check.remove_css_class("suggested-action");
        }
        if summary.phase == Phase::NeverChecked {
            i.check.add_css_class("suggested-action");
        }

        while let Some(row) = i.changes_list.first_child() {
            i.changes_list.remove(&row);
        }
        i.changes.set_visible(!changes.is_empty());
        i.changes_title.set_property(
            "label",
            match summary.phase {
                Phase::AwaitingConfirmation(_) => tr("Installed"),
                Phase::Staged(_) => tr("Ready to install"),
                _ => tr("Available"),
            },
        );
        for change in &changes {
            i.changes_list.append(&change_row(change));
        }

        match &summary.notice {
            Some(notice) if !i.banner_row.is_visible() => {
                i.banner.set_label(&notice_text(notice));
                i.banner_icon.set_icon_name(Some(match notice {
                    Notice::RolledBack { .. } => "dialog-information-symbolic",
                    Notice::RollbackFailed { .. } => "dialog-error-symbolic",
                }));
                i.banner_row.set_visible(true);
            }
            None if i
                .banner_icon
                .icon_name()
                .is_some_and(|n| n == "dialog-information-symbolic") =>
            {
                i.banner_row.set_visible(false)
            }
            _ => {}
        }

        i.rollback_section.set_visible(summary.can_roll_back);
        i.syncing_channel.set(true);
        i.channel
            .set_selected(if summary.channel == "dev" { 1 } else { 0 });
        i.syncing_channel.set(false);
        i.channel_note.set_label(&if summary.can_switch_channel {
            String::new()
        } else {
            tr("Finish or roll back the current update before switching channel.")
        });
        i.channel_note.set_visible(!summary.can_switch_channel);
        self.update_sensitivity();
    }
}

fn failure_text(failure: &Failure) -> String {
    match failure {
        Failure::NotInstalled => tr("Software updates are not set up on this computer."),
        Failure::Cancelled => tr("Cancelled."),
        Failure::NotAuthorized => tr("You don't have permission to change updates."),
        Failure::Busy => tr("Another update task is running. Try again in a minute."),
        Failure::RollbackFailed(detail) => format!(
            "{} {detail}",
            tr("Restoring the previous versions failed. The software needs attention:")
        ),
        Failure::Failed(detail) => detail.clone(),
    }
}

fn notice_text(notice: &Notice) -> String {
    match notice {
        Notice::RolledBack {
            requested: true, ..
        } => tr("You rolled back the last update."),
        Notice::RolledBack {
            requested: false,
            detail,
        } => {
            format!(
                "{} {detail}",
                tr("The last update was rolled back automatically and won't be offered again:")
            )
        }
        Notice::RollbackFailed { detail } => {
            format!(
                "{} {detail}",
                tr("Restoring the previous versions failed. The software needs attention:")
            )
        }
    }
}

struct Described {
    icon: &'static str,
    headline: String,
    subtitle: String,
    /// "Last checked 5 minutes ago · Channel: Stable"
    meta: String,
    changes: Vec<Change>,
    /// Whether "Install at Next Restart" applies.
    install: bool,
}

fn describe_phase(summary: &Summary, now: i64) -> Described {
    let mut meta: Vec<String> = summary
        .last_checked
        .map(|t| format!("{} {}", tr("Last checked"), describe_age(age(t, now))))
        .into_iter()
        .collect();
    meta.push(format!(
        "{}: {}",
        tr("Channel"),
        channel_name(&summary.channel)
    ));
    let meta = meta.join(" · ");
    let (icon, headline, subtitle, changes, install) = match &summary.phase {
        Phase::NeverChecked => (
            "software-update-available",
            tr("Updates haven't been checked yet"),
            tr("See whether newer versions of the M-series software are available."),
            vec![],
            false,
        ),
        Phase::UpToDate => (
            "process-completed",
            tr("The M-series software is up to date"),
            String::new(),
            vec![],
            false,
        ),
        Phase::Available(changes) => (
            "software-update-available",
            trn(
                "{n} update available",
                "{n} updates available",
                changes.len(),
            ),
            tr("Updates install the next time you restart. Your desktop is not interrupted."),
            changes.clone(),
            true,
        ),
        Phase::Staged(changes) => (
            "software-update-available",
            tr("Ready to install at your next restart"),
            tr(
                "The updates are downloaded and verified. If anything goes wrong, your previous versions are restored automatically.",
            ),
            changes.clone(),
            false,
        ),
        Phase::AwaitingConfirmation(changes) => (
            "process-completed",
            tr("Updates installed"),
            tr(
                "They are marked as working once the desktop has run normally for a few minutes. You can go back to the previous versions.",
            ),
            changes.clone(),
            false,
        ),
        Phase::Recovering => (
            "dialog-warning",
            tr("Restoring the previous versions"),
            tr(
                "An update was interrupted. The previous versions are restored at the next restart.",
            ),
            vec![],
            false,
        ),
    };
    Described {
        icon,
        headline,
        subtitle,
        meta,
        changes,
        install,
    }
}

fn change_row(change: &Change) -> gtk::ListBoxRow {
    let name = change
        .name
        .strip_prefix("mseries-")
        .unwrap_or(&change.name)
        .replace('-', " ");
    let versions = match &change.from {
        Some(from) if from != &change.to => format!("{from} → {}", change.to),
        _ => change.to.clone(),
    };
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_margin_top(8);
    text.set_margin_bottom(8);
    text.set_margin_start(12);
    text.set_margin_end(12);
    text.append(&label(&change.name, &["heading"]));
    let versions = label(&versions, &["dim-label", "monospace"]);
    versions.set_selectable(true);
    text.append(&versions);
    let row = gtk::ListBoxRow::builder()
        .child(&text)
        .activatable(false)
        .build();
    mseries_pantheon_kit::set_accessible_label(&row, &name);
    row
}
