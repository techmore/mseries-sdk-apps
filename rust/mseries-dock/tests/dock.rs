use msdk::logic::{Effect, Logic, Store};
use mseries_dock::apps::{AppEntry, Resolver, parse_desktop};
use mseries_dock::logic::DockLogic;
use serde_json::json;

fn app(id: &str, name: &str, wm: Option<&str>, exec: &str) -> AppEntry {
    AppEntry {
        id: format!("{id}.desktop"),
        name: name.into(),
        icon: format!("{id}-icon"),
        wm_class: wm.map(String::from),
        exec_base: Some(exec.into()),
    }
}

fn catalogue() -> Vec<AppEntry> {
    vec![
        app("io.elementary.files", "Files", None, "io.elementary.files"),
        app(
            "brave-browser",
            "Brave",
            Some("Brave-browser"),
            "brave-browser",
        ),
        app(
            "outlook-pwa",
            "Outlook",
            Some("brave-outlook"),
            "brave-browser",
        ),
        app("teams-pwa", "Teams", Some("brave-teams"), "brave-browser"),
    ]
}

fn win(id: &str, app_id: &str, focused: bool) -> serde_json::Value {
    json!({ "id": id, "app_id": app_id, "title": "t", "focused": focused, "attention": false })
}

#[test]
fn parses_desktop_files() {
    let e = parse_desktop("a.desktop", "[Desktop Entry]\nType=Application\nName=A\nName[de]=Ä\nIcon=a\nExec=/usr/bin/a --x %U\nStartupWMClass=AA\n[Desktop Action x]\nName=No\n").unwrap();
    assert_eq!(
        (
            e.name.as_str(),
            e.icon.as_str(),
            e.exec_base.as_deref(),
            e.wm_class.as_deref()
        ),
        ("A", "a", Some("a"), Some("AA"))
    );
    assert!(
        parse_desktop(
            "h.desktop",
            "[Desktop Entry]\nType=Application\nName=H\nNoDisplay=true\n"
        )
        .is_none()
    );
    assert!(parse_desktop("l.desktop", "[Desktop Entry]\nType=Link\nName=L\n").is_none());
}

#[test]
fn resolver_follows_the_tracking_rules() {
    let r = Resolver::new(catalogue());
    assert_eq!(r.resolve("io.elementary.files").unwrap().name, "Files");
    assert_eq!(
        r.resolve("io.elementary.files.desktop").unwrap().name,
        "Files"
    );
    assert_eq!(r.resolve("IO.ELEMENTARY.FILES").unwrap().name, "Files"); // unique case-folded id
    assert_eq!(r.resolve("brave-outlook").unwrap().name, "Outlook"); // StartupWMClass
    assert_eq!(r.resolve("Brave-browser").unwrap().name, "Brave");
    assert!(r.resolve("").is_none() && r.resolve("something-else").is_none());
}

#[test]
fn ambiguous_matches_resolve_to_nothing() {
    let mut c = catalogue();
    c.push(app("tool-a", "Tool A", Some("shared-class"), "tool-a"));
    c.push(app("tool-b", "Tool B", Some("shared-class"), "tool-b"));
    let r = Resolver::new(c);
    assert!(
        r.resolve("shared-class").is_none(),
        "two entries share this StartupWMClass"
    );
    let mut c = catalogue();
    c.retain(|e| e.id != "brave-browser.desktop");
    // outlook and teams share the executable brave-browser, so the executable alone must not pick one.
    assert!(Resolver::new(c).resolve("brave-browser").is_none());
}

#[test]
fn items_are_pins_then_unpinned_running_apps() {
    let d = DockLogic::new(
        catalogue(),
        vec!["io.elementary.files".into(), "brave-browser".into()],
    );
    let items = d.build(&[
        win("7", "brave-outlook", true),
        win("8", "io.elementary.files", false),
        win("9", "mystery", false),
    ]);
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "io.elementary.files.desktop",
            "brave-browser.desktop",
            "outlook-pwa.desktop",
            "tmp:mystery"
        ]
    );
    assert_eq!(items[0].indicator, "open");
    assert_eq!(items[1].indicator, "none"); // pinned, closed
    assert_eq!(items[2].indicator, "focused");
    assert!(!items[3].pinned && items[3].name == "mystery");
}

fn run(
    d: &mut DockLogic,
    store: &mut Store,
    msg: &str,
    payload: Option<serde_json::Value>,
) -> Vec<Effect> {
    d.update(store, msg, payload)
}

#[test]
fn clicking_launches_focuses_and_cycles() {
    let mut d = DockLogic::new(
        catalogue(),
        vec!["io.elementary.files".into(), "brave-browser".into()],
    );
    let mut st = Store::new();
    st.insert(
        "windows".into(),
        json!([
            win("1", "brave-browser", true),
            win("2", "brave-browser", false)
        ]),
    );
    run(&mut d, &mut st, "_feed", Some(json!("windows")));
    assert_eq!(st["items"].as_array().unwrap().len(), 2);
    // Closed pinned app launches.
    let e = run(
        &mut d,
        &mut st,
        "Activate",
        Some(json!("io.elementary.files.desktop")),
    );
    assert!(matches!(&e[0], Effect::Call { service, .. } if service == "windows.launch"));
    // Running and focused with two windows: the next click focuses the other window.
    let e = run(
        &mut d,
        &mut st,
        "Activate",
        Some(json!("brave-browser.desktop")),
    );
    assert!(
        matches!(&e[0], Effect::Call { service, args, .. } if service == "windows.activate" && args["id"] == json!("2"))
    );
    // Unknown id does nothing.
    assert!(run(&mut d, &mut st, "Activate", Some(json!("nope"))).is_empty());
}

#[test]
fn an_unpinnable_running_app_cannot_be_launched_from_the_dock() {
    let mut d = DockLogic::new(catalogue(), vec![]);
    let mut st = Store::new();
    st.insert("windows".into(), json!([win("1", "mystery", false)]));
    run(&mut d, &mut st, "_start", None);
    let e = run(&mut d, &mut st, "Activate", Some(json!("tmp:mystery")));
    assert!(matches!(&e[0], Effect::Call { service, .. } if service == "windows.activate"));
    st.insert("windows".into(), json!([]));
    run(&mut d, &mut st, "_feed", Some(json!("windows")));
    assert!(st["items"].as_array().unwrap().is_empty());
}

#[test]
fn the_screen_description_is_valid() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../msdk/examples/dock.ui.json")).unwrap();
    assert!(
        msdk::check(&doc)
            .iter()
            .all(|d| d.severity != msdk::Severity::Error)
    );
}
