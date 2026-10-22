//! Just enough Markdown for a chat: fenced code blocks become monospace boxes, and paragraphs get
//! Pango markup for bold, inline code, headings and bullet lists. Everything else is shown as typed.

use gtk::glib;

#[derive(Debug, PartialEq, Eq)]
pub enum Segment {
    /// Pango markup, ready for `Label::set_markup`.
    Para(String),
    Code {
        lang: String,
        text: String,
    },
}

/// Inline markup for one line: escapes it, then `code`, **bold** and *italic*.
pub fn inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut in_code = false;
    for (i, piece) in line.split('`').enumerate() {
        if i > 0 {
            in_code = !in_code;
        }
        if in_code {
            out.push_str("<tt>");
            out.push_str(&glib::markup_escape_text(piece));
            out.push_str("</tt>");
        } else {
            out.push_str(&emphasis(&glib::markup_escape_text(piece)));
        }
    }
    if in_code {
        out.push_str("</tt>");
    }
    out
}

fn emphasis(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let mut rest = escaped;
    while let Some(start) = rest.find("**") {
        if let Some(len) = rest[start + 2..].find("**") {
            if len > 0 {
                out.push_str(&rest[..start]);
                out.push_str("<b>");
                out.push_str(&rest[start + 2..start + 2 + len]);
                out.push_str("</b>");
                rest = &rest[start + 4 + len..];
                continue;
            }
        }
        out.push_str(&rest[..start + 2]);
        rest = &rest[start + 2..];
    }
    out.push_str(rest);
    out
}

/// Splits assistant text into paragraphs and code blocks.
pub fn render(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut para = String::new();
    let mut code: Option<(String, String)> = None;
    let flush_para = |para: &mut String, segments: &mut Vec<Segment>| {
        let trimmed = para.trim_end_matches('\n');
        if !trimmed.trim().is_empty() {
            segments.push(Segment::Para(trimmed.to_string()));
        }
        para.clear();
    };
    for line in text.lines() {
        if let Some((lang, body)) = &mut code {
            if line.trim_start().starts_with("```") {
                segments.push(Segment::Code { lang: lang.clone(), text: body.trim_end_matches('\n').to_string() });
                code = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            flush_para(&mut para, &mut segments);
            code = Some((rest.trim().to_string(), String::new()));
            continue;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let rendered = if let Some(h) = trimmed.strip_prefix("### ") {
            format!("<b>{}</b>", inline(h))
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            format!("<span size=\"large\" weight=\"bold\">{}</span>", inline(h))
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            format!("<span size=\"x-large\" weight=\"bold\">{}</span>", inline(h))
        } else if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            format!("{}• {}", " ".repeat(indent + 1), inline(item))
        } else if let Some(item) = numbered(trimmed) {
            format!("{}{}", " ".repeat(indent + 1), inline(item))
        } else {
            inline(line)
        };
        para.push_str(&rendered);
        para.push('\n');
    }
    if let Some((lang, body)) = code {
        segments.push(Segment::Code { lang, text: body.trim_end_matches('\n').to_string() });
    }
    flush_para(&mut para, &mut segments);
    segments
}

/// "1. item" keeps its number; returns the whole line so the number survives.
fn numbered(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0 && line[digits..].starts_with(". ")).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_markup() {
        assert_eq!(inline("a **b** `c<d`"), "a <b>b</b> <tt>c&lt;d</tt>");
        assert_eq!(inline("2 ** 3"), "2 ** 3");
    }

    #[test]
    fn code_fences_split() {
        let s = render("hi\n```rust\nfn x() {}\n```\n- one\n- two");
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], Segment::Code { lang: "rust".into(), text: "fn x() {}".into() });
        assert_eq!(s[2], Segment::Para(" • one\n • two".into()));
    }
}
