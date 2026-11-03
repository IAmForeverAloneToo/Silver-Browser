//! HTML tree construction (HTML section 13.2.6), into a `dom::Document`
//! (M24.8, `docs/design/scripting.md`; D57, `docs/design/text-dom-scripts.md`).
//!
//! The builder is the standard's machine. Every insertion mode is here
//! (section 13.2.6.4), from "initial" to "after after frameset", with the
//! stack of template insertion modes, "reset the insertion mode
//! appropriately", the head and form element pointers, the frameset-ok
//! flag and the scripting flag (with it, `noscript`'s content is text;
//! without it, "in head noscript" takes what is in a head's `noscript`).
//! The list of active formatting elements keeps its markers, is
//! reconstructed before content as section 13.2.4.3 says, keeps at most
//! three equal entries after the last marker (the Noah's Ark clause), and
//! a misnested end tag runs the adoption agency algorithm (section
//! 13.2.6.4.7) with its outer loop limit of 8 and inner loop limit of 3.
//! What a table must not hold is foster parented: put before the table
//! when the current node is a table, a `tbody`, `tfoot`, `thead` or `tr`
//! ("the appropriate place for inserting a node", section 13.2.6.1), and
//! a table's text is gathered as "in table text"'s pending character
//! tokens. `select` is parsed as the standard now has it: no "in select"
//! insertion modes, its content taken "in body", with the rules there for
//! `select`, `option`, `optgroup`, `hr` and `input` start tags. An
//! element is in the document from its start tag on, its open ancestors
//! with it, so a script run at its end tag sees the tree as it stands
//! there. The builder never panics and never fails: any token stream
//! produces a tree.
//!
//! A fragment (section 13.4) is built under a container that stands for
//! the fragment parser's root `html` element; the context element's name
//! and namespace set the insertion mode, by "reset the insertion mode
//! appropriately", stand in as the adjusted current node, and a `form`
//! context sets the form element pointer (an ancestor form of the
//! context's is not known here).
//!
//! Comments, the doctype and processing instructions go in as their own
//! nodes (D58), and what goes in a `template` goes in its contents, the
//! fragment the DOM made with it. A `template` with a valid
//! `shadowrootmode`, where the builder allows declarative shadow roots
//! (gap batch H, D68), attaches a shadow root to the element it is in
//! and stands on the stack for it, out of the tree, what goes in it
//! going in the shadow root. A doctype also sets the document's
//! quirks mode ([`TreeBuilder::quirks_mode`]), which decides whether a
//! `table` closes an open `p`.
//!
//! Three bounds keep hostile input linear. No element is put deeper than
//! [`MAX_DEPTH`] elements (Blink uses the same figure): a start tag
//! whose element would go deeper closes the innermost open element first,
//! so a page of nothing but `<b>` tags produces a tree 512 deep and a
//! million wide, not a million deep. That cap is also what keeps the
//! recursive walks over the finished tree within any thread's stack, and
//! it bounds the stack of open elements, so every scan of the stack costs
//! at most 512 steps. The list of active formatting elements holds at
//! most [`MAX_DEPTH`] entries, its earliest let go beyond that, so its
//! scans are bounded the same; the adoption agency algorithm's limits
//! bound its loops; and a token is reprocessed at most a fixed number of
//! times. A node the document has no room for (`dom`'s cap on live
//! nodes) is dropped.
//!
//! The builder says which tokenizer state an element's content starts
//! in (section 13.2.6.4.7's generic raw text and RCDATA element parsing,
//! `script`, `plaintext`, and `noscript` when scripting is enabled), in
//! HTML content alone.
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
//! Foreign content is taken as section 13.2.6.5 writes it (M32,
//! `docs/design/svg.md`): an `<svg>` or `<math>` start tag inserts an
//! element of its namespace, and what follows is inserted in the
//! namespace of the adjusted current node, SVG's names and attributes
//! given back their case (`linearGradient`, `viewBox`), a self-closing
//! tag closing its element, until a breakout tag (`<p>`, `<div>`, ...)
//! closes the foreign elements; the HTML integration points
//! (`foreignObject`, `desc` and `title` in SVG, an `annotation-xml` of
//! HTML in MathML) and MathML's text integration points take HTML
//! within them. CDATA sections are read in foreign content.

use std::collections::{HashMap, HashSet};

use dom::{Document, Namespace, NodeData, NodeId, ShadowRootInit, ShadowRootMode};
use encoding::Encoding;

use crate::encoding_sniffing::extract_from_meta_content;
use crate::tokenizer::{Doctype, TextState, Token};

/// The most elements that may be open at once, and the deepest an
/// element is put.
pub const MAX_DEPTH: usize = dom::MAX_DEPTH;

/// The most entries the list of active formatting elements holds.
const MAX_FORMATTING: usize = MAX_DEPTH;

/// How many times one token may be processed, reprocessing counted: the
/// standard's chains of "reprocess the token" are a handful long.
const MAX_STEPS: u32 = 256;

/// The elements the "in head" insertion mode handles itself; all but
/// `noscript` are put in the head even when they come after it (the
/// "after head" rules) and handled by its rules in the body.
pub const HEAD_ELEMENTS: [&str; 11] = [
    "base", "basefont", "bgsound", "link", "meta", "noframes", "noscript", "script", "style",
    "template", "title",
];

/// The HTML elements of the "special" category (section 13.2.4.3).
const SPECIAL: [&str; 83] = [
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "keygen",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "search",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
    "xmp",
];

/// The HTML elements that end a search "in scope" (section 13.2.4.2).
const SCOPE: [&str; 10] = [
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "select", "template",
];

/// The formatting elements (section 13.2.4.3).
const FORMATTING: [&str; 14] = [
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

/// What "generate implied end tags" closes (section 13.2.6.3).
const IMPLIED: [&str; 10] = [
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc",
];

/// What "generate all implied end tags thoroughly" closes besides.
const IMPLIED_THOROUGHLY: [&str; 8] = [
    "caption", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
];

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// Start tags "in body" that close an open `p` and open a block.
const BLOCKS: [&str; 25] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "search",
    "section",
    "summary",
    "ul",
];

/// End tags "in body" that close their element and what is in it.
const BLOCK_ENDS: [&str; 28] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "button",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "pre",
    "search",
    "section",
    "select",
    "summary",
    "ul",
];

/// The elements a table's text and stray content are kept out of.
const TABLE_ISH: [&str; 5] = ["table", "tbody", "tfoot", "thead", "tr"];

/// The insertion modes (section 13.2.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

/// A document's mode, as its doctype sets it (section 13.2.6.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

/// The kinds of "has an element in scope" (section 13.2.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Default,
    ListItem,
    Button,
    Table,
}

/// An element on the stack of open elements: its node, its tag and
/// namespace, kept to scan the stack without the document, whether it
/// is one of foreign content's integration points (section 13.2.6.5),
/// and how deep in the tree it is, as last worked out.
#[derive(Debug, Clone)]
struct Open {
    id: NodeId,
    tag: String,
    ns: Namespace,
    /// An HTML integration point: HTML is parsed within it.
    html_point: bool,
    /// A MathML text integration point: its text and most start tags
    /// are HTML's.
    text_point: bool,
    /// It is in the "special" category, and it ends a search in scope:
    /// kept, as the stack is scanned for them on every end tag.
    special: bool,
    ends_scope: bool,
    /// How many elements its path from the root holds, itself counted.
    depth: usize,
    /// The builder's count of moves when `depth` was worked out: the
    /// adoption agency algorithm moves elements, after which it is
    /// worked out again.
    at: u64,
}

impl Open {
    /// An HTML element on the stack, its depth not known yet.
    fn html(id: NodeId, tag: impl Into<String>) -> Open {
        Open {
            id,
            tag: tag.into(),
            ns: Namespace::Html,
            html_point: false,
            text_point: false,
            special: false,
            ends_scope: false,
            depth: 0,
            at: u64::MAX,
        }
        .classified()
    }

    /// It, its kept categories worked out from its name and namespace.
    fn classified(mut self) -> Open {
        self.special = self.is_one_of(&SPECIAL) || self.bounds_scope();
        self.ends_scope = self.is_one_of(&SCOPE) || self.bounds_scope();
        self
    }

    /// Whether it is the HTML element `name`.
    fn is(&self, name: &str) -> bool {
        self.ns == Namespace::Html && self.tag == name
    }

    /// Whether it is an HTML element named one of `names`.
    fn is_one_of(&self, names: &[&str]) -> bool {
        self.ns == Namespace::Html && names.contains(&self.tag.as_str())
    }

    /// Whether it ends a scope search as the spec's "has an element in
    /// scope" lists do for foreign elements: MathML's text integration
    /// points and `annotation-xml`, SVG's `foreignObject`, `desc` and
    /// `title`.
    fn bounds_scope(&self) -> bool {
        match self.ns {
            Namespace::MathMl => {
                matches!(
                    self.tag.as_str(),
                    "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
                )
            }
            Namespace::Svg => matches!(self.tag.as_str(), "foreignObject" | "desc" | "title"),
            _ => false,
        }
    }

    /// Whether it is in the "special" category.
    fn is_special(&self) -> bool {
        self.special
    }

    /// Whether a search in `scope` ends at it.
    fn ends(&self, scope: Scope) -> bool {
        match scope {
            Scope::Table => self.is_one_of(&["html", "table", "template"]),
            Scope::Default => self.ends_scope,
            Scope::ListItem => self.ends_scope || self.is_one_of(&["ol", "ul"]),
            Scope::Button => self.ends_scope || self.is("button"),
        }
    }
}

/// An element of the list of active formatting elements, with the
/// token it was made for: its tag and attributes, and the attributes
/// sorted, for the Noah's Ark clause's comparison.
#[derive(Debug, Clone)]
struct Formatting {
    id: NodeId,
    tag: String,
    attrs: Vec<(String, String)>,
    sorted: Vec<(String, String)>,
}

/// An entry of the list of active formatting elements.
#[derive(Debug, Clone)]
enum Entry {
    Marker,
    Element(Formatting),
}

impl Entry {
    fn element(id: NodeId, tag: String, attrs: Vec<(String, String)>) -> Entry {
        let mut sorted = attrs.clone();
        sorted.sort();
        Entry::Element(Formatting {
            id,
            tag,
            attrs,
            sorted,
        })
    }

    /// The element, unless a marker.
    fn id(&self) -> Option<NodeId> {
        match self {
            Entry::Marker => None,
            Entry::Element(element) => Some(element.id),
        }
    }
}

/// A start tag token.
#[derive(Debug, Clone)]
struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    self_closing: bool,
}

impl Tag {
    /// A start tag of `name` without attributes, as the standard makes
    /// for an implied element.
    fn bare(name: &str) -> Tag {
        Tag {
            name: name.to_owned(),
            attrs: Vec::new(),
            self_closing: false,
        }
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Where a node goes (section 13.2.6.1, "the appropriate place for
/// inserting a node"): last under a parent, or under it before a child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Last(NodeId),
    Before(NodeId, NodeId),
}

impl Place {
    fn parent(self) -> NodeId {
        match self {
            Place::Last(parent) | Place::Before(parent, _) => parent,
        }
    }
}

/// Builds a document's tree from a token stream.
#[derive(Debug)]
pub struct TreeBuilder {
    mode: Mode,
    /// The original insertion mode, for "text" and "in table text".
    original: Mode,
    /// The stack of template insertion modes.
    template_modes: Vec<Mode>,
    /// Stack of open elements, outermost first.
    open: Vec<Open>,
    /// The elements on the stack, to ask in one step.
    on_stack: HashSet<NodeId>,
    /// The list of active formatting elements.
    active: Vec<Entry>,
    /// The head element pointer.
    head: Option<NodeId>,
    /// The form element pointer.
    form: Option<NodeId>,
    /// The form element pointer is a form outside the fragment: the
    /// context element is one.
    form_outside: bool,
    frameset_ok: bool,
    /// The scripting flag (section 13.2.4.5).
    scripting: bool,
    /// Foster parenting is enabled.
    foster: bool,
    /// "In table text"'s pending table character tokens.
    pending: String,
    /// The fragment parsing algorithm's context element, standing in:
    /// its id is the container's.
    context: Option<Open>,
    quirks: QuirksMode,
    /// A `<pre>`, `<listing>` or `<textarea>` was just opened: a newline
    /// that is the next token is not content.
    skip_leading_newline: bool,
    /// The encoding the first `<meta>` declaring one names.
    encoding_declaration: Option<Encoding>,
    /// The line the open `script` element's text begins on.
    script_line: u32,
    /// A `script` element just closed by its end tag, and its line.
    script: Option<(NodeId, u32)>,
    /// How deep an element may go: [`MAX_DEPTH`], or less for a
    /// fragment that goes under an element already that deep.
    max_depth: usize,
    /// How many times the adoption agency algorithm has moved elements.
    moves: u64,
    /// How many times the token in hand was processed.
    steps: u32,
    /// The `option` elements popped off the stack while the token in
    /// hand was processed, whose content may go into their `select`'s
    /// `selectedcontent`.
    popped_options: Vec<NodeId>,
    /// A `selectedcontent` was parsed: before one, no option is looked
    /// at, so that a long `select` stays linear.
    selectedcontent: bool,
    /// Whether a `template` with a `shadowrootmode` attaches a shadow
    /// root (the document's "allow declarative shadow roots", HTML
    /// section 13.2.6.4.4).
    declarative_shadow_roots: bool,
    /// The declarative templates open, each with the shadow root what
    /// goes in it goes in.
    shadow_contents: HashMap<NodeId, NodeId>,
    /// Declarative templates popped, out of the tree, to free.
    dropped: Vec<NodeId>,
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
            mode: Mode::Initial,
            original: Mode::Initial,
            template_modes: Vec::new(),
            open: Vec::new(),
            on_stack: HashSet::new(),
            active: Vec::new(),
            head: None,
            form: None,
            form_outside: false,
            frameset_ok: true,
            scripting,
            foster: false,
            pending: String::new(),
            context: None,
            quirks: QuirksMode::NoQuirks,
            skip_leading_newline: false,
            encoding_declaration: None,
            script_line: 1,
            script: None,
            max_depth: MAX_DEPTH,
            moves: 0,
            steps: 0,
            popped_options: Vec::new(),
            selectedcontent: false,
            declarative_shadow_roots: true,
            shadow_contents: HashMap::new(),
            dropped: Vec::new(),
        }
    }

    /// Whether a `template` with a `shadowrootmode` attaches a shadow
    /// root to the element it is in: a document's parser's does unless
    /// told otherwise (`DOMParser`'s, a response's), a fragment's does
    /// not unless told (`setHTMLUnsafe`'s does, `innerHTML`'s not).
    pub fn set_declarative_shadow_roots(&mut self, allow: bool) {
        self.declarative_shadow_roots = allow;
    }

    /// A builder for a fragment (HTML section 13.4) that builds into the
    /// children of `container`, a detached element standing for the
    /// fragment parser's root `html` element. The context element, of
    /// local name `context` in the namespace `ns`, sets the insertion
    /// mode ("reset the insertion mode appropriately"), stands in as the
    /// adjusted current node while the root is the only open element, so
    /// that a foreign context's fragment is foreign content, pushes "in
    /// template" for a `template`, and is the form element pointer when
    /// it is a `form`. At most `max_depth` elements, the container among
    /// them, are open at once.
    pub fn fragment(
        container: NodeId,
        context: &str,
        ns: &Namespace,
        scripting: bool,
        max_depth: usize,
    ) -> TreeBuilder {
        let mut builder = TreeBuilder {
            max_depth,
            declarative_shadow_roots: false,
            ..TreeBuilder::new(scripting)
        };
        let mut root = Open::html(container, "html");
        root.depth = 1;
        root.at = 0;
        builder.push_open(root);
        let mut stand_in = Open::html(container, context);
        stand_in.ns = ns.clone();
        if *ns != Namespace::Html {
            stand_in.html_point = integration_point(ns, context, &[]);
            stand_in.text_point = text_integration_point(ns, context);
        }
        let stand_in = stand_in.classified();
        if stand_in.is("template") {
            builder.template_modes.push(Mode::InTemplate);
        }
        builder.form_outside = stand_in.is("form");
        builder.context = Some(stand_in);
        builder.reset_mode();
        builder
    }

    /// The encoding the first `<meta>` built so far declares, by its
    /// `charset` or, failing that, by `http-equiv=content-type` and its
    /// `content`; `None` when none has declared one.
    pub fn encoding_declaration(&self) -> Option<Encoding> {
        self.encoding_declaration
    }

    /// The document's mode, as its doctype (or the lack of one) set it.
    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks
    }

    /// The `script` element the last token closed, by its end tag, and
    /// the line its text began on; taken once.
    pub fn take_script(&mut self) -> Option<(NodeId, u32)> {
        self.script.take()
    }

    /// Whether the tokenizer is to read CDATA sections (section
    /// 13.2.5.42): the adjusted current node is a foreign element.
    pub fn cdata_allowed(&self) -> bool {
        self.adjusted_current()
            .is_some_and(|open| open.ns != Namespace::Html)
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
        self.steps = 0;
        let token = match token {
            Token::Text(text) if std::mem::take(&mut self.skip_leading_newline) => {
                match text.strip_prefix('\n') {
                    Some("") => return None,
                    Some(rest) => Token::Text(rest.to_owned()),
                    None => Token::Text(text),
                }
            }
            token => {
                self.skip_leading_newline = false;
                token
            }
        };
        let state = if self.foreign_rules_apply(&token) {
            self.foreign_token(document, token, line)
        } else {
            self.html_token(document, token, line)
        };
        self.clone_popped_options(document);
        self.free_dropped(document);
        state
    }

    /// Frees the declarative templates popped, which no tree holds.
    fn free_dropped(&mut self, document: &mut Document) {
        for template in std::mem::take(&mut self.dropped) {
            let _ = document.free(template);
        }
    }

    /// Finishes the document: the end-of-file token, which implies what
    /// of the skeleton is missing (even an empty input has `html`, `head`
    /// and `body`, or a frameset), and then every element is closed.
    pub fn finish(&mut self, document: &mut Document) {
        self.skip_leading_newline = false;
        // Each step closes an element or moves the mode on toward a
        // mode that stops.
        for _ in 0..4 * MAX_DEPTH + 64 {
            if !self.end_of_file(document) {
                break;
            }
        }
        // "The end" pops every node off the stack.
        self.popped_options.extend(
            self.open
                .iter()
                .rev()
                .filter(|open| open.is("option"))
                .map(|open| open.id),
        );
        self.clone_popped_options(document);
        self.dropped
            .extend(self.shadow_contents.drain().map(|(template, _)| template));
        self.free_dropped(document);
        self.open.clear();
        self.on_stack.clear();
        self.active.clear();
        self.template_modes.clear();
        self.pending.clear();
    }

    // ---- Dispatch ----------------------------------------------------------

    /// One more processing of the token in hand; `false` once it was
    /// processed too many times, when it is dropped.
    fn step(&mut self) -> bool {
        self.steps += 1;
        self.steps <= MAX_STEPS
    }

    /// A token by the rules for HTML content, in the current insertion
    /// mode.
    fn html_token(
        &mut self,
        document: &mut Document,
        token: Token,
        line: u32,
    ) -> Option<TextState> {
        match token {
            Token::Text(text) => self.text(document, &text),
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let state = self.start(
                    document,
                    Tag {
                        name,
                        attrs,
                        self_closing,
                    },
                );
                if state.is_some() {
                    self.script_line = line;
                }
                return state;
            }
            Token::EndTag(name) => self.end(document, &name),
            Token::Comment(data) => self.comment(document, &Note::Comment(&data)),
            Token::ProcessingInstruction { target, data } => {
                self.comment(document, &Note::Instruction(&target, &data))
            }
            Token::Doctype(doctype) => self.doctype(document, &doctype),
        }
        None
    }

    /// A start tag in the current insertion mode.
    fn start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        if !self.step() {
            return None;
        }
        match self.mode {
            Mode::Initial => {
                self.no_doctype();
                self.start(document, tag)
            }
            Mode::BeforeHtml => self.before_html_start(document, tag),
            Mode::BeforeHead => self.before_head_start(document, tag),
            Mode::InHead => self.in_head_start(document, tag),
            Mode::InHeadNoscript => self.in_head_noscript_start(document, tag),
            Mode::AfterHead => self.after_head_start(document, tag),
            Mode::InBody => self.in_body_start(document, tag),
            // The tokenizer reads text in "text": no tag comes.
            Mode::Text => None,
            Mode::InTable => self.in_table_start(document, tag),
            Mode::InTableText => {
                self.flush_table_text(document);
                self.start(document, tag)
            }
            Mode::InCaption => self.in_caption_start(document, tag),
            Mode::InColumnGroup => self.in_column_group_start(document, tag),
            Mode::InTableBody => self.in_table_body_start(document, tag),
            Mode::InRow => self.in_row_start(document, tag),
            Mode::InCell => self.in_cell_start(document, tag),
            Mode::InTemplate => self.in_template_start(document, tag),
            Mode::AfterBody => {
                if tag.name == "html" {
                    return self.in_body_start(document, tag);
                }
                self.mode = Mode::InBody;
                self.start(document, tag)
            }
            Mode::InFrameset => match tag.name.as_str() {
                "html" => self.in_body_start(document, tag),
                "frameset" => {
                    self.insert_html(document, tag);
                    None
                }
                "frame" => {
                    self.insert_void(document, tag);
                    None
                }
                "noframes" => self.in_head_start(document, tag),
                _ => None,
            },
            Mode::AfterFrameset | Mode::AfterAfterFrameset => match tag.name.as_str() {
                "html" => self.in_body_start(document, tag),
                "noframes" => self.in_head_start(document, tag),
                _ => None,
            },
            Mode::AfterAfterBody => {
                if tag.name == "html" {
                    return self.in_body_start(document, tag);
                }
                self.mode = Mode::InBody;
                self.start(document, tag)
            }
        }
    }

    /// An end tag in the current insertion mode.
    fn end(&mut self, document: &mut Document, name: &str) {
        if !self.step() {
            return;
        }
        match self.mode {
            Mode::Initial => {
                self.no_doctype();
                self.end(document, name);
            }
            Mode::BeforeHtml => {
                if matches!(name, "head" | "body" | "html" | "br") {
                    self.insert_root(document, Vec::new());
                    self.end(document, name);
                }
            }
            Mode::BeforeHead => {
                if matches!(name, "head" | "body" | "html" | "br") {
                    self.insert_head(document, Tag::bare("head"));
                    self.end(document, name);
                }
            }
            Mode::InHead => self.in_head_end(document, name),
            Mode::InHeadNoscript => match name {
                "noscript" => {
                    self.pop();
                    self.mode = Mode::InHead;
                }
                "br" => {
                    self.pop();
                    self.mode = Mode::InHead;
                    self.end(document, name);
                }
                _ => {}
            },
            Mode::AfterHead => match name {
                "template" => self.in_head_end(document, name),
                "body" | "html" | "br" => {
                    self.imply_body(document);
                    self.end(document, name);
                }
                _ => {}
            },
            Mode::InBody => self.in_body_end(document, name),
            Mode::Text => self.text_end(name),
            Mode::InTable => self.in_table_end(document, name),
            Mode::InTableText => {
                self.flush_table_text(document);
                self.end(document, name);
            }
            Mode::InCaption => self.in_caption_end(document, name),
            Mode::InColumnGroup => match name {
                "colgroup" => {
                    if self.current_is("colgroup") {
                        self.pop();
                        self.mode = Mode::InTable;
                    }
                }
                "col" => {}
                "template" => self.in_head_end(document, name),
                _ => {
                    if self.current_is("colgroup") {
                        self.pop();
                        self.mode = Mode::InTable;
                        self.end(document, name);
                    }
                }
            },
            Mode::InTableBody => self.in_table_body_end(document, name),
            Mode::InRow => self.in_row_end(document, name),
            Mode::InCell => self.in_cell_end(document, name),
            Mode::InTemplate => {
                if name == "template" {
                    self.in_head_end(document, name);
                }
            }
            Mode::AfterBody => {
                if name == "html" {
                    if self.context.is_none() {
                        self.mode = Mode::AfterAfterBody;
                    }
                } else {
                    self.mode = Mode::InBody;
                    self.end(document, name);
                }
            }
            Mode::InFrameset => {
                if name == "frameset" && self.open.len() > 1 {
                    self.pop();
                    if self.context.is_none() && !self.current_is("frameset") {
                        self.mode = Mode::AfterFrameset;
                    }
                }
            }
            Mode::AfterFrameset => {
                if name == "html" {
                    self.mode = Mode::AfterAfterFrameset;
                }
            }
            Mode::AfterAfterBody => {
                self.mode = Mode::InBody;
                self.end(document, name);
            }
            Mode::AfterAfterFrameset => {}
        }
    }

    /// Characters in the current insertion mode: a run is split where
    /// the mode treats white space and other characters apart.
    fn text(&mut self, document: &mut Document, text: &str) {
        let mut rest = text;
        while !rest.is_empty() {
            if !self.step() {
                return;
            }
            match self.mode {
                Mode::Initial | Mode::BeforeHtml | Mode::BeforeHead => {
                    rest = rest.trim_start_matches(is_html_whitespace);
                    if rest.is_empty() {
                        return;
                    }
                    match self.mode {
                        Mode::Initial => self.no_doctype(),
                        Mode::BeforeHtml => self.insert_root(document, Vec::new()),
                        _ => self.insert_head(document, Tag::bare("head")),
                    }
                }
                Mode::InHead | Mode::InHeadNoscript | Mode::AfterHead => {
                    let (space, more) = split_whitespace(rest);
                    self.insert_text(document, space);
                    rest = more;
                    if rest.is_empty() {
                        return;
                    }
                    match self.mode {
                        Mode::InHead => {
                            self.pop();
                            self.mode = Mode::AfterHead;
                        }
                        Mode::InHeadNoscript => {
                            self.pop();
                            self.mode = Mode::InHead;
                        }
                        _ => {
                            self.imply_body(document);
                        }
                    }
                }
                Mode::InBody | Mode::InCaption | Mode::InCell | Mode::InTemplate => {
                    self.body_text(document, rest);
                    return;
                }
                Mode::Text => {
                    self.insert_text(document, rest);
                    return;
                }
                Mode::InTable | Mode::InTableBody | Mode::InRow => {
                    if self.open.last().is_some_and(|open| {
                        open.is_one_of(&["table", "tbody", "template", "tfoot", "thead", "tr"])
                    }) {
                        self.pending.clear();
                        self.original = self.mode;
                        self.mode = Mode::InTableText;
                        continue;
                    }
                    self.foster = true;
                    self.body_text(document, rest);
                    self.foster = false;
                    return;
                }
                Mode::InTableText => {
                    self.pending.extend(rest.chars().filter(|&c| c != '\0'));
                    return;
                }
                Mode::InColumnGroup => {
                    let (space, more) = split_whitespace(rest);
                    self.insert_text(document, space);
                    rest = more;
                    if rest.is_empty() {
                        return;
                    }
                    if !self.current_is("colgroup") {
                        let space: String =
                            rest.chars().filter(|&c| is_html_whitespace(c)).collect();
                        self.insert_text(document, &space);
                        return;
                    }
                    self.pop();
                    self.mode = Mode::InTable;
                }
                Mode::AfterBody | Mode::AfterAfterBody => {
                    let (space, more) = split_whitespace(rest);
                    self.body_text(document, space);
                    rest = more;
                    if !rest.is_empty() {
                        self.mode = Mode::InBody;
                    }
                }
                Mode::InFrameset | Mode::AfterFrameset | Mode::AfterAfterFrameset => {
                    let space: String = rest.chars().filter(|&c| is_html_whitespace(c)).collect();
                    if self.mode == Mode::AfterAfterFrameset {
                        self.body_text(document, &space);
                    } else {
                        self.insert_text(document, &space);
                    }
                    return;
                }
            }
        }
    }

    /// A comment or a processing instruction in the current insertion
    /// mode.
    fn comment(&mut self, document: &mut Document, note: &Note) {
        if !self.step() {
            return;
        }
        let place = match self.mode {
            Mode::InTableText => {
                self.flush_table_text(document);
                return self.comment(document, note);
            }
            Mode::Initial | Mode::BeforeHtml | Mode::AfterAfterBody | Mode::AfterAfterFrameset => {
                Place::Last(document.root())
            }
            Mode::AfterBody => match self.open.first() {
                Some(html) => Place::Last(html.id),
                None => Place::Last(document.root()),
            },
            _ => self.place(document, None),
        };
        match note {
            Note::Comment(data) => self.insert_comment(document, data, place),
            Note::Instruction(target, data) => {
                self.insert_processing_instruction(document, target, data, place);
            }
        }
    }

    /// A doctype in the current insertion mode: in "initial", it is
    /// appended to the document and sets its mode; elsewhere it is
    /// ignored.
    fn doctype(&mut self, document: &mut Document, doctype: &Doctype) {
        match self.mode {
            Mode::Initial => {
                self.append_doctype(document, doctype);
                self.quirks = quirks_mode_of(doctype);
                self.mode = Mode::BeforeHtml;
            }
            Mode::InTableText => {
                self.flush_table_text(document);
                self.doctype(document, doctype);
            }
            _ => {}
        }
    }

    /// The end-of-file token in the current insertion mode; `true` while
    /// it is to be processed again.
    fn end_of_file(&mut self, document: &mut Document) -> bool {
        match self.mode {
            Mode::Initial => self.no_doctype(),
            Mode::BeforeHtml => self.insert_root(document, Vec::new()),
            Mode::BeforeHead => self.insert_head(document, Tag::bare("head")),
            Mode::InHead => {
                self.pop();
                self.mode = Mode::AfterHead;
            }
            Mode::InHeadNoscript => {
                self.pop();
                self.mode = Mode::InHead;
            }
            Mode::AfterHead => {
                self.imply_body(document);
            }
            // The script the input ends in is marked already started:
            // it is never noted for the parser to run.
            Mode::Text => {
                self.pop();
                self.mode = self.original;
            }
            Mode::InTableText => self.flush_table_text(document),
            Mode::InBody
            | Mode::InTable
            | Mode::InCaption
            | Mode::InColumnGroup
            | Mode::InTableBody
            | Mode::InRow
            | Mode::InCell
            | Mode::InTemplate => {
                if self.template_modes.is_empty() || !self.template_on_stack() {
                    return false;
                }
                self.pop_until(&["template"]);
                self.clear_to_marker();
                self.template_modes.pop();
                self.reset_mode();
            }
            Mode::AfterBody
            | Mode::InFrameset
            | Mode::AfterFrameset
            | Mode::AfterAfterBody
            | Mode::AfterAfterFrameset => return false,
        }
        true
    }

    // ---- The insertion modes ----------------------------------------------

    /// "Initial"'s anything else: no doctype, so the document is in
    /// quirks mode.
    fn no_doctype(&mut self) {
        if self.context.is_none() {
            self.quirks = QuirksMode::Quirks;
        }
        self.mode = Mode::BeforeHtml;
    }

    fn before_html_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        if tag.name == "html" {
            self.insert_root(document, tag.attrs);
            return None;
        }
        self.insert_root(document, Vec::new());
        self.start(document, tag)
    }

    fn before_head_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "html" => self.in_body_start(document, tag),
            "head" => {
                self.insert_head(document, tag);
                None
            }
            _ => {
                self.insert_head(document, Tag::bare("head"));
                self.start(document, tag)
            }
        }
    }

    /// "After head"'s anything else: a body is implied, and a frameset
    /// may still take its place.
    fn imply_body(&mut self, document: &mut Document) {
        self.insert_html(document, Tag::bare("body"));
        self.frameset_ok = true;
        self.mode = Mode::InBody;
    }

    /// Inserts the head, which the head element pointer is then, and
    /// goes "in head".
    fn insert_head(&mut self, document: &mut Document, tag: Tag) {
        self.head = self.insert_html(document, tag);
        self.mode = Mode::InHead;
    }

    fn in_head_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "html" => self.in_body_start(document, tag),
            "base" | "basefont" | "bgsound" | "link" => {
                self.insert_void(document, tag);
                None
            }
            "meta" => {
                if self.encoding_declaration.is_none() {
                    self.encoding_declaration = meta_encoding_declaration(&tag.attrs);
                }
                self.insert_void(document, tag);
                None
            }
            "title" => self.raw_text(document, tag, TextState::Rcdata),
            "noscript" if self.scripting => self.raw_text(document, tag, TextState::Rawtext),
            "noframes" | "style" => self.raw_text(document, tag, TextState::Rawtext),
            "noscript" => {
                if self.insert_html(document, tag).is_some() {
                    self.mode = Mode::InHeadNoscript;
                }
                None
            }
            "script" => self.raw_text(document, tag, TextState::ScriptData),
            "template" => {
                match self.declarative_shadow(&tag) {
                    Some(init) => self.insert_declarative(document, tag, init),
                    None => {
                        self.insert_html(document, tag);
                    }
                }
                self.push_marker();
                self.frameset_ok = false;
                self.mode = Mode::InTemplate;
                if self.template_modes.len() < MAX_DEPTH {
                    self.template_modes.push(Mode::InTemplate);
                }
                None
            }
            "head" => None,
            _ => {
                self.pop();
                self.mode = Mode::AfterHead;
                self.start(document, tag)
            }
        }
    }

    fn in_head_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "head" => {
                self.pop();
                self.mode = Mode::AfterHead;
            }
            "body" | "html" | "br" => {
                self.pop();
                self.mode = Mode::AfterHead;
                self.end(document, name);
            }
            "template" => {
                if !self.template_on_stack() {
                    return;
                }
                self.generate_implied_thoroughly();
                self.pop_until(&["template"]);
                self.clear_to_marker();
                self.template_modes.pop();
                self.reset_mode();
            }
            _ => {}
        }
    }

    fn in_head_noscript_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "html" => self.in_body_start(document, tag),
            "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style" => {
                self.in_head_start(document, tag)
            }
            "head" | "noscript" => None,
            _ => {
                self.pop();
                self.mode = Mode::InHead;
                self.start(document, tag)
            }
        }
    }

    fn after_head_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "html" => self.in_body_start(document, tag),
            "body" => {
                self.insert_html(document, tag);
                self.frameset_ok = false;
                self.mode = Mode::InBody;
                None
            }
            "frameset" => {
                self.insert_html(document, tag);
                self.mode = Mode::InFrameset;
                None
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => {
                // Back into the head, for this element alone.
                let Some(head) = self.head else {
                    return self.in_head_start(document, tag);
                };
                self.push_open(Open::html(head, "head"));
                let state = self.in_head_start(document, tag);
                if let Some(at) = self.stack_position(head) {
                    self.remove_open(at);
                }
                state
            }
            "head" => None,
            _ => {
                self.imply_body(document);
                self.start(document, tag)
            }
        }
    }

    /// Characters by the "in body" rules: NUL dropped, the active
    /// formatting elements reconstructed first.
    fn body_text(&mut self, document: &mut Document, text: &str) {
        let text = if text.contains('\0') {
            std::borrow::Cow::Owned(text.replace('\0', ""))
        } else {
            std::borrow::Cow::Borrowed(text)
        };
        if text.is_empty() {
            return;
        }
        self.reconstruct(document);
        self.insert_text(document, &text);
        if text.chars().any(|c| !is_html_whitespace(c)) {
            self.frameset_ok = false;
        }
    }

    fn in_body_start(&mut self, document: &mut Document, mut tag: Tag) -> Option<TextState> {
        let name = tag.name.as_str();
        match name {
            "html" => {
                if !self.template_on_stack()
                    && let Some(root) = self.open.first()
                {
                    merge_attrs(document, root.id, tag.attrs);
                }
            }
            _ if HEAD_ELEMENTS.contains(&name) && name != "noscript" => {
                return self.in_head_start(document, tag);
            }
            "body" => {
                if self.open.len() > 1 && self.open[1].is("body") && !self.template_on_stack() {
                    self.frameset_ok = false;
                    merge_attrs(document, self.open[1].id, tag.attrs);
                }
            }
            "frameset" => {
                if self.open.len() < 2 || !self.open[1].is("body") || !self.frameset_ok {
                    return None;
                }
                let body = self.open[1].id;
                if let Some(parent) = document.parent(body) {
                    let _ = document.remove_child(parent, body);
                }
                self.truncate(1);
                self.insert_html(document, tag);
                self.mode = Mode::InFrameset;
            }
            _ if BLOCKS.contains(&name) => {
                self.close_p_in_button_scope();
                self.insert_html(document, tag);
            }
            _ if HEADINGS.contains(&name) => {
                self.close_p_in_button_scope();
                if self
                    .open
                    .last()
                    .is_some_and(|open| open.is_one_of(&HEADINGS))
                {
                    self.pop();
                }
                self.insert_html(document, tag);
            }
            "pre" | "listing" => {
                self.close_p_in_button_scope();
                self.insert_html(document, tag);
                self.skip_leading_newline = true;
                self.frameset_ok = false;
            }
            "form" => {
                let template = self.in_template_contents();
                if self.form_is_set() && !template {
                    return None;
                }
                self.close_p_in_button_scope();
                let form = self.insert_html(document, tag);
                if !template {
                    self.form = form;
                }
            }
            "li" | "dd" | "dt" => {
                self.frameset_ok = false;
                let closes: &[&str] = if name == "li" { &["li"] } else { &["dd", "dt"] };
                for at in (0..self.open.len()).rev() {
                    let node = &self.open[at];
                    if node.is_one_of(closes) {
                        let found = node.tag.clone();
                        self.generate_implied(Some(found.as_str()));
                        self.pop_until(&[found.as_str()]);
                        break;
                    }
                    if node.is_special() && !node.is_one_of(&["address", "div", "p"]) {
                        break;
                    }
                }
                self.close_p_in_button_scope();
                self.insert_html(document, tag);
            }
            "plaintext" => {
                self.close_p_in_button_scope();
                return self
                    .insert_html(document, tag)
                    .map(|_| TextState::Plaintext);
            }
            "button" => {
                if self.in_scope(&["button"], Scope::Default) {
                    self.generate_implied(None);
                    self.pop_until(&["button"]);
                }
                self.reconstruct(document);
                self.insert_html(document, tag);
                self.frameset_ok = false;
            }
            "a" => {
                if let Some(at) = self.formatting_after_marker("a")
                    && let Some(a) = self.active[at].id()
                {
                    self.adoption_agency(document, "a");
                    if let Some(at) = self.list_position(a) {
                        self.active.remove(at);
                    }
                    if let Some(at) = self.stack_position(a) {
                        self.remove_open(at);
                    }
                }
                self.reconstruct(document);
                self.insert_formatting(document, tag);
            }
            "nobr" => {
                self.reconstruct(document);
                if self.in_scope(&["nobr"], Scope::Default) {
                    if !self.adoption_agency(document, "nobr") {
                        self.any_other_end("nobr");
                    }
                    self.reconstruct(document);
                }
                self.insert_formatting(document, tag);
            }
            _ if FORMATTING.contains(&name) => {
                self.reconstruct(document);
                self.insert_formatting(document, tag);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct(document);
                self.insert_html(document, tag);
                self.push_marker();
                self.frameset_ok = false;
            }
            "table" => {
                if self.quirks != QuirksMode::Quirks {
                    self.close_p_in_button_scope();
                }
                self.insert_html(document, tag);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct(document);
                self.insert_void(document, tag);
                self.frameset_ok = false;
            }
            "input" => {
                if self.context_is("select") {
                    return None;
                }
                if self.in_scope(&["select"], Scope::Default) {
                    self.pop_until(&["select"]);
                }
                self.reconstruct(document);
                let hidden = tag
                    .attr("type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("hidden"));
                self.insert_void(document, tag);
                if !hidden {
                    self.frameset_ok = false;
                }
            }
            "param" | "source" | "track" => {
                self.insert_void(document, tag);
            }
            "hr" => {
                self.close_p_in_button_scope();
                if self.in_scope(&["select"], Scope::Default) {
                    self.generate_implied(None);
                }
                self.insert_void(document, tag);
                self.frameset_ok = false;
            }
            "image" => {
                tag.name = "img".to_owned();
                return self.in_body_start(document, tag);
            }
            "textarea" => {
                self.insert_html(document, tag)?;
                self.skip_leading_newline = true;
                self.original = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
                return Some(TextState::Rcdata);
            }
            "xmp" => {
                self.close_p_in_button_scope();
                self.reconstruct(document);
                self.frameset_ok = false;
                return self.raw_text(document, tag, TextState::Rawtext);
            }
            "iframe" => {
                self.frameset_ok = false;
                return self.raw_text(document, tag, TextState::Rawtext);
            }
            "noembed" => return self.raw_text(document, tag, TextState::Rawtext),
            "noscript" if self.scripting => {
                return self.raw_text(document, tag, TextState::Rawtext);
            }
            "select" => {
                if self.context_is("select") {
                    return None;
                }
                if self.in_scope(&["select"], Scope::Default) {
                    self.pop_until(&["select"]);
                    return None;
                }
                self.reconstruct(document);
                self.insert_html(document, tag);
                self.frameset_ok = false;
            }
            "option" | "optgroup" => {
                if self.in_scope(&["select"], Scope::Default) {
                    let except = (name == "option").then_some("optgroup");
                    self.generate_implied(except);
                } else if self.current_is("option") {
                    self.pop();
                }
                self.reconstruct(document);
                self.insert_html(document, tag);
            }
            "rb" | "rtc" | "rp" | "rt" => {
                if self.in_scope(&["ruby"], Scope::Default) {
                    let except = matches!(name, "rp" | "rt").then_some("rtc");
                    self.generate_implied(except);
                }
                self.insert_html(document, tag);
            }
            "math" | "svg" => {
                self.reconstruct(document);
                let ns = if name == "math" {
                    Namespace::MathMl
                } else {
                    Namespace::Svg
                };
                self.insert_foreign(document, ns, tag, 0);
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {}
            _ => {
                self.reconstruct(document);
                self.insert_html(document, tag);
            }
        }
        None
    }

    fn in_body_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "template" => self.in_head_end(document, name),
            "body" | "html" => {
                if !self.in_scope(&["body"], Scope::Default) {
                    return;
                }
                self.mode = Mode::AfterBody;
                if name == "html" {
                    self.end(document, name);
                }
            }
            _ if BLOCK_ENDS.contains(&name) => {
                if self.in_scope(&[name], Scope::Default) {
                    self.generate_implied(None);
                    self.pop_until(&[name]);
                }
            }
            "form" => {
                if self.in_template_contents() {
                    if self.in_scope(&["form"], Scope::Default) {
                        self.generate_implied(None);
                        self.pop_until(&["form"]);
                    }
                    return;
                }
                self.form_outside = false;
                let Some(form) = self.form.take() else {
                    return;
                };
                if !self.node_in_scope(form) {
                    return;
                }
                self.generate_implied(None);
                if let Some(at) = self.stack_position(form) {
                    self.remove_open(at);
                }
            }
            "p" => {
                if !self.in_scope(&["p"], Scope::Button) {
                    self.insert_html(document, Tag::bare("p"));
                }
                self.close_p_in_button_scope();
            }
            "li" => {
                if self.in_scope(&["li"], Scope::ListItem) {
                    self.generate_implied(Some("li"));
                    self.pop_until(&["li"]);
                }
            }
            "dd" | "dt" => {
                if self.in_scope(&[name], Scope::Default) {
                    self.generate_implied(Some(name));
                    self.pop_until(&[name]);
                }
            }
            _ if HEADINGS.contains(&name) => {
                if self.in_scope(&HEADINGS, Scope::Default) {
                    self.generate_implied(None);
                    self.pop_until(&HEADINGS);
                }
            }
            _ if FORMATTING.contains(&name) => {
                if !self.adoption_agency(document, name) {
                    self.any_other_end(name);
                }
            }
            "applet" | "marquee" | "object" => {
                if self.in_scope(&[name], Scope::Default) {
                    self.generate_implied(None);
                    self.pop_until(&[name]);
                    self.clear_to_marker();
                }
            }
            "br" => {
                self.in_body_start(document, Tag::bare("br"));
            }
            _ => self.any_other_end(name),
        }
    }

    /// "In body"'s rule for any other end tag: the innermost element of
    /// that name closes, unless a special element is open inside it.
    fn any_other_end(&mut self, name: &str) {
        for at in (1..self.open.len()).rev() {
            let node = &self.open[at];
            if node.is(name) {
                self.generate_implied(Some(name));
                self.truncate(at);
                return;
            }
            if node.is_special() {
                return;
            }
        }
    }

    /// An end tag in "text": the element closes, and a script's is noted
    /// for the parser.
    fn text_end(&mut self, name: &str) {
        let script = name == "script" && self.current_is("script");
        if let Some(open) = self.pop()
            && script
        {
            self.script = Some((open.id, self.script_line));
        }
        self.mode = self.original;
    }

    fn in_table_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "caption" => {
                self.clear_to_table_context();
                self.push_marker();
                self.insert_html(document, tag);
                self.mode = Mode::InCaption;
            }
            "colgroup" => {
                self.clear_to_table_context();
                self.insert_html(document, tag);
                self.mode = Mode::InColumnGroup;
            }
            "col" => {
                self.clear_to_table_context();
                self.insert_html(document, Tag::bare("colgroup"));
                self.mode = Mode::InColumnGroup;
                return self.start(document, tag);
            }
            "tbody" | "tfoot" | "thead" => {
                self.clear_to_table_context();
                self.insert_html(document, tag);
                self.mode = Mode::InTableBody;
            }
            "td" | "th" | "tr" => {
                self.clear_to_table_context();
                self.insert_html(document, Tag::bare("tbody"));
                self.mode = Mode::InTableBody;
                return self.start(document, tag);
            }
            "table" => {
                if self.in_scope(&["table"], Scope::Table) {
                    self.pop_until(&["table"]);
                    self.reset_mode();
                    return self.start(document, tag);
                }
            }
            "style" | "script" | "template" => return self.in_head_start(document, tag),
            "input"
                if tag
                    .attr("type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("hidden")) =>
            {
                self.insert_void(document, tag);
            }
            "form" => {
                let template = self.in_template_contents();
                if self.form_is_set() && !template {
                    return None;
                }
                let form = self.insert_void(document, tag);
                if !template {
                    self.form = form;
                }
            }
            _ => {
                self.foster = true;
                let state = self.in_body_start(document, tag);
                self.foster = false;
                return state;
            }
        }
        None
    }

    fn in_table_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "table" => {
                if self.in_scope(&["table"], Scope::Table) {
                    self.pop_until(&["table"]);
                    self.reset_mode();
                }
            }
            "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {}
            "template" => self.in_head_end(document, name),
            _ => {
                self.foster = true;
                self.in_body_end(document, name);
                self.foster = false;
            }
        }
    }

    /// "In table text"'s anything else: the pending characters go in,
    /// foster parented if any is not white space, and the mode is what
    /// it was.
    fn flush_table_text(&mut self, document: &mut Document) {
        let pending = std::mem::take(&mut self.pending);
        self.mode = self.original;
        if pending.chars().any(|c| !is_html_whitespace(c)) {
            self.foster = true;
            self.body_text(document, &pending);
            self.foster = false;
        } else {
            self.insert_text(document, &pending);
        }
    }

    fn in_caption_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr" => {
                if self.close_caption() {
                    return self.start(document, tag);
                }
                None
            }
            _ => self.in_body_start(document, tag),
        }
    }

    fn in_caption_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "caption" => {
                self.close_caption();
            }
            "table" => {
                if self.close_caption() {
                    self.end(document, name);
                }
            }
            "body" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th" | "thead"
            | "tr" => {}
            _ => self.in_body_end(document, name),
        }
    }

    /// Closes the caption, if one is in table scope, and goes back "in
    /// table"; whether it did.
    fn close_caption(&mut self) -> bool {
        if !self.in_scope(&["caption"], Scope::Table) {
            return false;
        }
        self.generate_implied(None);
        self.pop_until(&["caption"]);
        self.clear_to_marker();
        self.mode = Mode::InTable;
        true
    }

    fn in_column_group_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "html" => self.in_body_start(document, tag),
            "col" => {
                self.insert_void(document, tag);
                None
            }
            "template" => self.in_head_start(document, tag),
            _ => {
                if !self.current_is("colgroup") {
                    return None;
                }
                self.pop();
                self.mode = Mode::InTable;
                self.start(document, tag)
            }
        }
    }

    fn in_table_body_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "tr" => {
                self.clear_to_table_body_context();
                self.insert_html(document, tag);
                self.mode = Mode::InRow;
                None
            }
            "th" | "td" => {
                self.clear_to_table_body_context();
                self.insert_html(document, Tag::bare("tr"));
                self.mode = Mode::InRow;
                self.start(document, tag)
            }
            "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" => {
                if self.close_table_body() {
                    return self.start(document, tag);
                }
                None
            }
            _ => self.in_table_start(document, tag),
        }
    }

    fn in_table_body_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "tbody" | "tfoot" | "thead" => {
                if self.in_scope(&[name], Scope::Table) {
                    self.clear_to_table_body_context();
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            "table" => {
                if self.close_table_body() {
                    self.end(document, name);
                }
            }
            "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr" => {}
            _ => self.in_table_end(document, name),
        }
    }

    /// Closes the table body, if one is in table scope, and goes back
    /// "in table"; whether it did.
    fn close_table_body(&mut self) -> bool {
        if !self.in_scope(&["tbody", "thead", "tfoot"], Scope::Table) {
            return false;
        }
        self.clear_to_table_body_context();
        self.pop();
        self.mode = Mode::InTable;
        true
    }

    fn in_row_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "th" | "td" => {
                self.clear_to_row_context();
                self.insert_html(document, tag);
                self.mode = Mode::InCell;
                self.push_marker();
                None
            }
            "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr" => {
                if self.close_row() {
                    return self.start(document, tag);
                }
                None
            }
            _ => self.in_table_start(document, tag),
        }
    }

    fn in_row_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "tr" => {
                self.close_row();
            }
            "table" => {
                if self.close_row() {
                    self.end(document, name);
                }
            }
            "tbody" | "tfoot" | "thead" => {
                if self.in_scope(&[name], Scope::Table) && self.close_row() {
                    self.end(document, name);
                }
            }
            "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" => {}
            _ => self.in_table_end(document, name),
        }
    }

    /// Closes the row, if one is in table scope, and goes back "in table
    /// body"; whether it did.
    fn close_row(&mut self) -> bool {
        if !self.in_scope(&["tr"], Scope::Table) {
            return false;
        }
        self.clear_to_row_context();
        self.pop();
        self.mode = Mode::InTableBody;
        true
    }

    fn in_cell_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        match tag.name.as_str() {
            "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr" => {
                if !self.in_scope(&["td", "th"], Scope::Table) {
                    return None;
                }
                self.close_cell();
                self.start(document, tag)
            }
            _ => self.in_body_start(document, tag),
        }
    }

    fn in_cell_end(&mut self, document: &mut Document, name: &str) {
        match name {
            "td" | "th" => {
                if !self.in_scope(&[name], Scope::Table) {
                    return;
                }
                self.generate_implied(None);
                self.pop_until(&[name]);
                self.clear_to_marker();
                self.mode = Mode::InRow;
            }
            "body" | "caption" | "col" | "colgroup" | "html" => {}
            "table" | "tbody" | "tfoot" | "thead" | "tr" => {
                if self.in_scope(&[name], Scope::Table) {
                    self.close_cell();
                    self.end(document, name);
                }
            }
            _ => self.in_body_end(document, name),
        }
    }

    /// "Close the cell".
    fn close_cell(&mut self) {
        self.generate_implied(None);
        self.pop_until(&["td", "th"]);
        self.clear_to_marker();
        self.mode = Mode::InRow;
    }

    fn in_template_start(&mut self, document: &mut Document, tag: Tag) -> Option<TextState> {
        let name = tag.name.as_str();
        if HEAD_ELEMENTS.contains(&name) && name != "noscript" {
            return self.in_head_start(document, tag);
        }
        let mode = match name {
            "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
            "col" => Mode::InColumnGroup,
            "tr" => Mode::InTableBody,
            "td" | "th" => Mode::InRow,
            _ => Mode::InBody,
        };
        self.template_modes.pop();
        self.template_modes.push(mode);
        self.mode = mode;
        self.start(document, tag)
    }

    /// "Reset the insertion mode appropriately" (section 13.2.4.1).
    fn reset_mode(&mut self) {
        for at in (0..self.open.len()).rev() {
            let last = at == 0;
            let node = match &self.context {
                Some(context) if last => context,
                _ => &self.open[at],
            };
            if node.ns != Namespace::Html {
                if last {
                    break;
                }
                continue;
            }
            self.mode = match node.tag.as_str() {
                "td" | "th" if !last => Mode::InCell,
                "tr" => Mode::InRow,
                "tbody" | "thead" | "tfoot" => Mode::InTableBody,
                "caption" => Mode::InCaption,
                "colgroup" => Mode::InColumnGroup,
                "table" => Mode::InTable,
                "template" => self.template_modes.last().copied().unwrap_or(Mode::InBody),
                "head" if !last => Mode::InHead,
                "body" => Mode::InBody,
                "frameset" => Mode::InFrameset,
                "html" => {
                    if self.head.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    }
                }
                _ if last => Mode::InBody,
                _ => continue,
            };
            return;
        }
        self.mode = Mode::InBody;
    }

    // ---- Foreign content ---------------------------------------------------

    /// The adjusted current node: the fragment's context element while
    /// the root is all that is open, else the current node.
    fn adjusted_current(&self) -> Option<&Open> {
        match &self.context {
            Some(context) if self.open.len() == 1 => Some(context),
            _ => self.open.last(),
        }
    }

    /// Whether `token` is processed by the rules for foreign content
    /// (section 13.2.6, the tree construction dispatcher): the adjusted
    /// current node is a foreign element, and not an integration point
    /// the token is HTML's at.
    fn foreign_rules_apply(&self, token: &Token) -> bool {
        let Some(current) = self.adjusted_current() else {
            return false;
        };
        if current.ns == Namespace::Html {
            return false;
        }
        match token {
            Token::StartTag { name, .. } => {
                if current.text_point && name != "mglyph" && name != "malignmark" {
                    return false;
                }
                if current.ns == Namespace::MathMl
                    && current.tag == "annotation-xml"
                    && name == "svg"
                {
                    return false;
                }
                !current.html_point
            }
            Token::Text(_) => !(current.text_point || current.html_point),
            _ => true,
        }
    }

    /// A token by the rules for parsing tokens in foreign content
    /// (section 13.2.6.5).
    fn foreign_token(
        &mut self,
        document: &mut Document,
        token: Token,
        line: u32,
    ) -> Option<TextState> {
        match token {
            Token::Doctype(_) => None,
            Token::Comment(data) => {
                let place = self.place(document, None);
                self.insert_comment(document, &data, place);
                None
            }
            Token::ProcessingInstruction { target, data } => {
                let place = self.place(document, None);
                self.insert_processing_instruction(document, &target, &data, place);
                None
            }
            Token::Text(text) => {
                // A NUL becomes U+FFFD, which leaves the frameset-ok flag.
                if text.chars().any(|c| c != '\0' && !is_html_whitespace(c)) {
                    self.frameset_ok = false;
                }
                let text = text.replace('\0', "\u{fffd}");
                self.insert_text(document, &text);
                None
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                // A breakout tag is taken again by HTML's rules, whatever
                // stands in as the adjusted current node then.
                if breaks_out(&name, &attrs) {
                    self.pop_foreign();
                    return self.html_token(
                        document,
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        line,
                    );
                }
                let ns = self.adjusted_current()?.ns.clone();
                let tag = Tag {
                    name,
                    attrs,
                    self_closing,
                };
                self.insert_foreign(document, ns, tag, line);
                None
            }
            Token::EndTag(name) => {
                if name == "br" || name == "p" {
                    self.pop_foreign();
                    return self.html_token(document, Token::EndTag(name), line);
                }
                if name == "script"
                    && self
                        .open
                        .last()
                        .is_some_and(|open| open.ns == Namespace::Svg && open.tag == "script")
                {
                    if let Some(open) = self.pop() {
                        self.script = Some((open.id, self.script_line));
                    }
                    return None;
                }
                // Up the stack: the innermost element of that name
                // closes, unless an HTML element comes first, whose rules
                // then take the end tag.
                let mut at = self.open.len().checked_sub(1)?;
                loop {
                    if at == 0 {
                        return None;
                    }
                    if self.open[at].tag.eq_ignore_ascii_case(&name) {
                        self.truncate(at);
                        return None;
                    }
                    at -= 1;
                    if self.open[at].ns == Namespace::Html {
                        return self.html_token(document, Token::EndTag(name), line);
                    }
                }
            }
        }
    }

    /// Closes foreign elements until the current node is an HTML
    /// element or an integration point (a breakout tag's rule).
    fn pop_foreign(&mut self) {
        while let Some(current) = self.open.last() {
            if current.ns == Namespace::Html || current.html_point || current.text_point {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// Inserts a foreign element of the namespace `ns` (section
    /// 13.2.6.1, "insert a foreign element"): SVG's names and attributes
    /// given back their case, MathML's `definitionURL`; a self-closing
    /// one closed at once, an SVG `script` then processed as its end tag
    /// would be.
    fn insert_foreign(&mut self, document: &mut Document, ns: Namespace, tag: Tag, line: u32) {
        let name = match ns {
            Namespace::Svg => svg_tag_name(&tag.name),
            _ => tag.name.as_str(),
        };
        let attrs: Vec<(String, String)> = tag
            .attrs
            .into_iter()
            .map(|(attr, value)| {
                let attr = match ns {
                    Namespace::Svg => svg_attribute_name(&attr).to_owned(),
                    Namespace::MathMl if attr == "definitionurl" => "definitionURL".to_owned(),
                    _ => attr,
                };
                (attr, value)
            })
            .collect();
        let html_point = integration_point(&ns, name, &attrs);
        let text_point = text_integration_point(&ns, name);
        let Some((place, depth)) = self.place_for_element(document) else {
            return;
        };
        let Ok(id) = document.create_element_ns(ns.clone(), name, attrs) else {
            return;
        };
        if !put(document, place, id) {
            return;
        }
        if ns == Namespace::Svg && name == "script" {
            self.script_line = line;
            if tag.self_closing {
                self.script = Some((id, line));
                return;
            }
        }
        if !tag.self_closing {
            let open = Open {
                id,
                tag: name.to_owned(),
                ns,
                html_point,
                text_point,
                special: false,
                ends_scope: false,
                depth,
                at: self.moves,
            }
            .classified();
            self.push_open(open);
        }
    }

    // ---- Inserting ---------------------------------------------------------

    /// The appropriate place for inserting a node (section 13.2.6.1),
    /// its target the open element at `target` or the current node: last
    /// in it, or, with foster parenting enabled and a table-ish target,
    /// before the last open table (or in a template after it); in a
    /// template, in its contents.
    fn place(&self, document: &Document, target: Option<usize>) -> Place {
        match self.place_in(document, target) {
            Place::Last(parent) => Place::Last(
                self.shadow_contents
                    .get(&parent)
                    .copied()
                    .or_else(|| document.template_content(parent))
                    .unwrap_or(parent),
            ),
            before => before,
        }
    }

    fn place_in(&self, document: &Document, target: Option<usize>) -> Place {
        let Some(at) = target.or(self.open.len().checked_sub(1)) else {
            return Place::Last(document.root());
        };
        let node = &self.open[at];
        if self.foster && node.is_one_of(&TABLE_ISH) {
            let template = self.open.iter().rposition(|open| open.is("template"));
            let table = self.open.iter().rposition(|open| open.is("table"));
            if let Some(template) = template
                && table.is_none_or(|table| template > table)
            {
                return Place::Last(self.open[template].id);
            }
            let Some(table) = table else {
                return Place::Last(self.open[0].id);
            };
            let id = self.open[table].id;
            if let Some(parent) = document.parent(id) {
                return Place::Before(parent, id);
            }
            return Place::Last(self.open[table.saturating_sub(1)].id);
        }
        Place::Last(node.id)
    }

    /// Where an element is to go, and how deep it is there: at the depth
    /// cap the innermost open element is closed first; `None` when the
    /// element has no room, which only a place not under the current
    /// node can leave.
    fn place_for_element(&mut self, document: &Document) -> Option<(Place, usize)> {
        let max = self.max_depth.max(2);
        loop {
            let place = self.place(document, None);
            let parent = place.parent();
            let depth = self.depth_of_node(document, parent) + 1;
            if depth <= max {
                return Some((place, depth));
            }
            if self.open.last().map(|open| open.id) != Some(parent) || self.open.len() <= 1 {
                return None;
            }
            self.pop_at_the_cap();
        }
    }

    /// At the depth cap, closes the innermost open element to make room
    /// (see the module doc), keeping the insertion mode true to what is
    /// still open.
    fn pop_at_the_cap(&mut self) {
        let Some(open) = self.pop() else {
            return;
        };
        if open.is("template") {
            self.template_modes.pop();
        }
        if open.is_one_of(&[
            "table", "tbody", "thead", "tfoot", "tr", "td", "th", "caption", "colgroup", "template",
        ]) {
            self.reset_mode();
        }
    }

    /// Inserts an HTML element for `tag` and pushes it onto the stack.
    fn insert_html(&mut self, document: &mut Document, tag: Tag) -> Option<NodeId> {
        self.insert_element(document, tag, true)
    }

    /// What the declarative shadow root a `template` start tag asks for
    /// is made with (HTML section 13.2.6.4.4, "in head", a start tag
    /// "template"): `None` unless its `shadowrootmode` is `open` or
    /// `closed`, the builder allows declarative shadow roots, and the
    /// current node is an element of the tree, not its topmost or a
    /// fragment's context.
    fn declarative_shadow(&self, tag: &Tag) -> Option<ShadowRootInit> {
        if !self.declarative_shadow_roots || self.open.len() <= 1 {
            return None;
        }
        let mode = match tag.attr("shadowrootmode")?.to_ascii_lowercase().as_str() {
            "open" => ShadowRootMode::Open,
            "closed" => ShadowRootMode::Closed,
            _ => return None,
        };
        let manual = tag
            .attr("shadowrootslotassignment")
            .is_some_and(|value| value.eq_ignore_ascii_case("manual"));
        Some(ShadowRootInit {
            mode,
            delegates_focus: tag.attr("shadowrootdelegatesfocus").is_some(),
            slot_assignment: match manual {
                true => dom::SlotAssignmentMode::Manual,
                false => dom::SlotAssignmentMode::Named,
            },
            clonable: tag.attr("shadowrootclonable").is_some(),
            serializable: tag.attr("shadowrootserializable").is_some(),
        })
    }

    /// A declarative shadow root's `template`: made and pushed onto the
    /// stack alone, a shadow root attached to the current node with
    /// `init` and what goes in the template going in it; inserted as any
    /// template is when the current node hosts a shadow root already or
    /// may not host one.
    fn insert_declarative(&mut self, document: &mut Document, tag: Tag, init: ShadowRootInit) {
        let Some(host) = self.open.last().map(|open| open.id) else {
            return;
        };
        let Some((place, depth)) = self.place_for_element(document) else {
            return;
        };
        let reference = tag.attr("shadowrootreferencetarget").map(str::to_owned);
        let Ok(template) = document.create_element_with(&tag.name, tag.attrs) else {
            return;
        };
        let attached = match document.shadow_root(host) {
            Some(_) => None,
            None => document.attach_shadow(host, init, true).ok(),
        };
        match attached {
            Some(root) => {
                let _ = document.set_reference_target(root, reference.as_deref());
                self.shadow_contents.insert(template, root);
            }
            None if !put(document, place, template) => return,
            None => {}
        }
        let mut open = Open::html(template, "template");
        open.depth = depth;
        open.at = self.moves;
        self.push_open(open);
    }

    /// Inserts an HTML element for `tag` and pops it at once (a void
    /// element, its self-closing flag acknowledged).
    fn insert_void(&mut self, document: &mut Document, tag: Tag) -> Option<NodeId> {
        self.insert_element(document, tag, false)
    }

    fn insert_element(&mut self, document: &mut Document, tag: Tag, push: bool) -> Option<NodeId> {
        let (place, depth) = self.place_for_element(document)?;
        self.selectedcontent |= tag.name == "selectedcontent";
        let id = document.create_element_with(&tag.name, tag.attrs).ok()?;
        if !put(document, place, id) {
            return None;
        }
        if push {
            let mut open = Open::html(id, tag.name);
            open.depth = depth;
            open.at = self.moves;
            self.push_open(open);
        }
        Some(id)
    }

    /// Inserts a formatting element and pushes it onto the list of
    /// active formatting elements.
    fn insert_formatting(&mut self, document: &mut Document, tag: Tag) {
        let (name, attrs) = (tag.name.clone(), tag.attrs.clone());
        if let Some(id) = self.insert_html(document, tag) {
            self.push_formatting(id, name, attrs);
        }
    }

    /// The generic raw text and RCDATA element parsing algorithms, and a
    /// script's: the element goes in, and its content is read as text in
    /// "text".
    fn raw_text(
        &mut self,
        document: &mut Document,
        tag: Tag,
        state: TextState,
    ) -> Option<TextState> {
        self.insert_html(document, tag)?;
        self.original = self.mode;
        self.mode = Mode::Text;
        Some(state)
    }

    /// The `html` element, the document's: appended to it, and "before
    /// head" next.
    fn insert_root(&mut self, document: &mut Document, attrs: Vec<(String, String)>) {
        self.mode = Mode::BeforeHead;
        let Ok(id) = document.create_element_with("html", attrs) else {
            return;
        };
        let root = document.root();
        if !put(document, Place::Last(root), id) {
            return;
        }
        let mut open = Open::html(id, "html");
        open.depth = 1;
        open.at = self.moves;
        self.push_open(open);
    }

    /// Inserts characters at the appropriate place, joining a text node
    /// that is just before it; nothing in the document node itself.
    fn insert_text(&mut self, document: &mut Document, text: &str) {
        if text.is_empty() {
            return;
        }
        let place = self.place(document, None);
        let (parent, before) = match place {
            Place::Last(parent) => (parent, document.last_child(parent)),
            Place::Before(parent, child) => (parent, document.prev_sibling(child)),
        };
        if !matches!(
            document.data(parent),
            Some(NodeData::Element { .. } | NodeData::DocumentFragment | NodeData::ShadowRoot(_))
        ) {
            return;
        }
        if let Some(before) = before
            && document.is_text(before)
        {
            let _ = document.append_data(before, text);
            return;
        }
        if let Ok(node) = document.create_text(text) {
            put(document, place, node);
        }
    }

    /// Inserts a comment at `place` (section 13.2.6.1).
    fn insert_comment(&mut self, document: &mut Document, data: &str, place: Place) {
        if let Ok(node) = document.create_comment(data) {
            put(document, place, node);
        }
    }

    /// Inserts a processing instruction at `place`.
    fn insert_processing_instruction(
        &mut self,
        document: &mut Document,
        target: &str,
        data: &str,
        place: Place,
    ) {
        if let Ok(node) = document.create_processing_instruction(target, data) {
            put(document, place, node);
        }
    }

    /// Appends a doctype to the document ("initial"), its missing name
    /// and identifiers empty.
    fn append_doctype(&mut self, document: &mut Document, doctype: &Doctype) {
        let field = |value: &Option<String>| value.clone().unwrap_or_default();
        if let Ok(node) = document.create_doctype(
            &field(&doctype.name),
            &field(&doctype.public_id),
            &field(&doctype.system_id),
        ) {
            put(document, Place::Last(document.root()), node);
        }
    }

    // ---- The stack of open elements ----------------------------------------

    fn push_open(&mut self, open: Open) {
        self.on_stack.insert(open.id);
        self.open.push(open);
    }

    /// Pops the current node, never the root.
    fn pop(&mut self) -> Option<Open> {
        if self.open.len() <= 1 {
            return None;
        }
        let open = self.open.pop()?;
        self.on_stack.remove(&open.id);
        if open.is("option") {
            self.popped_options.push(open.id);
        }
        if !self.shadow_contents.is_empty() && self.shadow_contents.remove(&open.id).is_some() {
            self.dropped.push(open.id);
        }
        Some(open)
    }

    /// The option elements' popped steps for those popped: "maybe clone
    /// an option into selectedcontent" (the customizable `select`).
    fn clone_popped_options(&mut self, document: &mut Document) {
        let popped = std::mem::take(&mut self.popped_options);
        if !self.selectedcontent {
            return;
        }
        for option in popped {
            maybe_clone_into_selectedcontent(document, option);
        }
    }

    /// Pops elements until `len` are open, the root never.
    fn truncate(&mut self, len: usize) {
        while self.open.len() > len.max(1) {
            self.pop();
        }
    }

    /// Takes the open element at `at` (never the root) off the stack.
    fn remove_open(&mut self, at: usize) {
        if at == 0 || at >= self.open.len() {
            return;
        }
        let open = self.open.remove(at);
        self.on_stack.remove(&open.id);
    }

    fn insert_open(&mut self, at: usize, open: Open) {
        self.on_stack.insert(open.id);
        self.open.insert(at.min(self.open.len()), open);
    }

    fn replace_open(&mut self, at: usize, open: Open) {
        self.on_stack.remove(&self.open[at].id);
        self.on_stack.insert(open.id);
        self.open[at] = open;
    }

    /// Where `id` is on the stack.
    fn stack_position(&self, id: NodeId) -> Option<usize> {
        if !self.on_stack.contains(&id) {
            return None;
        }
        self.open.iter().rposition(|open| open.id == id)
    }

    /// Pops elements until one of the HTML elements `names` is popped:
    /// the innermost one open, if any is (never the root).
    fn pop_until(&mut self, names: &[&str]) {
        if let Some(at) = self.open.iter().rposition(|open| open.is_one_of(names))
            && at > 0
        {
            self.truncate(at);
        }
    }

    /// Whether the current node is the HTML element `name`.
    fn current_is(&self, name: &str) -> bool {
        self.open.last().is_some_and(|open| open.is(name))
    }

    /// Whether the fragment's context element is the HTML element `name`.
    fn context_is(&self, name: &str) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| context.is(name))
    }

    fn template_on_stack(&self) -> bool {
        self.open.iter().any(|open| open.is("template"))
    }

    /// Whether the parser is parsing a template's contents: a `template`
    /// is open, or is the fragment's context. The form element pointer
    /// is neither heeded nor set there.
    fn in_template_contents(&self) -> bool {
        self.template_on_stack() || self.context_is("template")
    }

    fn form_is_set(&self) -> bool {
        self.form.is_some() || self.form_outside
    }

    /// Whether one of the HTML elements `names` is in `scope` (section
    /// 13.2.4.2).
    fn in_scope(&self, names: &[&str], scope: Scope) -> bool {
        for open in self.open.iter().rev() {
            if open.is_one_of(names) {
                return true;
            }
            if open.ends(scope) {
                return false;
            }
        }
        false
    }

    /// Whether the element `id` is in scope.
    fn node_in_scope(&self, id: NodeId) -> bool {
        for open in self.open.iter().rev() {
            if open.id == id {
                return true;
            }
            if open.ends(Scope::Default) {
                return false;
            }
        }
        false
    }

    /// "Generate implied end tags", `except` an element of that name.
    fn generate_implied(&mut self, except: Option<&str>) {
        while let Some(current) = self.open.last() {
            if !current.is_one_of(&IMPLIED) || except.is_some_and(|name| current.is(name)) {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// "Generate all implied end tags thoroughly".
    fn generate_implied_thoroughly(&mut self) {
        while let Some(current) = self.open.last() {
            if !current.is_one_of(&IMPLIED) && !current.is_one_of(&IMPLIED_THOROUGHLY) {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// "Close a p element", when one is in button scope.
    fn close_p_in_button_scope(&mut self) {
        if self.in_scope(&["p"], Scope::Button) {
            self.generate_implied(Some("p"));
            self.pop_until(&["p"]);
        }
    }

    /// Pops elements until the current node is one of `names`, a
    /// `template` or the `html` element.
    fn clear_back_to(&mut self, names: &[&str]) {
        while let Some(current) = self.open.last() {
            if current.is_one_of(names) || current.is_one_of(&["template", "html"]) {
                break;
            }
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// "Clear the stack back to a table context".
    fn clear_to_table_context(&mut self) {
        self.clear_back_to(&["table"]);
    }

    /// "Clear the stack back to a table body context".
    fn clear_to_table_body_context(&mut self) {
        self.clear_back_to(&["tbody", "tfoot", "thead"]);
    }

    /// "Clear the stack back to a table row context".
    fn clear_to_row_context(&mut self) {
        self.clear_back_to(&["tr"]);
    }

    /// How many elements the path from the root to `node` holds, `node`
    /// counted: kept for the current node, worked out up the tree for
    /// another.
    fn depth_of_node(&mut self, document: &Document, node: NodeId) -> usize {
        let moves = self.moves;
        if self.open.first().is_some_and(|root| root.id == node) {
            return 1;
        }
        match self.open.last_mut() {
            Some(current) if current.id == node => {
                if current.at != moves {
                    current.depth = element_depth(document, node);
                    current.at = moves;
                }
                current.depth
            }
            _ => element_depth(document, node),
        }
    }

    // ---- The list of active formatting elements ----------------------------

    /// Pushes onto the list, keeping at most three equal entries after
    /// the last marker (the Noah's Ark clause) and at most
    /// [`MAX_FORMATTING`] in all.
    fn push_formatting(&mut self, id: NodeId, tag: String, attrs: Vec<(String, String)>) {
        let entry = Entry::element(id, tag, attrs);
        let Entry::Element(new) = &entry else {
            return;
        };
        let mut equal = 0;
        let mut earliest = None;
        for at in (0..self.active.len()).rev() {
            match &self.active[at] {
                Entry::Marker => break,
                Entry::Element(old) => {
                    if old.tag == new.tag && old.sorted == new.sorted {
                        equal += 1;
                        earliest = Some(at);
                    }
                }
            }
        }
        if equal >= 3
            && let Some(at) = earliest
        {
            self.active.remove(at);
        }
        self.push_entry(entry);
    }

    fn push_marker(&mut self) {
        self.push_entry(Entry::Marker);
    }

    fn push_entry(&mut self, entry: Entry) {
        if self.active.len() >= MAX_FORMATTING {
            self.active.remove(0);
        }
        self.active.push(entry);
    }

    /// "Clear the list of active formatting elements up to the last
    /// marker".
    fn clear_to_marker(&mut self) {
        while let Some(entry) = self.active.pop() {
            if matches!(entry, Entry::Marker) {
                break;
            }
        }
    }

    /// Where the element `id` is in the list.
    fn list_position(&self, id: NodeId) -> Option<usize> {
        self.active.iter().rposition(|entry| entry.id() == Some(id))
    }

    /// The last element named `name` in the list after its last marker.
    fn formatting_after_marker(&self, name: &str) -> Option<usize> {
        for at in (0..self.active.len()).rev() {
            match &self.active[at] {
                Entry::Marker => return None,
                Entry::Element(element) if element.tag == name => return Some(at),
                Entry::Element(_) => {}
            }
        }
        None
    }

    /// "Reconstruct the active formatting elements" (section 13.2.4.3):
    /// the entries after the last marker or open element are opened
    /// again, in order, each in the place of its entry.
    fn reconstruct(&mut self, document: &mut Document) {
        let closed = |entry: &Entry, on_stack: &HashSet<NodeId>| match entry {
            Entry::Marker => false,
            Entry::Element(element) => !on_stack.contains(&element.id),
        };
        let Some(last) = self.active.last() else {
            return;
        };
        if !closed(last, &self.on_stack) {
            return;
        }
        let mut first = self.active.len() - 1;
        while first > 0 && closed(&self.active[first - 1], &self.on_stack) {
            first -= 1;
        }
        for at in first..self.active.len() {
            let Entry::Element(element) = &self.active[at] else {
                return;
            };
            let (tag, attrs) = (element.tag.clone(), element.attrs.clone());
            let Some(id) = self.insert_html(
                document,
                Tag {
                    name: tag.clone(),
                    attrs: attrs.clone(),
                    self_closing: false,
                },
            ) else {
                return;
            };
            self.active[at] = Entry::element(id, tag, attrs);
        }
    }

    /// The adoption agency algorithm (section 13.2.6.4.7) for an end tag
    /// (or an `a` or `nobr` start tag) named `subject`; `false` when it
    /// is to act as "any other end tag" instead.
    fn adoption_agency(&mut self, document: &mut Document, subject: &str) -> bool {
        if let Some(current) = self.open.last()
            && current.is(subject)
            && self.list_position(current.id).is_none()
        {
            self.pop();
            return true;
        }
        for _ in 0..8 {
            let Some(at) = self.formatting_after_marker(subject) else {
                return false;
            };
            let Entry::Element(formatting) = self.active[at].clone() else {
                return false;
            };
            let Some(formatting_at) = self.stack_position(formatting.id) else {
                self.active.remove(at);
                return true;
            };
            if !self.node_in_scope(formatting.id) {
                return true;
            }
            let furthest_at =
                (formatting_at + 1..self.open.len()).find(|&i| self.open[i].is_special());
            let Some(furthest_at) = furthest_at else {
                self.truncate(formatting_at);
                if let Some(at) = self.list_position(formatting.id) {
                    self.active.remove(at);
                }
                return true;
            };
            if formatting_at == 0 {
                return true;
            }
            let common_at = formatting_at - 1;
            let furthest = self.open[furthest_at].id;
            // Where the new element goes in the list: in the formatting
            // element's place, or just after an element.
            let mut bookmark: Option<NodeId> = None;
            let mut node_at = furthest_at;
            let mut last = furthest;
            let mut inner = 0;
            loop {
                inner += 1;
                node_at -= 1;
                let node = self.open[node_at].id;
                if node == formatting.id {
                    break;
                }
                let mut entry_at = self.list_position(node);
                if inner > 3
                    && let Some(at) = entry_at
                {
                    self.active.remove(at);
                    entry_at = None;
                }
                let Some(entry_at) = entry_at else {
                    self.remove_open(node_at);
                    continue;
                };
                let Entry::Element(entry) = &self.active[entry_at] else {
                    return true;
                };
                let (tag, attrs) = (entry.tag.clone(), entry.attrs.clone());
                let Ok(clone) = document.create_element_with(&tag, attrs.clone()) else {
                    return true;
                };
                self.active[entry_at] = Entry::element(clone, tag.clone(), attrs);
                self.replace_open(node_at, Open::html(clone, tag));
                if last == furthest {
                    bookmark = Some(clone);
                }
                let _ = document.append_child(clone, last);
                last = clone;
            }
            let _ = match self.place(document, Some(common_at)) {
                Place::Last(parent) => document.append_child(parent, last),
                Place::Before(parent, child) => document.insert_before(parent, last, Some(child)),
            };
            let Ok(new) = document.create_element_with(&formatting.tag, formatting.attrs.clone())
            else {
                return true;
            };
            // The new element goes under the furthest block first, then
            // takes its other children: the same tree as the standard's
            // order, with the depth checked once for each.
            let children: Vec<NodeId> = document.children(furthest).collect();
            if document.append_child(furthest, new).is_err() {
                let _ = document.free(new);
                return true;
            }
            for child in children {
                let _ = document.append_child(new, child);
            }
            let entry = Entry::element(new, formatting.tag.clone(), formatting.attrs.clone());
            match bookmark.and_then(|after| self.list_position(after)) {
                Some(after) => {
                    self.active.insert(after + 1, entry);
                    if let Some(at) = self.list_position(formatting.id) {
                        self.active.remove(at);
                    }
                }
                None => {
                    if let Some(at) = self.list_position(formatting.id) {
                        self.active[at] = entry;
                    }
                }
            }
            if let Some(at) = self.stack_position(formatting.id) {
                self.remove_open(at);
            }
            if let Some(at) = self.stack_position(furthest) {
                self.insert_open(at + 1, Open::html(new, formatting.tag.clone()));
            }
            self.moves += 1;
        }
        true
    }
}

/// A comment or a processing instruction, which go where the same rules
/// put them.
enum Note<'a> {
    Comment(&'a str),
    Instruction(&'a str, &'a str),
}

/// The `select` whose option `option` is (HTML's "option element nearest
/// ancestor select"): none past a `datalist`, an `hr`, an `option` or a
/// second `optgroup`.
pub fn option_select(document: &Document, option: NodeId) -> Option<NodeId> {
    let mut optgroup = false;
    for ancestor in document.ancestors(option) {
        match document.tag(ancestor) {
            Some("datalist" | "hr" | "option") => return None,
            Some("optgroup") if optgroup => return None,
            Some("optgroup") => optgroup = true,
            Some("select") => return Some(ancestor),
            _ => {}
        }
    }
    None
}

/// A `select`'s enabled `selectedcontent`: its first `selectedcontent`
/// descendant, none for a `select` of many.
pub fn enabled_selectedcontent(document: &Document, select: NodeId) -> Option<NodeId> {
    if document.attr(select, "multiple").is_some() {
        return None;
    }
    document
        .subtree(select)
        .skip(1)
        .find(|&node| document.tag(node) == Some("selectedcontent"))
}

/// The option a parsed `select` has selected, as the selectedness
/// setting algorithm leaves it while the parser inserts its options:
/// the last with a `selected` attribute, else the first not disabled.
fn parsed_selection(document: &Document, select: NodeId) -> Option<NodeId> {
    let options: Vec<NodeId> = document
        .subtree(select)
        .skip(1)
        .filter(|&node| {
            document.tag(node) == Some("option") && option_select(document, node) == Some(select)
        })
        .collect();
    let disabled = |option: NodeId| {
        document.attr(option, "disabled").is_some()
            || document.parent(option).is_some_and(|parent| {
                document.tag(parent) == Some("optgroup")
                    && document.attr(parent, "disabled").is_some()
            })
    };
    options
        .iter()
        .rev()
        .find(|&&option| document.attr(option, "selected").is_some())
        .or_else(|| options.iter().find(|&&option| !disabled(option)))
        .copied()
}

/// HTML's "maybe clone an option into selectedcontent", run as the
/// parser pops `option`: when its `select` has it selected and has an
/// enabled `selectedcontent`, that element's children become copies
/// of the option's.
fn maybe_clone_into_selectedcontent(document: &mut Document, option: NodeId) {
    let Some(select) = option_select(document, option) else {
        return;
    };
    let Some(selectedcontent) = enabled_selectedcontent(document, select) else {
        return;
    };
    if parsed_selection(document, select) == Some(option) {
        clone_option_into(document, option, selectedcontent);
    }
}

/// HTML's "update a select's selectedcontent", run when what `select`
/// has selected changes: its enabled `selectedcontent`'s children made
/// copies of its selected option's, or none when it has none selected.
pub fn update_selectedcontent(document: &mut Document, select: NodeId) {
    let Some(selectedcontent) = enabled_selectedcontent(document, select) else {
        return;
    };
    match dom::forms::selected_option(document, select) {
        Some(option) => clone_option_into(document, option, selectedcontent),
        None => {
            while let Some(child) = document.first_child(selectedcontent) {
                if document.remove_child(selectedcontent, child).is_err() {
                    return;
                }
            }
        }
    }
}

/// [`update_selectedcontent`] for every `select` under `root`, as a
/// form's reset needs.
pub fn update_selectedcontents_under(document: &mut Document, root: NodeId) {
    let selects: Vec<NodeId> = document
        .subtree(root)
        .filter(|&node| document.tag(node) == Some("select"))
        .collect();
    for select in selects {
        update_selectedcontent(document, select);
    }
}

/// HTML's "clone an option into a selectedcontent": `selectedcontent`'s
/// children replaced by deep copies of `option`'s.
pub fn clone_option_into(document: &mut Document, option: NodeId, selectedcontent: NodeId) {
    if document.subtree(option).any(|node| node == selectedcontent)
        || document.subtree(selectedcontent).any(|node| node == option)
    {
        return;
    }
    while let Some(child) = document.first_child(selectedcontent) {
        if document.remove_child(selectedcontent, child).is_err() {
            return;
        }
    }
    let children: Vec<NodeId> = document.children(option).collect();
    for child in children {
        let Some(copy) = deep_copy(document, child) else {
            return;
        };
        if document.append_child(selectedcontent, copy).is_err() {
            let _ = document.free(copy);
            return;
        }
    }
}

/// A detached deep copy of `node` (its descendants' kinds the parser
/// makes: elements, text and comments); none when the document is full.
fn deep_copy(document: &mut Document, node: NodeId) -> Option<NodeId> {
    let copy_one = |document: &mut Document, from: NodeId| match document.data(from)?.clone() {
        NodeData::Element { tag, attrs, ns } => document.create_element_ns(ns, &tag, attrs).ok(),
        NodeData::Text(text) => document.create_text(&text).ok(),
        NodeData::Comment(text) => document.create_comment(&text).ok(),
        _ => None,
    };
    let top = copy_one(document, node)?;
    // Each pair: a node whose children are still to copy, and its copy.
    let mut pending = vec![(node, top)];
    while let Some((from, to)) = pending.pop() {
        let children: Vec<NodeId> = document.children(from).collect();
        for child in children {
            let Some(copy) = copy_one(document, child) else {
                continue;
            };
            if document.append_child(to, copy).is_err() {
                let _ = document.free(copy);
                continue;
            }
            pending.push((child, copy));
        }
    }
    Some(top)
}

/// Puts the new node `node` at `place`, freeing it when the document
/// will not take it; whether it went in.
fn put(document: &mut Document, place: Place, node: NodeId) -> bool {
    let done = match place {
        Place::Last(parent) => document.append_new(parent, node),
        Place::Before(parent, child) => document.insert_before(parent, node, Some(child)),
    };
    if done.is_err() {
        let _ = document.free(node);
        return false;
    }
    true
}

/// How many elements the path from the root to `node` holds, `node`
/// counted, a template's contents counted under the template and a
/// shadow tree under its host.
fn element_depth(document: &Document, node: NodeId) -> usize {
    let mut depth = 0;
    let mut at = Some(node);
    while let Some(n) = at {
        depth += usize::from(document.is_element(n));
        at = document
            .parent(n)
            .or_else(|| document.template_host(n))
            .or_else(|| document.shadow_host(n));
    }
    depth
}

/// The spec's ASCII white space: tab, line feed, form feed, carriage
/// return and space.
fn is_html_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// `text`'s leading white space, and the rest.
fn split_whitespace(text: &str) -> (&str, &str) {
    let rest = text.trim_start_matches(is_html_whitespace);
    text.split_at(text.len() - rest.len())
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

/// The mode a doctype sets the document in ("initial", section
/// 13.2.6.4.1), its identifiers compared in any ASCII case.
fn quirks_mode_of(doctype: &Doctype) -> QuirksMode {
    const QUIRKY_PREFIXES: [&str; 55] = [
        "+//silmaril//dtd html pro v0r11 19970101//",
        "-//as//dtd html 3.0 aswedit + extensions//",
        "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
        "-//ietf//dtd html 2.0 level 1//",
        "-//ietf//dtd html 2.0 level 2//",
        "-//ietf//dtd html 2.0 strict level 1//",
        "-//ietf//dtd html 2.0 strict level 2//",
        "-//ietf//dtd html 2.0 strict//",
        "-//ietf//dtd html 2.0//",
        "-//ietf//dtd html 2.1e//",
        "-//ietf//dtd html 3.0//",
        "-//ietf//dtd html 3.2 final//",
        "-//ietf//dtd html 3.2//",
        "-//ietf//dtd html 3//",
        "-//ietf//dtd html level 0//",
        "-//ietf//dtd html level 1//",
        "-//ietf//dtd html level 2//",
        "-//ietf//dtd html level 3//",
        "-//ietf//dtd html strict level 0//",
        "-//ietf//dtd html strict level 1//",
        "-//ietf//dtd html strict level 2//",
        "-//ietf//dtd html strict level 3//",
        "-//ietf//dtd html strict//",
        "-//ietf//dtd html//",
        "-//metrius//dtd metrius presentational//",
        "-//microsoft//dtd internet explorer 2.0 html strict//",
        "-//microsoft//dtd internet explorer 2.0 html//",
        "-//microsoft//dtd internet explorer 2.0 tables//",
        "-//microsoft//dtd internet explorer 3.0 html strict//",
        "-//microsoft//dtd internet explorer 3.0 html//",
        "-//microsoft//dtd internet explorer 3.0 tables//",
        "-//netscape comm. corp.//dtd html//",
        "-//netscape comm. corp.//dtd strict html//",
        "-//o'reilly and associates//dtd html 2.0//",
        "-//o'reilly and associates//dtd html extended 1.0//",
        "-//o'reilly and associates//dtd html extended relaxed 1.0//",
        "-//sq//dtd html 2.0 hotmetal + extensions//",
        "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
        "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
        "-//spyglass//dtd html 2.0 extended//",
        "-//sun microsystems corp.//dtd hotjava html//",
        "-//sun microsystems corp.//dtd hotjava strict html//",
        "-//w3c//dtd html 3 1995-03-24//",
        "-//w3c//dtd html 3.2 draft//",
        "-//w3c//dtd html 3.2 final//",
        "-//w3c//dtd html 3.2//",
        "-//w3c//dtd html 3.2s draft//",
        "-//w3c//dtd html 4.0 frameset//",
        "-//w3c//dtd html 4.0 transitional//",
        "-//w3c//dtd html experimental 19960712//",
        "-//w3c//dtd html experimental 970421//",
        "-//w3c//dtd w3 html//",
        "-//w3o//dtd w3 html 3.0//",
        "-//webtechs//dtd mozilla html 2.0//",
        "-//webtechs//dtd mozilla html//",
    ];
    let public = doctype.public_id.as_deref().map(str::to_ascii_lowercase);
    let system = doctype.system_id.as_deref().map(str::to_ascii_lowercase);
    let public_is = |id: &str| public.as_deref() == Some(id);
    let public_starts = |prefix: &str| public.as_deref().is_some_and(|p| p.starts_with(prefix));
    let html4 = public_starts("-//w3c//dtd html 4.01 frameset//")
        || public_starts("-//w3c//dtd html 4.01 transitional//");
    if doctype.force_quirks
        || doctype.name.as_deref() != Some("html")
        || public_is("-//w3o//dtd w3 html strict 3.0//en//")
        || public_is("-/w3c/dtd html 4.0 transitional/en")
        || public_is("html")
        || system.as_deref() == Some("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
        || QUIRKY_PREFIXES.iter().any(|prefix| public_starts(prefix))
        || (system.is_none() && html4)
    {
        return QuirksMode::Quirks;
    }
    if public_starts("-//w3c//dtd xhtml 1.0 frameset//")
        || public_starts("-//w3c//dtd xhtml 1.0 transitional//")
        || (system.is_some() && html4)
    {
        return QuirksMode::LimitedQuirks;
    }
    QuirksMode::NoQuirks
}

/// Whether a start tag in foreign content breaks out of it (section
/// 13.2.6.5): one of HTML's elements that never appear in SVG or MathML,
/// or `font` with a presentational attribute.
fn breaks_out(name: &str, attrs: &[(String, String)]) -> bool {
    const BREAKOUT: [&str; 44] = [
        "b",
        "big",
        "blockquote",
        "body",
        "br",
        "center",
        "code",
        "dd",
        "div",
        "dl",
        "dt",
        "em",
        "embed",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "head",
        "hr",
        "i",
        "img",
        "li",
        "listing",
        "menu",
        "meta",
        "nobr",
        "ol",
        "p",
        "pre",
        "ruby",
        "s",
        "small",
        "span",
        "strong",
        "strike",
        "sub",
        "sup",
        "table",
        "tt",
        "u",
        "ul",
        "var",
    ];
    BREAKOUT.contains(&name)
        || (name == "font"
            && attrs
                .iter()
                .any(|(attr, _)| matches!(attr.as_str(), "color" | "face" | "size")))
}

/// Whether an element of `ns` named `tag` with `attrs` is an HTML
/// integration point (section 13.2.6.5): an `annotation-xml` whose
/// `encoding` is HTML's or XHTML's, an SVG `foreignObject`, `desc` or
/// `title`.
fn integration_point(ns: &Namespace, tag: &str, attrs: &[(String, String)]) -> bool {
    match ns {
        Namespace::MathMl => {
            tag == "annotation-xml"
                && attrs.iter().any(|(attr, value)| {
                    attr == "encoding"
                        && (value.eq_ignore_ascii_case("text/html")
                            || value.eq_ignore_ascii_case("application/xhtml+xml"))
                })
        }
        Namespace::Svg => matches!(tag, "foreignObject" | "desc" | "title"),
        _ => false,
    }
}

/// Whether an element of `ns` named `tag` is a MathML text integration
/// point: `mi`, `mo`, `mn`, `ms` or `mtext`.
fn text_integration_point(ns: &Namespace, tag: &str) -> bool {
    *ns == Namespace::MathMl && matches!(tag, "mi" | "mo" | "mn" | "ms" | "mtext")
}

/// An SVG element's name in its case, from the lowercase one the
/// tokenizer gives (section 13.2.6.5's table).
fn svg_tag_name(name: &str) -> &str {
    const NAMES: [(&str, &str); 37] = [
        ("altglyph", "altGlyph"),
        ("altglyphdef", "altGlyphDef"),
        ("altglyphitem", "altGlyphItem"),
        ("animatecolor", "animateColor"),
        ("animatemotion", "animateMotion"),
        ("animatetransform", "animateTransform"),
        ("clippath", "clipPath"),
        ("feblend", "feBlend"),
        ("fecolormatrix", "feColorMatrix"),
        ("fecomponenttransfer", "feComponentTransfer"),
        ("fecomposite", "feComposite"),
        ("feconvolvematrix", "feConvolveMatrix"),
        ("fediffuselighting", "feDiffuseLighting"),
        ("fedisplacementmap", "feDisplacementMap"),
        ("fedistantlight", "feDistantLight"),
        ("fedropshadow", "feDropShadow"),
        ("feflood", "feFlood"),
        ("fefunca", "feFuncA"),
        ("fefuncb", "feFuncB"),
        ("fefuncg", "feFuncG"),
        ("fefuncr", "feFuncR"),
        ("fegaussianblur", "feGaussianBlur"),
        ("feimage", "feImage"),
        ("femerge", "feMerge"),
        ("femergenode", "feMergeNode"),
        ("femorphology", "feMorphology"),
        ("feoffset", "feOffset"),
        ("fepointlight", "fePointLight"),
        ("fespecularlighting", "feSpecularLighting"),
        ("fespotlight", "feSpotLight"),
        ("fetile", "feTile"),
        ("feturbulence", "feTurbulence"),
        ("foreignobject", "foreignObject"),
        ("glyphref", "glyphRef"),
        ("lineargradient", "linearGradient"),
        ("radialgradient", "radialGradient"),
        ("textpath", "textPath"),
    ];
    NAMES
        .iter()
        .find(|(lower, _)| *lower == name)
        .map_or(name, |(_, adjusted)| adjusted)
}

/// An SVG attribute's name in its case (section 13.2.6.1, "adjust SVG
/// attributes").
pub fn svg_attribute_name(name: &str) -> &str {
    const NAMES: [(&str, &str); 58] = [
        ("attributename", "attributeName"),
        ("attributetype", "attributeType"),
        ("basefrequency", "baseFrequency"),
        ("baseprofile", "baseProfile"),
        ("calcmode", "calcMode"),
        ("clippathunits", "clipPathUnits"),
        ("diffuseconstant", "diffuseConstant"),
        ("edgemode", "edgeMode"),
        ("filterunits", "filterUnits"),
        ("glyphref", "glyphRef"),
        ("gradienttransform", "gradientTransform"),
        ("gradientunits", "gradientUnits"),
        ("kernelmatrix", "kernelMatrix"),
        ("kernelunitlength", "kernelUnitLength"),
        ("keypoints", "keyPoints"),
        ("keysplines", "keySplines"),
        ("keytimes", "keyTimes"),
        ("lengthadjust", "lengthAdjust"),
        ("limitingconeangle", "limitingConeAngle"),
        ("markerheight", "markerHeight"),
        ("markerunits", "markerUnits"),
        ("markerwidth", "markerWidth"),
        ("maskcontentunits", "maskContentUnits"),
        ("maskunits", "maskUnits"),
        ("numoctaves", "numOctaves"),
        ("pathlength", "pathLength"),
        ("patterncontentunits", "patternContentUnits"),
        ("patterntransform", "patternTransform"),
        ("patternunits", "patternUnits"),
        ("pointsatx", "pointsAtX"),
        ("pointsaty", "pointsAtY"),
        ("pointsatz", "pointsAtZ"),
        ("preservealpha", "preserveAlpha"),
        ("preserveaspectratio", "preserveAspectRatio"),
        ("primitiveunits", "primitiveUnits"),
        ("refx", "refX"),
        ("refy", "refY"),
        ("repeatcount", "repeatCount"),
        ("repeatdur", "repeatDur"),
        ("requiredextensions", "requiredExtensions"),
        ("requiredfeatures", "requiredFeatures"),
        ("specularconstant", "specularConstant"),
        ("specularexponent", "specularExponent"),
        ("spreadmethod", "spreadMethod"),
        ("startoffset", "startOffset"),
        ("stddeviation", "stdDeviation"),
        ("stitchtiles", "stitchTiles"),
        ("surfacescale", "surfaceScale"),
        ("systemlanguage", "systemLanguage"),
        ("tablevalues", "tableValues"),
        ("targetx", "targetX"),
        ("targety", "targetY"),
        ("textlength", "textLength"),
        ("viewbox", "viewBox"),
        ("viewtarget", "viewTarget"),
        ("xchannelselector", "xChannelSelector"),
        ("ychannelselector", "yChannelSelector"),
        ("zoomandpan", "zoomAndPan"),
    ];
    NAMES
        .iter()
        .find(|(lower, _)| *lower == name)
        .map_or(name, |(_, adjusted)| adjusted)
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
    fn a_popped_option_selected_is_cloned_into_its_selects_selectedcontent() {
        let selected = |source: &str| {
            let (_, document) = parsed(source);
            let content = document
                .subtree(document.root())
                .find(|&n| document.tag(n) == Some("selectedcontent"))
                .expect("a selectedcontent");
            crate::serialize_children(&document, content, true)
        };
        assert_eq!(
            selected(
                "<select><button><selectedcontent></button><option>a<b>c</b></option><option>d</select>"
            ),
            "a<b>c</b>"
        );
        // The last with `selected` wins; a disabled first is passed over.
        assert_eq!(
            selected(
                "<select><button><selectedcontent></button><option selected>a<option selected>b<option>c</select>"
            ),
            "b"
        );
        assert_eq!(
            selected(
                "<select><button><selectedcontent></button><option disabled>a<option>b</select>"
            ),
            "b"
        );
        // A select of many has none, nor does an option past a datalist.
        assert_eq!(
            selected("<select multiple><button><selectedcontent></button><option>a</select>"),
            ""
        );
        assert_eq!(
            selected(
                "<select><button><selectedcontent></button><datalist><option>a</datalist></select>"
            ),
            ""
        );
    }

    #[test]
    fn a_selectedcontent_follows_the_selection_made_later() {
        let (_, mut document) =
            parsed("<select><button><selectedcontent></button><option>a<option>b<i>c</i></select>");
        let find = |document: &Document, tag: &str| -> Vec<NodeId> {
            document
                .subtree(document.root())
                .filter(|&n| document.tag(n) == Some(tag))
                .collect()
        };
        let select = find(&document, "select")[0];
        let content = find(&document, "selectedcontent")[0];
        let options = find(&document, "option");
        let shown = |document: &Document| crate::serialize_children(document, content, true);
        assert_eq!(shown(&document), "a");
        dom::forms::set_selectedness(&mut document, options[1], true, true).unwrap();
        update_selectedcontent(&mut document, select);
        assert_eq!(shown(&document), "b<i>c</i>");
        // None selected: none shown.
        dom::forms::set_selectedness(&mut document, options[1], false, true).unwrap();
        update_selectedcontent(&mut document, select);
        assert_eq!(shown(&document), "");
        // Every select under a node, as a form's reset brings them.
        dom::forms::set_selectedness(&mut document, options[0], true, true).unwrap();
        let root = document.root();
        update_selectedcontents_under(&mut document, root);
        assert_eq!(shown(&document), "a");
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
        // The rows go in an implied `tbody` ("in table").
        let root = build("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = expect_element(body_children(&root), 0);
        assert_eq!(tags(&table.children), vec!["tbody"]);
        let tbody = expect_element(&table.children, 0);
        assert_eq!(tags(&tbody.children), vec!["tr", "tr"]);
        let first = expect_element(&tbody.children, 0);
        assert_eq!(tags(&first.children), vec!["td", "td"]);
        assert_eq!(
            expect_element(&first.children, 1).children,
            vec![Node::text("b")]
        );
    }

    #[test]
    fn a_block_inside_a_table_cell_does_not_close_a_p_outside_the_table() {
        let root = build("<!DOCTYPE html><p>x<table><tr><td><div>y</div></table>z</p>");
        // The `p` is closed by the `table` start tag itself (a block, in
        // a document not in quirks mode), but the `div` inside the cell
        // does not reach out past it; the `</p>` with no `p` open makes
        // an empty one ("in body").
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["p", "table", "#text", "p"]);
        let table = expect_element(children, 1);
        let tbody = expect_element(&table.children, 0);
        let tr = expect_element(&tbody.children, 0);
        let td = expect_element(&tr.children, 0);
        assert_eq!(tags(&td.children), vec!["div"]);
        // Without a doctype the document is in quirks mode, where the
        // table goes in the `p`.
        let root = build("<p>x<table></table>z</p>");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["p"]);
        let p = expect_element(children, 0);
        assert_eq!(tags(&p.children), vec!["#text", "table", "#text"]);
    }

    #[test]
    fn misnested_formatting_elements_are_adopted_and_reopened() {
        // `</b>` with an `i` open inside it: the `i` is reopened after
        // it ("reconstruct the active formatting elements").
        let root = build("<p><b>1<i>2</b>3</i>4</p>");
        let p = expect_element(body_children(&root), 0);
        assert_eq!(tags(&p.children), vec!["b", "i", "#text"]);
        let b = expect_element(&p.children, 0);
        assert_eq!(tags(&b.children), vec!["#text", "i"]);
        let i = expect_element(&p.children, 1);
        assert_eq!(i.children, vec![Node::text("3")]);
        // A block inside a formatting element its end tag misses: the
        // adoption agency algorithm moves the block out and the
        // formatting element into it.
        let root = build("<a href=x>1<div>2</a>3</div>");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["a", "div"]);
        let div = expect_element(children, 1);
        assert_eq!(tags(&div.children), vec!["a", "#text"]);
        let inner = expect_element(&div.children, 0);
        assert_eq!(inner.attrs, vec![("href".to_owned(), "x".to_owned())]);
        assert_eq!(inner.children, vec![Node::text("2")]);
    }

    #[test]
    fn what_a_table_cannot_hold_is_foster_parented() {
        let root = build("<table>a<tr><td>b</td>c</tr><div>d</div></table>");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["#text", "div", "table"]);
        assert_eq!(children[0], Node::text("ac"));
        // White space alone stays in the table; a stray cell outside a
        // table is no element.
        let root = build("<table> <tr> </tr></table><td>x");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["table", "#text"]);
        let table = expect_element(children, 0);
        assert_eq!(tags(&table.children), vec!["#text", "tbody"]);
    }

    #[test]
    fn noscript_in_the_head_and_framesets_follow_their_modes() {
        // Without scripting, a head's `noscript` takes head content and
        // is closed by anything else ("in head noscript").
        let root = build("<head><noscript><link><p>x");
        let (_, head, body) = skeleton(&root);
        assert_eq!(tags(&head.children), vec!["noscript"]);
        let noscript = expect_element(&head.children, 0);
        assert_eq!(tags(&noscript.children), vec!["link"]);
        assert_eq!(tags(&body.children), vec!["p"]);
        // A frameset document has no body.
        let root = build("<frameset><frame></frameset>");
        let html = expect_element(children_of(&root), 0);
        assert_eq!(tags(&html.children), vec!["head", "frameset"]);
    }

    #[test]
    fn a_page_of_distinct_formatting_elements_stays_bounded() {
        // Every `<b id=N>` is a distinct entry of the list of active
        // formatting elements, each reopened in every `p`: the list's
        // bound keeps the work per token bounded.
        let mut source = String::from("<div>");
        for n in 0..2_000 {
            source.push_str(&format!("<b id={n}>"));
        }
        source.push_str("</div>");
        source.push_str(&"<p>x</p>".repeat(50));
        let (_, document) = parsed(&source);
        document.validate().unwrap();
        let mut builder = TreeBuilder::new(false);
        let mut document = Document::new();
        let mut tokenizer = crate::Tokenizer::new(&source);
        while let Some(token) = tokenizer.next_token() {
            if let Some(state) = builder.process(&mut document, token, 1) {
                tokenizer.set_state(state);
            }
            assert!(builder.active.len() <= MAX_FORMATTING);
            assert!(builder.open.len() <= MAX_DEPTH);
        }
        builder.finish(&mut document);
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
    fn svg_elements_are_svgs_their_names_and_attributes_in_their_case() {
        let (_, document) = parsed(
            "<svg viewbox='0 0 1 1' PreserveAspectRatio=none xlink:href=a><lineargradient              gradientunits=x><stop/></lineargradient><clippath/></svg><math definitionurl=u><mi/></math>",
        );
        let elements: Vec<_> = document
            .subtree(document.root())
            .filter_map(|node| document.element_name(node))
            .map(|(ns, tag)| format!("{ns:?} {tag}"))
            .collect();
        assert_eq!(
            elements,
            [
                "Html html",
                "Html head",
                "Html body",
                "Svg svg",
                "Svg linearGradient",
                "Svg stop",
                "Svg clipPath",
                "MathMl math",
                "MathMl mi"
            ]
        );
        let svg = document
            .subtree(document.root())
            .find(|&node| document.svg_tag(node) == Some("svg"))
            .unwrap();
        let names: Vec<_> = document
            .attrs(svg)
            .unwrap()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["viewBox", "preserveAspectRatio", "xlink:href"]);
        let gradient = document.first_child(svg).unwrap();
        assert_eq!(document.attr(gradient, "gradientUnits"), Some("x"));
        let math = document
            .subtree(document.root())
            .find(|&node| document.local_name(node) == Some("math"))
            .unwrap();
        assert_eq!(document.attr(math, "definitionURL"), Some("u"));
    }

    #[test]
    fn cdata_is_text_in_svg_and_a_bogus_comment_in_html() {
        let (_, document) = parsed("<svg><style><![CDATA[a<b]]></style></svg><p><![CDATA[c]]>d");
        let texts: Vec<_> = document
            .subtree(document.root())
            .filter_map(|node| document.text(node))
            .collect();
        assert_eq!(texts, ["a<b", "d"]);
    }

    #[test]
    fn an_integration_point_takes_html_and_a_breakout_tag_leaves_svg() {
        let (_, document) = parsed(
            "<svg><foreignObject><div>x</div></foreignObject><desc><b>y</b></desc>             <g><font size=2>z</font></g></svg>",
        );
        let named: Vec<_> = document
            .subtree(document.root())
            .filter_map(|node| document.element_name(node))
            .map(|(ns, tag)| format!("{ns:?} {tag}"))
            .collect();
        assert_eq!(
            named,
            [
                "Html html",
                "Html head",
                "Html body",
                "Svg svg",
                "Svg foreignObject",
                "Html div",
                "Svg desc",
                "Html b",
                "Svg g",
                "Html font"
            ]
        );
        // A `font` without a presentational attribute stays SVG's, and a
        // MathML text integration point takes HTML.
        let (_, document) = parsed("<svg><font>a</font></svg><math><mi><b>c</b></mi></math>");
        let named: Vec<_> = document
            .subtree(document.root())
            .filter_map(|node| document.element_name(node))
            .map(|(ns, tag)| format!("{ns:?} {tag}"))
            .collect();
        assert_eq!(
            &named[3..],
            ["Svg svg", "Svg font", "MathMl math", "MathMl mi", "Html b"]
        );
    }

    #[test]
    fn a_fragment_in_an_svg_context_is_svg_and_serializes_in_its_case() {
        let mut document = Document::new();
        let container = document.create_element("html").unwrap();
        crate::parse_fragment(
            &mut document,
            container,
            (&Namespace::Svg, "svg"),
            false,
            dom::MAX_DEPTH,
            "<lineargradient id=g><stop offset=1 /></lineargradient><![CDATA[t]]><p>x",
        );
        let kids: Vec<_> = document
            .children(container)
            .filter_map(|node| document.element_name(node))
            .map(|(ns, tag)| format!("{ns:?} {tag}"))
            .collect();
        assert_eq!(kids, ["Svg linearGradient", "Html p"]);
        assert_eq!(
            crate::serialize_children(&document, container, true),
            "<linearGradient id=\"g\"><stop offset=\"1\"></stop></linearGradient>t<p>x</p>"
        );
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
        // follows is a sibling of the div, not a child of the b: in a
        // new `b`, the formatting element reopened after the div.
        let root = build("<div><b>bold</div>after");
        let children = body_children(&root);
        assert_eq!(tags(children), vec!["div", "b"]);
        let div = expect_element(children, 0);
        let b = expect_element(&div.children, 0);
        assert_eq!(b.tag, "b");
        assert_eq!(b.children, vec![Node::text("bold")]);
        let reopened = expect_element(children, 1);
        assert_eq!(reopened.children, vec![Node::text("after")]);
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
    fn hostile_misnesting_tables_and_templates_stay_bounded() {
        // Deep misnesting, foster parenting and the template modes on
        // hostile input: never deeper than the cap, and quick.
        let sources = [
            "<table>".repeat(5_000) + &"x<td>".repeat(5_000),
            "<a><b><i><table><td>".repeat(3_000) + &"</a></b></i>".repeat(3_000),
            "<b>".repeat(3_000) + &"<p>".repeat(3_000) + &"</b>x".repeat(3_000),
            "<select><option>".repeat(3_000) + &"<optgroup>".repeat(3_000),
            "<template>".repeat(3_000) + &"<td><tr><col>".repeat(3_000),
            "<frameset>".repeat(2_000) + &"<frame>".repeat(2_000) + &"</frameset>x".repeat(2_000),
            "<table><tr>".to_owned() + &" x ".repeat(20_000) + &"<!-- c -->".repeat(2_000),
            "<a>".repeat(2_000) + &"<div>".repeat(2_000) + &"</a>".repeat(2_000),
        ];
        for source in sources {
            let start = std::time::Instant::now();
            let root = build(&source);
            // An element at the cap may hold text, a level below it.
            let depth = depth_of(&root);
            assert!(depth <= MAX_DEPTH + 1, "{:?}: {depth}", &source[..40]);
            assert!(
                start.elapsed() < std::time::Duration::from_secs(30),
                "{:?} took {:?}",
                &source[..40],
                start.elapsed()
            );
        }
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
        // </head> closes the head; a head element before the body goes
        // back into it ("after head"), one after the body start stays.
        let root = build("<head><title>t</title></head><style>a{}</style>x<link>");
        let (_, head, body) = skeleton(&root);
        assert_eq!(tags(&head.children), vec!["title", "style"]);
        assert_eq!(tags(&body.children), vec!["#text", "link"]);
        // A second <head> is ignored; its content goes where it falls.
        let root = build("<p>x</p><head><p>y</p>");
        let (_, head, body) = skeleton(&root);
        assert!(head.children.is_empty());
        assert_eq!(tags(&body.children), vec!["p", "p"]);
    }

    #[test]
    fn an_element_in_svg_has_its_content_parsed_as_markup() {
        // Foreign content's `style` is not raw text (section 13.2.6.5),
        // and HTML's `<b>` breaks out of it, closing the SVG elements.
        let root = build("<svg><style><g>x</g></style></svg>");
        let (_, _, body) = skeleton(&root);
        let svg = expect_element(&body.children, 0);
        let style = expect_element(&svg.children, 0);
        assert_eq!(tags(&style.children), vec!["g"]);
        let root = build("<svg><style><b>x</b></style></svg>");
        let (_, _, body) = skeleton(&root);
        assert_eq!(tags(&body.children), vec!["svg", "b"]);
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
