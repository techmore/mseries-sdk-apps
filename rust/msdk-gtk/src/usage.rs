//! Candidate reusable usage rows. Host placement, refresh, authentication,
//! focus/dismissal and provider collectors belong to separate adapters.
use gtk::prelude::*;
use msdk::usage::{Coverage, Observation, Snapshot, SnapshotError, State};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! { static STYLE_INSTALLED: Cell<bool> = const { Cell::new(false) }; }

fn install_style() {
    STYLE_INSTALLED.with(|installed| {
        if installed.get() {
            return;
        }
        if let Some(display) = gtk::gdk::Display::default() {
            let css = gtk::CssProvider::new();
            css.load_from_string(".msdk-usage .usage-card { padding: 10px; border-radius: 10px; background: alpha(currentColor, 0.045); box-shadow: inset 0 0 0 1px alpha(currentColor, 0.06); } .msdk-usage .usage-provider { font-weight: 700; } .msdk-usage .usage-account { font-weight: 600; } .msdk-usage .usage-detail { opacity: 0.8; } ");
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            installed.set(true);
        }
    });
}

fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

// Match the optional marks installed by the existing CodexBar package. Keep
// the provider label visible whether or not that package or icon exists.
fn provider_mark(provider: &str) -> Option<gtk::Image> {
    let icon = match provider {
        "codex" => "mseries-codexbar-openai-symbolic",
        "claude" => "mseries-codexbar-claude-symbolic",
        "grok" => "mseries-codexbar-grok-symbolic",
        "opencode" | "opencodego" => "mseries-codexbar-opencode-symbolic",
        _ => return None,
    };
    let display = gtk::gdk::Display::default()?;
    if !gtk::IconTheme::for_display(&display).has_icon(icon) {
        return None;
    }
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(18);
    Some(image)
}

fn state_text(state: State) -> &'static str {
    match state {
        State::Loading => "Loading",
        State::Empty => "No usage",
        State::Unknown => "Unknown",
        State::Fresh => "Current",
        State::Stale => "Stale",
        State::Unavailable => "Unavailable",
        State::Failed => "Refresh failed",
        State::Unsupported => "Unsupported",
        State::Expired => "Expired — refresh required",
    }
}

fn append_observation(parent: &gtk::Box, observation: &Observation, now: i64) {
    let heading = observation
        .account_label
        .as_deref()
        .unwrap_or("Provider total");
    let title = if observation.selected {
        format!("{heading} · Selected")
    } else {
        heading.to_string()
    };
    let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let heading = label(&title);
    heading.add_css_class("usage-account");
    row.append(&heading);
    if observation.state != State::Fresh {
        let status = label(state_text(observation.state));
        status.add_css_class("usage-detail");
        row.append(&status);
    }
    for window in &observation.windows {
        let value = match window.quota {
            Some(quota)
                if matches!(
                    quota.state_at(observation.state, now),
                    State::Fresh | State::Stale
                ) =>
            {
                let basis = quota.remaining_basis_points();
                let reset = quota
                    .reset_at()
                    .and_then(|at| gtk::glib::DateTime::from_unix_local(at).ok())
                    .and_then(|at| at.format("%x %X").ok())
                    .map(|at| format!("Resets {at}"))
                    .unwrap_or_else(|| "Reset unknown".into());
                let value = if basis % 100 == 0 {
                    format!("{}%", basis / 100)
                } else {
                    format!("{}.{:02}%", basis / 100, basis % 100)
                };
                let detail = label(&reset);
                detail.add_css_class("usage-detail");
                let line = format!("{}: {value} remaining", window.label);
                row.append(&label(&line));
                row.append(&detail);
                continue;
            }
            Some(quota) => format!(
                "{}: {}",
                window.label,
                state_text(quota.state_at(observation.state, now))
            ),
            None => format!("{}: Unknown", window.label),
        };
        row.append(&label(&value));
    }
    if let Some(tokens) = observation.tokens {
        let value = match (observation.state, tokens.total()) {
            (State::Fresh, Some(total)) => format!("Tokens: {total}"),
            (State::Stale, Some(total)) => format!("Tokens: {total}"),
            (_, Some(_)) => format!("Tokens: {}", state_text(observation.state)),
            (_, None) => "Tokens: Unknown".into(),
        };
        row.append(&label(&value));
    }
    parent.append(&row);
}

/// Build a fresh, read-only view from a validated snapshot. The caller owns
/// updates and must preserve focus/scroll when embedding it in an interactive
/// host. This function neither starts polling nor performs account actions.
pub fn render(snapshot: &Snapshot, now: i64) -> Result<gtk::Box, SnapshotError> {
    let groups = snapshot.groups()?;
    install_style();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("msdk-usage");
    if groups.is_empty() {
        root.append(&label(match snapshot.coverage {
            Coverage::Complete => "No usage available",
            Coverage::Partial => "Usage coverage is partial",
            Coverage::Unknown => "Usage coverage is unknown",
        }));
    } else {
        // Show provider coverage before the scrollable cards. A provider can
        // appear at several sources; list its name once without merging cards.
        let mut seen = std::collections::HashSet::new();
        let names: Vec<&str> = groups
            .iter()
            .filter_map(|group| seen.insert(group.provider).then_some(group.label))
            .collect();
        if names.len() > 1 {
            root.append(&label(&format!("Providers: {}", names.join(" · "))));
        }
        if !matches!(snapshot.coverage, Coverage::Complete) {
            root.append(&label(match snapshot.coverage {
                Coverage::Partial => "Usage coverage is partial",
                _ => "Usage coverage is unknown",
            }));
        }
    }
    for group in groups {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("usage-card");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if let Some(mark) = provider_mark(group.provider) {
            header.append(&mark);
        }
        let heading = label(group.label);
        heading.add_css_class("usage-provider");
        header.append(&heading);
        card.append(&header);
        let source = label(group.source_label);
        source.add_css_class("usage-detail");
        card.append(&source);
        if let Some(aggregate) = group.aggregate {
            append_observation(&card, aggregate, now);
        }
        for account in group.accounts {
            append_observation(&card, account, now);
        }
        root.append(&card);
    }
    Ok(root)
}

/// Read-only, bounded usage viewport for embedding in a host. Providers and
/// hosts retain responsibility for refresh, context lifetime and actions.
pub struct UsagePanel {
    scroll: gtk::ScrolledWindow,
    generation: Rc<Cell<u64>>,
    pending_position: Rc<Cell<Option<f64>>>,
    snapshot: RefCell<Snapshot>,
}

/// The final state-thread result of applying a context-bound read.
pub enum ScopedUpdate {
    Applied,
    Expired,
    ReadFailed(crate::codexbar::SnapshotReadError),
}

impl UsagePanel {
    pub fn new(snapshot: &Snapshot, now: i64) -> Result<Self, SnapshotError> {
        let content = render(snapshot, now)?;
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&content)
            .build();
        Ok(Self {
            scroll,
            generation: Rc::new(Cell::new(0)),
            pending_position: Rc::new(Cell::new(None)),
            snapshot: RefCell::new(snapshot.clone()),
        })
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scroll
    }

    /// Validate and build before replacing the current content. A rejected
    /// snapshot leaves the current view intact. Restore the previous numeric
    /// scroll offset after layout, clamped if the new content is shorter.
    /// This does not promise semantic row anchoring or preserve focus in an
    /// interactive child; the current rows contain no interactive actions.
    pub fn update(&self, snapshot: &Snapshot, now: i64) -> Result<(), SnapshotError> {
        let content = render(snapshot, now)?;
        self.replace(content, snapshot.clone());
        Ok(())
    }

    /// Invalidate the freshness of retained values without starting a read.
    pub fn mark_stale(&self, now: i64) -> Result<(), SnapshotError> {
        let mut stale = self.snapshot.borrow().clone();
        for observation in &mut stale.observations {
            if observation.state == State::Fresh {
                observation.state = State::Stale;
            }
        }
        self.update(&stale, now)
    }

    /// Apply on the GTK/state-owner thread, after any cross-thread queue. An
    /// expired response is ignored. A failed read retains last-known data as
    /// stale and adds a visible failure notice without exposing private output.
    pub fn update_scoped(
        &self,
        reply: msdk::context::Scoped<Result<Snapshot, crate::codexbar::SnapshotReadError>>,
        now: i64,
    ) -> Result<ScopedUpdate, SnapshotError> {
        match reply.into_current() {
            None => Ok(ScopedUpdate::Expired),
            Some(Ok(snapshot)) => {
                self.update(&snapshot, now)?;
                Ok(ScopedUpdate::Applied)
            }
            Some(Err(error)) => {
                let mut stale = self.snapshot.borrow().clone();
                for observation in &mut stale.observations {
                    if observation.state == State::Fresh {
                        observation.state = State::Stale;
                    }
                }
                let content = render(&stale, now)?;
                content.prepend(&label(
                    "Couldn't refresh usage. Showing last-known data where available.",
                ));
                self.replace(content, stale);
                Ok(ScopedUpdate::ReadFailed(error))
            }
        }
    }

    fn replace(&self, content: gtk::Box, snapshot: Snapshot) {
        let adjustment = self.scroll.vadjustment();
        let position = self
            .pending_position
            .get()
            .unwrap_or_else(|| adjustment.value());
        self.pending_position.set(Some(position));
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.scroll.set_child(Some(&content));
        *self.snapshot.borrow_mut() = snapshot;
        let current = self.generation.clone();
        let pending = self.pending_position.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            if current.get() == generation {
                adjustment.set_value(
                    position.min((adjustment.upper() - adjustment.page_size()).max(0.0)),
                );
                pending.set(None);
            }
        });
    }
}
