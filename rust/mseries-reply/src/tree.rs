//! Bounded walks over an accessibility tree. The tree itself is a trait, so the walkers are tested on in-memory trees and
//! the AT-SPI bus client ([`crate::a11y`]) is only the transport.
//!
//! Limits match the first (Python) slice: a read stops after 1.2 s or 2500 nodes, looks at no more than 200 children of
//! a node, and keeps the newest text. Nothing here reads text except [`read_lines`], which the context gate calls only
//! after the rules allowed the read.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// AT-SPI role numbers (`AtspiRole`).
pub mod role {
    pub const DIALOG: u32 = 16;
    pub const FILLER: u32 = 20;
    pub const FRAME: u32 = 23;
    pub const LABEL: u32 = 29;
    pub const LIST_ITEM: u32 = 32;
    pub const PASSWORD_TEXT: u32 = 40;
    pub const TEXT: u32 = 61;
    pub const WINDOW: u32 = 69;
    pub const PARAGRAPH: u32 = 73;
    pub const APPLICATION: u32 = 75;
    pub const ENTRY: u32 = 79;
    pub const HEADING: u32 = 83;
    pub const DOCUMENT_TEXT: u32 = 94;
    pub const DOCUMENT_WEB: u32 = 95;
    pub const STATIC: u32 = 116;

    /// Roles that hold read-only conversation text.
    pub const CONVERSATION: &[u32] = &[LABEL, PARAGRAPH, STATIC, TEXT, LIST_ITEM, HEADING];
    /// Roles the dictation input method accepts as an editable field (`safe_accessible`).
    pub const EDITABLE_FIELD: &[u32] = &[TEXT, PARAGRAPH, ENTRY, DOCUMENT_TEXT, DOCUMENT_WEB];
    /// Roles whose text lives in the `Text` interface; the others carry it in their name.
    pub const TEXT_INTERFACE: &[u32] = &[TEXT, PARAGRAPH];

    pub fn name(role: u32) -> String {
        match role {
            ENTRY => "entry".into(),
            TEXT => "text".into(),
            PARAGRAPH => "paragraph".into(),
            DOCUMENT_TEXT => "document text".into(),
            DOCUMENT_WEB => "document web".into(),
            PASSWORD_TEXT => "password text".into(),
            other => format!("role {other}"),
        }
    }
}

/// `AtspiStateType` bit positions.
pub mod state {
    pub const ACTIVE: u32 = 1;
    pub const EDITABLE: u32 = 7;
    pub const FOCUSED: u32 = 12;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct States(pub u64);

impl States {
    pub fn has(self, bit: u32) -> bool {
        self.0 & (1 << bit) != 0
    }

    pub fn from_words(words: &[u32]) -> Self {
        let low = words.first().copied().unwrap_or(0) as u64;
        let high = words.get(1).copied().unwrap_or(0) as u64;
        States(low | (high << 32))
    }
}

/// A tree whose calls can fail or time out (each returns `None` then). Nothing here is cached.
pub trait Tree {
    type Node: Clone + PartialEq;

    /// The applications on the desktop.
    fn applications(&self) -> Vec<Self::Node>;
    /// At most `limit` children, in document order.
    fn children(&self, node: &Self::Node, limit: usize) -> Vec<Self::Node>;
    fn role(&self, node: &Self::Node) -> Option<u32>;
    fn states(&self, node: &Self::Node) -> Option<States>;
    fn parent(&self, node: &Self::Node) -> Option<Self::Node>;
    /// Role and state of many nodes at once. A transport may fetch them concurrently; the default asks one by one.
    fn inspect(&self, nodes: &[Self::Node]) -> Vec<Option<(u32, States)>> {
        nodes
            .iter()
            .map(|node| Some((self.role(node)?, self.states(node)?)))
            .collect()
    }
    fn name(&self, node: &Self::Node) -> Option<String>;
    /// The node's text, at most `max_chars` of it.
    fn text(&self, node: &Self::Node, max_chars: usize) -> Option<String>;
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub window: Duration,
    pub max_nodes: usize,
    pub max_children: usize,
}

impl Limits {
    /// Reading a conversation: same as the Python bridge (1.2 s, 2500 nodes, 200 children).
    pub const READ: Limits = Limits {
        window: Duration::from_millis(1200),
        max_nodes: 2500,
        max_children: 200,
    };
    /// Finding the focused field and checking it (the dictation input's metadata budget).
    pub const FOCUS: Limits = Limits {
        window: Duration::from_millis(600),
        max_nodes: 1024,
        max_children: 128,
    };
}

struct Budget {
    deadline: Instant,
    nodes_left: usize,
    max_children: usize,
}

impl Budget {
    fn new(limits: Limits) -> Self {
        Self {
            deadline: Instant::now() + limits.window,
            nodes_left: limits.max_nodes,
            max_children: limits.max_children,
        }
    }

    fn alive(&self) -> bool {
        self.nodes_left > 0 && Instant::now() < self.deadline
    }

    fn visit(&mut self) -> bool {
        if !self.alive() {
            return false;
        }
        self.nodes_left -= 1;
        true
    }
}

const MAX_APPLICATIONS: usize = 128;
const MAX_WINDOWS: usize = 32;
const MAX_ANCESTORS: usize = 64;
/// Candidate text nodes remembered while scanning; only the newest are ever read.
const MAX_CANDIDATES: usize = 64;

/// The active top-level window, found like the Python bridge did: an application's window with the ACTIVE state.
pub fn active_window<T: Tree>(tree: &T, limits: Limits) -> Option<T::Node> {
    let budget = Budget::new(limits);
    for app in tree.applications().into_iter().take(MAX_APPLICATIONS) {
        for window in tree.children(&app, MAX_WINDOWS) {
            if !budget.alive() {
                return None;
            }
            if tree
                .states(&window)
                .is_some_and(|states| states.has(state::ACTIVE))
            {
                return Some(window);
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct Composer<N> {
    pub node: N,
    pub role: u32,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Scan<N> {
    /// The focused, editable field, if the walk reached it.
    pub composer: Option<Composer<N>>,
    /// Read-only text nodes before the composer in document order, oldest first. Their text has not been read.
    pub candidates: Vec<N>,
    pub visited: usize,
}

/// One document-order walk of a window: finds the focused editable field and remembers the conversation nodes above it.
/// It reads roles and states only (and the composer's name, which is a field label); no message text.
pub fn scan_window<T: Tree>(tree: &T, window: &T::Node, limits: Limits) -> Scan<T::Node> {
    let mut budget = Budget::new(limits);
    let mut candidates: VecDeque<T::Node> = VecDeque::new();
    let mut composer = None;
    let mut visited = 0;
    let Some(&Some((role, states))) = tree.inspect(std::slice::from_ref(window)).first() else {
        return Scan {
            composer: None,
            candidates: Vec::new(),
            visited: 0,
        };
    };
    let mut stack = vec![(window.clone(), role, states)];
    while let Some((node, role, states)) = stack.pop() {
        if !budget.visit() {
            break;
        }
        visited += 1;
        if states.has(state::FOCUSED) && states.has(state::EDITABLE) {
            composer = Some(Composer {
                label: tree.name(&node).unwrap_or_default(),
                role,
                node,
            });
            break;
        }
        if role::CONVERSATION.contains(&role) && !states.has(state::EDITABLE) {
            if candidates.len() == MAX_CANDIDATES {
                candidates.pop_front();
            }
            candidates.push_back(node.clone());
        }
        let children = tree.children(&node, budget.max_children);
        let inspected = tree.inspect(&children);
        stack.extend(
            children
                .into_iter()
                .zip(inspected)
                .rev()
                .filter_map(|(child, info)| info.map(|(role, states)| (child, role, states))),
        );
    }
    Scan {
        composer,
        candidates: candidates.into(),
        visited,
    }
}

pub const MAX_SNIPPET_CHARS: usize = 600;

/// Reads the text of the newest candidates and returns up to `max_lines` non-empty snippets, oldest first. Neighbouring
/// duplicates are dropped, whitespace is collapsed and each snippet is capped.
pub fn read_lines<T: Tree>(tree: &T, candidates: &[T::Node], max_lines: usize) -> Vec<String> {
    let mut newest_first: Vec<String> = Vec::new();
    for node in candidates.iter().rev() {
        if newest_first.len() >= max_lines {
            break;
        }
        let Some(role) = tree.role(node) else {
            continue;
        };
        let raw = if role::TEXT_INTERFACE.contains(&role) {
            tree.text(node, 4_000)
        } else {
            tree.name(node)
        };
        let text = raw
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let text: String = text.chars().take(MAX_SNIPPET_CHARS).collect();
        if text.is_empty() || newest_first.last() == Some(&text) {
            continue;
        }
        newest_first.push(text);
    }
    newest_first.reverse();
    newest_first
}

/// Finds the focused node inside the active window (the dictation input's `discover_focus`).
pub fn focused_node<T: Tree>(tree: &T, limits: Limits) -> Option<T::Node> {
    let window = active_window(tree, limits)?;
    let mut budget = Budget::new(limits);
    let mut stack = vec![window];
    while let Some(node) = stack.pop() {
        if !budget.visit() {
            return None;
        }
        if tree
            .states(&node)
            .is_some_and(|states| states.has(state::FOCUSED))
        {
            return Some(node);
        }
        stack.extend(tree.children(&node, budget.max_children).into_iter().rev());
    }
    None
}

/// The dictation input's field check: an editable text-like field that has focus and sits under the active window.
/// Password fields and unknown roles fail. Reads roles and states only, never text.
pub fn check_field<T: Tree>(tree: &T, node: &T::Node, limits: Limits) -> Result<(), &'static str> {
    let budget = Budget::new(limits);
    let (Some(role), Some(states)) = (tree.role(node), tree.states(node)) else {
        return Err("Accessible field metadata unavailable");
    };
    if !role::EDITABLE_FIELD.contains(&role)
        || !states.has(state::FOCUSED)
        || !states.has(state::EDITABLE)
    {
        return Err("Focused field is sensitive, not focused or not editable");
    }
    let mut current = node.clone();
    for _ in 0..MAX_ANCESTORS {
        if !budget.alive() {
            return Err("Field metadata deadline exceeded");
        }
        let parent = tree.parent(&current);
        let top_level = parent
            .as_ref()
            .and_then(|parent| tree.role(parent))
            .is_some_and(|role| role == role::APPLICATION);
        let window_role = tree
            .role(&current)
            .is_some_and(|role| matches!(role, role::FRAME | role::DIALOG | role::WINDOW));
        if (window_role || top_level)
            && tree
                .states(&current)
                .is_some_and(|states| states.has(state::ACTIVE))
        {
            return Ok(());
        }
        match parent {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Err("The field is not inside the active window")
}

#[cfg(test)]
pub(crate) mod fake {
    //! An in-memory tree that counts text reads.
    use super::*;
    use std::cell::Cell;

    #[derive(Clone, Debug, Default)]
    pub struct FakeNode {
        pub role: u32,
        pub states: u64,
        pub name: String,
        pub text: String,
        pub children: Vec<usize>,
        pub parent: Option<usize>,
    }

    #[derive(Default)]
    pub struct FakeTree {
        pub nodes: Vec<FakeNode>,
        pub apps: Vec<usize>,
        pub text_reads: Cell<usize>,
        pub name_reads: Cell<usize>,
    }

    pub fn bits(list: &[u32]) -> u64 {
        list.iter().fold(0, |acc, bit| acc | (1 << bit))
    }

    impl FakeTree {
        pub fn add(&mut self, parent: Option<usize>, mut node: FakeNode) -> usize {
            let index = self.nodes.len();
            node.parent = parent;
            self.nodes.push(node);
            if let Some(parent) = parent {
                self.nodes[parent].children.push(index);
            }
            index
        }

        pub fn leaf(role: u32, text: &str) -> FakeNode {
            FakeNode {
                role,
                name: text.into(),
                text: text.into(),
                ..FakeNode::default()
            }
        }

        /// application > window(active) > [messages..., composer(focused, editable)]
        pub fn chat(messages: &[&str], composer_role: u32, composer_label: &str) -> (Self, usize) {
            let mut tree = FakeTree::default();
            let app = tree.add(
                None,
                FakeNode {
                    role: role::APPLICATION,
                    ..FakeNode::default()
                },
            );
            tree.apps.push(app);
            let window = tree.add(
                Some(app),
                FakeNode {
                    role: role::FRAME,
                    states: bits(&[state::ACTIVE]),
                    ..FakeNode::default()
                },
            );
            for message in messages {
                tree.add(Some(window), FakeTree::leaf(role::PARAGRAPH, message));
            }
            let composer = tree.add(
                Some(window),
                FakeNode {
                    role: composer_role,
                    states: bits(&[state::FOCUSED, state::EDITABLE]),
                    name: composer_label.into(),
                    text: "typed secret".into(),
                    ..FakeNode::default()
                },
            );
            (tree, composer)
        }
    }

    impl Tree for FakeTree {
        type Node = usize;

        fn applications(&self) -> Vec<usize> {
            self.apps.clone()
        }
        fn children(&self, node: &usize, limit: usize) -> Vec<usize> {
            self.nodes[*node]
                .children
                .iter()
                .copied()
                .take(limit)
                .collect()
        }
        fn role(&self, node: &usize) -> Option<u32> {
            Some(self.nodes[*node].role)
        }
        fn states(&self, node: &usize) -> Option<States> {
            Some(States(self.nodes[*node].states))
        }
        fn parent(&self, node: &usize) -> Option<usize> {
            self.nodes[*node].parent
        }
        fn name(&self, node: &usize) -> Option<String> {
            self.name_reads.set(self.name_reads.get() + 1);
            Some(self.nodes[*node].name.clone())
        }
        fn text(&self, node: &usize, max_chars: usize) -> Option<String> {
            self.text_reads.set(self.text_reads.get() + 1);
            Some(self.nodes[*node].text.chars().take(max_chars).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    #[test]
    fn the_scan_finds_the_composer_and_remembers_text_above_it_without_reading_any() {
        let (tree, composer) = FakeTree::chat(&["hello", "lunch?"], role::ENTRY, "Type a message");
        let window = active_window(&tree, Limits::READ).unwrap();
        let scan = scan_window(&tree, &window, Limits::READ);
        let found = scan.composer.unwrap();
        assert_eq!(found.node, composer);
        assert_eq!(found.label, "Type a message");
        assert_eq!(scan.candidates.len(), 2);
        // Only the composer's label was read: no message text and not the composer's own text.
        assert_eq!(tree.text_reads.get(), 0);
        assert_eq!(tree.name_reads.get(), 1);
    }

    #[test]
    fn lines_come_back_oldest_first_with_the_newest_kept_and_duplicates_dropped() {
        let many: Vec<String> = (0..30).map(|i| format!("message {i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let (tree, _) = FakeTree::chat(&refs, role::ENTRY, "Message");
        let window = active_window(&tree, Limits::READ).unwrap();
        let scan = scan_window(&tree, &window, Limits::READ);
        let lines = read_lines(&tree, &scan.candidates, 5);
        assert_eq!(lines.first().unwrap(), "message 25");
        assert_eq!(lines.last().unwrap(), "message 29");
        assert_eq!(lines.len(), 5);
        // Only the newest were read.
        assert_eq!(tree.text_reads.get(), 5);

        let (tree, _) = FakeTree::chat(&["a  b\n c", "a b c", "", "next"], role::ENTRY, "Message");
        let window = active_window(&tree, Limits::READ).unwrap();
        let scan = scan_window(&tree, &window, Limits::READ);
        assert_eq!(read_lines(&tree, &scan.candidates, 12), ["a b c", "next"]);
    }

    #[test]
    fn text_after_the_composer_is_not_collected() {
        let (mut tree, _) = FakeTree::chat(&["before"], role::ENTRY, "Message");
        let window = tree.nodes[0].children[0];
        tree.add(Some(window), FakeTree::leaf(role::LABEL, "Send button"));
        let scan = scan_window(&tree, &window, Limits::READ);
        let lines = read_lines(&tree, &scan.candidates, 12);
        assert_eq!(lines, ["before"]);
    }

    #[test]
    fn the_node_limit_stops_the_walk() {
        let many: Vec<String> = (0..50).map(|i| format!("m{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let (tree, _) = FakeTree::chat(&refs, role::ENTRY, "Message");
        let window = active_window(&tree, Limits::READ).unwrap();
        let tiny = Limits {
            max_nodes: 10,
            ..Limits::READ
        };
        let scan = scan_window(&tree, &window, tiny);
        assert!(scan.composer.is_none());
        assert_eq!(scan.visited, 10);
    }

    #[test]
    fn the_field_check_matches_the_dictation_input() {
        let (tree, composer) = FakeTree::chat(&[], role::ENTRY, "Message");
        assert!(check_field(&tree, &composer, Limits::FOCUS).is_ok());
        assert_eq!(focused_node(&tree, Limits::FOCUS), Some(composer));

        let (tree, composer) = FakeTree::chat(&[], role::PASSWORD_TEXT, "Password");
        assert!(check_field(&tree, &composer, Limits::FOCUS).is_err());

        // Editable but not focused.
        let (mut tree, composer) = FakeTree::chat(&[], role::ENTRY, "Message");
        tree.nodes[composer].states = bits(&[state::EDITABLE]);
        assert!(check_field(&tree, &composer, Limits::FOCUS).is_err());

        // A window that is not the active one.
        let (mut tree, composer) = FakeTree::chat(&[], role::ENTRY, "Message");
        let window = tree.nodes[composer].parent.unwrap();
        tree.nodes[window].states = 0;
        assert!(check_field(&tree, &composer, Limits::FOCUS).is_err());
        assert_eq!(active_window(&tree, Limits::READ), None);
    }

    #[test]
    fn a_deep_composer_is_still_found_under_the_active_window() {
        // Newelle's composer sits under 25 wrapper nodes.
        let (mut tree, _) = FakeTree::chat(&[], role::ENTRY, "Message");
        let window = tree.nodes[0].children[0];
        let mut parent = window;
        for _ in 0..25 {
            parent = tree.add(
                Some(parent),
                FakeNode {
                    role: role::FILLER,
                    ..FakeNode::default()
                },
            );
        }
        let deep = tree.add(
            Some(parent),
            FakeNode {
                role: role::ENTRY,
                states: bits(&[state::FOCUSED, state::EDITABLE]),
                name: "Ask".into(),
                ..FakeNode::default()
            },
        );
        assert!(check_field(&tree, &deep, Limits::FOCUS).is_ok());
    }

    #[test]
    fn state_words_become_bits() {
        let states = States::from_words(&[1 << state::ACTIVE, 1 << 7]);
        assert!(states.has(state::ACTIVE));
        assert!(states.has(32 + 7));
        assert!(!states.has(state::FOCUSED));
    }
}
