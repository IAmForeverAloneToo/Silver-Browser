//! The document object model — the leaf crate of the engine.
//!
//! Two forms of a tree. [`Node`] is an owned tree, the literal the
//! parser hands out and tests build with [`Element`]'s builder. The
//! live document is [`Document`] (M13): every node in an arena, named by
//! a [`NodeId`] that survives changes around it, changed through the
//! DOM's operations, each change recorded in a journal for the stages
//! after the DOM. [`Document::from_node`] and [`Document::to_node`]
//! convert between the two. See `docs/architecture.md` for the layering
//! contract and `docs/design/incremental.md` for the live document.

use std::fmt;

mod document;
mod microsyntax;

pub use document::{
    Change, Children, Document, MAX_DEPTH, MAX_JOURNAL, MAX_NODES, MutationError, NodeData, NodeId,
    Subtree,
};
pub use microsyntax::{parse_integer, parse_non_negative_integer};

/// A text node's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// The character data.
    pub data: String,
}

/// An element node's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Lowercased tag name, e.g. `"h1"`.
    pub tag: String,
    /// Attribute list, order preserved.
    pub attrs: Vec<(String, String)>,
    /// Child nodes.
    pub children: Vec<Node>,
}

/// A node in the document tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// The document root.
    Document(Vec<Node>),
    /// An element with children.
    Element(Element),
    /// Character data.
    Text(Text),
}

impl Element {
    /// Creates an empty element with the given tag name.
    pub fn new(tag: impl Into<String>) -> Self {
        Element {
            tag: tag.into(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Sets an attribute, consuming and returning the element (builder style).
    pub fn attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        if let Some(slot) = self.attrs.iter_mut().find(|(k, _)| *k == name) {
            slot.1 = value;
        } else {
            self.attrs.push((name, value));
        }
        self
    }

    /// Appends a child node (builder style). Accepts anything that converts
    /// into a [`Node`] — both `Element` and `Node` itself.
    pub fn push(mut self, child: impl Into<Node>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Appends multiple children at once (builder style) — convenient for
    /// generated fixtures (e.g. N sibling `<li>`s in a test).
    pub fn push_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    /// Appends a text child (builder style).
    pub fn text(self, data: impl Into<String>) -> Self {
        self.push(Node::Text(Text { data: data.into() }))
    }
}

impl Node {
    /// Creates a document node with the given children.
    pub fn document(children: Vec<Node>) -> Self {
        Node::Document(children)
    }

    /// Creates a text node.
    pub fn text(data: impl Into<String>) -> Self {
        Node::Text(Text { data: data.into() })
    }

    /// Creates an element (shorthand for [`Element::new`]).
    pub fn element(tag: impl Into<String>) -> Element {
        Element::new(tag)
    }

    /// The children of a document or element; a text node has none.
    pub fn children(&self) -> &[Node] {
        match self {
            Node::Document(children) | Node::Element(Element { children, .. }) => children,
            Node::Text(_) => &[],
        }
    }

    /// Concatenated descendant text, in document order. Walks with an
    /// explicit stack, so a tree of any depth is fine (see [`Drop`]
    /// below for why depth is a concern at all).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<&Node> = vec![self];
        while let Some(node) = stack.pop() {
            match node {
                Node::Text(Text { data }) => out.push_str(data),
                other => stack.extend(other.children().iter().rev()),
            }
        }
        out
    }

    /// Renders an indented, human-readable dump of the tree.
    pub fn pretty_print(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<(&Node, usize)> = vec![(self, 0)];
        while let Some((node, depth)) = stack.pop() {
            for _ in 0..depth {
                out.push_str("  ");
            }
            match node {
                Node::Document(_) => out.push_str("document\n"),
                Node::Element(Element { tag, attrs, .. }) => {
                    out.push_str("element ");
                    out.push_str(tag);
                    for (k, v) in attrs {
                        out.push_str(&format!(" {k}=\"{v}\""));
                    }
                    out.push('\n');
                }
                Node::Text(Text { data }) => out.push_str(&format!("text \"{data}\"\n")),
            }
            stack.extend(node.children().iter().rev().map(|c| (c, depth + 1)));
        }
        out
    }
}

/// Dropping an element drains its subtree into a worklist first, so a
/// deeply nested tree is freed iteratively. The compiler-generated drop
/// recurses once per level, and a tree a few tens of thousands of levels
/// deep overflows the stack when it is simply let go of. The HTML parser
/// caps nesting at 512 levels for that reason among others; this guards
/// trees built any other way (scripting, later).
impl Drop for Element {
    fn drop(&mut self) {
        let mut pending: Vec<Node> = std::mem::take(&mut self.children);
        while let Some(node) = pending.pop() {
            match node {
                Node::Document(mut children) => pending.append(&mut children),
                Node::Element(mut el) => pending.append(&mut el.children),
                Node::Text(_) => {}
            }
        }
    }
}

impl From<Element> for Node {
    fn from(el: Element) -> Self {
        Node::Element(el)
    }
}

impl From<Text> for Node {
    fn from(t: Text) -> Self {
        Node::Text(t)
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> Node {
        let h1 = Element::new("h1").text("Hello from Silver Browser");
        let p = Element::new("p")
            .attr("class", "tagline")
            .text("A hobby browser engine in Rust.");
        let body = Element::new("body").push(h1).push(p);
        Node::document(vec![Element::new("html").push(body).into()])
    }

    #[test]
    fn builder_produces_expected_structure() {
        let tree = sample_tree();
        let Node::Document(children) = &tree else {
            panic!("root must be a document");
        };
        assert_eq!(children.len(), 1);
        let Node::Element(html) = &children[0] else {
            panic!("child must be an element");
        };
        assert_eq!(html.tag, "html");
        let Node::Element(body) = &html.children[0] else {
            panic!("html's child must be an element");
        };
        assert_eq!(body.children.len(), 2);
        let Node::Element(p) = &body.children[1] else {
            panic!("second body child must be an element");
        };
        assert_eq!(p.attrs, vec![("class".to_string(), "tagline".to_string())]);
    }

    #[test]
    fn attr_replaces_existing_name() {
        let el = Element::new("div").attr("id", "a").attr("id", "b");
        assert_eq!(el.attrs.len(), 1);
        assert_eq!(el.attrs[0].1, "b");
    }

    #[test]
    fn push_children_appends_in_order() {
        let items: Vec<Node> = (0..3)
            .map(|i| Element::new("li").text(i.to_string()).into())
            .collect();
        let ul = Element::new("ul").push_children(items);
        assert_eq!(ul.children.len(), 3);
        assert_eq!(ul.children[0].text_content(), "0");
        assert_eq!(ul.children[2].text_content(), "2");
    }

    #[test]
    fn text_content_concatenates_in_order() {
        let tree = sample_tree();
        assert_eq!(
            tree.text_content(),
            "Hello from Silver BrowserA hobby browser engine in Rust."
        );
    }

    #[test]
    fn a_very_deep_tree_drops_and_walks_without_overflowing_the_stack() {
        // Built by wrapping, which is iterative; dropped, read and
        // printed on a thread with a small stack, which the derived
        // recursive drop could not survive.
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut el = Element::new("b").text("leaf");
                for _ in 0..100_000 {
                    el = Element::new("b").push(el);
                }
                let tree = Node::document(vec![el.into()]);
                assert_eq!(tree.text_content(), "leaf");
                drop(tree);
                // The printed form is quadratic in depth by nature (each
                // line is indented to its depth), so print a shallower one.
                let mut el = Element::new("b").text("leaf");
                for _ in 0..2_000 {
                    el = Element::new("b").push(el);
                }
                let tree = Node::document(vec![el.into()]);
                assert!(tree.pretty_print().ends_with("text \"leaf\"\n"));
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn pretty_print_indents() {
        let printed = sample_tree().pretty_print();
        assert!(printed.starts_with("document\n"));
        assert!(printed.contains("  element html\n"));
        assert!(printed.contains("    element h1\n"));
        assert!(printed.contains("      text \"Hello from Silver Browser\"\n"));
    }
}
