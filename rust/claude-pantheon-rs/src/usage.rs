//! Read-only usage monitor. T3 token history and account plan observations come from the
//! selected server; supplemental quota reports describe this computer only.

use crate::app::App;
use crate::chat::label;
use crate::t3::Rpc;
use crate::t3::model::{Provider, ServerConfig, UsageBucket, UsageSummary, UsageTotals};
use gtk::{glib, pango, prelude::*};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

pub struct UsagePage {
    root: gtk::Box,
    context: gtk::Label,
    windows: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    summary: RefCell<Option<UsageSummary>>,
    local: Rc<crate::quota_view::QuotaView>,
    generation: Cell<u64>,
    received_at: Cell<Option<i64>>,
    last_providers: RefCell<Vec<Provider>>,
    history_states: RefCell<Vec<gtk::Label>>,
}

pub fn tokens_text(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

pub fn usd(v: f64) -> String {
    if v >= 100.0 { format!("${v:.0}") } else { format!("${v:.2}") }
}

pub fn provider_title(kind: &str) -> &str {
    match kind {
        "claude" => "Claude",
        "codex" => "Codex",
        "grok" => "Grok",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        "antigravity" => "Antigravity",
        "pi" => "Pi",
        other => other,
    }
}

/// Provider order as CodexBar draws it: Codex, Claude, OpenCode, Grok, then the rest.
fn provider_rank(kind: &str) -> usize {
    match kind {
        "codex" => 0,
        "claude" => 1,
        "opencode" => 2,
        "grok" => 3,
        _ => 9,
    }
}

#[derive(Default, Clone)]
pub struct ProviderUsage {
    pub kind: String,
    pub totals: UsageTotals,
    pub cost: f64,
    pub savings: f64,
    pub sessions: u64,
    pub models: BTreeMap<String, (UsageTotals, f64)>,
    /// Tokens per day, last 7 days, oldest first.
    pub days: Vec<(String, u64)>,
}

/// Folds buckets into one entry per provider.
pub fn fold(summary: &UsageSummary, last_days: &[String]) -> Vec<ProviderUsage> {
    let mut by: BTreeMap<String, ProviderUsage> = BTreeMap::new();
    for b in &summary.buckets {
        let e = by.entry(b.provider.clone()).or_insert_with(|| ProviderUsage {
            kind: b.provider.clone(),
            days: last_days.iter().map(|d| (d.clone(), 0)).collect(),
            ..Default::default()
        });
        e.totals.add(&b.totals);
        e.cost += b.cost_usd;
        e.savings += b.cache_savings_usd;
        let m = e.models.entry(b.model.clone()).or_default();
        m.0.add(&b.totals);
        m.1 += b.cost_usd;
        if let Some(slot) = e.days.iter_mut().find(|(d, _)| *d == b.day) {
            slot.1 += b.totals.total();
        }
    }
    for s in &summary.sources {
        if let Some(e) = by.get_mut(&s.provider()) {
            e.sessions += s.distinct_sessions;
        }
    }
    let mut out: Vec<ProviderUsage> = by.into_values().collect();
    out.sort_by_key(|p| (provider_rank(&p.kind), p.kind.clone()));
    out
}

fn sparkline(days: Vec<(String, u64)>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(140).content_height(28).hexpand(true).build();
    area.set_tooltip_text(Some("Tokens per day, last 7 days"));
    area.set_draw_func(move |w, cr, width, height| {
        let max = days.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1) as f64;
        let n = days.len().max(1) as f64;
        let gap = 3.0;
        let bar = ((width as f64 - gap * (n - 1.0)) / n).max(2.0);
        let color = w.color();
        for (i, (_, v)) in days.iter().enumerate() {
            let h = (*v as f64 / max * (height as f64 - 2.0)).max(if *v > 0 { 2.0 } else { 1.0 });
            let x = i as f64 * (bar + gap);
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                if *v > 0 { 0.75 } else { 0.2 },
            );
            cr.rectangle(x, height as f64 - h, bar, h);
            let _ = cr.fill();
        }
    });
    area
}

fn bar_row(parts: &[(f64, &str)]) -> gtk::DrawingArea {
    let parts: Vec<(f64, String)> = parts.iter().map(|(v, c)| (*v, c.to_string())).collect();
    let area = gtk::DrawingArea::builder().content_height(6).hexpand(true).build();
    area.set_draw_func(move |w, cr, width, height| {
        let total: f64 = parts.iter().map(|(v, _)| v).sum::<f64>().max(1.0);
        let color = w.color();
        let mut x = 0.0;
        for (i, (v, _)) in parts.iter().enumerate() {
            let wpx = v / total * width as f64;
            let alpha = [0.9, 0.55, 0.3, 0.15][i % 4];
            cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, alpha);
            cr.rectangle(x, 0.0, wpx, height as f64);
            let _ = cr.fill();
            x += wpx;
        }
    });
    area
}

fn stat(title: &str, value: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let v = label(value, &["usage-value"]);
    let t = label(title, &["caption", "dim-label"]);
    b.append(&v);
    b.append(&t);
    b
}

impl UsagePage {
    pub fn new() -> UsagePage {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let status = label("", &["dim-label"]);
        status.set_margin_start(24);
        status.set_margin_top(12);
        status.set_margin_end(24);
        status.set_wrap(true);
        status.set_ellipsize(pango::EllipsizeMode::None);
        let context = label("T3 server · not connected. Computer-local reports are separate below.", &["dim-label"]);
        context.set_wrap(true);
        context.set_wrap_mode(pango::WrapMode::WordChar);
        context.set_margin_start(24);
        context.set_margin_end(24);
        context.set_margin_top(12);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let local = crate::quota_view::QuotaView::new();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        toolbar.set_margin_start(24);
        toolbar.set_margin_end(24);
        toolbar.set_margin_top(12);
        let title = label("Usage", &["title-2"]);
        title.set_hexpand(true);
        toolbar.append(&title);
        let refresh = gtk::Button::with_label("Refresh usage");
        refresh.set_action_name(Some("win.usage"));
        refresh.set_tooltip_text(Some("Refresh T3 server history, account plan windows and computer-local reports"));
        refresh.update_property(&[gtk::accessible::Property::Label("Refresh usage monitor")]);
        toolbar.append(&refresh);
        body.append(&toolbar);
        body.append(&context);
        body.append(&status);
        let windows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        windows.set_margin_start(24);
        windows.set_margin_end(24);
        windows.set_margin_top(12);
        body.append(&windows);
        body.append(&content);
        body.append(local.widget());
        let scroll = gtk::ScrolledWindow::builder().child(&body)
            .hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        scroll.update_property(&[gtk::accessible::Property::Label("Usage monitor: server plan windows, token history and computer-local reports")]);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroll);
        UsagePage { root, context, windows, content, status, summary: RefCell::new(None), local,
            generation: Cell::new(0), received_at: Cell::new(None), last_providers: RefCell::new(Vec::new()),
            history_states: RefCell::new(Vec::new()) }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn summary(&self) -> Option<UsageSummary> {
        self.summary.borrow().clone()
    }

    pub fn load_local(&self) {
        self.local.load();
    }

    pub fn render_local(&self, snapshot: &crate::quota::Snapshot) {
        self.local.render(snapshot);
    }

    /// Invalidate in-flight completions when the monitor is left or its source changes.
    pub fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        for label in self.history_states.borrow().iter() {
            label.set_text("Last-known historical scan; refresh to check freshness");
        }
    }

    pub(crate) fn history_failed(&self, message: &str) {
        let retained = self.summary.borrow().is_some();
        for label in self.history_states.borrow().iter() {
            label.set_text("Stale last-known history · latest refresh failed");
        }
        if !retained {
            while let Some(child) = self.content.first_child() {
                self.content.remove(&child);
            }
            self.content.append(&label("T3 server token history unavailable. Refresh to try again.", &["dim-label"]));
        }
        let receipt = self.received_at.get().map(|t| format!(" · last received {}", crate::look::relative_time(t, crate::look::now_unix())))
            .unwrap_or_default();
        self.status.set_text(&format!("T3 server token history · {message}{}{receipt}",
            if retained { " · last-known history is stale; not a live quota observation" } else { "" }));
    }

    /// Keep account identities but never keep gauges live after a failed plan refresh.
    pub fn plan_refresh_failed(&self, providers: &[Provider], message: &str) {
        let mut rows = crate::usage_monitor::account_rows(providers, crate::look::now_unix());
        for row in &mut rows {
            row["detail"] = json!("Server plan refresh unavailable; any amounts are last known, not live quota");
            row["state"] = json!("unavailable");
            if let Some(windows) = row["windows"].as_array_mut() {
                for window in windows {
                    window["state"] = json!("unavailable");
                    window["live"] = json!(false);
                }
            }
        }
        self.render_plan_rows(rows);
        let error = label(&format!("Server plan refresh failed: {message}. Any shown percentages are last known, not live."),
            &["dim-label"]);
        error.set_wrap(true);
        self.windows.append(&error);
    }

    pub fn offline(&self) {
        self.invalidate();
        while let Some(child) = self.windows.first_child() {
            self.windows.remove(&child);
        }
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        *self.summary.borrow_mut() = None;
        self.received_at.set(None);
        self.last_providers.borrow_mut().clear();
        self.history_states.borrow_mut().clear();
        self.context.set_text("T3 server · not connected. Computer-local reports are separate below.");
        self.status.set_text("Connect to a T3 server for token history and server plan windows.");
    }

    /// Account identities and plan windows only; this monitor makes no routing recommendation.
    pub fn render_windows(&self, providers: &[Provider]) {
        self.render_plan_rows(crate::usage_monitor::account_rows(providers, crate::look::now_unix()));
    }

    fn render_plan_rows(&self, rows: Vec<serde_json::Value>) {
        while let Some(c) = self.windows.first_child() {
            self.windows.remove(&c);
        }
        let title = label("T3 server · account plan windows", &["title-4"]);
        self.windows.append(&title);
        let note = label("Percent of subscription plan used, not token totals or a best-account recommendation. Fresh observations are at most 15 minutes old.",
            &["caption", "dim-label"]);
        note.set_wrap(true);
        self.windows.append(&note);
        if rows.is_empty() {
            self.windows.append(&label("No enabled accounts reported by this server.", &["dim-label"]));
        }
        for row in rows {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
            for class in ["card", "pantheon-surface", "usage-card"] {
                card.add_css_class(class);
            }
            let state = row["state"].as_str().unwrap_or("unknown");
            let heading = format!("{} · {} · {state}", row["title"].as_str().unwrap_or("Account"),
                row["instance_id"].as_str().unwrap_or("Unknown account"));
            let name = label(&heading, &["usage-provider"]);
            name.set_wrap(true);
            name.set_wrap_mode(pango::WrapMode::WordChar);
            card.append(&name);
            let checked = row["checked_at"].as_str().filter(|s| !s.is_empty()).unwrap_or("not supplied");
            let detail = label(&format!("{} · server checked at {checked}", row["detail"].as_str().unwrap_or("")),
                &["caption", "dim-label"]);
            detail.set_wrap(true);
            detail.set_wrap_mode(pango::WrapMode::WordChar);
            card.append(&detail);
            for w in row["windows"].as_array().into_iter().flatten() {
                let title = w["title"].as_str().filter(|s| !s.is_empty()).unwrap_or("Plan window");
                let wstate = w["state"].as_str().unwrap_or("unknown");
                let amount = w["used_percent"].as_f64();
                let text = match amount {
                    Some(pct) if wstate == "fresh" => format!("{title} · {pct:.0}% used"),
                    Some(pct) => format!("{title} · last known {pct:.0}% used · {wstate}"),
                    None => format!("{title} · quota unknown · {wstate}"),
                };
                let description = label(&text, &["caption"]);
                description.set_wrap(true);
                card.append(&description);
                if w["live"].as_bool() == Some(true) {
                    if let Some(pct) = amount {
                        let gauge = gtk::LevelBar::builder().min_value(0.0).max_value(100.0)
                            .value(pct).hexpand(true).build();
                        gauge.add_offset_value("low", 60.0);
                        gauge.add_offset_value("high", 90.0);
                        gauge.add_offset_value("full", 100.0);
                        gauge.add_css_class("usage-gauge");
                        gauge.update_property(&[gtk::accessible::Property::Label(&format!("{heading}, {text}"))]);
                        card.append(&gauge);
                    }
                }
                if let Some(reset) = w["resets_at"].as_str() {
                    let reset = label(&format!("Reported reset: {reset}"), &["caption", "dim-label"]);
                    reset.set_wrap(true);
                    card.append(&reset);
                }
            }
            card.update_property(&[gtk::accessible::Property::Label(&heading)]);
            self.windows.append(&card);
        }
    }

    pub fn load(&self, app: Rc<App>, rpc: Rpc) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let previous_providers = if self.last_providers.borrow().is_empty() {
            app.providers()
        } else { self.last_providers.borrow().clone() };
        self.render_windows(&previous_providers);
        let current = app.current_environment_id();
        let environments = app.environments();
        let environment = environments.iter().find(|e| Some(e.id.as_str()) == current.as_deref())
            .map(|e| e.name.as_str()).unwrap_or("Default environment");
        self.context.set_text(&format!("T3 server · {environment} · {}. Plan windows and token history are server-side; computer-local reports are separate below.",
            app.current_server()));
        for label in self.history_states.borrow().iter() {
            label.set_text("Last-known historical scan · refreshing; freshness not yet confirmed");
        }
        self.status.set_text(if self.summary.borrow().is_some() {
            "T3 server token history · refreshing; showing last-known history"
        } else { "T3 server token history · reading history…" });
        if self.content.first_child().is_none() {
            self.content.append(&crate::skeleton::rows(4));
        }
        // Dock and the full Usage page share one guarded read on App's existing connection.
        let source_generation = app.usage_source_generation();
        let environment_id = app.current_environment_id();
        app.refresh_usage_history();
        glib::spawn_future_local(async move {
            let config = rpc.call("server.getConfig", json!({})).await;
            if app.usage_page().generation.get() != generation
                || app.usage_source_generation() != source_generation
                || app.current_environment_id() != environment_id {
                return;
            }
            match config {
                Ok(v) if v["providers"].is_array() => match serde_json::from_value::<ServerConfig>(v) {
                    Ok(config) => {
                        app.update_plan_providers(config.providers.clone());
                        app.usage_page().render_windows(&config.providers);
                        *app.usage_page().last_providers.borrow_mut() = config.providers;
                    }
                    Err(_) => app.usage_page().plan_refresh_failed(&previous_providers, "Invalid server account response"),
                },
                Ok(_) => app.usage_page().plan_refresh_failed(&previous_providers, "Missing server account list"),
                Err(e) => app.usage_page().plan_refresh_failed(&previous_providers, &e.message()),
            }
        });
    }

    pub fn render(&self, summary: &UsageSummary, last7: &[String]) {
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
        self.history_states.borrow_mut().clear();
        let providers = fold(summary, last7);
        let total_cost: f64 = providers.iter().map(|p| p.cost).sum();
        let total_savings: f64 = providers.iter().map(|p| p.savings).sum();
        let mut total_tokens = UsageTotals::default();
        for p in &providers {
            total_tokens.add(&p.totals);
        }
        let received = crate::look::now_unix();
        let history_rows = crate::usage_monitor::history_rows_at(summary, received);
        let all_priced = !history_rows.is_empty() && history_rows.iter().all(|r| r["api_equivalent_cost_usd"].is_number());
        let historical: BTreeMap<&str, &serde_json::Value> = history_rows.iter()
            .filter_map(|row| row["provider"].as_str().map(|provider| (provider, row))).collect();
        self.received_at.set(Some(received));
        self.status.set_text(&format!(
            "T3 server token history · {} to {} · server scan reported at {} · received {} · {} buckets · scan {} ms. Historical provider totals, not account quotas; per-provider freshness below.",
            summary.since_day, summary.until_day,
            if summary.read_at.is_empty() { "not supplied" } else { &summary.read_at },
            glib::DateTime::from_unix_local(received).ok().and_then(|d| d.format("%Y-%m-%d %H:%M %Z").ok())
                .map(|s| s.to_string()).unwrap_or_else(|| received.to_string()),
            summary.buckets.len(), summary.scan_duration_ms
        ));
        let title = label("T3 server · token history", &["title-4"]);
        self.content.append(&title);
        let explanation = label("Provider totals combine historical model/day records across server accounts. Reasoning tokens are already included in output. Costs are API-equivalent estimates, not your subscription bill.",
            &["caption", "dim-label"]);
        explanation.set_wrap(true);
        self.content.append(&explanation);
        if providers.is_empty() {
            self.content.append(&label("No token buckets reported for this range. Missing or failed sources do not imply zero usage.", &["dim-label"]));
        }

        // A flow box wraps the totals rather than imposing a wide minimum window.
        let strip = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None)
            .homogeneous(false).column_spacing(24).row_spacing(12).max_children_per_line(5).build();
        strip.add_css_class("card");
        strip.add_css_class("pantheon-surface");
        strip.add_css_class("usage-card");
        if !providers.is_empty() {
            strip.insert(&stat("tokens in reported range", &tokens_text(total_tokens.total())), -1);
            strip.insert(&stat("output (includes reasoning)", &tokens_text(total_tokens.output)), -1);
            strip.insert(&stat("cache reads", &tokens_text(total_tokens.cached_input)), -1);
            strip.insert(&stat("API-equivalent cost", &if all_priced { usd(total_cost) } else { "Unknown / incomplete".into() }), -1);
            strip.insert(&stat("estimated cache savings", &if all_priced { usd(total_savings) } else { "Unknown / incomplete".into() }), -1);
        }
        self.content.append(&strip);

        for p in &providers {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.add_css_class("card");
            card.add_css_class("pantheon-surface");
            card.add_css_class("usage-card");
            let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let mark = label(
                &p.kind.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
                &["usage-mark"],
            );
            head.append(&mark);
            let history = historical.get(p.kind.as_str()).copied();
            let state = history.and_then(|r| r["state"].as_str()).unwrap_or("unknown");
            let name = label(&format!("{} · provider aggregate", provider_title(&p.kind)), &["title-4"]);
            name.set_wrap(true);
            head.append(&name);
            let freshness = label(&format!("{state} historical scan · not live quota"), &["caption", "dim-label"]);
            freshness.set_wrap(true);
            head.append(&freshness);
            self.history_states.borrow_mut().push(freshness);
            let sessions = label(&format!("{} reported sessions · last 7 days below", p.sessions), &["caption", "dim-label"]);
            sessions.set_wrap(true);
            sessions.set_hexpand(true);
            head.append(&sessions);
            card.append(&head);
            let chart = sparkline(p.days.clone());
            chart.update_property(&[gtk::accessible::Property::Label(&format!("{} historical tokens, last seven days: {}",
                provider_title(&p.kind), p.days.iter().map(|(day, n)| format!("{day}: {n}")).collect::<Vec<_>>().join(", ")))]);
            card.append(&chart);

            // Token-first bar row: uncached input, cache writes, cache reads, output.
            let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let tokens = label(&format!("{} tokens", tokens_text(p.totals.total())), &["usage-tokens"]);
            row.append(&tokens);
            let bar = bar_row(&[
                (p.totals.uncached_input as f64, "input"),
                (p.totals.cache_creation as f64, "cache write"),
                (p.totals.cached_input as f64, "cache read"),
                (p.totals.output as f64, "output"),
            ]);
            bar.update_property(&[gtk::accessible::Property::Label("Historical token proportions: input, cache writes, cache reads and output; not plan usage")]);
            bar.set_valign(gtk::Align::Center);
            row.append(&bar);
            let priced = history.is_some_and(|r| r["api_equivalent_cost_usd"].is_number());
            let cost = label(&if priced { format!("{} API-equivalent · estimated cache savings {}", usd(p.cost), usd(p.savings)) }
                else { "API-equivalent cost / cache savings unknown or incomplete".into() }, &["usage-cost"]);
            cost.set_wrap(true);
            row.append(&cost);
            card.append(&row);
            let legend = label(
                &format!(
                    "input {} · cache write {} · cache read {} · output {} ({} reasoning)",
                    tokens_text(p.totals.uncached_input),
                    tokens_text(p.totals.cache_creation),
                    tokens_text(p.totals.cached_input),
                    tokens_text(p.totals.output),
                    tokens_text(p.totals.reasoning)
                ),
                &["caption", "dim-label"],
            );
            legend.set_wrap(true);
            card.append(&legend);

            // Model breakdown.
            let grid = gtk::Grid::builder().column_spacing(16).row_spacing(2).build();
            grid.add_css_class("usage-models");
            for (c, h) in ["model", "input", "output", "cost"].iter().enumerate() {
                let l = label(h, &["caption", "dim-label"]);
                if c > 0 {
                    l.set_xalign(1.0);
                }
                grid.attach(&l, c as i32, 0, 1, 1);
            }
            let mut models: Vec<(&String, &(UsageTotals, f64))> = p.models.iter().collect();
            models.sort_by(|a, b| b.1.1.partial_cmp(&a.1.1).unwrap_or(std::cmp::Ordering::Equal));
            for (r, (model, (t, cost))) in models.iter().enumerate().take(8) {
                let name = label(model, &["usage-model"]);
                name.set_hexpand(true);
                name.set_wrap(true);
                name.set_wrap_mode(pango::WrapMode::WordChar);
                name.set_max_width_chars(24);
                grid.attach(&name, 0, r as i32 + 1, 1, 1);
                for (c, text) in [tokens_text(t.input()), tokens_text(t.output),
                    if priced { usd(*cost) } else { "Unknown".into() }].iter().enumerate() {
                    let l = label(text, &["usage-num"]);
                    l.set_xalign(1.0);
                    grid.attach(&l, c as i32 + 1, r as i32 + 1, 1, 1);
                }
            }
            card.append(&grid);
            self.content.append(&card);
        }

        // Sources that could not be read.
        let problems: Vec<String> = summary
            .sources
            .iter()
            .filter(|s| s.status != "ok")
            .map(|s| {
                format!(
                    "{}: {}{}",
                    provider_title(&s.provider()),
                    s.status,
                    s.message.as_ref().map(|m| format!(" ({m})")).unwrap_or_default()
                )
            })
            .collect();
        if !problems.is_empty() {
            let mut seen = std::collections::BTreeSet::new();
            let text = problems.into_iter().filter(|p| seen.insert(p.clone())).collect::<Vec<_>>().join("\n");
            let l = label(&text, &["caption", "dim-label"]);
            l.set_wrap(true);
            l.set_ellipsize(pango::EllipsizeMode::None);
            self.content.append(&l);
        }
        *self.summary.borrow_mut() = Some(summary.clone());
    }
}

impl Default for UsagePage {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
fn _bucket_type_check(_: &UsageBucket) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_buckets_per_provider() {
        let summary: UsageSummary = serde_json::from_str(include_str!("../tests/fixtures/usage-summary.json")).unwrap();
        let days: Vec<String> =
            ["2026-10-15", "2026-10-16", "2026-10-17", "2026-10-18", "2026-10-19", "2026-10-20", "2026-10-21"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let folded = fold(&summary, &days);
        assert!(folded.iter().any(|p| p.kind == "claude"));
        let claude = folded.iter().find(|p| p.kind == "claude").unwrap();
        assert!(claude.totals.total() > 0);
        assert!(claude.cost > 0.0);
        assert_eq!(folded[0].kind, "codex", "Codex first, as CodexBar orders cards");
        assert_eq!(tokens_text(1_234_567), "1.2M");
        assert_eq!(usd(4.5), "$4.50");
    }
}
