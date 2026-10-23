//! The SwiftUI/QML-style text form of an app description. It compiles to exactly the JSON the rest of the SDK uses
//! (`parse`) and prints back from it canonically (`print`), so `msdk fmt` is parse then print.
//!
//! See `mseries/SDK-DESIGN-LANGUAGE.md` for the grammar and the mapping rules.

mod lex;
mod names;
mod parse;
mod print;

use std::collections::BTreeMap;

use serde_json::Value;

pub use parse::parse;
pub use print::print;

#[derive(Debug)]
/// A parsed document and where each JSON pointer came from, so diagnostics can name a source line.
pub struct Parsed {
    pub doc: Value,
    /// JSON pointer to 1-based source line.
    pub lines: BTreeMap<String, usize>,
}

impl Parsed {
    /// The line of the longest recorded pointer that is a prefix of `pointer`.
    pub fn line_for(&self, pointer: &str) -> Option<usize> {
        let mut p = pointer;
        loop {
            if let Some(l) = self.lines.get(p) {
                return Some(*l);
            }
            match p.rfind('/') {
                Some(i) => p = &p[..i],
                None => return self.lines.get("").copied(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.col, self.message)
    }
}

/// True for file names in the text form.
pub fn is_text_path(path: &str) -> bool {
    path.ends_with(".msdk")
}

/// Loads a description from either form: `.msdk` text or JSON. The second value maps pointers to lines for text sources.
pub fn load(path: &std::path::Path) -> Result<(Value, Option<Parsed>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if is_text_path(&path.to_string_lossy()) {
        let p = parse(&text).map_err(|e| e.to_string())?;
        Ok((p.doc.clone(), Some(p)))
    } else {
        serde_json::from_str(&text).map(|d| (d, None)).map_err(|e| format!("line {}: {e}", e.line()))
    }
}
