//! The focused window, from the compositor's Gala-compatible D-Bus interface (`org.pantheon.gala` `GetWindows`).

use crate::context::WindowInfo;
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

pub type GalaWindow = (u64, HashMap<String, OwnedValue>);

/// The window that has focus, if exactly one says so. Two focused windows mean the answer cannot be trusted.
pub fn pick_focused(windows: &[GalaWindow]) -> Option<WindowInfo> {
    let mut focused = windows.iter().filter(|(_, properties)| {
        properties
            .get("has-focus")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false)
    });
    let (_, properties) = focused.next()?;
    if focused.next().is_some() {
        return None;
    }
    let text = |key: &str| {
        properties
            .get(key)
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            .unwrap_or_default()
    };
    Some(WindowInfo {
        app_id: text("app-id"),
        title: text("title"),
        pid: 0,
    })
}

pub fn focused_window() -> Result<Option<WindowInfo>, String> {
    zbus::block_on(async {
        let connection = zbus::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_secs(2))
            .build()
            .await
            .map_err(|error| format!("no session bus: {error}"))?;
        let reply = connection
            .call_method(
                Some("org.pantheon.gala"),
                "/org/pantheon/gala/DesktopInterface",
                Some("org.pantheon.gala.DesktopIntegration"),
                "GetWindows",
                &(),
            )
            .await
            .map_err(|error| format!("the compositor did not list windows: {error}"))?;
        let windows: Vec<GalaWindow> = reply
            .body()
            .deserialize()
            .map_err(|error| format!("unexpected window list: {error}"))?;
        Ok(pick_focused(&windows))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn window(app: &str, title: &str, focus: bool) -> GalaWindow {
        let own = |value: Value<'_>| OwnedValue::try_from(value).unwrap();
        (
            1,
            HashMap::from([
                ("app-id".to_string(), own(Value::from(app))),
                ("title".to_string(), own(Value::from(title))),
                ("has-focus".to_string(), own(Value::from(focus))),
            ]),
        )
    }

    #[test]
    fn the_focused_window_is_picked_by_has_focus() {
        let windows = vec![
            window("a", "A", false),
            window("brave-browser", "New Tab - Brave", true),
        ];
        let found = pick_focused(&windows).unwrap();
        assert_eq!(found.app_id, "brave-browser");
        assert_eq!(found.title, "New Tab - Brave");
    }

    #[test]
    fn no_or_ambiguous_focus_gives_nothing() {
        assert!(pick_focused(&[window("a", "A", false)]).is_none());
        assert!(pick_focused(&[window("a", "A", true), window("b", "B", true)]).is_none());
        assert!(pick_focused(&[]).is_none());
    }
}
