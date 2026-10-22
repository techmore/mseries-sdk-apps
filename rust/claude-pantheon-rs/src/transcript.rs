//! Claude Code JSONL transcripts: one JSON record per line under `~/.claude/projects/<slug>/<uuid>.jsonl`.
//! Only `user` and `assistant` records carry conversation; everything else (api-request blobs, hooks,
//! snapshots, cost state) is skipped without being parsed. Lines are read one at a time and never held as
//! a whole-file `Value`, so a 20 MB session costs a few hundred kilobytes of memory while it loads.

use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A top-level line. Every field is optional so unknown shapes and future fields are tolerated.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Record {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: Option<Msg>,
    #[serde(rename = "isMeta")]
    pub is_meta: bool,
    #[serde(rename = "isSidechain")]
    pub is_sidechain: bool,
    pub cwd: Option<String>,
    pub timestamp: Option<String>,
    pub summary: Option<String>,
    #[serde(rename = "customTitle")]
    pub custom_title: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    /// stream-json only: `{"type":"system","subtype":"init","session_id":...}` and `{"type":"result",...}`.
    pub subtype: Option<String>,
    pub result: Option<String>,
    pub is_error: Option<bool>,
    /// stream-json with `--include-partial-messages`: `{"type":"stream_event","event":{...}}`.
    pub event: Option<StreamEvent>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Msg {
    pub role: String,
    pub content: Content,
    pub id: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<RawBlock>),
    Other(serde_json::Value),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RawBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub name: Option<String>,
    pub id: Option<String>,
    pub input: Option<serde_json::Value>,
    pub tool_use_id: Option<String>,
    pub content: Option<Content>,
    pub is_error: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub delta: Option<Delta>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Delta {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
}

// ---- the parsed conversation -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

#[derive(Clone, Debug)]
pub enum Block {
    Text(String),
    ToolUse { id: String, name: String, summary: String, input: String, result: Option<ToolResult> },
}

#[derive(Clone, Debug)]
pub struct Turn {
    pub role: Role,
    pub blocks: Vec<Block>,
    pub timestamp: Option<String>,
}

#[derive(Default, Debug)]
pub struct Transcript {
    pub turns: Vec<Turn>,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub bad_lines: usize,
}

/// Cheap pre-filter: only lines that can be conversation are handed to serde.
fn may_be_turn(line: &str) -> bool {
    line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"")
}

fn may_be_title(line: &str) -> bool {
    line.contains("\"type\":\"summary\"") || line.contains("\"type\":\"custom-title\"")
}

/// Text of a `Content` as a reader would see it: plain string, or the text blocks joined.
pub fn content_text(content: &Content) -> String {
    match content {
        Content::Text(s) => s.clone(),
        Content::Blocks(blocks) => {
            let mut out = String::new();
            for b in blocks {
                if b.kind == "text"
                    && let Some(t) = &b.text
                {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            out
        }
        Content::Other(_) => String::new(),
    }
}

/// First line of a user prompt, trimmed for a sidebar row. Local-command echoes (`<command-name>…`) and
/// injected reminders start with `<` and never make a good title.
pub fn title_from_prompt(text: &str) -> Option<String> {
    let text = text.trim_start();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut title: String = line.chars().take(90).collect();
    if line.chars().count() > 90 {
        title.push('…');
    }
    Some(title)
}

/// The record from a line, or None for anything that is not JSON.
pub fn parse_line(line: &str) -> Option<Record> {
    serde_json::from_str::<Record>(line).ok()
}

/// Read a whole session into turns, joining each tool result onto the tool call that produced it.
pub fn load(path: &Path) -> std::io::Result<Transcript> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut line = String::new();
    let mut t = Transcript::default();
    let mut builder = Builder::default();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if may_be_turn(&line) {
            match parse_line(&line) {
                Some(rec) => builder.push(rec, &mut t),
                None => t.bad_lines += 1,
            }
        } else if may_be_title(&line)
            && let Some(rec) = parse_line(&line)
            && let Some(title) = rec.custom_title.or(rec.summary)
        {
            t.title = Some(title);
        }
    }
    builder.finish(&mut t);
    if t.title.is_none() {
        t.title = t.turns.iter().find(|turn| turn.role == Role::User).and_then(|turn| {
            turn.blocks.iter().find_map(|b| if let Block::Text(s) = b { title_from_prompt(s) } else { None })
        });
    }
    Ok(t)
}

/// Accumulates records into turns; shared by the file loader and the live stream.
#[derive(Default)]
pub struct Builder {
    /// Tool results that arrived before (or without) their tool call, keyed by tool_use_id.
    orphan_results: Vec<(String, ToolResult)>,
}

impl Builder {
    pub fn push(&mut self, rec: Record, t: &mut Transcript) {
        if rec.is_sidechain || rec.is_meta {
            return;
        }
        if t.cwd.is_none() {
            t.cwd = rec.cwd.clone();
        }
        if t.session_id.is_none() {
            t.session_id = rec.session_id.clone();
        }
        let Some(msg) = rec.message else { return };
        match (rec.kind.as_str(), msg.role.as_str()) {
            ("user", "user") => self.push_user(msg.content, rec.timestamp, t),
            ("assistant", "assistant") => self.push_assistant(msg.content, rec.timestamp, t),
            _ => {}
        }
    }

    fn push_user(&mut self, content: Content, timestamp: Option<String>, t: &mut Transcript) {
        match content {
            Content::Text(s) => {
                if !s.trim().is_empty() {
                    t.turns.push(Turn { role: Role::User, blocks: vec![Block::Text(s)], timestamp });
                }
            }
            Content::Blocks(blocks) => {
                let mut text = String::new();
                for b in blocks {
                    match b.kind.as_str() {
                        "text" => {
                            if let Some(s) = b.text
                                && !s.trim().is_empty()
                            {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(&s);
                            }
                        }
                        "tool_result" => {
                            let result = ToolResult {
                                text: b.content.as_ref().map(content_text).unwrap_or_default(),
                                is_error: b.is_error.unwrap_or(false),
                            };
                            let id = b.tool_use_id.unwrap_or_default();
                            if !attach_result(t, &id, result.clone()) {
                                self.orphan_results.push((id, result));
                            }
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    t.turns.push(Turn { role: Role::User, blocks: vec![Block::Text(text)], timestamp });
                }
            }
            Content::Other(_) => {}
        }
    }

    fn push_assistant(&mut self, content: Content, timestamp: Option<String>, t: &mut Transcript) {
        let mut blocks = Vec::new();
        match content {
            Content::Text(s) => {
                if !s.is_empty() {
                    blocks.push(Block::Text(s));
                }
            }
            Content::Blocks(raw) => {
                for b in raw {
                    match b.kind.as_str() {
                        "text" => {
                            if let Some(s) = b.text
                                && !s.trim().is_empty()
                            {
                                blocks.push(Block::Text(s));
                            }
                        }
                        "tool_use" => {
                            let name = b.name.unwrap_or_else(|| "tool".into());
                            let input = b.input.unwrap_or(serde_json::Value::Null);
                            let id = b.id.unwrap_or_default();
                            let mut result = None;
                            if let Some(i) = self.orphan_results.iter().position(|(rid, _)| *rid == id) {
                                result = Some(self.orphan_results.remove(i).1);
                            }
                            blocks.push(Block::ToolUse {
                                summary: tool_summary(&name, &input),
                                input: tool_input_text(&name, &input),
                                id,
                                name,
                                result,
                            });
                        }
                        _ => {} // thinking, redacted_thinking, images
                    }
                }
            }
            Content::Other(_) => {}
        }
        if blocks.is_empty() {
            return;
        }
        // One API message arrives as several records (one per content block); keep them as one turn.
        if let Some(last) = t.turns.last_mut()
            && last.role == Role::Assistant
        {
            last.blocks.extend(blocks);
        } else {
            t.turns.push(Turn { role: Role::Assistant, blocks, timestamp });
        }
    }

    pub fn finish(&mut self, _t: &mut Transcript) {
        self.orphan_results.clear();
    }
}

/// Puts a result on its tool call; searches from the end since results follow their calls closely.
fn attach_result(t: &mut Transcript, id: &str, result: ToolResult) -> bool {
    for turn in t.turns.iter_mut().rev().take(8) {
        if turn.role != Role::Assistant {
            continue;
        }
        for b in turn.blocks.iter_mut() {
            if let Block::ToolUse { id: bid, result: slot, .. } = b
                && bid == id
                && slot.is_none()
            {
                *slot = Some(result);
                return true;
            }
        }
    }
    false
}

fn str_field<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(|v| v.as_str())
}

fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && p.starts_with(&home) { format!("~{}", &p[home.len()..]) } else { p.to_string() }
}

/// First non-empty line, cut to `max` characters.
pub fn first_line(s: &str, max: usize) -> String {
    one_line(s, max)
}

fn one_line(s: &str, max: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// "Ran Bash: …", "Edited file …": the one-line row a tool call collapses to.
pub fn tool_summary(name: &str, input: &serde_json::Value) -> String {
    let path = || str_field(input, "file_path").or(str_field(input, "path")).map(short_path).unwrap_or_default();
    match name {
        "Bash" => {
            let what = str_field(input, "description").or(str_field(input, "command")).unwrap_or("");
            format!("Ran Bash: {}", one_line(what, 100))
        }
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Edited file {}", path()),
        "Write" => format!("Wrote file {}", path()),
        "Read" => format!("Read {}", path()),
        "Grep" => format!("Searched for {}", one_line(str_field(input, "pattern").unwrap_or(""), 80)),
        "Glob" => format!("Listed files {}", one_line(str_field(input, "pattern").unwrap_or(""), 80)),
        "Agent" | "Task" => format!("Ran agent: {}", one_line(str_field(input, "description").unwrap_or(""), 80)),
        "WebSearch" => format!("Searched the web: {}", one_line(str_field(input, "query").unwrap_or(""), 80)),
        "WebFetch" => format!("Fetched {}", one_line(str_field(input, "url").unwrap_or(""), 80)),
        "TodoWrite" => "Updated the to-do list".into(),
        "Skill" => format!("Used skill {}", str_field(input, "skill").unwrap_or("")),
        "AskUserQuestion" => "Asked a question".into(),
        _ => {
            if let Some(rest) = name.strip_prefix("mcp__") {
                let tool = rest.split_once("__").map_or(rest, |(_, t)| t);
                format!("Called {}", tool.replace('_', " "))
            } else {
                format!("Used {name}")
            }
        }
    }
}

/// The expanded view of a tool call's input: the command or the diff where that reads better than JSON.
pub fn tool_input_text(name: &str, input: &serde_json::Value) -> String {
    let text = match name {
        "Bash" => str_field(input, "command").map(str::to_string),
        "Edit" => match (str_field(input, "old_string"), str_field(input, "new_string")) {
            (Some(old), Some(new)) => Some(format!("--- old\n{old}\n+++ new\n{new}")),
            _ => None,
        },
        "Write" => str_field(input, "content").map(str::to_string),
        "Read" | "Glob" | "Grep" => None,
        _ => None,
    };
    let text = text.unwrap_or_else(|| serde_json::to_string_pretty(input).unwrap_or_default());
    clip(&text, 6000)
}

/// Keeps the widget tree bounded: nobody scrolls a 300 KB tool result inside a chat row.
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… ({} more characters)", &text[..end], text.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_skip_injected_blocks() {
        assert_eq!(title_from_prompt("<command-name>/clear</command-name>"), None);
        assert_eq!(title_from_prompt("  fix the build\nplease").as_deref(), Some("fix the build"));
    }

    #[test]
    fn summaries_read_like_actions() {
        let bash = serde_json::json!({"command": "ls -la", "description": "List files"});
        assert_eq!(tool_summary("Bash", &bash), "Ran Bash: List files");
        let edit = serde_json::json!({"file_path": "/x/y.rs", "old_string": "a", "new_string": "b"});
        assert_eq!(tool_summary("Edit", &edit), "Edited file /x/y.rs");
        assert_eq!(tool_summary("mcp__ccd_turn__update_plan", &serde_json::Value::Null), "Called update plan");
    }

    #[test]
    fn tool_results_attach_to_their_call() {
        let mut t = Transcript::default();
        let mut b = Builder::default();
        let call: Record = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"true"}}]}}"#,
        )
        .unwrap();
        let result: Record = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        )
        .unwrap();
        b.push(call, &mut t);
        b.push(result, &mut t);
        assert_eq!(t.turns.len(), 1);
        match &t.turns[0].blocks[0] {
            Block::ToolUse { result: Some(r), .. } => assert_eq!(r.text, "ok"),
            other => panic!("{other:?}"),
        }
    }
}
