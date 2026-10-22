//! Visual replies (INTEGRATIONS.md 4f): an agent's HTML page (the `html_render` tool's output, an
//! `htmlRender` reference with an attachment id) shown in a WebKitGTK 6 view that is created only
//! when the owner asks for it and destroyed when the thread is left, so idle RSS does not move
//! while no page is open. The page is sandboxed (WebKit's process sandbox, no local file access),
//! links open in the browser, and a style sheet with the Granite palette is injected so the page
//! follows light and dark.

use crate::chat::label;
use gtk::{gdk, gio, glib, prelude::*};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;

/// What a tool item carries when it is a visual reply.
#[derive(Clone, Debug, PartialEq)]
pub struct HtmlRender {
    pub attachment_id: String,
    pub title: String,
    pub height: f64,
}

/// The `htmlRender` reference of a completed `html_render` tool call, if any. The server keeps the
/// tool's output as a JSON string (`{"htmlRender": {attachmentId, title, height, heights?}}`); the
/// caller parses it and passes the value.
pub fn html_render_of(tool_name: Option<&str>, output: Option<&Value>) -> Option<HtmlRender> {
    let name = tool_name?;
    if !(name == "html_render" || name.ends_with("__html_render") || name.ends_with(".html_render")) {
        return None;
    }
    let output = output?;
    if output.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let r = output.get("htmlRender")?;
    Some(HtmlRender {
        attachment_id: r.get("attachmentId")?.as_str()?.to_string(),
        title: r.get("title").and_then(Value::as_str).unwrap_or("Page").to_string(),
        height: r.get("height").and_then(Value::as_f64).unwrap_or(400.0).clamp(80.0, 2000.0),
    })
}

/// Granite's palette as CSS variables plus sensible defaults for a page that did not style itself.
pub fn theme_css(dark: bool) -> String {
    let (bg, fg, accent, muted) =
        if dark { ("#1e1e1e", "#eeeeee", "#64baff", "#a0a0a0") } else { ("#ffffff", "#333333", "#3689e6", "#666666") };
    format!(
        ":root {{ color-scheme: {scheme}; --bg: {bg}; --fg: {fg}; --accent: {accent}; --muted: {muted}; }}\n\
         body {{ background: var(--bg); color: var(--fg); font-family: Inter, 'Open Sans', sans-serif; margin: 12px; }}\n\
         a {{ color: var(--accent); }}",
        scheme = if dark { "dark" } else { "light" }
    )
}

/// Resolves the page's signed URL for one open; called with the continuation.
pub type Resolve = Rc<dyn Fn(Rc<dyn Fn(Option<String>)>)>;

pub struct PageView {
    pub root: gtk::Box,
    view: RefCell<Option<webkit6::WebView>>,
    fullscreen: RefCell<Option<gtk::Window>>,
}

impl PageView {
    /// A card with the page's title and a button that creates the view; `url` is resolved lazily by
    /// the app (a signed asset URL expires), so the resolver runs on each open.
    pub fn new(render: &HtmlRender, resolve: Resolve) -> Rc<PageView> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("card");
        root.add_css_class("pantheon-surface");
        root.add_css_class("page-card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&gtk::Image::from_icon_name("web-browser-symbolic"));
        let title = label(&render.title, &["title-4"]);
        title.set_hexpand(true);
        head.append(&title);
        let show = gtk::Button::with_label("Show page");
        show.add_css_class("suggested-action");
        head.append(&show);
        let expand = gtk::Button::from_icon_name("view-fullscreen-symbolic");
        expand.update_property(&[gtk::accessible::Property::Label("Open full screen")]);
        expand.set_tooltip_text(Some("Open full screen"));
        expand.set_visible(false);
        head.append(&expand);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.update_property(&[gtk::accessible::Property::Label("Close the page")]);
        close.set_tooltip_text(Some("Close the page (frees its web process)"));
        close.set_visible(false);
        head.append(&close);
        root.append(&head);
        let height = render.height as i32;
        let page = Rc::new(PageView { root, view: RefCell::new(None), fullscreen: RefCell::new(None) });

        let p = page.clone();
        let resolver = resolve.clone();
        let (e, c) = (expand.clone(), close.clone());
        show.connect_clicked(move |b| {
            b.set_sensitive(false);
            let p = p.clone();
            let b = b.clone();
            let (e, c) = (e.clone(), c.clone());
            resolver(Rc::new(move |url| {
                b.set_sensitive(true);
                match url {
                    Some(u) => {
                        p.open(&u, height);
                        b.set_visible(false);
                        e.set_visible(true);
                        c.set_visible(true);
                    }
                    None => b.set_label("Page unavailable"),
                }
            }));
        });
        let p = page.clone();
        let (s, e, c) = (show.clone(), expand.clone(), close.clone());
        close.connect_clicked(move |_| {
            p.close();
            s.set_visible(true);
            s.set_sensitive(true);
            e.set_visible(false);
            c.set_visible(false);
        });
        let p = page.clone();
        let title_text = render.title.clone();
        expand.connect_clicked(move |_| p.open_fullscreen(&title_text));
        page
    }

    fn make_view() -> webkit6::WebView {
        let manager = webkit6::UserContentManager::new();
        #[allow(deprecated)]
        let dark = gtk::Settings::default().map(|s| s.is_gtk_application_prefer_dark_theme()).unwrap_or(false);
        let sheet = webkit6::UserStyleSheet::new(
            &theme_css(dark),
            webkit6::UserContentInjectedFrames::AllFrames,
            webkit6::UserStyleLevel::User,
            &[],
            &[],
        );
        manager.add_style_sheet(&sheet);
        let view = webkit6::WebView::builder().user_content_manager(&manager).build();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_developer_extras(false);
            settings.set_allow_file_access_from_file_urls(false);
            settings.set_allow_universal_access_from_file_urls(false);
            settings.set_enable_media_stream(false);
            settings.set_enable_webgl(false);
        }
        // Links leave the sandbox for the browser; the page itself stays put.
        view.connect_decide_policy(|v, decision, kind| {
            if kind == webkit6::PolicyDecisionType::NavigationAction
                && let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>()
                && let Some(action) = nav.navigation_action().as_mut()
                && action.navigation_type() == webkit6::NavigationType::LinkClicked
                && let Some(req) = action.request()
                && let Some(uri) = req.uri()
            {
                if uri.starts_with("http://") || uri.starts_with("https://") {
                    gtk::UriLauncher::new(&uri).launch(gtk::Window::NONE, gio::Cancellable::NONE, |_| {});
                }
                decision.ignore();
                let _ = v;
                return true;
            }
            false
        });
        view
    }

    pub fn open(&self, url: &str, height: i32) {
        let view = Self::make_view();
        view.set_size_request(-1, height.clamp(80, 2000));
        view.set_vexpand(false);
        view.load_uri(url);
        self.root.append(&view);
        *self.view.borrow_mut() = Some(view);
    }

    pub fn is_open(&self) -> bool {
        self.view.borrow().is_some()
    }

    /// Removes the view and its web process.
    pub fn close(&self) {
        if let Some(w) = self.fullscreen.borrow_mut().take() {
            w.destroy();
        }
        if let Some(v) = self.view.borrow_mut().take() {
            v.try_close();
            self.root.remove(&v);
        }
    }

    fn open_fullscreen(&self, title: &str) {
        let Some(url) = self.view.borrow().as_ref().and_then(|v| v.uri()).map(|u| u.to_string()) else { return };
        let view = Self::make_view();
        view.load_uri(&url);
        let window = gtk::Window::builder().title(title).default_width(1100).default_height(760).child(&view).build();
        let keys = gtk::EventControllerKey::new();
        let w = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                w.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        window.present();
        *self.fullscreen.borrow_mut() = Some(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_html_renders() {
        let out = serde_json::json!({"htmlRender": {"attachmentId": "att-1", "title": "Chart", "height": 420}});
        let r = html_render_of(Some("mcp__t3__html_render"), Some(&out)).unwrap();
        assert_eq!((r.attachment_id.as_str(), r.title.as_str(), r.height), ("att-1", "Chart", 420.0));
        assert!(html_render_of(Some("Bash"), Some(&out)).is_none());
        let err = serde_json::json!({"isError": true, "htmlRender": {"attachmentId": "x", "title": "t", "height": 1}});
        assert!(html_render_of(Some("html_render"), Some(&err)).is_none());
        assert!(theme_css(true).contains("color-scheme: dark"));
    }
}
