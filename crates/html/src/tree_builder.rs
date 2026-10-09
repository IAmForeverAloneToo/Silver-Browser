//! HTML tree construction (pragmatic subset), into a `dom::Document`
//! (M24.8, `docs/design/scripting.md`).
//!
//! Consumes [`Token`]s and builds the document's tree as they come, with
//! WHATWG-flavored error recovery: unclosed elements are closed at EOF,
//! unmatched end tags are dropped, and the implied end tags of the spec's
//! "in body" insertion mode are applied for the elements that matter on
//! real pages (`p`, `li`, `dd`/`dt`, `option`, the headings, table rows
//! and cells). An element is in the document from its start tag on, its
//! open ancestors with it, so a script run at its end tag sees the tree as
//! it stands there. Doctypes, comments and processing instructions are
//! accepted and dropped: the DOM has no nodes for them yet. The builder
//! never panics and never fails: any token stream produces a tree.
//!
//! Two bounds keep hostile input linear. The stack of open elements is
//! capped at [`MAX_DEPTH`] levels (Blink uses the same figure): a start
//! tag at the cap closes the innermost open element first, so a page of
//! nothing but `<b>` tags produces a tree 512 deep and a million wide,
//! not a million deep. That cap is also what keeps the recursive walks
//! over the finished tree within any thread's stack. And every scan of
//! the open stack is bounded by it, so a start or end tag costs at most
//! 512 comparisons. A node the document has no room for (`dom`'s cap on
//! live nodes) is dropped.
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
//! The builder says which tokenizer state an element's content starts
//! in (section 13.2.6.4.7's generic raw text and RCDATA element parsing,
//! `script`, `plaintext`, and `noscript` when scripting is enabled), in
//! HTML content alone. With the scripting flag, `noscript`'s content is
//! text, as the "in head" and "in body" rules have it.
//!
//! A `script` element's end tag is noted with the line its text began
//! on ([`TreeBuilder::take_script`]), for the parser to stop at (section
//! 13.2.6.4.8). A script the input ends inside is never one: the spec
//! marks it already started.
//!
//! A `<meta>` that declares a character encoding, by `charset` or by
//! `http-equiv=content-type` and `content`, is noted as it is built:
//! the first one is [`TreeBuilder::encoding_declaration`], which the
//! caller decoding the bytes acts on (HTML section 13.2.3.4, see
//! [`crate::parse_document_from_bytes`]).
//!
//! What the full tree construction algorithm has and this does not:
//! the table insertion modes (a table's cells are not wrapped in a
//! `tbody`, a stray `<td>` outside a table is an ordinary element), the
//! active formatting elements list (misnested `<b><i></b></i>` is not
//! reconstructed), foster parenting, "in head noscript", and foreign
//! content beyond a self-closing tag inside `<svg>` or `<math>` being
//! honoured (G44).

use dom::{Document, NodeId};
use encoding::Encoding;

use crate::encoding_sniffing::extract_from_meta_content;
use crate::tokenizer::{TextState, Token, content_state};

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
/// nested list — `ul`, `ol`, and `menu` and `dir` (M21.6) — or a table
/// cell is not closed by an `li` inside it.
const LI_SCOPE_STOPS: [&str; 8] = ["ul", "ol", "menu", "dir", "table", "td", "th", "caption"];

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

/// An element on the stack of open elements: its node, and its tag,
/// kept to scan the stack without the document.
#[derive(Debug, Clone)]
struct Open {
    id: NodeId,
    tag: String,
}

/// Builds a document's tree from a token stream.
#[derive(Debug)]
pub struct TreeBuilder {
    /// Stack of open elements, outermost first.
    open: Vec<Open>,
    /// A `<pre>` (or `<listing>`/`<textarea>`) was just opened: a
    /// newline that immediately follows is not content (the spec's
    /// "in body" rule for those start tags).
    skip_leading_newline: bool,
    phase: Phase,
    /// The encoding the first `<meta>` declaring one names.
    encoding_declaration: Option<Encoding>,
    /// The scripting flag (section 13.2.4.5).
    scripting: bool,
    /// The line the open `script` element's text begins on.
    script_line: u32,
    /// A `script` element just closed by its end tag, and its line.
    script: Option<(NodeId, u32)>,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        TreeBuilder::new(false)
    }
}

impl TreeBuilder {
    /// A builder for a fresh document, its scripting flag `scripting`.
    pub fn new(scripting: bool) -> Self {
        TreeBuilder {
            open: Vec::new(),
            skip_leading_newline: false,
            phase: Phase::Initial,
            encoding_declaration: None,
            scripting,
            script_line: 1,
            script: None,
        }
    }

    /// The encoding the first `<meta>` built so far declares, by its
    /// `charset` or, failing that, by `http-equiv=content-type` and its
    /// `content`; `None` when none has declared one.
    pub fn encoding_declaration(&self) -> Option<Encoding> {
        self.encoding_declaration
    }

    /// The `script` element the last token closed, by its end tag, and
    /// the line its text began on; taken once.
    pub fn take_script(&mut self) -> Option<(NodeId, u32)> {
        self.script.take()
    }

    /// Feeds one token into the builder, which builds into `document`;
    /// `line` is the tokenizer's line once the token was read. Returns
    /// the state the tokenizer is to read what follows in, when the
    /// token opened an element whose content is text.
    pub fn process(
        &mut self,
        document: &mut Document,
        token: Token,
        line: u32,
    ) -> Option<TextState> {
        let skip_newline = std::mem::take(&mut self.skip_leading_newline);
        match token {
            Token::Doctype(_) | Token::Comment(_) | Token::ProcessingInstruction { .. } => {
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
                    self.push_text(document, text);
                } else {
                    // White space between the skeleton's elements is not
                    // content; the first other character is body content.
                    let content = text.trim_start_matches(is_html_whitespace);
                    if !content.is_empty() {
                        self.ensure_body(document);
                        self.push_text(document, content);
                    }
                }
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let state = self.start_tag(document, name, attrs, self_closing);
                if state.is_some() {
                    self.script_line = line;
                }
                return state;
            }
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
                // A script's end tag: the parser stops after it.
                "script" if self.current_is(&["script"]) && !self.in_foreign_content() => {
                    if let Some(open) = self.open.pop() {
                        self.script = Some((open.id, self.script_line));
                    }
                }
                // Unmatched end tags are dropped (recovery); a matched one
                // closes everything opened after its element too.
                _ => self.close_through(&name, &[]),
            },
        }
        None
    }

    /// Places a start tag: the skeleton elements by name, head-only
    /// elements in the head while it is still open, everything else in
    /// the body, implied as needed.
    fn start_tag(
        &mut self,
        document: &mut Document,
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    ) -> Option<TextState> {
        if name == "meta" && self.encoding_declaration.is_none() {
            self.encoding_declaration = meta_encoding_declaration(&attrs);
        }
        match name.as_str() {
            "html" => {
                if self.phase == Phase::Initial {
                    self.open_skeleton(document, "html", attrs);
                    self.phase = Phase::BeforeHead;
                } else if let Some(html) = self.open.first() {
                    merge_attrs(document, html.id, attrs);
                }
                return None;
            }
            "head" => {
                match self.phase {
                    Phase::Initial => self.open_skeleton(document, "html", Vec::new()),
                    Phase::BeforeHead => {}
                    _ => return None, // a second head: ignored
                }
                self.open_skeleton(document, "head", attrs);
                self.phase = Phase::InHead;
                return None;
            }
            "body" => {
                match self.phase {
                    Phase::InBody => {
                        if let Some(body) = self.open.iter().find(|e| e.tag == "body") {
                            merge_attrs(document, body.id, attrs);
                        }
                        return None;
                    }
                    Phase::Initial => {
                        self.open_skeleton(document, "html", Vec::new());
                        self.imply_empty_head(document);
                    }
                    Phase::BeforeHead => self.imply_empty_head(document),
                    Phase::InHead => self.close_through("head", &[]),
                    Phase::AfterHead => {}
                }
                self.open_skeleton(document, "body", attrs);
                self.phase = Phase::InBody;
                return None;
            }
            _ => {}
        }
        if HEAD_ELEMENTS.contains(&name.as_str()) {
            match self.phase {
                Phase::Initial => {
                    self.open_skeleton(document, "html", Vec::new());
                    self.open_skeleton(document, "head", Vec::new());
                    self.phase = Phase::InHead;
                }
                Phase::BeforeHead => {
                    self.open_skeleton(document, "head", Vec::new());
                    self.phase = Phase::InHead;
                }
                Phase::InHead | Phase::InBody => {}
                // After the head was closed: into the body (the spec
                // reopens the head; see the module doc).
                Phase::AfterHead => self.ensure_body(document),
            }
        } else {
            self.ensure_body(document);
        }

        self.imply_end_tags_before(&name);
        let is_void = VOID_ELEMENTS.contains(&name.as_str());
        // The self-closing flag means nothing on an HTML element (the
        // spec ignores it with a parse error): `<div/>` is an open
        // `<div>`. It is honoured in foreign content, where `<path/>`
        // really is empty.
        let foreign = self.in_foreign_content();
        if is_void || (self_closing && foreign) {
            self.insert(document, &name, attrs);
            return None;
        }
        self.pop_at_the_cap();
        if SKIPS_LEADING_NEWLINE.contains(&name.as_str()) {
            self.skip_leading_newline = true;
        }
        let state = if foreign {
            None
        } else if name == "noscript" && self.scripting {
            Some(TextState::Rawtext)
        } else {
            content_state(&name)
        };
        if let Some(id) = self.insert(document, &name, attrs) {
            self.open.push(Open { id, tag: name });
        }
        state
    }

    /// Inserts an element at the current place: last under the innermost
    /// open element, or the document. `None` when the document has no
    /// room for it.
    fn insert(
        &mut self,
        document: &mut Document,
        tag: &str,
        attrs: Vec<(String, String)>,
    ) -> Option<NodeId> {
        let parent = self.open.last().map_or(document.root(), |open| open.id);
        let id = document.create_element_with(tag, attrs).ok()?;
        match document.append_new(parent, id) {
            Ok(()) => Some(id),
            Err(_) => {
                let _ = document.free(id);
                None
            }
        }
    }

    /// Opens one of the skeleton elements (`html`, `head`, `body`).
    fn open_skeleton(&mut self, document: &mut Document, tag: &str, attrs: Vec<(String, String)>) {
        self.pop_at_the_cap();
        if let Some(id) = self.insert(document, tag, attrs) {
            self.open.push(Open {
                id,
                tag: tag.to_string(),
            });
        }
    }

    /// An empty `head`, when body content arrives before any head.
    fn imply_empty_head(&mut self, document: &mut Document) {
        self.open_skeleton(document, "head", Vec::new());
        self.close_through("head", &[]);
    }

    /// Makes sure the body is open, implying whatever of the skeleton
    /// is still missing.
    fn ensure_body(&mut self, document: &mut Document) {
        match self.phase {
            Phase::Initial => {
                self.open_skeleton(document, "html", Vec::new());
                self.imply_empty_head(document);
            }
            Phase::BeforeHead => self.imply_empty_head(document),
            Phase::InHead => self.close_through("head", &[]),
            Phase::AfterHead => {}
            Phase::InBody => return,
        }
        self.open_skeleton(document, "body", Vec::new());
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
        if self.open.len() >= MAX_DEPTH {
            self.open.pop();
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
                    self.open.truncate(idx);
                }
            }
            "option" | "optgroup" => {
                if self.current_is(&["option"]) {
                    self.open.pop();
                }
                if name == "optgroup" && self.current_is(&["optgroup"]) {
                    self.open.pop();
                }
            }
            "tr" => self.close_through("tr", &["table"]),
            "td" | "th" => {
                if let Some(idx) = self.rposition_before(&CELLS, &["tr", "table"]) {
                    self.open.truncate(idx);
                }
            }
            _ => {}
        }
        if CLOSES_P.contains(&name) {
            self.close_through("p", &P_SCOPE_STOPS);
        }
        if HEADINGS.contains(&name) && self.current_is(&HEADINGS) {
            self.open.pop();
        }
    }

    /// Finishes the document, closing any elements left open. Even an
    /// empty input has the skeleton: `html`, `head`, `body`.
    pub fn finish(&mut self, document: &mut Document) {
        self.ensure_body(document);
        self.open.clear();
    }

    /// Appends text to the innermost open element (or the document),
    /// joining a text node that ends it.
    fn push_text(&mut self, document: &mut Document, text: &str) {
        if text.is_empty() {
            return;
        }
        let parent = self.open.last().map_or(document.root(), |open| open.id);
        if let Some(last) = document.last_child(parent)
            && document.is_text(last)
        {
            let _ = document.append_data(last, text);
            return;
        }
        if let Ok(node) = document.create_text(text)
            && document.append_new(parent, node).is_err()
        {
            let _ = document.free(node);
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
            self.open.truncate(idx);
        }
    }
}

/// The spec's ASCII white space: tab, line feed, form feed, carriage
/// return and space.
fn is_html_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// The encoding a `<meta>` declares (the "in head" rules for `meta`):
/// its `charset`, if that names an encoding; otherwise, with
/// `http-equiv` set to `Content-Type` in any case, the one its
/// `content` names.
fn meta_encoding_declaration(attrs: &[(String, String)]) -> Option<Encoding> {
    let attr = |name: &str| {
        attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.as_str())
    };
    if let Some(encoding) = attr("charset").and_then(Encoding::for_label) {
        return Some(encoding);
    }
    if attr("http-equiv").is_some_and(|value| value.eq_ignore_ascii_case("content-type")) {
        return attr("content").and_then(extract_from_meta_content);
    }
    None
}

/// A repeated `<html>` or `<body>` start tag adds the attributes the
/// open element does not have yet (the spec's rule), never replaces.
fn merge_attrs(document: &mut Document, element: NodeId, attrs: Vec<(String, String)>) {
    for (name, value) in attrs {
        if document.attr(element, &name).is_none() {
            let _ = document.set_attribute(element, &name, &value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parser, Stop};
    use dom::{Element, Node};

    /// `source` parsed to its end.
    fn parsed(source: &str) -> (Parser, Document) {
        let mut parser = Parser::new(false);
        parser.feed(source);
        parser.end();
        let mut document = Document::new();
        while parser.parse(&mut document, usize::MAX) != Stop::Done {}
        (parser, document)
    }

    /// The first encoding a `<meta>` in `source` declares.
    fn declared(source: &str) -> Option<&'static str> {
        parsed(source).0.encoding_declaration().map(Encoding::name)
    }

    #[test]
    fn meta_encoding_declarations() {
        assert_eq!(declared("<meta charset=koi8-r>"), Some("KOI8-R"));
        assert_eq!(declared("<p>x<meta charset=koi8-r>"), Some("KOI8-R"));
        assert_eq!(
            declared("<meta http-equiv=Content-Type content='text/html; charset=koi8-r'>"),
            Some("KOI8-R")
        );
        // Without the pragma, content declares nothing.
        assert_eq!(declared("<meta content='text/html; charset=koi8-r'>"), None);
        // A charset naming nothing gives way to the pragma.
        assert_eq!(
            declared("<meta charset=bogus http-equiv=content-type content='charset=koi8-r'>"),
            Some("KOI8-R")
        );
        // The first declaration stays.
        assert_eq!(
            declared("<meta charset=bogus><meta charset=koi8-r><meta charset=latin2>"),
            Some("KOI8-R")
        );
        // Character references are resolved before the label is read,
        // and text that only looks like a <meta> is not one.
        assert_eq!(declared("<meta charset=&#x6B;oi8-r>"), Some("KOI8-R"));
        assert_eq!(declared("<title><meta charset=koi8-r></title>"), None);
        assert_eq!(declared("<!-- <meta charset=koi8-r> -->"), None);
        assert_eq!(declared("<p>no meta</p>"), None);
    }

    /// Builds a tree from HTML source (test convenience).
    fn build(source: &str) -> Node {
        parsed(source).1.to_node()
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
        // A `menu` and a `dir` are lists too.
        for list in ["menu", "dir"] {
            let root = build(&format!("<ol><li>a<{list}><li>b<li>c</{list}><li>d</ol>"));
            let outer = expect_element(body_children(&root), 0);
            assert_eq!(tags(&outer.children), vec!["li", "li"], "{list}");
            let a = expect_element(&outer.children, 0);
            let inner = expect_element(&a.children, 1);
            assert_eq!(tags(&inner.children), vec!["li", "li"], "{list}");
        }
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
    fn an_element_in_svg_has_its_content_parsed_as_markup() {
        // Foreign content's `style` is not raw text (section 13.2.6.5).
        let root = build("<svg><style><b>x</b></style></svg>");
        let (_, _, body) = skeleton(&root);
        let svg = expect_element(&body.children, 0);
        let style = expect_element(&svg.children, 0);
        assert_eq!(tags(&style.children), vec!["b"]);
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
