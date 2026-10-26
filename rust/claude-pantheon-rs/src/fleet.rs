//! Native Fleet Shepherd adapter: explicit aliases, on-demand read-only snapshots.
use futures_util::future::{Either, select};
use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::OsStr,
    rc::Rc,
    time::Duration,
};

const MAX_BYTES: usize = 4 * 1024 * 1024;

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

pub fn connectors(value: &Value) -> Result<&Vec<Value>, String> {
    if value["schemaVersion"].as_u64() != Some(1) {
        return Err("Unsupported Fleet snapshot. Update the Fleet helper.".into());
    }
    let rows = value["connectors"].as_array().ok_or("Fleet returned no connector list")?;
    if rows.len() > 64 {
        return Err("Fleet exceeded the 64-machine limit".into());
    }
    let mut seen = HashSet::new();
    for row in rows {
        let id = row["id"].as_str().ok_or("Missing connector identity")?;
        if !valid_id(id) || !seen.insert(id) {
            return Err("Invalid or duplicate connector identity".into());
        }
        if !matches!(row["mode"].as_str(), Some("local" | "ssh")) {
            return Err("Unknown connector mode".into());
        }
        if row["label"].as_str().is_none_or(|s| s.is_empty() || s.chars().count() > 96) {
            return Err("Invalid connector label".into());
        }
        if let Some(agents) = row["herdr"]["agents"].as_array() {
            if agents.len() > 64 {
                return Err("Fleet agent list exceeds its bound".into());
            }
            for agent in agents {
                for key in ["title", "status", "workspaceLabel", "cwd"] {
                    if agent[key].as_str().is_some_and(|s| s.chars().count() > 96) {
                        return Err("Fleet agent text exceeds its bound".into());
                    }
                }
            }
        }
        if row["mode"] == "ssh" {
            let target = row["target"].as_str().or_else(|| row["focusTarget"].as_str());
            if target.is_none_or(|s| !valid_id(s)) {
                return Err("Invalid SSH alias".into());
            }
        }
    }
    Ok(rows)
}

pub fn observed(row: &Value, generated: &str, now: i64) -> String {
    let timestamp = row["freshness"].as_str().unwrap_or(generated);
    match glib::DateTime::from_iso8601(timestamp, None).map(|date| date.to_unix()) {
        Ok(time) if time <= now + 60 && now - time <= 300 => format!("Observed {timestamp}"),
        Ok(time) if time <= now + 60 => format!("Stale · last observed {timestamp}"),
        _ => "Observation time unknown".into(),
    }
}

struct Child {
    process: gio::Subprocess,
    completed: Cell<bool>,
}
impl Drop for Child {
    fn drop(&mut self) {
        // The vendored collector handles SIGTERM by killing all owned probe groups.
        if !self.completed.get() {
            self.process.send_signal(15);
        }
    }
}

async fn run(args: &[String]) -> Result<Value, String> {
    let helper = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("mseries-fleet")))
        .filter(|path| path.is_file())
        .or_else(|| glib::find_program_in_path("mseries-fleet"))
        .ok_or("Fleet helper is unavailable. Install T3's Fleet helper, then retry.")?;
    let mut argv = vec![helper.as_os_str()];
    argv.extend(args.iter().map(OsStr::new));
    let child = Child {
        process: gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE)
            .map_err(|_| "Fleet helper is unavailable. Install T3's Fleet helper, then retry.".to_owned())?,
        completed: Cell::new(false),
    };
    let operation = async {
        let stream = child.process.stdout_pipe().ok_or("Fleet helper has no output pipe")?;
        let mut bytes = Vec::new();
        loop {
            let next = stream
                .read_bytes_future(16384, glib::Priority::DEFAULT)
                .await
                .map_err(|_| "Could not read Fleet output")?;
            if next.is_empty() {
                break;
            }
            if bytes.len() + next.len() > MAX_BYTES {
                return Err("Fleet output exceeds 4 MiB".into());
            }
            bytes.extend_from_slice(next.as_ref());
        }
        child.process.wait_future().await.map_err(|_| "Fleet helper failed to finish")?;
        child.completed.set(true);
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Fleet helper returned invalid output")?;
        if !child.process.has_exited() || child.process.exit_status() != 0 {
            return Err(value["error"]
                .as_str()
                .filter(|s| s.len() <= 1024)
                .unwrap_or("Fleet refresh failed. Check connector settings and SSH access.")
                .into());
        }
        connectors(&value)?;
        Ok(value)
    };
    match select(Box::pin(operation), Box::pin(glib::timeout_future(Duration::from_secs(70)))).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err("Fleet refresh timed out. Check SSH access, then retry.".into()),
    }
}

fn label(text: &str, heading: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    if heading {
        label.add_css_class("heading");
    }
    label
}

pub struct FleetPage {
    root: gtk::Box,
    machines: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    label_entry: gtk::Entry,
    alias_entry: gtk::Entry,
    controls: gtk::Box,
    busy: Cell<bool>,
    snapshot_running: Cell<bool>,
    generation: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    fixture: Cell<bool>,
}

impl FleetPage {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_start(18);
        root.set_margin_end(18);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        root.append(&label("Remote Machines", true));
        root.append(&label("Read-only agent status and OMP usage over SSH. Uses this computer's SSH aliases and verified host keys, separately from the paired T3 server. Refresh contacts the configured machines; there is no background polling.", false));
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let form = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let label_entry = gtk::Entry::builder().placeholder_text("Machine label").max_length(96).build();
        label_entry.update_property(&[gtk::accessible::Property::Label("Machine label")]);
        let alias_entry = gtk::Entry::builder().placeholder_text("Existing SSH alias").max_length(96).build();
        alias_entry.update_property(&[gtk::accessible::Property::Label("Existing SSH alias")]);
        form.append(&label_entry);
        form.append(&alias_entry);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let add = gtk::Button::with_label("Add Machine");
        let refresh = gtk::Button::with_label("Refresh Status");
        refresh.add_css_class("suggested-action");
        actions.append(&add);
        actions.append(&refresh);
        form.append(&actions);
        controls.append(&form);
        let machines = gtk::Box::new(gtk::Orientation::Vertical, 6);
        controls.append(&machines);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let status = label("Open this page to load connector settings. No machine has been contacted.", false);
        root.append(&status);
        let cancel = gtk::Button::with_label("Cancel Refresh");
        cancel.set_halign(gtk::Align::Start);
        cancel.set_visible(false);
        root.append(&cancel);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.append(&controls);
        body.append(&content);
        root.append(
            &gtk::ScrolledWindow::builder()
                .child(&body)
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build(),
        );
        let page = Rc::new(Self {
            root,
            machines,
            content,
            status,
            label_entry,
            alias_entry,
            controls,
            busy: Cell::new(false),
            snapshot_running: Cell::new(false),
            generation: Cell::new(0),
            task: RefCell::new(None),
            fixture: Cell::new(false),
        });
        let weak = Rc::downgrade(&page);
        add.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                let alias = page.alias_entry.text();
                let name = page.label_entry.text();
                if !valid_id(&alias) || name.trim().is_empty() {
                    page.status.set_text("Enter a machine label and a simple SSH alias, such as build-server.");
                    return;
                }
                page.load(vec!["add".into(), name.trim().into(), alias.into()]);
            }
        });
        let weak = Rc::downgrade(&page);
        refresh.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.load(vec!["snapshot".into()]);
            }
        });
        let weak = Rc::downgrade(&page);
        cancel.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.cancel();
                page.status.set_text("Refresh cancelled. Data shown below is last-known.");
            }
        });
        let cancel_weak = cancel.downgrade();
        let weak = Rc::downgrade(&page);
        page.status.connect_notify_local(Some("label"), move |_, _| {
            if let (Some(page), Some(cancel)) = (weak.upgrade(), cancel_weak.upgrade()) {
                cancel.set_visible(page.snapshot_running.get());
            }
        });
        let weak = Rc::downgrade(&page);
        page.root.connect_unmap(move |_| {
            if let Some(page) = weak.upgrade()
                && page.snapshot_running.get()
            {
                page.cancel();
                page.status.set_text("Refresh cancelled while this page was hidden. Data below is last-known.");
            }
        });
        page
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn open(self: &Rc<Self>) {
        if self.fixture.get() || self.busy.get() {
            return;
        }
        self.load(vec!["list".into()]);
    }
    fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.busy.set(false);
        self.snapshot_running.set(false);
        self.controls.set_sensitive(true);
    }
    fn load(self: &Rc<Self>, args: Vec<String>) {
        if self.fixture.get() || self.busy.get() {
            return;
        }
        let snapshot = args[0] == "snapshot";
        self.snapshot_running.set(snapshot);
        self.busy.set(true);
        self.controls.set_sensitive(false);
        self.status.set_text(if snapshot {
            "Contacting configured machines… Previous data is last-known until this finishes."
        } else {
            "Reading connector settings…"
        });
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            let result = run(&args).await;
            let Some(page) = weak.upgrade() else { return };
            if page.generation.get() != generation {
                return;
            }
            page.task.borrow_mut().take();
            page.busy.set(false);
            page.snapshot_running.set(false);
            page.controls.set_sensitive(true);
            match result {
                Ok(value) if snapshot => page.render_snapshot(&value),
                Ok(value) => {
                    page.render_settings(&value);
                    page.status.set_text("Connector settings loaded. Choose Refresh Status to contact these machines.");
                    clear(&page.content);
                    if args[0] != "list" {
                        page.label_entry.set_text("");
                        page.alias_entry.set_text("");
                    }
                }
                Err(error) => page.status.set_text(&format!("{error} Previous results, if shown, are last-known.")),
            }
        });
        self.task.replace(Some(task));
    }
    fn render_settings(self: &Rc<Self>, value: &Value) {
        clear(&self.machines);
        let Ok(rows) = connectors(value) else { return };
        for machine in rows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let title = label(
                &format!(
                    "{} · {}",
                    machine["label"].as_str().unwrap_or(""),
                    machine["target"].as_str().unwrap_or("This machine")
                ),
                false,
            );
            title.set_hexpand(true);
            row.append(&title);
            if machine["mode"] == "ssh" {
                let remove = gtk::Button::with_label("Remove");
                remove.set_tooltip_text(Some(&format!(
                    "Remove {} from Fleet",
                    machine["label"].as_str().unwrap_or("machine")
                )));
                let id = machine["id"].as_str().unwrap_or("").to_owned();
                let weak = Rc::downgrade(self);
                remove.connect_clicked(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.load(vec!["remove".into(), id.clone()]);
                    }
                });
                row.append(&remove);
            }
            self.machines.append(&row);
        }
    }
    pub fn show_fixture(self: &Rc<Self>, value: &Value) {
        self.cancel();
        self.fixture.set(true);
        self.controls.set_sensitive(false);
        self.render_snapshot(value);
    }
    fn render_snapshot(&self, value: &Value) {
        clear(&self.content);
        let rows = match connectors(value) {
            Ok(rows) => rows,
            Err(error) => {
                self.status.set_text(&error);
                return;
            }
        };
        self.status
            .set_text("Snapshot received. Source times below show freshness; missing sources remain unavailable.");
        let generated = value["generatedAt"].as_str().unwrap_or("");
        let mut ordered: Vec<_> = rows.iter().collect();
        ordered.sort_by_key(|row| (row["mode"] != "local", row["id"].as_str().unwrap_or("")));
        for row in ordered {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.set_margin_start(12);
            card.set_margin_end(12);
            card.set_margin_top(12);
            card.set_margin_bottom(12);
            let health = match row["health"].as_str() {
                Some("online") => "Online",
                Some("degraded") => "Partially available",
                Some("offline") => "Offline",
                _ => "Unknown",
            };
            let age = observed(row, generated, crate::look::now_unix());
            let health =
                if age.starts_with("Observed ") { health.to_owned() } else { format!("Last reported: {health}") };
            card.append(&label(&format!("{} · {health}", row["label"].as_str().unwrap_or("Machine")), true));
            card.append(&label(&age, false));
            card.append(&label(
                &format!(
                    "Connector: {} · {}",
                    row["id"].as_str().unwrap_or(""),
                    row["focusTarget"].as_str().unwrap_or("local")
                ),
                false,
            ));
            if let Some(agents) = row["herdr"]["agents"].as_array().filter(|a| a.len() <= 64) {
                card.append(&label(
                    &format!(
                        "Herdr · {} reported agents{}",
                        agents.len(),
                        if row["herdr"]["truncated"] == true { " · partial" } else { "" }
                    ),
                    true,
                ));
                for agent in agents {
                    let title = agent["title"].as_str().unwrap_or("");
                    let status = agent["status"].as_str().unwrap_or("unknown");
                    let workspace = agent["workspaceLabel"].as_str().unwrap_or("");
                    card.append(&label(&format!("{status} · {title}\n{workspace}"), false));
                }
            } else {
                card.append(&label("Herdr agent status unavailable", false));
            }
            let overall = &row["omp"]["overall"];
            let requests = overall["totalRequests"].as_u64().map(|n| n.to_string()).unwrap_or("unavailable".into());
            let cost = overall["totalCost"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| format!("${n:.4} USD"))
                .unwrap_or("unavailable".into());
            let input = overall["totalInputTokens"].as_u64().map(|n| n.to_string()).unwrap_or("unavailable".into());
            let output = overall["totalOutputTokens"].as_u64().map(|n| n.to_string()).unwrap_or("unavailable".into());
            card.append(&label(&format!("OMP statistics · requests {requests} · cost {cost}\nInput tokens {input} · output tokens {output}\nOMP's reported history on this machine; date range unavailable. Independent of T3 account quotas."),false));
            let metric = |key: &str| overall[key].as_u64().map(|n| n.to_string()).unwrap_or("unavailable".into());
            card.append(&label(
                &format!(
                    "Cache-read tokens {} · cache-write tokens {} · failed requests {}{}",
                    metric("totalCacheReadTokens"),
                    metric("totalCacheWriteTokens"),
                    metric("failedRequests"),
                    if row["omp"]["truncated"] == true { " · partial report" } else { "" }
                ),
                false,
            ));
            if let Some(models) = row["omp"]["byModel"].as_array().filter(|models| models.len() <= 64) {
                card.append(&label(
                    &format!("Models · showing {} of {} reported", models.len().min(5), models.len()),
                    true,
                ));
                for model in models.iter().take(5) {
                    let name = model["model"].as_str().filter(|s| s.chars().count() <= 96).unwrap_or("Unknown model");
                    let provider =
                        model["provider"].as_str().filter(|s| s.chars().count() <= 96).unwrap_or("Unknown provider");
                    let requests =
                        model["totalRequests"].as_u64().map(|n| n.to_string()).unwrap_or("unavailable".into());
                    card.append(&label(&format!("{name} · {provider} · requests {requests}"), false));
                }
            }
            if let Some(error) = row["error"].as_str().filter(|s| s.len() <= 1024 && !s.is_empty()) {
                card.append(&label(error, false));
            }
            if row["mode"] == "ssh" {
                let alias = row["focusTarget"].as_str().unwrap_or("").to_owned();
                if valid_id(&alias) {
                    let terminal = gtk::Button::with_label("Open SSH Terminal");
                    terminal.set_sensitive(
                        !self.fixture.get()
                            && glib::find_program_in_path("xdg-terminal-exec").is_some()
                            && glib::find_program_in_path("ssh").is_some(),
                    );
                    terminal.set_tooltip_text(Some(&format!("Open an interactive SSH session to {alias}")));
                    terminal.set_halign(gtk::Align::Start);
                    let status = self.status.clone();
                    terminal.connect_clicked(move |_| {
                        let argv = [
                            OsStr::new("xdg-terminal-exec"),
                            OsStr::new("ssh"),
                            OsStr::new("-o"),
                            OsStr::new("StrictHostKeyChecking=yes"),
                            OsStr::new("--"),
                            OsStr::new(&alias),
                        ];
                        if gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE).is_err() {
                            status.set_text("Could not open a terminal. Check xdg-terminal-exec and SSH installation.");
                        }
                    });
                    card.append(&terminal);
                }
            }
            self.content.append(&gtk::Frame::builder().child(&card).build());
        }
    }
}
fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alias_boundaries_reject_options_and_shell_text() {
        for alias in ["-oProxyCommand=x", "host;touch", "user@host", "../host", ""] {
            assert!(!valid_id(alias));
        }
        assert!(valid_id("build-server"));
    }
    #[test]
    fn schema_and_connector_identity_are_required() {
        let mut value: Value =
            serde_json::from_str(include_str!("../../../mseries/fleet-shepherd/tests/fixtures/fleet_demo.json"))
                .unwrap();
        for row in value["connectors"].as_array_mut().unwrap() {
            if row["mode"] == "ssh" {
                row["focusTarget"] = row["id"].clone();
            }
        }
        assert!(connectors(&value).is_ok());
        value["schemaVersion"] = serde_json::json!(2);
        assert!(connectors(&value).is_err());
    }
    #[test]
    fn old_and_missing_observations_do_not_claim_fresh() {
        let row = serde_json::json!({});
        assert!(observed(&row, "", 100).contains("unknown"));
        assert!(observed(&row, "1970-01-01T00:00:01Z", 1000).contains("Stale"));
    }
}
