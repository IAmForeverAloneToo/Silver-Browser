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
//! The document always has the spec's skeleton (M10's parser step):
//! `html`, `head` and `body` are implied when the source does not
//! write them, the head-only elements (`title`, `style`, `script`,
//! `meta`, `link`, `base` and the rest of [`HEAD_ELEMENTS`]) that come
//! before any body content go into the head, the first body content
//! closes the head and opens the body, and the `</head>`, `</body>` and
//! `</html>` end tags switch nothing off, so content after them still
//! lands in the body, as in browsers. White space before the body is
//! dropped rather than kept as text nodes between the skeleton's
//! elements, which no rendering can tell apart. A head-only element
//! that arrives after the head was closed goes into the body rather
//! than back into the head (the spec reopens the head); a stylesheet
//! or script there is collected and hidden all the same.
//!
//! What the full tree construction algorithm has and this does not:
//! the table insertion modes (a table's cells are not wrapped in a
//! `tbody`, a stray `<td>` outside a table is an ordinary element), the
//! active formatting elements list (misnested `<b><i></b></i>` is not
//! reconstructed), and foster parenting. Foreign content is recognised
//! only to the extent that a self-closing tag inside `<svg>` or `<math>`
//! is honoured, as the spec does there and nowhere else in HTML.

use crate::tokenizer::Token;
use dom::{Element, Node};

/// The most elements that may be open at once.
pub const MAX_DEPTH: usize = dom::MAX_DEPTH;

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

/// The elements the spec's "in head" insertion mode keeps in the head:
/// before any body content they go there, implying the head if need be.
pub const HEAD_ELEMENTS: [&str; 11] = [
    "base", "basefont", "bgsound", "link", "meta", "noframes", "noscript", "script", "style",
    "template", "title",
];

/// How far the document's skeleton has got: the spec's insertion modes
/// for the parts of a document outside the body, reduced to what
/// decides where a token goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing yet: the next element implies `html`.
    Initial,
    /// `html` is open; the next element implies `head` or `body`.
    BeforeHead,
    /// `head` is open.
    InHead,
    /// `head` was closed; the next body content implies `body`.
    AfterHead,
    /// `body` is open; everything goes in it.
    InBody,
}

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    root: Node,
    /// Stack of open elements, outermost first.
    open: Vec<Element>,
    /// A `<pre>` (or `<listing>`/`<textarea>`) was just opened: a
    /// newline that immediately follows is not content (the spec's
    /// "in body" rule for those start tags).
    skip_leading_newline: bool,
    phase: Phase,
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
            phase: Phase::Initial,
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
                if self.phase == Phase::InBody || self.inside_a_head_element() {
                    self.push_text(text);
                } else {
                    // White space between the skeleton's elements is not
                    // content; the first other character is body content.
                    let content = text.trim_start_matches(is_html_whitespace);
                    if !content.is_empty() {
                        self.ensure_body();
                        self.push_text(content);
                    }
                }
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => self.start_tag(name, attrs, self_closing),
            Token::EndTag(name) => match name.as_str() {
                // The skeleton stays open to the end: content after
                // `</body>` or `</html>` is still body content.
                "html" | "body" => {}
                "head" => {
                    if self.phase == Phase::InHead {
                        self.close_through("head", &[]);
                        self.phase = Phase::AfterHead;
                    }
                }
                // Unmatched end tags are dropped (recovery); a matched one
                // closes everything opened after its element too.
                _ => self.close_through(&name, &[]),
            },
        }
    }

    /// Places a start tag: the skeleton elements by name, head-only
    /// elements in the head while it is still open, everything else in
    /// the body, implied as needed.
    fn start_tag(&mut self, name: String, attrs: Vec<(String, String)>, self_closing: bool) {
        match name.as_str() {
            "html" => {
                if self.phase == Phase::Initial {
                    self.open_skeleton("html", attrs);
                    self.phase = Phase::BeforeHead;
                } else {
                    merge_attrs(self.open.first_mut(), attrs);
                }
                return;
            }
            "head" => {
                match self.phase {
                    Phase::Initial => self.open_skeleton("html", Vec::new()),
                    Phase::BeforeHead => {}
                    _ => return, // a second head: ignored
                }
                self.open_skeleton("head", attrs);
                self.phase = Phase::InHead;
                return;
            }
            "body" => {
                match self.phase {
                    Phase::InBody => {
                        merge_attrs(self.open.iter_mut().find(|e| e.tag == "body"), attrs);
                        return;
                    }
                    Phase::Initial => {
                        self.open_skeleton("html", Vec::new());
                        self.imply_empty_head();
                    }
                    Phase::BeforeHead => self.imply_empty_head(),
                    Phase::InHead => self.close_through("head", &[]),
                    Phase::AfterHead => {}
                }
                self.open_skeleton("body", attrs);
                self.phase = Phase::InBody;
                return;
            }
            _ => {}
        }
        if HEAD_ELEMENTS.contains(&name.as_str()) {
            match self.phase {
                Phase::Initial => {
                    self.open_skeleton("html", Vec::new());
                    self.open_skeleton("head", Vec::new());
                    self.phase = Phase::InHead;
                }
                Phase::BeforeHead => {
                    self.open_skeleton("head", Vec::new());
                    self.phase = Phase::InHead;
                }
                Phase::InHead | Phase::InBody => {}
                // After the head was closed: into the body (the spec
                // reopens the head; see the module doc).
                Phase::AfterHead => self.ensure_body(),
            }
        } else {
            self.ensure_body();
        }

        self.imply_end_tags_before(&name);
        let el = Element {
            tag: name,
            attrs,
            children: Vec::new(),
        };
        let is_void = VOID_ELEMENTS.contains(&el.tag.as_str());
        // The self-closing flag means nothing on an HTML element (the
        // spec ignores it with a parse error): `<div/>` is an open
        // `<div>`. It is honoured in foreign content, where `<path/>`
        // really is empty.
        let closes_itself = is_void || (self_closing && self.in_foreign_content());
        if closes_itself {
            self.attach(Node::Element(el));
        } else {
            self.pop_at_the_cap();
            if SKIPS_LEADING_NEWLINE.contains(&el.tag.as_str()) {
                self.skip_leading_newline = true;
            }
            self.open.push(el);
        }
    }

    /// Opens one of the skeleton elements (`html`, `head`, `body`).
    fn open_skeleton(&mut self, tag: &str, attrs: Vec<(String, String)>) {
        self.pop_at_the_cap();
        self.open.push(Element {
            tag: tag.to_string(),
            attrs,
            children: Vec::new(),
        });
    }

    /// An empty `head`, when body content arrives before any head.
    fn imply_empty_head(&mut self) {
        self.open_skeleton("head", Vec::new());
        self.close_through("head", &[]);
    }

    /// Makes sure the body is open, implying whatever of the skeleton
    /// is still missing.
    fn ensure_body(&mut self) {
        match self.phase {
            Phase::Initial => {
                self.open_skeleton("html", Vec::new());
                self.imply_empty_head();
            }
            Phase::BeforeHead => self.imply_empty_head(),
            Phase::InHead => self.close_through("head", &[]),
            Phase::AfterHead => {}
            Phase::InBody => return,
        }
        self.open_skeleton("body", Vec::new());
        self.phase = Phase::InBody;
    }

    /// Whether text goes into an element of the head (a `title`, a
    /// `style`, a `script`) rather than between the skeleton's elements.
    fn inside_a_head_element(&self) -> bool {
        self.open
            .last()
            .is_some_and(|el| el.tag != "html" && el.tag != "head")
    }

    /// At the depth cap, closes the innermost open element to make
    /// room (see the module doc).
    fn pop_at_the_cap(&mut self) {
        if self.open.len() >= MAX_DEPTH
            && let Some(inner) = self.open.pop()
        {
            self.attach(Node::Element(inner));
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

    /// Finishes the document, closing any elements left open. Even an
    /// empty input has the skeleton: `html`, `head`, `body`.
    pub fn finish(mut self) -> Node {
        self.ensure_body();
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

/// The spec's ASCII white space: tab, line feed, form feed, carriage
/// return and space.
fn is_html_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// A repeated `<html>` or `<body>` start tag adds the attributes the
/// open element does not have yet (the spec's rule), never replaces.
fn merge_attrs(target: Option<&mut Element>, attrs: Vec<(String, String)>) {
    if let Some(el) = target {
        for (name, value) in attrs {
            if !el.attrs.iter().any(|(n, _)| *n == name) {
                el.attrs.push((name, value));
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

    /// The document's skeleton: `html` with `head` and `body`, which
    /// every parse has (implied when the source did not write them).
    fn skeleton(doc: &Node) -> (&Element, &Element, &Element) {
        let top = children_of(doc);
        assert_eq!(tags(top), vec!["html"], "one html element at the top");
        let html = expect_element(top, 0);
        assert_eq!(tags(&html.children), vec!["head", "body"]);
        (
            html,
            expect_element(&html.children, 0),
            expect_element(&html.children, 1),
        )
    }

    /// The body's children: what a test's source wrote.
    fn body_children(doc: &Node) -> &[Node] {
        &skeleton(doc).2.children
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
        let div = expect_element(body_children(&root), 0);
        assert_eq!(div.tag, "div");
        let p = expect_element(&div.children, 0);
        assert_eq!(p.tag, "p");
        assert_eq!(p.children, vec![Node::text("hi")]);
    }

    #[test]
    fn void_elements_do_not_nest() {
        let root = build("<p>a<br>b<img src=\"x\">c</p>");
        let p = expect_element(body_children(&root), 0);
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
        let div = expect_element(body_children(&root), 0);
        let p = expect_element(&div.children, 0);
        assert_eq!(p.children, vec![Node::text("a")]);
    }

    #[test]
    fn unmatched_end_tags_are_dropped() {
        let root = build("<div>a</span>b</div>");
        let div = expect_element(body_children(&root), 0);
        // The two text runs merge in the surviving div.
        assert_eq!(div.children, vec![Node::text("ab")]);
    }

    #[test]
    fn p_auto_closes() {
        let root = build("<p>one<p>two");
        let children = body_children(&root);
        assert_eq!(children.len(), 2);
        assert_eq!(expect_element(children, 0).tag, "p");
        assert_eq!(expect_element(children, 1).tag, "p");
    }

    #[test]
    fn blocks_close_open_p() {
        let root = build("<p>one<div>d</div>");
        let children = body_children(&root);
        assert_eq!(children.len(), 2);
        assert_eq!(expect_element(children, 0).tag, "p");
        assert_eq!(expect_element(children, 1).tag, "div");
    }

    #[test]
    fn li_auto_closes() {
        let root = build("<ul><li>a<li>b</ul>");
        let ul = expect_element(body_children(&root), 0);
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
        let ul = expect_element(body_children(&root), 0);
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
        let outer = expect_element(body_children(&root), 0);
        assert_eq!(tags(&outer.children), vec!["li", "li"]);
        let a = expect_element(&outer.children, 0);
        assert_eq!(tags(&a.children), vec!["#text", "ul"]);
        let inner = expect_element(&a.children, 1);
        assert_eq!(tags(&inner.children), vec!["li", "li"]);
    }

    #[test]
    fn dd_dt_option_and_heading_start_tags_close_their_predecessors() {
        let root = build("<dl><dt>a<dd>b<dt>c<dd>d</dl>");
        let dl = expect_element(body_children(&root), 0);
        assert_eq!(tags(&dl.children), vec!["dt", "dd", "dt", "dd"]);

        let root = build("<select><option>a<option>b<optgroup><option>c</select>");
        let select = expect_element(body_children(&root), 0);
        assert_eq!(tags(&select.children), vec!["option", "option", "optgroup"]);
        let group = expect_element(&select.children, 2);
        assert_eq!(tags(&group.children), vec!["option"]);

        let root = build("<h1>a<h2>b</h2>");
        assert_eq!(tags(body_children(&root)), vec!["h1", "h2"]);
    }

    #[test]
    fn table_rows_and_cells_become_siblings() {
        let root = build("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = expect_element(body_children(&root), 0);
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
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["p", "table", "#text"]);
        let table = expect_element(children, 1);
        let tr = expect_element(&table.children, 0);
        let td = expect_element(&tr.children, 0);
        assert_eq!(tags(&td.children), vec!["div"]);
    }

    #[test]
    fn self_closing_flag_is_ignored_on_html_elements() {
        let root = build("<p><span/>a</p><div/>text");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["p", "div"]);
        let p = expect_element(children, 0);
        let span = expect_element(&p.children, 0);
        assert_eq!(span.children, vec![Node::text("a")]);
        let div = expect_element(children, 1);
        assert_eq!(div.children, vec![Node::text("text")]);

        let root = build(r#"<a href="x"/>link</a>"#);
        let a = expect_element(body_children(&root), 0);
        assert_eq!(a.children, vec![Node::text("link")]);
    }

    #[test]
    fn self_closing_flag_is_honoured_in_foreign_content() {
        let root = build(r#"<svg><path d="M0 0"/><circle r="1"/></svg>after"#);
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["svg", "#text"]);
        let svg = expect_element(children, 0);
        assert_eq!(tags(&svg.children), vec!["path", "circle"]);
    }

    #[test]
    fn newline_right_after_pre_is_not_content() {
        let pre = |source: &str| {
            let root = build(source);
            expect_element(body_children(&root), 0).children.clone()
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
        let a = expect_element(body_children(&root), 0);
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
        let children = body_children(&root);
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
                // 512 open elements (html and body among them) deep,
                // plus the text node under the last.
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

    #[test]
    fn the_skeleton_is_implied_and_head_elements_go_into_the_head() {
        let root = build("<title>t</title><style>p{}</style><p>x</p>");
        let (_html, head, body) = skeleton(&root);
        assert_eq!(tags(&head.children), vec!["title", "style"]);
        assert_eq!(tags(&body.children), vec!["p"]);

        // Written out in full, with attributes on the skeleton.
        let root = build(
            "<html lang=en><head><meta charset=utf-8></head><body class=x><p>y</p></body></html>",
        );
        let (html, head, body) = skeleton(&root);
        assert_eq!(html.attrs, vec![("lang".to_string(), "en".to_string())]);
        assert_eq!(tags(&head.children), vec!["meta"]);
        assert_eq!(body.attrs, vec![("class".to_string(), "x".to_string())]);
        assert_eq!(tags(&body.children), vec!["p"]);

        // Empty input has the skeleton too.
        let root = build("");
        let (_, head, body) = skeleton(&root);
        assert!(head.children.is_empty() && body.children.is_empty());

        // Text implies the body; white space before it is not content.
        let root = build("  \n  hello");
        let (_, head, body) = skeleton(&root);
        assert!(head.children.is_empty());
        assert_eq!(body.children, vec![Node::text("hello")]);
    }

    #[test]
    fn end_tags_of_the_skeleton_switch_nothing_off() {
        // Content after </body> and </html> is still body content, and a
        // second <body> or <html> start tag only adds attributes.
        let root = build("<body id=a><p>x</p></body><p>y</p></html><p>z</p><body id=b class=c>");
        let (_, _, body) = skeleton(&root);
        assert_eq!(tags(&body.children), vec!["p", "p", "p"]);
        assert_eq!(
            body.attrs,
            vec![
                ("id".to_string(), "a".to_string()),
                ("class".to_string(), "c".to_string()),
            ]
        );
        // </head> closes the head; a later head element lands in the body.
        let root = build("<head><title>t</title></head><style>a{}</style>x");
        let (_, head, body) = skeleton(&root);
        assert_eq!(tags(&head.children), vec!["title"]);
        assert_eq!(tags(&body.children), vec!["style", "#text"]);
        // A second <head> is ignored; its content goes where it falls.
        let root = build("<p>x</p><head><p>y</p>");
        let (_, head, body) = skeleton(&root);
        assert!(head.children.is_empty());
        assert_eq!(tags(&body.children), vec!["p", "p"]);
    }

    #[test]
    fn raw_text_elements_keep_their_content_as_text() {
        // A '<' inside a stylesheet or a script is text, not a tag; the
        // escapable ones decode character references, the raw ones not.
        let root = build("<style>a < b { } p > q</style><script>if (a<b && c>d) {}</script>");
        let (_, head, _) = skeleton(&root);
        let style = expect_element(&head.children, 0);
        assert_eq!(style.children, vec![Node::text("a < b { } p > q")]);
        let script = expect_element(&head.children, 1);
        assert_eq!(script.children, vec![Node::text("if (a<b && c>d) {}")]);
        let root = build("<title>a &amp; b</title><textarea>&lt;x</textarea><style>&amp;</style>");
        let (_, head, body) = skeleton(&root);
        assert_eq!(
            expect_element(&head.children, 0).children,
            vec![Node::text("a & b")]
        );
        assert_eq!(
            expect_element(&body.children, 0).children,
            vec![Node::text("<x")]
        );
        assert_eq!(
            expect_element(&body.children, 1).children,
            vec![Node::text("&amp;")]
        );
        // The end tag is found whatever its case, and an unclosed raw
        // element runs to the end of the input.
        let root = build("<STYLE>x</Style><script>y");
        let (_, head, _) = skeleton(&root);
        assert_eq!(tags(&head.children), vec!["style", "script"]);
        assert_eq!(
            expect_element(&head.children, 1).children,
            vec![Node::text("y")]
        );
        // plaintext runs to the end of the input, tags included.
        let root = build("<p>a</p><plaintext>b</p><c>");
        let (_, _, body) = skeleton(&root);
        assert_eq!(tags(&body.children), vec!["p", "plaintext"]);
        assert_eq!(
            expect_element(&body.children, 1).children,
            vec![Node::text("b</p><c>")]
        );
    }
}
