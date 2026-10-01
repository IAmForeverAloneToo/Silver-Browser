//! The document object model — the leaf crate of the engine.
//!
//! Owns the node tree that `html` produces, `style` decorates, and `paint`
//! walks. See `docs/architecture.md` for the layering contract.

use std::fmt;

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

    /// Concatenated descendant text, in document order.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        match self {
            Node::Document(children) | Node::Element(Element { children, .. }) => {
                for child in children {
                    child.collect_text(out);
                }
            }
            Node::Text(Text { data }) => out.push_str(data),
        }
    }

    /// Renders an indented, human-readable dump of the tree.
    pub fn pretty_print(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }

    fn write_pretty(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        match self {
            Node::Document(children) => {
                out.push_str(&indent);
                out.push_str("document\n");
                for child in children {
                    child.write_pretty(out, depth + 1);
                }
            }
            Node::Element(Element {
                tag,
                attrs,
                children,
            }) => {
                out.push_str(&indent);
                out.push_str("element ");
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push_str(&format!(" {k}=\"{v}\""));
                }
                out.push('\n');
                for child in children {
                    child.write_pretty(out, depth + 1);
                }
            }
            Node::Text(Text { data }) => {
                out.push_str(&indent);
                out.push_str(&format!("text \"{data}\"\n"));
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
        let h1 = Element::new("h1").text("Hello from Random Browser");
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
            "Hello from Random BrowserA hobby browser engine in Rust."
        );
    }

    #[test]
    fn pretty_print_indents() {
        let printed = sample_tree().pretty_print();
        assert!(printed.starts_with("document\n"));
        assert!(printed.contains("  element html\n"));
        assert!(printed.contains("    element h1\n"));
        assert!(printed.contains("      text \"Hello from Random Browser\"\n"));
    }
}
