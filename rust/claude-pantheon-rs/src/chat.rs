//! Chat widgets shared by the offline transcript view and the live T3 thread view: user bubbles,
//! assistant Markdown, collapsed tool rows, diffs and permission prompts.

use crate::markdown::{self, Segment};
use crate::t3::model::TurnItem;
use crate::transcript::{self, Block, Role, ToolResult, Turn};
use gtk::{pango, prelude::*};
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn mono(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn user_bubble(text: &str) -> gtk::Box {
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bubble.add_css_class("user-bubble");
    bubble.set_halign(gtk::Align::End);
    bubble.set_margin_start(96);
    let l = gtk::Label::builder()
        .label(transcript::clip(text, 20_000))
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .build();
    bubble.append(&l);
    bubble
}

pub fn segment_widget(seg: &Segment) -> gtk::Widget {
    match seg {
        Segment::Para(markup) => gtk::Label::builder()
            .use_markup(true)
            .label(markup)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .xalign(0.0)
            .selectable(true)
            .css_classes(["assistant-text"])
            .build()
            .upcast(),
        Segment::Code { lang, text } => {
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.add_css_class("code-block");
            frame.add_css_class("card");
            if !lang.is_empty() {
                frame.append(&label(lang, &["caption", "dim-label", "code-lang"]));
            }
            frame.append(&mono(text, &["code-text"]));
            frame.upcast()
        }
    }
}

/// Assistant prose as a column of paragraphs and code blocks.
pub fn assistant_text(text: &str, into: &gtk::Box) {
    for seg in markdown::render(text) {
        into.append(&segment_widget(&seg));
    }
}

fn tool_icon(name: &str) -> &'static str {
    match name {
        "Bash" | "command" => "utilities-terminal-symbolic",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "file_change" => "document-edit-symbolic",
        "Read" | "Glob" | "file_search" => "document-open-symbolic",
        "Grep" | "WebSearch" | "web_search" => "edit-find-symbolic",
        "WebFetch" => "web-browser-symbolic",
        "Agent" | "Task" | "subagent" => "system-run-symbolic",
        "reasoning" => "dialog-information-symbolic",
        _ => "application-x-addon-symbolic",
    }
}

/// A collapsed one-line row whose details are built on first expand (keeps big transcripts cheap).
pub fn tool_row(icon_key: &str, summary: &str, input: String, result: Option<ToolResult>) -> gtk::Widget {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&gtk::Image::from_icon_name(tool_icon(icon_key)));
    let text = label(summary, &["tool-summary"]);
    text.set_hexpand(true);
    head.append(&text);
    if result.as_ref().is_some_and(|r| r.is_error) {
        let err = gtk::Image::from_icon_name("dialog-warning-symbolic");
        err.set_tooltip_text(Some("The tool reported an error"));
        head.append(&err);
    }
    let expander = gtk::Expander::builder().label_widget(&head).build();
    expander.add_css_class("tool-row");
    let built = Cell::new(false);
    expander.connect_expanded_notify(move |e| {
        if built.replace(true) {
            return;
        }
        let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
        details.add_css_class("tool-details");
        if !input.trim().is_empty() {
            details.append(&mono(&transcript::clip(&input, 6000), &["tool-input"]));
        }
        match &result {
            Some(r) => {
                let classes: &[&str] = if r.is_error { &["tool-result", "tool-error"] } else { &["tool-result"] };
                let shown =
                    if r.text.trim().is_empty() { "(no output)".to_string() } else { transcript::clip(&r.text, 6000) };
                details.append(&mono(&shown, classes));
            }
            None => details.append(&label("No result recorded", &["caption", "dim-label"])),
        }
        e.set_child(Some(&details));
    });
    expander.upcast()
}

/// One JSONL turn as a widget.
pub fn turn_widget(turn: &Turn) -> gtk::Widget {
    match turn.role {
        Role::User => {
            let text = turn
                .blocks
                .iter()
                .filter_map(|b| if let Block::Text(s) = b { Some(s.as_str()) } else { None })
                .collect::<Vec<_>>()
                .join("\n");
            user_bubble(&text).upcast()
        }
        Role::Assistant => {
            let column = assistant_column();
            for block in &turn.blocks {
                match block {
                    Block::Text(text) => assistant_text(text, &column),
                    Block::ToolUse { summary, input, result, name, .. } => {
                        column.append(&tool_row(name, summary, input.clone(), result.clone()));
                    }
                }
            }
            column.upcast()
        }
    }
}

pub fn assistant_column() -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.add_css_class("assistant");
    column.set_margin_end(48);
    column
}

// ---- T3 turn items ------------------------------------------------------------------------------

/// Imported sessions flatten tool calls into the assistant text as
/// `[external_agent_tool_call: Name]\ninput: {…}\n[/external_agent_tool_call]` blocks followed by
/// `[external_agent_tool_result…]` blocks. Split them back into prose and tool rows.
pub enum Imported {
    Text(String),
    Tool { name: String, input: String },
    Result { text: String, is_error: bool },
}

pub fn split_imported(text: &str) -> Vec<Imported> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[external_agent_tool_") {
        let before = &rest[..start];
        if !before.trim().is_empty() {
            out.push(Imported::Text(before.trim().to_string()));
        }
        let tail = &rest[start..];
        if let Some(head) = tail.strip_prefix("[external_agent_tool_call: ") {
            let name_end = head.find(']').unwrap_or(0);
            let name = head[..name_end].to_string();
            let body_start = name_end + 1;
            let body_end = head.find("[/external_agent_tool_call]").unwrap_or(head.len());
            let body = head[body_start..body_end]
                .trim()
                .strip_prefix("input: ")
                .unwrap_or(&head[body_start..body_end])
                .trim()
                .to_string();
            out.push(Imported::Tool { name, input: body });
            rest = &head[(body_end + "[/external_agent_tool_call]".len()).min(head.len())..];
        } else if let Some(head) = tail.strip_prefix("[external_agent_tool_result") {
            let tag_end = head.find(']').unwrap_or(0);
            let is_error = head[..tag_end].contains("error");
            let body_end = head.find("[/external_agent_tool_result]").unwrap_or(head.len());
            let body = head[tag_end + 1..body_end].trim().to_string();
            out.push(Imported::Result { text: body, is_error });
            rest = &head[(body_end + "[/external_agent_tool_result]".len()).min(head.len())..];
        } else {
            out.push(Imported::Text(tail.to_string()));
            rest = "";
        }
    }
    if !rest.trim().is_empty() {
        out.push(Imported::Text(rest.trim().to_string()));
    }
    out
}

fn tool_summary_from_name(name: &str, input: &str) -> String {
    let parsed: Value = serde_json::from_str(input).unwrap_or(Value::Null);
    transcript::tool_summary(name, &parsed)
}

/// Callback for a permission prompt: (request id, decision).
pub type Decide = Rc<dyn Fn(&str, &str)>;

/// One T3 turn item as a widget; `pending_result` lets an imported tool call pick up the result that
/// follows it in the same text.
pub fn item_widget(item: &TurnItem, decide: Option<Decide>) -> Option<gtk::Widget> {
    match item.kind.as_str() {
        "user_message" => Some(user_bubble(&item.text).upcast()),
        "assistant_message" => {
            let column = assistant_column();
            let parts = split_imported(&item.text);
            let mut i = 0;
            while i < parts.len() {
                match &parts[i] {
                    Imported::Text(t) => assistant_text(t, &column),
                    Imported::Tool { name, input } => {
                        let result = match parts.get(i + 1) {
                            Some(Imported::Result { text, is_error }) => {
                                i += 1;
                                Some(ToolResult { text: text.clone(), is_error: *is_error })
                            }
                            _ => None,
                        };
                        column.append(&tool_row(name, &tool_summary_from_name(name, input), input.clone(), result));
                    }
                    Imported::Result { text, is_error } => {
                        column.append(&tool_row(
                            "",
                            "Tool result",
                            String::new(),
                            Some(ToolResult { text: text.clone(), is_error: *is_error }),
                        ));
                    }
                }
                i += 1;
            }
            if item.streaming {
                column.append(&label("▍", &["live-cursor"]));
            }
            Some(column.upcast())
        }
        "reasoning" => {
            let text = if item.streaming { format!("{}▍", item.text) } else { item.text.clone() };
            if text.trim().is_empty() {
                return None;
            }
            Some(tool_row("reasoning", "Thinking", text, None))
        }
        "command_execution" => {
            let input = item.input.clone().unwrap_or_default();
            let summary = format!("Ran command: {}", transcript::first_line(&input, 100));
            let failed = item.exit_code.is_some_and(|c| c != 0)
                || item.rest.get("outputIndicatesFailure").and_then(Value::as_bool).unwrap_or(false);
            let result = item.output.as_ref().map(|o| ToolResult { text: o.clone(), is_error: failed });
            Some(tool_row("command", &summary, input, result))
        }
        "file_change" => {
            let name = item.file_name.clone().unwrap_or_default();
            let mut summary = format!("Edited file {}", crate::look::tilde(&name));
            if item.additions.is_some() || item.deletions.is_some() {
                summary.push_str(&format!("  +{} −{}", item.additions.unwrap_or(0), item.deletions.unwrap_or(0)));
            }
            let diff = item.diff_str.clone().unwrap_or_default();
            Some(diff_row(&summary, diff))
        }
        "file_search" => Some(tool_row(
            "file_search",
            &format!("Searched files for {}", item.pattern.clone().unwrap_or_default()),
            results_text(item),
            None,
        )),
        "web_search" => Some(tool_row(
            "web_search",
            &format!("Searched the web: {}", item.patterns.clone().unwrap_or_default().join(", ")),
            results_text(item),
            None,
        )),
        "tool_call" => {
            let name = item.name.clone().or(item.title.clone()).unwrap_or_else(|| "tool".into());
            let input = item
                .rest
                .get("input")
                .or_else(|| item.rest.get("arguments"))
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .unwrap_or_default();
            let output = item.rest.get("output").or_else(|| item.rest.get("result")).map(|v| match v {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            });
            let summary = transcript::tool_summary(&name, &item.rest.get("input").cloned().unwrap_or(Value::Null));
            Some(tool_row(
                &name,
                &summary,
                input,
                output.map(|o| ToolResult { text: o, is_error: item.status == "failed" }),
            ))
        }
        "approval_request" => Some(approval_card(item, decide)),
        "user_input_request" => {
            let card = card("Claude has a question");
            if let Some(qs) = item.rest.get("questions").and_then(Value::as_array) {
                for q in qs {
                    let text = q
                        .get("prompt")
                        .or_else(|| q.get("question"))
                        .or_else(|| q.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let l = label(text, &[]);
                    l.set_wrap(true);
                    l.set_ellipsize(pango::EllipsizeMode::None);
                    card.append(&l);
                }
            }
            card.append(&label("Answer in the message box below.", &["caption", "dim-label"]));
            Some(card.upcast())
        }
        "proposed_plan" => {
            let column = assistant_column();
            column.append(&label("Proposed plan", &["title-4"]));
            assistant_text(item.markdown.as_deref().unwrap_or(""), &column);
            Some(column.upcast())
        }
        "todo_list" => {
            let column = assistant_column();
            if let Some(steps) = &item.steps {
                for s in steps {
                    let status = s.get("status").and_then(Value::as_str).unwrap_or("");
                    let mark = match status {
                        "completed" => "☑",
                        "running" => "▶",
                        _ => "☐",
                    };
                    let l = label(&format!("{mark} {}", s.get("text").and_then(Value::as_str).unwrap_or("")), &[]);
                    l.set_wrap(true);
                    column.append(&l);
                }
            }
            Some(column.upcast())
        }
        "notification" | "system" | "hook" | "run_interrupt_request" | "run_interrupt_result" | "checkpoint" => {
            let text = if !item.text.is_empty() {
                item.text.clone()
            } else {
                item.rest
                    .get("message")
                    .or_else(|| item.rest.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            if text.trim().is_empty() {
                return None;
            }
            let l = label(&text, &["caption", "dim-label"]);
            l.set_wrap(true);
            l.set_ellipsize(pango::EllipsizeMode::None);
            l.set_halign(gtk::Align::Center);
            Some(l.upcast())
        }
        "subagent" => Some(tool_row(
            "subagent",
            &format!("Ran agent: {}", item.title.clone().or(item.name.clone()).unwrap_or_default()),
            item.text.clone(),
            None,
        )),
        _ => {
            if item.text.trim().is_empty() {
                None
            } else {
                let column = assistant_column();
                assistant_text(&item.text, &column);
                Some(column.upcast())
            }
        }
    }
}

fn results_text(item: &TurnItem) -> String {
    item.rest.get("results").map(|r| serde_json::to_string_pretty(r).unwrap_or_default()).unwrap_or_default()
}

fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.add_css_class("prompt-card");
    card.set_margin_end(48);
    card.append(&label(title, &["title-4"]));
    card
}

/// A diff shown line by line with added and removed lines coloured.
pub fn diff_row(summary: &str, diff: String) -> gtk::Widget {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
    let text = label(summary, &["tool-summary"]);
    text.set_hexpand(true);
    head.append(&text);
    let expander = gtk::Expander::builder().label_widget(&head).build();
    expander.add_css_class("tool-row");
    let built = Cell::new(false);
    expander.connect_expanded_notify(move |e| {
        if built.replace(true) {
            return;
        }
        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.add_css_class("tool-details");
        details.add_css_class("diff");
        if diff.trim().is_empty() {
            details.append(&label("No diff recorded", &["caption", "dim-label"]));
        }
        for line in diff.lines().take(400) {
            let l = mono(line, &[]);
            l.set_ellipsize(pango::EllipsizeMode::None);
            if line.starts_with('+') && !line.starts_with("+++") {
                l.add_css_class("diff-add");
            } else if line.starts_with('-') && !line.starts_with("---") {
                l.add_css_class("diff-del");
            } else if line.starts_with("@@") {
                l.add_css_class("dim-label");
            }
            details.append(&l);
        }
        e.set_child(Some(&details));
    });
    expander.upcast()
}

fn approval_card(item: &TurnItem, decide: Option<Decide>) -> gtk::Widget {
    let kind = item.request_kind.clone().unwrap_or_else(|| "permission".into());
    let title = match kind.as_str() {
        "command" => "Permission to run a command",
        "file-change" => "Permission to change a file",
        "file-read" => "Permission to read a file",
        _ => "Permission requested",
    };
    let card = card(title);
    let prompt = item.prompt.clone().or(item.reason.clone()).unwrap_or_default();
    if !prompt.trim().is_empty() {
        card.append(&mono(&transcript::clip(&prompt, 4000), &["tool-input"]));
    }
    let pending = item.is_pending_request();
    let request_id = item.request_id.clone().unwrap_or_default();
    if pending && !request_id.is_empty() {
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_halign(gtk::Align::End);
        let options: Vec<(String, String)> = match &item.options {
            Some(opts) if !opts.is_empty() => opts
                .iter()
                .filter_map(|o| {
                    Some((
                        o.get("decision")?.as_str()?.to_string(),
                        o.get("label").and_then(Value::as_str).unwrap_or("OK").to_string(),
                    ))
                })
                .collect(),
            _ => vec![("decline".into(), "Deny".into()), ("accept".into(), "Allow".into())],
        };
        for (decision, text) in options {
            let b = gtk::Button::with_label(&text);
            if decision.starts_with("accept") {
                b.add_css_class("suggested-action");
            } else if decision == "decline" || decision == "cancel" {
                b.add_css_class("destructive-action");
            }
            let decide = decide.clone();
            let rid = request_id.clone();
            b.connect_clicked(move |_| {
                if let Some(d) = &decide {
                    d(&rid, &decision);
                }
            });
            buttons.append(&b);
        }
        card.append(&buttons);
    } else {
        let decision = item.rest.get("decision").and_then(Value::as_str).unwrap_or(&item.status);
        card.append(&label(&format!("Answered: {decision}"), &["caption", "dim-label"]));
    }
    card.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_tool_blocks_split() {
        let text = "Looking.\n[external_agent_tool_call: Bash]\ninput: {\"command\":\"ls\"}\n[/external_agent_tool_call]\n[external_agent_tool_result: error]\nnope\n[/external_agent_tool_result]\nDone.";
        let parts = split_imported(text);
        assert_eq!(parts.len(), 4);
        assert!(matches!(&parts[0], Imported::Text(t) if t == "Looking."));
        assert!(
            matches!(&parts[1], Imported::Tool { name, input } if name == "Bash" && input == "{\"command\":\"ls\"}")
        );
        assert!(matches!(&parts[2], Imported::Result { text, is_error: true } if text == "nope"));
        assert!(matches!(&parts[3], Imported::Text(t) if t == "Done."));
    }
}
