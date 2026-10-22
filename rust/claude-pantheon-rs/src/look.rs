//! Small text helpers shared by the sidebar and the chat.

/// "just now", "5 min ago", "Yesterday", "Oct 3": what a sidebar row shows next to the title.
pub fn relative_time(unix: i64, now: i64) -> String {
    let delta = now - unix;
    if delta < 60 {
        return "just now".into();
    }
    if delta < 3600 {
        return format!("{} min ago", delta / 60);
    }
    if delta < 86_400 {
        let h = delta / 3600;
        return if h == 1 { "1 hour ago".into() } else { format!("{h} hours ago") };
    }
    let then = gtk::glib::DateTime::from_unix_local(unix).ok();
    let now_dt = gtk::glib::DateTime::from_unix_local(now).ok();
    if let (Some(a), Some(b)) = (&then, &now_dt) {
        let days = b.day_of_year() - a.day_of_year() + 365 * (b.year() - a.year());
        if days == 1 {
            return "Yesterday".into();
        }
        if days < 7 {
            return format!("{days} days ago");
        }
        let fmt = if a.year() == b.year() { "%b %-d" } else { "%b %-d, %Y" };
        if let Ok(s) = a.format(fmt) {
            return s.to_string();
        }
    }
    format!("{} days ago", delta / 86_400)
}

/// Unix seconds of an ISO-8601 timestamp (`2026-10-21T12:25:26.634Z`), 0 when unparsable.
pub fn iso_to_unix(iso: &str) -> i64 {
    gtk::glib::DateTime::from_iso8601(iso, None).map(|d| d.to_unix()).unwrap_or(0)
}

/// "21:40" (today) or "Mon 08:00" for an ISO timestamp, in local time.
pub fn clock(iso: &str) -> String {
    let Ok(d) = gtk::glib::DateTime::from_iso8601(iso, None) else { return "?".into() };
    let Ok(local) = d.to_local() else { return "?".into() };
    let today = gtk::glib::DateTime::now_local()
        .map(|n| n.day_of_year() == local.day_of_year() && n.year() == local.year())
        .unwrap_or(false);
    local.format(if today { "%H:%M" } else { "%a %H:%M" }).map(|s| s.to_string()).unwrap_or_else(|_| "?".into())
}

/// "in 2 h 14 min", "in 3 d 5 h", or "now" when the moment has passed.
pub fn countdown(iso: &str, now: i64) -> String {
    let at = iso_to_unix(iso);
    let left = at - now;
    if at == 0 {
        return String::new();
    }
    if left <= 0 {
        return "now".into();
    }
    let (d, h, m) = (left / 86_400, (left % 86_400) / 3600, (left % 3600) / 60);
    if d > 0 {
        format!("in {d} d {h} h")
    } else if h > 0 {
        format!("in {h} h {m} min")
    } else {
        format!("in {m} min")
    }
}

/// "3.2 MB" with the same SI units Files shows.
pub fn size_text(bytes: u64) -> String {
    gtk::glib::format_size(bytes).to_string()
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "~/Projects/foo" for a path under the home folder.
pub fn tilde(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) { format!("~{}", &path[home.len()..]) } else { path.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(1000, 1030), "just now");
        assert_eq!(relative_time(1000, 1000 + 5 * 60), "5 min ago");
        assert_eq!(relative_time(1000, 1000 + 3 * 3600), "3 hours ago");
        assert_eq!(iso_to_unix("1970-01-01T00:01:00Z"), 60);
        assert_eq!(countdown("1970-01-01T03:00:00Z", 60 * 46), "in 2 h 14 min");
        assert_eq!(countdown("1970-01-01T00:00:01Z", 10), "now");
    }
}
