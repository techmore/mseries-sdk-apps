//! The Usage page: what the server read from every provider's local history, in the dense card style
//! of the CodexBar panel: one card per provider with a token-first bar row, a model table, cache
//! savings and API-equivalent cost, and a 7-day sparkline.

use crate::app::App;
use crate::chat::label;
use crate::t3::Rpc;
use crate::t3::model::{Provider, UsageBucket, UsageSummary, UsageTotals, pick_provider};
use gtk::{glib, pango, prelude::*};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub struct UsagePage {
    root: gtk::Box,
    windows: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    summary: RefCell<Option<UsageSummary>>,
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
        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let windows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        windows.set_margin_start(24);
        windows.set_margin_end(24);
        windows.set_margin_top(12);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&status);
        root.append(&windows);
        root.append(&scroll);
        UsagePage { root, windows, content, status, summary: RefCell::new(None) }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn summary(&self) -> Option<UsageSummary> {
        self.summary.borrow().clone()
    }

    /// Plan windows (5-hour session, weekly, monthly) per provider as gauges with reset countdowns,
    /// and the rotation decision for the next unattended task.
    pub fn render_windows(&self, providers: &[Provider]) {
        while let Some(c) = self.windows.first_child() {
            self.windows.remove(&c);
        }
        let now = crate::look::now_unix();
        let with_limits: Vec<&Provider> =
            providers.iter().filter(|p| p.usage_limits.as_ref().is_some_and(|u| !u.windows.is_empty())).collect();
        if with_limits.is_empty() {
            return;
        }
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("card");
        card.add_css_class("usage-card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let t = label("Plan windows", &["title-4"]);
        t.set_hexpand(true);
        head.append(&t);
        if let Some((_, reason)) = pick_provider(providers) {
            let decision = label(&reason, &["usage-decision"]);
            decision.set_halign(gtk::Align::End);
            head.append(&decision);
        } else {
            head.append(&label("every window is full; the runner idles until the earliest reset", &["usage-decision"]));
        }
        card.append(&head);
        let grid = gtk::Grid::builder().column_spacing(14).row_spacing(6).build();
        let mut row = 0;
        for p in with_limits {
            let name = label(&p.name(), &["usage-provider"]);
            name.set_valign(gtk::Align::Start);
            grid.attach(&name, 0, row, 1, 1);
            for w in &p.usage_limits.as_ref().unwrap().windows {
                let kind = label(&w.label, &["caption", "dim-label"]);
                kind.set_width_chars(8);
                grid.attach(&kind, 1, row, 1, 1);
                let gauge = gtk::LevelBar::builder()
                    .min_value(0.0)
                    .max_value(100.0)
                    .value(w.used_percent)
                    .hexpand(true)
                    .valign(gtk::Align::Center)
                    .build();
                gauge.add_offset_value("low", 60.0);
                gauge.add_offset_value("high", 90.0);
                gauge.add_offset_value("full", 100.0);
                gauge.add_css_class("usage-gauge");
                grid.attach(&gauge, 2, row, 1, 1);
                let pct = label(&format!("{:.0}%", w.used_percent), &["usage-num"]);
                pct.set_xalign(1.0);
                pct.set_width_chars(5);
                grid.attach(&pct, 3, row, 1, 1);
                let reset = w
                    .resets_at
                    .as_deref()
                    .map(|r| format!("resets {} ({})", crate::look::countdown(r, now), crate::look::clock(r)))
                    .unwrap_or_default();
                grid.attach(&label(&reset, &["caption", "dim-label"]), 4, row, 1, 1);
                row += 1;
            }
            if row == 0 {
                row += 1;
            }
        }
        card.append(&grid);
        self.windows.append(&card);
    }

    pub fn load(&self, app: Rc<App>, rpc: Rpc) {
        self.render_windows(&app.providers());
        self.status.set_text("Reading usage…");
        let today = glib::DateTime::now_local().unwrap();
        let since = today.add_days(-30).unwrap();
        let fmt = |d: &glib::DateTime| d.format("%Y-%m-%d").unwrap().to_string();
        let payload = json!({"sinceDay": fmt(&since), "untilDay": fmt(&today), "timeZone": today.timezone().identifier().to_string()});
        let last7: Vec<String> = (0..7).rev().map(|i| fmt(&today.add_days(-i).unwrap())).collect();
        glib::spawn_future_local(async move {
            match rpc.call("server.getUsageSummary", payload).await {
                Ok(v) => match serde_json::from_value::<UsageSummary>(v) {
                    Ok(summary) => app.usage_page().render(&summary, &last7),
                    Err(e) => app.usage_page().status.set_text(&format!("Unexpected usage shape: {e}")),
                },
                Err(e) => app.usage_page().status.set_text(&format!("Usage failed: {}", e.message())),
            }
        });
    }

    pub fn render(&self, summary: &UsageSummary, last7: &[String]) {
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
        let providers = fold(summary, last7);
        let total_cost: f64 = providers.iter().map(|p| p.cost).sum();
        let total_savings: f64 = providers.iter().map(|p| p.savings).sum();
        let mut total_tokens = UsageTotals::default();
        for p in &providers {
            total_tokens.add(&p.totals);
        }
        self.status.set_text(&format!(
            "{} to {} · {} buckets · scanned in {} ms · pricing {}",
            summary.since_day,
            summary.until_day,
            summary.buckets.len(),
            summary.scan_duration_ms,
            summary.pricing.get("status").and_then(|v| v.as_str()).unwrap_or("?")
        ));

        // Totals strip.
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        strip.add_css_class("card");
        strip.add_css_class("usage-card");
        strip.append(&stat("tokens, 30 days", &tokens_text(total_tokens.total())));
        strip.append(&stat("output", &tokens_text(total_tokens.output)));
        strip.append(&stat("cache reads", &tokens_text(total_tokens.cached_input)));
        strip.append(&stat("API-equivalent cost", &usd(total_cost)));
        strip.append(&stat("saved by caching", &usd(total_savings)));
        self.content.append(&strip);

        for p in &providers {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.add_css_class("card");
            card.add_css_class("usage-card");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let mark = label(
                &p.kind.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
                &["usage-mark"],
            );
            head.append(&mark);
            let name = label(provider_title(&p.kind), &["title-4"]);
            head.append(&name);
            let sessions = label(&format!("{} sessions", p.sessions), &["caption", "dim-label"]);
            sessions.set_hexpand(true);
            head.append(&sessions);
            head.append(&sparkline(p.days.clone()));
            card.append(&head);

            // Token-first bar row: uncached input, cache writes, cache reads, output.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let tokens = label(&format!("{} tokens", tokens_text(p.totals.total())), &["usage-tokens"]);
            row.append(&tokens);
            let bar = bar_row(&[
                (p.totals.uncached_input as f64, "input"),
                (p.totals.cache_creation as f64, "cache write"),
                (p.totals.cached_input as f64, "cache read"),
                (p.totals.output as f64, "output"),
            ]);
            bar.set_valign(gtk::Align::Center);
            row.append(&bar);
            let cost = label(&format!("{} · saved {}", usd(p.cost), usd(p.savings)), &["usage-cost"]);
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
                grid.attach(&name, 0, r as i32 + 1, 1, 1);
                for (c, text) in [tokens_text(t.input()), tokens_text(t.output), usd(*cost)].iter().enumerate() {
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
