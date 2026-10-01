//! HTML tree construction (pragmatic subset).
//!
//! Consumes [`Token`]s and builds a `dom::Node` tree with WHATWG-flavored
//! error recovery: unclosed elements are closed at EOF, unmatched end
//! tags are dropped, and the implied end tags of the spec's "in body"
//! insertion mode are applied for the elements that matter on real
//! pages (`p`, `li`, `dd`/`dt`, `option`, the headings, table rows and
//! cells). Doctypes and comments are accepted and ignored — they are not
//! represented in the DOM yet. The builder never panics and never fails:
//! any token stream produces a tree.
//!
//! Two bounds keep hostile input linear. The stack of open elements is
//! capped at [`MAX_DEPTH`] levels (Blink uses the same figure): a start
//! tag at the cap closes the innermost open element first, so a page of
//! nothing but `<b>` tags produces a tree 512 deep and a million wide,
//! not a million deep. That cap is also what keeps the recursive walks
//! over the finished tree (styling, layout, the derived `Clone`) within
//! any thread's stack. And every scan of the open stack is bounded by
//! it, so a start or end tag costs at most 512 comparisons.
//!
//! What the full tree construction algorithm has and this does not:
//! insertion modes (a table's cells are not wrapped in a `tbody`, a
//! stray `<td>` outside a table is an ordinary element), the active
//! formatting elements list (misnested `<b><i></b></i>` is not
//! reconstructed), foster parenting, and `<script>`/`<style>` raw text
//! (roadmap M2's noted gap, paid in M24). Foreign content is recognised
//! only to the extent that a self-closing tag inside `<svg>` or `<math>`
//! is honoured, as the spec does there and nowhere else in HTML.

use crate::tokenizer::Token;
use dom::{Element, Node};

/// The most elements that may be open at once.
pub const MAX_DEPTH: usize = 512;

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

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// Where a search for an open `p` stops (the spec's "button scope"):
/// a `p` outside these is not the one a block inside them closes.
const P_SCOPE_STOPS: [&str; 5] = ["button", "table", "td", "th", "caption"];

/// Where a search for an open `li` stops (the spec's "list item scope"
/// plus the special elements its loop stops at): an `li` outside a
/// nested list or a table cell is not closed by an `li` inside it.
const LI_SCOPE_STOPS: [&str; 6] = ["ul", "ol", "table", "td", "th", "caption"];

/// Where a search for an open `dd`/`dt` stops.
const DD_SCOPE_STOPS: [&str; 5] = ["dl", "table", "td", "th", "caption"];

/// Elements whose start tag ends an open `dd` or `dt`.
const DD_DT: [&str; 2] = ["dd", "dt"];

/// Elements whose start tag ends an open table cell.
const CELLS: [&str; 2] = ["td", "th"];

/// Elements whose content starts after a newline that is dropped.
const SKIPS_LEADING_NEWLINE: [&str; 3] = ["pre", "listing", "textarea"];

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    root: Node,
    /// Stack of open elements, outermost first.
    open: Vec<Element>,
    /// A `<pre>` (or `<listing>`/`<textarea>`) was just opened: a
    /// newline that immediately follows is not content (the spec's
    /// "in body" rule for those start tags).
    skip_leading_newline: bool,
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
            skip_leading_newline: false,
        }
    }

    /// Feeds one token into the builder.
    pub fn process(&mut self, token: Token) {
        let skip_newline = std::mem::take(&mut self.skip_leading_newline);
        match token {
            Token::Doctype(_) | Token::Comment(_) => {
                // Accepted by the tokenizer, not represented in the DOM yet.
                // A comment between `<pre>` and its newline does not
                // cancel the newline rule (the spec checks the next
                // *token*, but a comment is invisible here either way).
                self.skip_leading_newline = skip_newline;
            }
            Token::Text(text) => {
                let text = match text.strip_prefix('\n') {
                    Some(rest) if skip_newline => rest,
                    _ => text.as_str(),
                };
                self.push_text(text);
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                self.imply_end_tags_before(&name);
                let el = Element {
                    tag: name,
                    attrs,
                    children: Vec::new(),
                };
                let is_void = VOID_ELEMENTS.contains(&el.tag.as_str());
                // The self-closing flag means nothing on an HTML element
                // (the spec ignores it with a parse error): `<div/>` is
                // an open `<div>`. It is honoured in foreign content,
                // where `<path/>` really is empty.
                let closes_itself = is_void || (self_closing && self.in_foreign_content());
                if closes_itself {
                    self.attach(Node::Element(el));
                } else {
                    if self.open.len() >= MAX_DEPTH
                        && let Some(inner) = self.open.pop()
                    {
                        self.attach(Node::Element(inner));
                    }
                    if SKIPS_LEADING_NEWLINE.contains(&el.tag.as_str()) {
                        self.skip_leading_newline = true;
                    }
                    self.open.push(el);
                }
            }
            Token::EndTag(name) => {
                // Unmatched end tags are dropped (recovery); a matched one
                // closes everything opened after its element too.
                self.close_through(&name, &[]);
            }
        }
    }

    /// The implied end tags a start tag for `name` triggers, in the
    /// spec's order for the "in body" insertion mode: an `li` closes an
    /// open `li` before the block rule closes an open `p`, so
    /// `<li>a<p>b<li>c` yields two sibling items.
    fn imply_end_tags_before(&mut self, name: &str) {
        match name {
            "li" => self.close_through("li", &LI_SCOPE_STOPS),
            "dd" | "dt" => {
                if let Some(idx) = self.rposition_before(&DD_DT, &DD_SCOPE_STOPS) {
                    self.close_from(idx);
                }
            }
            "option" | "optgroup" => {
                if self.current_is(&["option"]) {
                    self.close_from(self.open.len() - 1);
                }
                if name == "optgroup" && self.current_is(&["optgroup"]) {
                    self.close_from(self.open.len() - 1);
                }
            }
            "tr" => self.close_through("tr", &["table"]),
            "td" | "th" => {
                if let Some(idx) = self.rposition_before(&CELLS, &["tr", "table"]) {
                    self.close_from(idx);
                }
            }
            _ => {}
        }
        if CLOSES_P.contains(&name) {
            self.close_through("p", &P_SCOPE_STOPS);
        }
        if HEADINGS.contains(&name) && self.current_is(&HEADINGS) {
            self.close_from(self.open.len() - 1);
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

    /// Whether the innermost open element is one of `names`.
    fn current_is(&self, names: &[&str]) -> bool {
        self.open
            .last()
            .is_some_and(|el| names.contains(&el.tag.as_str()))
    }

    /// Whether an `svg` or `math` element is open.
    fn in_foreign_content(&self) -> bool {
        self.open
            .iter()
            .any(|el| el.tag == "svg" || el.tag == "math")
    }

    /// The index of the innermost open element named one of `names`,
    /// unless one of `stops` is open above it.
    fn rposition_before(&self, names: &[&str], stops: &[&str]) -> Option<usize> {
        for (idx, el) in self.open.iter().enumerate().rev() {
            let tag = el.tag.as_str();
            if names.contains(&tag) {
                return Some(idx);
            }
            if stops.contains(&tag) {
                return None;
            }
        }
        None
    }

    /// Closes the innermost `name` element and everything opened after
    /// it, unless one of `stops` is open above it. One scan of the
    /// stack, never two.
    fn close_through(&mut self, name: &str, stops: &[&str]) {
        if let Some(idx) = self.rposition_before(&[name], stops) {
            self.close_from(idx);
        }
    }

    /// Closes the open elements from `idx` outwards, attaching each to
    /// its parent in reverse (innermost first).
    fn close_from(&mut self, idx: usize) {
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

    fn tags(nodes: &[Node]) -> Vec<&str> {
        nodes
            .iter()
            .map(|n| match n {
                Node::Element(el) => el.tag.as_str(),
                Node::Text(_) => "#text",
                Node::Document(_) => "#document",
            })
            .collect()
    }

    /// The deepest nesting anywhere in the tree.
    fn depth_of(node: &Node) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(node, 0usize)];
        while let Some((n, d)) = stack.pop() {
            deepest = deepest.max(d);
            for child in children_of(n) {
                stack.push((child, d + 1));
            }
        }
        deepest
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
    fn li_closes_the_previous_li_even_with_a_p_open_inside_it() {
        // `li` is also a block that closes `p`; the two rules used to be
        // an if/else, so with a `p` open the `li` was never closed and
        // every item nested inside the one before.
        let root = build("<ul><li>a<p>b<li>c</ul>");
        let ul = expect_element(children_of(&root), 0);
        assert_eq!(tags(&ul.children), vec!["li", "li"]);
        let first = expect_element(&ul.children, 0);
        assert_eq!(tags(&first.children), vec!["#text", "p"]);
        assert_eq!(
            expect_element(&ul.children, 1).children,
            vec![Node::text("c")]
        );
    }

    #[test]
    fn li_inside_a_nested_list_does_not_close_the_outer_item() {
        let root = build("<ul><li>a<ul><li>b<li>c</ul><li>d</ul>");
        let outer = expect_element(children_of(&root), 0);
        assert_eq!(tags(&outer.children), vec!["li", "li"]);
        let a = expect_element(&outer.children, 0);
        assert_eq!(tags(&a.children), vec!["#text", "ul"]);
        let inner = expect_element(&a.children, 1);
        assert_eq!(tags(&inner.children), vec!["li", "li"]);
    }

    #[test]
    fn dd_dt_option_and_heading_start_tags_close_their_predecessors() {
        let root = build("<dl><dt>a<dd>b<dt>c<dd>d</dl>");
        let dl = expect_element(children_of(&root), 0);
        assert_eq!(tags(&dl.children), vec!["dt", "dd", "dt", "dd"]);

        let root = build("<select><option>a<option>b<optgroup><option>c</select>");
        let select = expect_element(children_of(&root), 0);
        assert_eq!(tags(&select.children), vec!["option", "option", "optgroup"]);
        let group = expect_element(&select.children, 2);
        assert_eq!(tags(&group.children), vec!["option"]);

        let root = build("<h1>a<h2>b</h2>");
        assert_eq!(tags(children_of(&root)), vec!["h1", "h2"]);
    }

    #[test]
    fn table_rows_and_cells_become_siblings() {
        let root = build("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = expect_element(children_of(&root), 0);
        assert_eq!(tags(&table.children), vec!["tr", "tr"]);
        let first = expect_element(&table.children, 0);
        assert_eq!(tags(&first.children), vec!["td", "td"]);
        assert_eq!(
            expect_element(&first.children, 1).children,
            vec![Node::text("b")]
        );
    }

    #[test]
    fn a_block_inside_a_table_cell_does_not_close_a_p_outside_the_table() {
        let root = build("<p>x<table><tr><td><div>y</div></table>z</p>");
        // The `p` is closed by the `table` start tag itself (a block),
        // but the `div` inside the cell does not reach out past it.
        let children = children_of(&root);
        assert_eq!(tags(children), vec!["p", "table", "#text"]);
        let table = expect_element(children, 1);
        let tr = expect_element(&table.children, 0);
        let td = expect_element(&tr.children, 0);
        assert_eq!(tags(&td.children), vec!["div"]);
    }

    #[test]
    fn self_closing_flag_is_ignored_on_html_elements() {
        let root = build("<p><span/>a</p><div/>text");
        let children = children_of(&root);
        assert_eq!(tags(children), vec!["p", "div"]);
        let p = expect_element(children, 0);
        let span = expect_element(&p.children, 0);
        assert_eq!(span.children, vec![Node::text("a")]);
        let div = expect_element(children, 1);
        assert_eq!(div.children, vec![Node::text("text")]);

        let root = build(r#"<a href="x"/>link</a>"#);
        let a = expect_element(children_of(&root), 0);
        assert_eq!(a.children, vec![Node::text("link")]);
    }

    #[test]
    fn self_closing_flag_is_honoured_in_foreign_content() {
        let root = build(r#"<svg><path d="M0 0"/><circle r="1"/></svg>after"#);
        let children = children_of(&root);
        assert_eq!(tags(children), vec!["svg", "#text"]);
        let svg = expect_element(children, 0);
        assert_eq!(tags(&svg.children), vec!["path", "circle"]);
    }

    #[test]
    fn newline_right_after_pre_is_not_content() {
        let pre = |source: &str| {
            let root = build(source);
            expect_element(children_of(&root), 0).children.clone()
        };
        assert_eq!(pre("<pre>\ncode</pre>"), vec![Node::text("code")]);
        assert_eq!(pre("<pre>\r\ncode</pre>"), vec![Node::text("code")]);
        // Only the first newline, and only right after the start tag.
        assert_eq!(pre("<pre>\n\ncode</pre>"), vec![Node::text("\ncode")]);
        assert_eq!(pre("<pre>x\ncode</pre>"), vec![Node::text("x\ncode")]);
        assert_eq!(pre("<textarea>\nt</textarea>"), vec![Node::text("t")]);
        assert_eq!(pre("<p>\nkept</p>"), vec![Node::text("\nkept")]);
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
        // `</div>` closes the unclosed `<b>` inside it too, and what
        // follows is a sibling of the div, not a child of the b.
        let root = build("<div><b>bold</div>after");
        let children = children_of(&root);
        assert_eq!(children.len(), 2);
        let div = expect_element(children, 0);
        let b = expect_element(&div.children, 0);
        assert_eq!(b.tag, "b");
        assert_eq!(b.children, vec![Node::text("bold")]);
        assert_eq!(children[1], Node::text("after"));
    }

    #[test]
    fn nesting_depth_is_capped_and_the_tree_survives_on_a_small_stack() {
        // 200k nested `<b>` used to build a 200k-deep tree, which
        // overflowed the stack when dropped. The cap makes the excess
        // siblings instead, and the whole thing lives on a thread with
        // a quarter of the stack a test thread normally gets.
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let source = "<b>".repeat(200_000) + "x";
                let root = build(&source);
                // 512 elements deep, plus the text node under the last.
                assert_eq!(depth_of(&root), MAX_DEPTH + 1);
                assert!(root.text_content().ends_with('x'));
                drop(root);
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn a_page_of_nothing_but_unmatched_end_tags_is_linear() {
        // Each end tag scans the open stack, which the depth cap bounds.
        let source = "<b>".repeat(50_000) + &"</x>".repeat(200_000);
        let root = build(&source);
        assert_eq!(depth_of(&root), MAX_DEPTH);
    }
}
