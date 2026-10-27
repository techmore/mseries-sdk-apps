//! Run under a private D-Bus session and Xvfb.
use gtk::prelude::*;
use msdk::usage::{
    Coverage, Observation, Quota, QuotaUnit, Scope, Snapshot, Source, State, Window,
};

fn labels(widget: &gtk::Widget, output: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        output.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        labels(&current, output);
        child = current.next_sibling();
    }
}

#[test]
fn usage_rows_preserve_accounts_and_do_not_show_expired_quota_as_live() {
    assert_eq!(std::env::var("MSDK_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
    gtk::init().unwrap();
    let first = Observation {
        scope: Scope {
            provider: "claude".into(),
            source: Source::LocalClient,
            account: Some("a".into()),
        },
        provider_label: "Claude".into(),
        source_label: "This device".into(),
        account_label: Some("Work".into()),
        selected: true,
        state: State::Fresh,
        observed_at: Some(100),
        windows: vec![Window {
            key: "week".into(),
            label: "7 day".into(),
            quota: Some(Quota::new(2500, QuotaUnit::Used, 100, Some(200)).unwrap()),
        }],
        tokens: None,
    };
    let second = Observation {
        scope: Scope {
            account: Some("b".into()),
            ..first.scope.clone()
        },
        account_label: Some("Personal".into()),
        selected: false,
        state: State::Unknown,
        windows: vec![Window {
            key: "week".into(),
            label: "7 day".into(),
            quota: None,
        }],
        ..first.clone()
    };
    let snapshot = Snapshot {
        coverage: Coverage::Partial,
        observations: vec![first, second],
    };
    let root = msdk_gtk::usage::render(&snapshot, 200).unwrap();
    let mut text = Vec::new();
    labels(root.upcast_ref(), &mut text);
    for expected in [
        "Claude",
        "This device",
        "Work · Selected",
        "Personal",
        "7 day: Expired — refresh required",
        "7 day: Unknown",
        "Usage coverage is partial",
    ] {
        assert!(
            text.iter().any(|value| value == expected),
            "missing {expected}: {text:?}"
        );
    }
    assert!(
        !text.iter().any(|value| value.contains("75%")),
        "expired quota cannot appear current"
    );
    let stale = Observation {
        state: State::Stale,
        windows: vec![Window {
            key: "week".into(),
            label: "7 day".into(),
            quota: Some(Quota::new(2500, QuotaUnit::Used, 100, None).unwrap()),
        }],
        ..snapshot.observations[1].clone()
    };
    let stale_view = msdk_gtk::usage::render(
        &Snapshot {
            coverage: Coverage::Complete,
            observations: vec![stale],
        },
        200,
    )
    .unwrap();
    let mut stale_text = Vec::new();
    labels(stale_view.upcast_ref(), &mut stale_text);
    assert_eq!(
        stale_text
            .iter()
            .filter(|value| value.as_str() == "Stale")
            .count(),
        1
    );
    assert!(
        stale_text
            .iter()
            .any(|value| value == "7 day: 75% remaining")
    );
    assert!(stale_text.iter().any(|value| value == "Reset unknown"));

    let mut many = Vec::new();
    for (provider, display, count) in [
        ("codex", "Codex", 2),
        ("claude", "Claude", 2),
        ("grok", "Grok", 1),
        ("opencode", "OpenCode", 1),
    ] {
        for index in 0..count {
            let mut item = snapshot.observations[0].clone();
            item.scope.provider = provider.into();
            item.scope.account = Some(format!("fixture-{index}"));
            item.provider_label = display.into();
            item.account_label = Some(format!("{display} account {index}"));
            item.selected = index == 0;
            many.push(item);
        }
    }
    let mut many_snapshot = Snapshot {
        coverage: Coverage::Partial,
        observations: many,
    };
    let panel = msdk_gtk::usage::UsagePanel::new(&many_snapshot, 150).unwrap();
    let scroll = panel.widget().clone();
    let mut all_text = Vec::new();
    labels(scroll.upcast_ref(), &mut all_text);
    assert_eq!(
        all_text.first().map(String::as_str),
        Some("Providers: Codex · Claude · Grok · OpenCode")
    );
    assert_eq!(
        all_text.get(1).map(String::as_str),
        Some("Usage coverage is partial")
    );
    for expected in [
        "Codex account 1",
        "Claude account 1",
        "Grok account 0",
        "OpenCode account 0",
    ] {
        assert!(
            all_text
                .iter()
                .any(|value| value == expected || value == &format!("{expected} · Selected"))
        );
    }
    let window = gtk::Window::builder()
        .default_width(360)
        .default_height(300)
        .child(&scroll)
        .build();
    window.present();
    let context = gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while scroll.vadjustment().upper() <= scroll.vadjustment().page_size()
        && std::time::Instant::now() < deadline
    {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let adjustment = scroll.vadjustment();
    assert!(
        adjustment.upper() > adjustment.page_size(),
        "long content must scroll"
    );
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    assert!(
        adjustment.value() > 0.0,
        "the last provider must be reachable by scrolling"
    );
    let position = adjustment.value();
    let invalid = Snapshot {
        coverage: Coverage::Partial,
        observations: vec![
            many_snapshot.observations[0].clone(),
            many_snapshot.observations[0].clone(),
        ],
    };
    assert!(panel.update(&invalid, 150).is_err());
    let mut before = Vec::new();
    labels(scroll.upcast_ref(), &mut before);
    assert!(
        before
            .iter()
            .any(|value| value == "OpenCode account 0 · Selected")
    );
    many_snapshot.observations[0].account_label = Some("Updated account".into());
    panel.update(&many_snapshot, 150).unwrap();
    panel.update(&many_snapshot, 150).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        (adjustment.value() - position).abs() < 1.0,
        "refresh preserves the numeric scroll offset"
    );
    let mut updated = Vec::new();
    labels(scroll.upcast_ref(), &mut updated);
    assert!(
        updated
            .iter()
            .any(|value| value == "Updated account · Selected")
    );
    window.close();
    let source = br#"{"schemaVersion":1,"entries":[{"provider":"codex","updatedAt":"2026-01-01T00:00:00Z","windows":[{"key":"week","label":"7 day","remaining":75,"resetsAt":"2026-01-02T00:00:00Z"}]}]}"#;
    let parsed = msdk_gtk::codexbar::decode_snapshot(source, 1_767_225_600, 300).unwrap();
    let source_view = msdk_gtk::usage::render(&parsed, 1_767_225_600).unwrap();
    let mut parsed_text = Vec::new();
    labels(source_view.upcast_ref(), &mut parsed_text);
    for expected in [
        "Codex",
        "Provider total",
        "7 day: 75% remaining",
        "Usage coverage is partial",
    ] {
        assert!(
            parsed_text.iter().any(|value| value == expected),
            "missing {expected}: {parsed_text:?}"
        );
    }
    #[cfg(unix)]
    {
        use msdk::context::RequestContext;
        use msdk_gtk::usage::ScopedUpdate;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("msdk-usage-feed-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let executable = dir.join("synthetic-snapshot");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\n",
                std::str::from_utf8(source).unwrap()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = RequestContext::new();
        let panel = msdk_gtk::usage::UsagePanel::new(&snapshot, 1_767_225_600).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _task = msdk_gtk::codexbar::read_snapshot_scoped(
            context.token().unwrap(),
            executable.clone(),
            u64::MAX,
            90,
            move |reply| tx.send(reply).unwrap(),
        )
        .unwrap();
        let reply = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(
            panel.update_scoped(reply, 1_767_225_600).unwrap(),
            ScopedUpdate::Applied
        ));
        let mut current = Vec::new();
        labels(panel.widget().upcast_ref(), &mut current);
        assert!(current.iter().any(|value| value == "7 day: 75% remaining"));

        let (tx, rx) = std::sync::mpsc::channel();
        let _task = msdk_gtk::codexbar::read_snapshot_scoped(
            context.token().unwrap(),
            executable.clone(),
            u64::MAX,
            91,
            move |reply| tx.send(reply).unwrap(),
        )
        .unwrap();
        let queued = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        context.advance().unwrap();
        assert!(matches!(
            panel.update_scoped(queued, 1_767_225_600).unwrap(),
            ScopedUpdate::Expired
        ));
        let mut after_old = Vec::new();
        labels(panel.widget().upcast_ref(), &mut after_old);
        assert_eq!(
            after_old, current,
            "expired queued replies must not mutate the view"
        );

        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf 'synthetic-private-error' >&2\nprintf 'not-json'\n",
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _task = msdk_gtk::codexbar::read_snapshot_scoped(
            context.token().unwrap(),
            executable.clone(),
            u64::MAX,
            92,
            move |reply| tx.send(reply).unwrap(),
        )
        .unwrap();
        let failed = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(
            panel.update_scoped(failed, 1_767_225_600).unwrap(),
            ScopedUpdate::ReadFailed(_)
        ));
        let mut after_failure = Vec::new();
        labels(panel.widget().upcast_ref(), &mut after_failure);
        assert!(
            after_failure
                .iter()
                .any(|value| value.contains("Couldn't refresh usage"))
        );
        assert!(after_failure.iter().any(|value| value == "Stale"));
        assert!(
            after_failure
                .iter()
                .any(|value| value == "7 day: 75% remaining")
        );
        assert!(
            !after_failure
                .iter()
                .any(|value| value.contains("synthetic-private-error"))
        );

        use msdk_gtk::codexbar_monitor::{CodexBarMonitor, RefreshStart, RefreshState};
        let marker = dir.join("reads");
        let safe_marker = marker.to_str().unwrap().replace('\'', "'\\''");
        let script = format!(
            "#!/bin/sh\nprintf x >> '{safe_marker}'\nsleep 0.2\nprintf '%s' '{}'\n",
            std::str::from_utf8(source).unwrap()
        );
        std::fs::write(&executable, &script).unwrap();
        let panel =
            std::rc::Rc::new(msdk_gtk::usage::UsagePanel::new(&snapshot, 1_767_225_600).unwrap());
        let states = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = states.clone();
        let monitor = CodexBarMonitor::new(panel.clone(), executable.clone(), 0, move |state| {
            received.borrow_mut().push(state)
        });
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Closed);
        assert!(!marker.exists(), "closed monitor must not start reads");
        monitor.open();
        assert!(!marker.exists(), "opening alone does not poll");
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Started);
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Coalesced);
        let main = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while !states.borrow().contains(&RefreshState::Updated)
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(states.borrow().contains(&RefreshState::Updated));
        assert!(!monitor.is_busy());
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
        let mut aged = Vec::new();
        labels(panel.widget().upcast_ref(), &mut aged);
        assert!(
            aged.iter().any(|value| value == "Stale"),
            "source age is rechecked on final delivery"
        );

        std::fs::write(
            &executable,
            format!("#!/bin/sh\nprintf x >> '{safe_marker}'\nexec sleep 20\n"),
        )
        .unwrap();
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Started);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::fs::read_to_string(&marker).unwrap().len() < 2
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xx");
        monitor.close();
        assert!(!monitor.is_busy());
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Closed);
        std::fs::write(&executable, &script).unwrap();
        monitor.open();
        assert_eq!(monitor.refresh().unwrap(), RefreshStart::Started);
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while (monitor.is_busy() || std::fs::read_to_string(&marker).unwrap().len() < 3)
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !monitor.is_busy(),
            "old completion must not clear or block the new read"
        );
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xxx");
        assert_eq!(
            states
                .borrow()
                .iter()
                .filter(|&&state| state == RefreshState::Updated)
                .count(),
            2
        );
        monitor.close();

        let control = msdk_gtk::usage_control::UsageControl::new(
            &snapshot,
            1_767_225_600,
            executable.clone(),
            0,
        )
        .unwrap();
        assert!(!control.refresh_button().is_sensitive());
        control.open().unwrap();
        control.open().unwrap();
        assert!(!control.refresh_button().is_sensitive());
        assert_eq!(control.status_label().text(), "Refreshing usage…");
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while control.status_label().text() != "Refresh complete"
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(control.status_label().text(), "Refresh complete");
        assert!(control.refresh_button().is_sensitive());
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "xxxx",
            "repeated open must not duplicate reads"
        );
        std::fs::write(&executable, format!("#!/bin/sh\nprintf x >> '{safe_marker}'\nprintf 'synthetic-private-error' >&2\nprintf 'not-json'\n")).unwrap();
        control.refresh_button().emit_clicked();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while control.status_label().text() != "Retry when ready"
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(control.status_label().text(), "Retry when ready");
        assert!(control.refresh_button().is_sensitive());
        let mut failure_text = Vec::new();
        labels(control.widget().upcast_ref(), &mut failure_text);
        assert!(failure_text.iter().any(|value| value == "Stale"));
        assert!(
            !failure_text
                .iter()
                .any(|value| value.contains("synthetic-private-error"))
        );
        std::fs::write(&executable, &script).unwrap();
        control.refresh_button().emit_clicked();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while control.status_label().text() != "Refresh complete"
            && std::time::Instant::now() < deadline
        {
            main.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(control.status_label().text(), "Refresh complete");
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xxxxxx");
        control.close();
        assert!(!control.refresh_button().is_sensitive());
        assert_eq!(control.status_label().text(), "Usage not open");
    }
}
