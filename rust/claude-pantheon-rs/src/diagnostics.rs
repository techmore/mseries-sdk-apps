//! The Diagnostics page: the server's resource telemetry subscription (sampled by its native
//! resource-monitor sidecar only while someone subscribes), shown as per-group CPU and RSS sparklines
//! and a process table. The client runs no sampler of its own.

use crate::app::App;
use crate::chat::label;
use crate::t3::Rpc;
use futures_util::StreamExt;
use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

const HISTORY: usize = 60;

/// (cpu %, rss bytes) samples, newest last.
type History = VecDeque<(f64, f64)>;

pub struct DiagnosticsPage {
    root: gtk::Box,
    status: gtk::Label,
    groups: gtk::Box,
    processes: gtk::Box,
    /// group name → (cpu %, rss bytes) history, newest last.
    history: RefCell<Vec<(String, History)>>,
    sub_id: RefCell<Option<String>>,
    samples: RefCell<u64>,
}

/// Effect encodes `Schema.Option` as `{"_tag":"Some","value":…}` / `{"_tag":"None"}`.
fn option_str(v: Option<&Value>) -> Option<&str> {
    let v = v?;
    v.as_str().or_else(|| v.get("value").and_then(Value::as_str))
}

fn mb(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else {
        format!("{:.0} MB", bytes / 1024.0 / 1024.0)
    }
}

fn sparkline(points: Vec<f64>, max_hint: f64) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(160).content_height(26).build();
    area.set_draw_func(move |w, cr, width, height| {
        let max = points.iter().cloned().fold(max_hint, f64::max).max(0.001);
        let color = w.color();
        cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, 0.8);
        cr.set_line_width(1.5);
        let n = points.len().max(2) as f64;
        for (i, p) in points.iter().enumerate() {
            let x = i as f64 / (n - 1.0) * width as f64;
            let y = height as f64 - (p / max) * (height as f64 - 2.0) - 1.0;
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        let _ = cr.stroke();
    });
    area
}

impl DiagnosticsPage {
    pub fn new() -> DiagnosticsPage {
        let status = label("", &["dim-label"]);
        status.set_margin_start(24);
        status.set_margin_top(12);
        let groups = gtk::Box::new(gtk::Orientation::Vertical, 8);
        groups.set_margin_start(24);
        groups.set_margin_end(24);
        groups.set_margin_top(12);
        let processes = gtk::Box::new(gtk::Orientation::Vertical, 2);
        processes.set_margin_start(24);
        processes.set_margin_end(24);
        processes.set_margin_top(12);
        processes.set_margin_bottom(18);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&status);
        column.append(&groups);
        column.append(&processes);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&column)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroll);
        DiagnosticsPage {
            root,
            status,
            groups,
            processes,
            history: RefCell::new(Vec::new()),
            sub_id: RefCell::new(None),
            samples: RefCell::new(0),
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn samples(&self) -> u64 {
        *self.samples.borrow()
    }

    /// Subscribes; the server samples only while this runs. `stop` ends it when the page is left.
    pub fn start(&self, app: Rc<App>, rpc: Rpc) {
        self.stop(&rpc);
        self.status.set_text("Waiting for the first sample…");
        let (id, mut stream) = rpc.subscribe("subscribeResourceTelemetry", json!({}));
        *self.sub_id.borrow_mut() = Some(id);
        glib::spawn_future_local(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(v) => app.diagnostics_page().apply(&v),
                    Err(e) => {
                        app.diagnostics_page().status.set_text(&format!("Telemetry stopped: {}", e.message()));
                        break;
                    }
                }
            }
        });
    }

    pub fn stop(&self, rpc: &Rpc) {
        if let Some(id) = self.sub_id.borrow_mut().take() {
            rpc.interrupt(&id);
        }
    }

    fn apply(&self, v: &Value) {
        *self.samples.borrow_mut() += 1;
        let interval = v.get("sampleIntervalMs").and_then(Value::as_u64).unwrap_or(0);
        let health = &v["health"];
        self.status.set_text(&format!(
            "every {} ms · sidecar {} · {} processes scanned, {} kept · collection {} µs",
            interval,
            option_str(health.get("sidecarVersion")).unwrap_or("?"),
            health.get("scannedProcessCount").and_then(Value::as_u64).unwrap_or(0),
            health.get("retainedProcessCount").and_then(Value::as_u64).unwrap_or(0),
            health.get("collectionDurationMicros").and_then(Value::as_u64).unwrap_or(0)
        ));
        {
            let mut history = self.history.borrow_mut();
            if let Some(groups) = v.get("groups").and_then(Value::as_object) {
                for (name, agg) in groups {
                    let cpu = agg.get("currentCpuPercent").and_then(Value::as_f64).unwrap_or(0.0);
                    let rss = agg.get("currentRssBytes").and_then(Value::as_f64).unwrap_or(0.0);
                    let slot = match history.iter_mut().find(|(n, _)| n == name) {
                        Some((_, h)) => h,
                        None => {
                            history.push((name.clone(), VecDeque::new()));
                            &mut history.last_mut().unwrap().1
                        }
                    };
                    slot.push_back((cpu, rss));
                    while slot.len() > HISTORY {
                        slot.pop_front();
                    }
                }
            }
        }
        while let Some(c) = self.groups.first_child() {
            self.groups.remove(&c);
        }
        let labels = [
            ("allT3", "All of T3"),
            ("backend", "Server"),
            ("electron", "Desktop app"),
            ("monitor", "Resource monitor"),
        ];
        for (name, hist) in self.history.borrow().iter() {
            let title = labels.iter().find(|(k, _)| *k == name).map(|(_, l)| *l).unwrap_or(name);
            let (cpu, rss) = hist.back().copied().unwrap_or((0.0, 0.0));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("card");
            row.add_css_class("pantheon-surface");
            row.add_css_class("diag-row");
            let t = label(title, &["title-4"]);
            t.set_width_chars(18);
            row.append(&t);
            row.append(&label(&format!("{cpu:.1} % CPU"), &["usage-num"]));
            row.append(&sparkline(hist.iter().map(|(c, _)| *c).collect(), 10.0));
            row.append(&label(&mb(rss), &["usage-num"]));
            row.append(&sparkline(hist.iter().map(|(_, r)| *r / 1024.0 / 1024.0).collect(), 100.0));
            self.groups.append(&row);
        }
        while let Some(c) = self.processes.first_child() {
            self.processes.remove(&c);
        }
        if let Some(procs) = v.get("processes").and_then(Value::as_array) {
            let mut rows: Vec<&Value> = procs.iter().collect();
            rows.sort_by(|a, b| {
                b.get("residentBytes")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .partial_cmp(&a.get("residentBytes").and_then(Value::as_f64).unwrap_or(0.0))
                    .unwrap()
            });
            let head = label("Processes by resident memory", &["title-4"]);
            self.processes.append(&head);
            for p in rows.iter().take(25) {
                let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(0) as usize;
                let line = format!(
                    "{}{}  {}  {:.1} %  {}",
                    "  ".repeat(depth),
                    p.get("name").and_then(Value::as_str).unwrap_or("?"),
                    p.get("category").and_then(Value::as_str).unwrap_or(""),
                    p.get("cpuPercent").and_then(Value::as_f64).unwrap_or(0.0),
                    mb(p.get("residentBytes").and_then(Value::as_f64).unwrap_or(0.0))
                );
                let l = label(&line, &["monospace", "diag-proc"]);
                l.set_tooltip_text(p.get("command").and_then(Value::as_str));
                self.processes.append(&l);
            }
        }
    }
}

impl Default for DiagnosticsPage {
    fn default() -> Self {
        Self::new()
    }
}
