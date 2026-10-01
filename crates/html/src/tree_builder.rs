//! HTML tree construction (pragmatic subset).
//!
//! Consumes [`Token`]s and builds a `dom::Node` tree with WHATWG-flavored
//! error recovery: unclosed elements are closed at EOF, unmatched end
//! tags are dropped, and start tags for block elements close an open
//! `p` (likewise `li`). Doctypes and comments are accepted and ignored —
//! they are not represented in the DOM yet. The builder never panics and
//! never fails: any token stream produces a tree.

use crate::tokenizer::Token;
use dom::{Element, Node};

/// Elements that never have children (never pushed onto the open stack).
const VOID_ELEMENTS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Start tags that imply the end of an open `p` element.
const CLOSES_P: [&str; 31] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    root: Node,
    /// Stack of open elements, outermost first.
    open: Vec<Element>,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        TreeBuilder::new()
    }
}

impl TreeBuilder {
    /// Creates a builder for a fresh document.
    pub fn new() -> Self {
        TreeBuilder {
            root: Node::document(Vec::new()),
            open: Vec::new(),
        }
    }

    /// Feeds one token into the builder.
    pub fn process(&mut self, token: Token) {
        match token {
            Token::Doctype(_) | Token::Comment(_) => {
                // Accepted by the tokenizer, not represented in the DOM yet.
            }
            Token::Text(text) => self.push_text(&text),
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                // Implied end tags.
                if CLOSES_P.contains(&name.as_str()) && self.has_open("p") {
                    self.close_through("p");
                } else if name == "li" && self.has_open("li") {
                    self.close_through("li");
                }
                let el = Element {
                    tag: name,
                    attrs,
                    children: Vec::new(),
                };
                if self_closing || VOID_ELEMENTS.contains(&el.tag.as_str()) {
                    self.attach(Node::Element(el));
                } else {
                    self.open.push(el);
                }
            }
            Token::EndTag(name) => {
                if self.has_open(&name) {
                    self.close_through(&name);
                }
                // Unmatched end tags are dropped (recovery).
            }
        }
    }

    /// Finishes the document, closing any elements left open.
    pub fn finish(mut self) -> Node {
        while let Some(el) = self.open.pop() {
            self.attach(Node::Element(el));
        }
        self.root
    }

    /// Appends text to the innermost open element (or the document),
    /// merging with a preceding text node when possible.
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let target: &mut Vec<Node> = match self.open.last_mut() {
            Some(parent) => &mut parent.children,
            None => match &mut self.root {
                Node::Document(children) => children,
                _ => return,
            },
        };
        if let Some(Node::Text(last)) = target.last_mut() {
            last.data.push_str(text);
        } else {
            target.push(Node::text(text));
        }
    }

    /// Attaches a completed node below the innermost open element (or
    /// the document).
    fn attach(&mut self, node: Node) {
        match self.open.last_mut() {
            Some(parent) => parent.children.push(node),
            None => {
                if let Node::Document(children) = &mut self.root {
                    children.push(node);
                }
            }
        }
    }

    fn has_open(&self, name: &str) -> bool {
        self.open.iter().any(|el| el.tag == name)
    }

    /// Closes the innermost `name` element and everything opened after
    /// it, attaching each to its parent in reverse (innermost first).
    fn close_through(&mut self, name: &str) {
        let Some(idx) = self.open.iter().rposition(|el| el.tag == name) else {
            return;
        };
        let mut closed = self.open.split_off(idx); // outermost..innermost
        while let Some(el) = closed.pop() {
            let node = Node::Element(el);
            if let Some(parent) = closed.last_mut() {
                parent.children.push(node);
            } else {
                self.attach(node);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::Tokenizer;

    /// Builds a tree from HTML source (test convenience).
    fn build(source: &str) -> Node {
        let mut tokenizer = Tokenizer::new(source);
        let mut builder = TreeBuilder::new();
        while let Some(token) = tokenizer.next_token() {
            builder.process(token);
        }
        builder.finish()
    }

    fn children_of(node: &Node) -> &[Node] {
        match node {
            Node::Document(children) => children,
            Node::Element(el) => &el.children,
            Node::Text(_) => &[],
        }
    }

    fn expect_element(nodes: &[Node], index: usize) -> &Element {
        match &nodes[index] {
            Node::Element(el) => el,
            other => panic!("expected element at {index}, got {other:?}"),
        }
    }

    #[test]
    fn builds_nested_structure() {
        let root = build("<div><p>hi</p></div>");
        let div = expect_element(children_of(&root), 0);
        assert_eq!(div.tag, "div");
        let p = expect_element(&div.children, 0);
        assert_eq!(p.tag, "p");
        assert_eq!(p.children, vec![Node::text("hi")]);
    }

    #[test]
    fn void_elements_do_not_nest() {
        let root = build("<p>a<br>b<img src=\"x\">c</p>");
        let p = expect_element(children_of(&root), 0);
        assert_eq!(
            p.children,
            vec![
                Node::text("a"),
                dom::Element::new("br").into(),
                Node::text("b"),
                dom::Element::new("img").attr("src", "x").into(),
                Node::text("c"),
            ]
        );
    }

    #[test]
    fn unclosed_elements_close_at_eof() {
        let root = build("<div><p>a");
        let div = expect_element(children_of(&root), 0);
        let p = expect_element(&div.children, 0);
        assert_eq!(p.children, vec![Node::text("a")]);
    }

    #[test]
    fn unmatched_end_tags_are_dropped() {
        let root = build("<div>a</span>b</div>");
        let div = expect_element(children_of(&root), 0);
        // The two text runs merge in the surviving div.
        assert_eq!(div.children, vec![Node::text("ab")]);
    }

    #[test]
    fn p_auto_closes() {
        let root = build("<p>one<p>two");
        let children = children_of(&root);
        assert_eq!(children.len(), 2);
        assert_eq!(expect_element(children, 0).tag, "p");
        assert_eq!(expect_element(children, 1).tag, "p");
    }

    #[test]
    fn blocks_close_open_p() {
        let root = build("<p>one<div>d</div>");
        let children = children_of(&root);
        assert_eq!(children.len(), 2);
        assert_eq!(expect_element(children, 0).tag, "p");
        assert_eq!(expect_element(children, 1).tag, "div");
    }

    #[test]
    fn li_auto_closes() {
        let root = build("<ul><li>a<li>b</ul>");
        let ul = expect_element(children_of(&root), 0);
        assert_eq!(ul.children.len(), 2);
        assert_eq!(expect_element(&ul.children, 0).tag, "li");
        assert_eq!(expect_element(&ul.children, 1).tag, "li");
    }

    #[test]
    fn attributes_are_preserved() {
        let root = build("<a href='x' id=y>z</a>");
        let a = expect_element(children_of(&root), 0);
        assert_eq!(
            a.attrs,
            vec![
                ("href".to_string(), "x".to_string()),
                ("id".to_string(), "y".to_string()),
            ]
        );
        assert_eq!(a.children, vec![Node::text("z")]);
    }

    #[test]
    fn end_tag_closes_interleaved_elements() {
        // </div> must close the unclosed <b> inside it too.
        let root = build("<div><b>bold");
        let div = expect_element(children_of(&root), 0);
        let b = expect_element(&div.children, 0);
        assert_eq!(b.tag, "b");
        assert_eq!(b.children, vec![Node::text("bold")]);
    }
}
