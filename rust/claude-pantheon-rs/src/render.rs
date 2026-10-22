//! Markdown as widgets: pulldown-cmark drives a small builder that emits Pango-marked paragraphs,
//! headings, bullet and numbered lists, block quotes, tables (a Grid), links (open in the browser),
//! images (inline, click to open) and fenced code in GtkSourceView 5 with language detection, the
//! elementary-matching light/dark style scheme, a copy button and a wrap toggle.

use crate::chat::label;
use gtk::{gdk, gio, glib, pango, prelude::*};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use sourceview5::prelude::*;

/// Builds the widget column for `text` into `into`.
pub fn render_into(text: &str, into: &gtk::Box) {
    let mut b = Builder::new(into);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    for event in Parser::new_ext(text, options) {
        b.event(event);
    }
    b.finish();
}

struct ListState {
    /// None for bullets, Some(next number) for ordered lists.
    number: Option<u64>,
}

struct TableState {
    grid: gtk::Grid,
    aligns: Vec<Alignment>,
    row: i32,
    col: i32,
    header: bool,
}

struct Builder<'a> {
    into: &'a gtk::Box,
    /// Markup of the paragraph being assembled.
    markup: String,
    /// Open inline tags, popped in order.
    inline: Vec<&'static str>,
    lists: Vec<ListState>,
    quote_depth: usize,
    heading: Option<HeadingLevel>,
    code: Option<(String, String)>,
    link: Option<String>,
    table: Option<TableState>,
    in_item: bool,
}

impl<'a> Builder<'a> {
    fn new(into: &'a gtk::Box) -> Self {
        Builder {
            into,
            markup: String::new(),
            inline: Vec::new(),
            lists: Vec::new(),
            quote_depth: 0,
            heading: None,
            code: None,
            link: None,
            table: None,
            in_item: false,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some((_, body)) = &mut self.code {
                    body.push_str(&t);
                } else {
                    self.markup.push_str(&glib::markup_escape_text(&t));
                }
            }
            Event::Code(t) => {
                self.markup.push_str("<tt>");
                self.markup.push_str(&glib::markup_escape_text(&t));
                self.markup.push_str("</tt>");
            }
            Event::SoftBreak => self.markup.push(' '),
            Event::HardBreak => self.markup.push('\n'),
            Event::Rule => {
                self.flush();
                let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
                sep.set_margin_top(4);
                sep.set_margin_bottom(4);
                self.into.append(&sep);
            }
            Event::TaskListMarker(done) => self.markup.push_str(if done { "☑ " } else { "☐ " }),
            Event::Html(t) | Event::InlineHtml(t) => self.markup.push_str(&glib::markup_escape_text(&t)),
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(ListState { number: start });
            }
            Tag::Item => {
                self.flush();
                self.in_item = true;
                let depth = self.lists.len().saturating_sub(1);
                let indent = "    ".repeat(depth);
                match self.lists.last_mut() {
                    Some(ListState { number: Some(n) }) => {
                        self.markup.push_str(&format!("{indent}{n}. "));
                        *n += 1;
                    }
                    _ => self.markup.push_str(&format!("{indent}• ")),
                }
            }
            Tag::Emphasis => self.push_inline("<i>", "</i>"),
            Tag::Strong => self.push_inline("<b>", "</b>"),
            Tag::Strikethrough => self.push_inline("<s>", "</s>"),
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
                self.markup.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(&dest_url)));
                self.inline.push("</a>");
            }
            Tag::Image { dest_url, title, .. } => {
                self.flush();
                self.into.append(&image_widget(&dest_url, &title));
            }
            Tag::Table(aligns) => {
                self.flush();
                let grid = gtk::Grid::builder().column_spacing(14).row_spacing(3).build();
                grid.add_css_class("md-table");
                self.table = Some(TableState { grid, aligns, row: 0, col: 0, header: false });
            }
            Tag::TableHead => {
                if let Some(t) = &mut self.table {
                    t.header = true;
                    t.col = 0;
                }
            }
            Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.col = 0;
                }
            }
            Tag::TableCell => {
                self.markup.clear();
            }
            Tag::FootnoteDefinition(_) | Tag::HtmlBlock | Tag::MetadataBlock(_) | Tag::DefinitionList
            | Tag::DefinitionListTitle | Tag::DefinitionListDefinition | Tag::Superscript | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.in_item {
                    // Keep list items on one paragraph each; the item end flushes.
                } else {
                    self.flush();
                }
            }
            TagEnd::Heading(_) => self.flush(),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some((lang, body)) = self.code.take() {
                    self.into.append(&code_widget(&lang, body.trim_end_matches('\n')));
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.in_item = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                if let Some(close) = self.inline.pop() {
                    self.markup.push_str(close);
                }
                if matches!(tag, TagEnd::Link) {
                    self.link = None;
                }
            }
            TagEnd::TableCell => {
                if let Some(t) = &mut self.table {
                    let cell = gtk::Label::builder()
                        .use_markup(true)
                        .label(&self.markup)
                        .wrap(true)
                        .wrap_mode(pango::WrapMode::WordChar)
                        .selectable(true)
                        .xalign(match t.aligns.get(t.col as usize) {
                            Some(Alignment::Right) => 1.0,
                            Some(Alignment::Center) => 0.5,
                            _ => 0.0,
                        })
                        .build();
                    if t.header {
                        cell.add_css_class("md-th");
                    }
                    t.grid.attach(&cell, t.col, t.row, 1, 1);
                    t.col += 1;
                    self.markup.clear();
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.header = false;
                    t.row += 1;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row += 1;
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    let scroll = gtk::ScrolledWindow::builder()
                        .child(&t.grid)
                        .vscrollbar_policy(gtk::PolicyType::Never)
                        .hscrollbar_policy(gtk::PolicyType::Automatic)
                        .propagate_natural_width(true)
                        .build();
                    scroll.add_css_class("md-table-scroll");
                    self.into.append(&scroll);
                }
            }
            TagEnd::Image => {}
            _ => {}
        }
    }

    fn push_inline(&mut self, open: &'static str, close: &'static str) {
        self.markup.push_str(open);
        self.inline.push(close);
    }

    /// Emits the pending paragraph (or heading) as a label.
    fn flush(&mut self) {
        while let Some(close) = self.inline.pop() {
            self.markup.push_str(close);
        }
        let text = self.markup.trim_end_matches('\n').to_string();
        self.markup.clear();
        if text.trim().is_empty() {
            self.heading = None;
            return;
        }
        let markup = match self.heading.take() {
            Some(HeadingLevel::H1) => format!("<span size=\"x-large\" weight=\"bold\">{text}</span>"),
            Some(HeadingLevel::H2) => format!("<span size=\"large\" weight=\"bold\">{text}</span>"),
            Some(_) => format!("<b>{text}</b>"),
            None => text,
        };
        let l = gtk::Label::builder()
            .use_markup(true)
            .label(&markup)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .xalign(0.0)
            .selectable(true)
            .css_classes(["assistant-text"])
            .build();
        l.connect_activate_link(|_, uri| {
            open_uri(uri);
            glib::Propagation::Stop
        });
        if self.quote_depth > 0 {
            let quote = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            quote.add_css_class("md-quote");
            let bar = gtk::Separator::new(gtk::Orientation::Vertical);
            bar.add_css_class("md-quote-bar");
            quote.append(&bar);
            l.set_hexpand(true);
            quote.append(&l);
            self.into.append(&quote);
        } else {
            self.into.append(&l);
        }
    }

    fn finish(&mut self) {
        self.flush();
        if let Some((lang, body)) = self.code.take() {
            self.into.append(&code_widget(&lang, body.trim_end_matches('\n')));
        }
    }
}

pub fn open_uri(uri: &str) {
    if uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("file://") {
        let launcher = gtk::UriLauncher::new(uri);
        launcher.launch(gtk::Window::NONE, gio::Cancellable::NONE, |_| {});
    }
}

/// A fenced code block: GtkSourceView with the language guessed from the fence (or the content),
/// the Adwaita light/dark scheme following the app, a copy button and a wrap toggle.
pub fn code_widget(lang: &str, code: &str) -> gtk::Widget {
    let buffer = sourceview5::Buffer::new(None);
    let manager = sourceview5::LanguageManager::default();
    let language = if lang.is_empty() {
        manager.guess_language(None::<&str>, None)
    } else {
        manager.language(&normalise_lang(lang)).or_else(|| manager.guess_language(Some(format!("x.{lang}")), None))
    };
    buffer.set_language(language.as_ref());
    buffer.set_highlight_syntax(true);
    buffer.set_text(code);
    apply_scheme(&buffer);
    let view = sourceview5::View::with_buffer(&buffer);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_left_margin(10);
    view.set_right_margin(10);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_show_line_numbers(code.lines().count() > 8);
    view.set_wrap_mode(gtk::WrapMode::None);
    view.add_css_class("code-view");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .build();

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("code-bar");
    let shown_lang = if lang.is_empty() { language.as_ref().map(|l| l.name().to_string()).unwrap_or_default() } else { lang.to_string() };
    let tag = label(&shown_lang, &["caption", "dim-label", "code-lang"]);
    tag.set_hexpand(true);
    bar.append(&tag);
    let wrap = gtk::ToggleButton::builder().icon_name("format-justify-left-symbolic").tooltip_text("Wrap long lines").build();
    wrap.add_css_class("flat");
    let v = view.clone();
    wrap.connect_toggled(move |b| v.set_wrap_mode(if b.is_active() { gtk::WrapMode::WordChar } else { gtk::WrapMode::None }));
    bar.append(&wrap);
    let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy").build();
    copy.add_css_class("flat");
    let text = code.to_string();
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&text);
        b.set_icon_name("emblem-ok-symbolic");
        let b2 = b.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || b2.set_icon_name("edit-copy-symbolic"));
    });
    bar.append(&copy);

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("code-block");
    frame.add_css_class("card");
    frame.append(&bar);
    frame.append(&scroll);
    frame.upcast()
}

fn normalise_lang(lang: &str) -> String {
    match lang.to_lowercase().as_str() {
        "rs" => "rust".into(),
        "py" => "python3".into(),
        "python" => "python3".into(),
        "sh" | "bash" | "shell" | "zsh" => "sh".into(),
        "js" => "js".into(),
        "ts" | "typescript" => "typescript".into(),
        "yml" => "yaml".into(),
        "md" => "markdown".into(),
        "jsonc" => "json".into(),
        "vala" => "vala".into(),
        other => other.to_string(),
    }
}

/// Adwaita or Adwaita-dark, following the app's dark setting (Granite's stylesheet follows the same flag).
pub fn apply_scheme(buffer: &sourceview5::Buffer) {
    #[allow(deprecated)]
    let dark = gtk::Settings::default().map(|s| s.is_gtk_application_prefer_dark_theme()).unwrap_or(false);
    let manager = sourceview5::StyleSchemeManager::default();
    let wanted = if dark { ["Adwaita-dark", "solarized-dark", "classic-dark"] } else { ["Adwaita", "solarized-light", "classic"] };
    for id in wanted {
        if let Some(scheme) = manager.scheme(id) {
            buffer.set_style_scheme(Some(&scheme));
            return;
        }
    }
}

/// An inline image from a URL or path; clicking opens it in the default viewer.
fn image_widget(url: &str, title: &str) -> gtk::Widget {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("md-image");
    let path = url.strip_prefix("file://").unwrap_or(url);
    let picture = if path.starts_with('/') && std::path::Path::new(path).exists() {
        let p = gtk::Picture::for_filename(path);
        p.set_can_shrink(true);
        p.set_content_fit(gtk::ContentFit::ScaleDown);
        p.set_halign(gtk::Align::Start);
        p.set_size_request(-1, 24);
        Some(p)
    } else {
        None
    };
    match picture {
        Some(p) => {
            p.set_tooltip_text(Some(if title.is_empty() { url } else { title }));
            button.set_child(Some(&p));
        }
        None => button.set_label(&format!("Image: {}", if title.is_empty() { url } else { title })),
    }
    let target = url.to_string();
    button.connect_clicked(move |_| {
        let uri = if target.starts_with('/') { format!("file://{target}") } else { target.clone() };
        open_uri(&uri);
    });
    button.upcast()
}

#[allow(dead_code)]
fn _keep(_: &gdk::Texture) {}
