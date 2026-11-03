//! Serializing HTML fragments (HTML section 13.3, M25): a node's
//! children as markup, what `innerHTML` and `outerHTML` read, and with
//! the shadow roots it is asked for, as declarative shadow roots, what
//! `getHTML()` reads (gap batch H, D68). Iterative at any depth of a
//! tree, recursive only from a host into its shadow tree.

use dom::{Document, Namespace, NodeData, NodeId, ShadowRootMode, SlotAssignmentMode};

/// The shadow roots a serialization writes, as `template` elements with
/// a `shadowrootmode` (`getHTML()`'s `serializableShadowRoots` and
/// `shadowRoots`): each serializable one when `serializable`, and each
/// of `roots`; none by default, as `innerHTML` writes none.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowRoots<'a> {
    pub serializable: bool,
    pub roots: &'a [NodeId],
}

/// The elements that serialize as void: HTML's void elements, and
/// `basefont`, `bgsound`, `frame`, `keygen` and `param`.
const VOID: [&str; 18] = [
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

/// The elements whose text is written as it is, unescaped (and
/// `noscript`, while scripts run).
const RAW_TEXT: [&str; 7] = [
    "style",
    "script",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
];

/// The markup of `node`'s children (the HTML fragment serialization
/// algorithm): empty for an element that serializes as void. A
/// `noscript`'s text is written as it is when `scripting`.
pub fn serialize_children(document: &Document, node: NodeId, scripting: bool) -> String {
    serialize_children_with(document, node, scripting, ShadowRoots::default())
}

/// [`serialize_children`] with the shadow roots `shadows` asks for
/// written as declarative ones: the node's own first, as HTML's fragment
/// serializing algorithm has it.
pub fn serialize_children_with(
    document: &Document,
    node: NodeId,
    scripting: bool,
    shadows: ShadowRoots<'_>,
) -> String {
    let mut out = String::new();
    let void = document.tag(node).is_some_and(|tag| VOID.contains(&tag));
    if !void {
        write_shadow(document, node, scripting, shadows, &mut out);
        write_children(document, node, scripting, shadows, &mut out);
    }
    out
}

/// Writes the shadow root `host` hosts as a `template`, when `shadows`
/// asks for it (HTML section 13.3, step 4).
fn write_shadow(
    document: &Document,
    host: NodeId,
    scripting: bool,
    shadows: ShadowRoots<'_>,
    out: &mut String,
) {
    let Some(root) = document.shadow_root(host) else {
        return;
    };
    let Some(init) = document.shadow_init(root) else {
        return;
    };
    if !(shadows.serializable && init.serializable) && !shadows.roots.contains(&root) {
        return;
    }
    out.push_str("<template shadowrootmode=\"");
    out.push_str(match init.mode {
        ShadowRootMode::Open => "open",
        ShadowRootMode::Closed => "closed",
    });
    out.push('"');
    if init.delegates_focus {
        out.push_str(" shadowrootdelegatesfocus=\"\"");
    }
    if init.serializable {
        out.push_str(" shadowrootserializable=\"\"");
    }
    if init.slot_assignment == SlotAssignmentMode::Manual {
        out.push_str(" shadowrootslotassignment=\"manual\"");
    }
    if init.clonable {
        out.push_str(" shadowrootclonable=\"\"");
    }
    if let Some(target) = document.reference_target(root) {
        out.push_str(" shadowrootreferencetarget=\"");
        escape(target, true, out);
        out.push('"');
    }
    // A tree given a scoped registry, or none where its host's document
    // has one (D70): what a parse of the markup gives it again, a tree
    // with none to be given one.
    let default = document.custom_element_registry(document.owner_document(host));
    let registry = document.custom_element_registry(root);
    let scoped = registry.is_some_and(|registry| registry != dom::WINDOW_REGISTRY);
    if scoped || (registry.is_none() && default.is_some()) {
        out.push_str(" shadowrootcustomelementregistry=\"\"");
    }
    out.push('>');
    write_children(document, root, scripting, shadows, out);
    out.push_str("</template>");
}

/// The markup of `node` itself, with its children's: what `outerHTML`
/// reads.
pub fn serialize_node(document: &Document, node: NodeId, scripting: bool) -> String {
    let mut out = String::new();
    match document.data(node) {
        Some(NodeData::Element { tag, attrs, ns }) => {
            start_tag(tag, attrs, document.is_value(node), &mut out);
            if !(*ns == Namespace::Html && VOID.contains(&tag.as_str())) {
                write_children(document, node, scripting, ShadowRoots::default(), &mut out);
                end_tag(tag, &mut out);
            }
        }
        Some(NodeData::Text(text)) => escape(text, false, &mut out),
        Some(
            data @ (NodeData::Comment(_)
            | NodeData::DocumentType { .. }
            | NodeData::ProcessingInstruction { .. }),
        ) => write_other(data, &mut out),
        Some(NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot(_)) | None => {
            write_children(document, node, scripting, ShadowRoots::default(), &mut out)
        }
    }
    out
}

/// A comment, doctype or processing instruction's markup.
fn write_other(data: &NodeData, out: &mut String) {
    match data {
        NodeData::Comment(text) => {
            out.push_str("<!--");
            out.push_str(text);
            out.push_str("-->");
        }
        NodeData::DocumentType { name, .. } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            out.push('>');
        }
        NodeData::ProcessingInstruction { target, data } => {
            out.push_str("<?");
            out.push_str(target);
            out.push(' ');
            out.push_str(data);
            out.push('>');
        }
        _ => {}
    }
}

/// Writes the children of `node`, and theirs, in tree order: each
/// element's start tag on the way down, then the shadow root `shadows`
/// asks for, its end tag on the way up; a template's contents stand for
/// its children.
fn write_children(
    document: &Document,
    node: NodeId,
    scripting: bool,
    shadows: ShadowRoots<'_>,
    out: &mut String,
) {
    let node = document.template_content(node).unwrap_or(node);
    // A noscript in a template's contents is not scripted: its document,
    // the template contents owner document, has no browsing context.
    let raw = |parent: NodeId| {
        document.tag(parent).is_some_and(|tag| {
            RAW_TEXT.contains(&tag)
                || (tag == "noscript"
                    && scripting
                    && !document
                        .ancestors(parent)
                        .any(|up| document.template_host(up).is_some()))
        })
    };
    let inner = |n: NodeId| document.template_content(n).unwrap_or(n);
    let Some(mut at) = document.first_child(node) else {
        return;
    };
    loop {
        let mut descend = false;
        match document.data(at) {
            Some(NodeData::Text(text)) => {
                if document.parent(at).is_some_and(raw) {
                    out.push_str(text);
                } else {
                    escape(text, false, out);
                }
            }
            Some(NodeData::Element { tag, attrs, ns }) => {
                start_tag(tag, attrs, document.is_value(at), out);
                // Only HTML's void elements have no end tag: a foreign
                // element empty or not has one (`<rect></rect>`).
                if !(*ns == Namespace::Html && VOID.contains(&tag.as_str())) {
                    write_shadow(document, at, scripting, shadows, out);

                    if document.first_child(inner(at)).is_some() {
                        descend = true;
                    } else {
                        end_tag(tag, out);
                    }
                }
            }
            Some(data) => write_other(data, out),
            None => {}
        }
        if descend && let Some(child) = document.first_child(inner(at)) {
            at = child;
            continue;
        }
        // Across to the next sibling, or up, closing each element whose
        // children are all written, until `node`.
        loop {
            if let Some(next) = document.next_sibling(at) {
                at = next;
                break;
            }
            match document.parent(at) {
                Some(parent) if parent != node => {
                    // A template's contents end at the template.
                    let parent = document.template_host(parent).unwrap_or(parent);
                    if let Some((_, tag)) = document.element_name(parent) {
                        end_tag(tag, out);
                    }
                    at = parent;
                }
                _ => return,
            }
        }
    }
}

/// A start tag: its name, its `is` value when it has no `is` attribute
/// (HTML section 13.3, "serializing HTML fragments", step 4.2.3), then
/// its attributes.
fn start_tag(tag: &str, attrs: &[(String, String)], is: Option<&str>, out: &mut String) {
    out.push('<');
    out.push_str(tag);
    if let Some(is) = is
        && !attrs.iter().any(|(name, _)| name == "is")
    {
        out.push_str(" is=\"");
        escape(is, true, out);
        out.push('"');
    }
    for (name, value) in attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape(value, true, out);
        out.push('"');
    }
    out.push('>');
}

fn end_tag(tag: &str, out: &mut String) {
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// HTML's "escaping a string": `&`, no-break spaces, `<` and `>`, and
/// in `attribute` mode `"` too.
fn escape(text: &str, attribute: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_document;

    fn parsed_body(source: &str) -> (Document, NodeId) {
        let document = Document::from_node(&parse_document(source).expect("parses"));
        let body = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("body"))
            .expect("a body");
        (document, body)
    }

    #[test]
    fn markup_is_written_back_as_html_serializes_it() {
        let (document, body) = parsed_body(
            "<p class=\"a&quot;b\" title='<&>\u{a0}'>x &amp; &lt;y&gt;\u{a0}</p><br><img src=i>\
             <ul><li>1<li>2</ul><style>a > b & c</style><script>if (a < b) x()</script>\
             <noscript><b>n</b></noscript><textarea>&lt;t&gt;</textarea>",
        );
        assert_eq!(
            serialize_children(&document, body, true),
            "<p class=\"a&quot;b\" title=\"&lt;&amp;&gt;&nbsp;\">x &amp; &lt;y&gt;&nbsp;</p><br>\
             <img src=\"i\"><ul><li>1</li><li>2</li></ul><style>a > b & c</style>\
             <script>if (a < b) x()</script><noscript><b>n</b></noscript>\
             <textarea>&lt;t&gt;</textarea>"
        );
        // Without scripts, noscript's content is markup, and its text is
        // escaped.
        // (In the body: in the head, the builder has no "in head
        // noscript" mode, G12.)
        // A noscript's text is raw while scripts run, escaped otherwise.
        let mut made = Document::new();
        let noscript = made.create_element("noscript").unwrap();
        let text = made.create_text("<&>").unwrap();
        made.append_child(noscript, text).unwrap();
        assert_eq!(serialize_children(&made, noscript, true), "<&>");
        assert_eq!(serialize_children(&made, noscript, false), "&lt;&amp;&gt;");
        let (document, body) = parsed_body("<p>x</p><noscript><b>a&amp;b</b></noscript>");
        assert_eq!(
            serialize_children(&document, body, false),
            "<p>x</p><noscript><b>a&amp;b</b></noscript>"
        );
        let noscript = document.last_child(body).unwrap();
        assert_eq!(
            serialize_node(&document, noscript, false),
            "<noscript><b>a&amp;b</b></noscript>"
        );
    }

    #[test]
    fn a_noscript_in_a_templates_contents_is_escaped_as_unscripted() {
        let mut parser = crate::Parser::new(true);
        parser.feed("<template><noscript><b></noscript></template><noscript><i></noscript>");
        parser.end();
        let mut document = Document::new();
        while parser.parse(&mut document, usize::MAX) != crate::Stop::Done {}
        let head = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("head"))
            .unwrap();
        assert_eq!(
            serialize_children(&document, head, true),
            "<template><noscript>&lt;b&gt;</noscript></template><noscript><i></noscript>"
        );
    }

    #[test]
    fn a_void_element_has_no_markup_inside_and_a_deep_tree_is_written_whole() {
        let mut document = Document::new();
        let br = document.create_element("br").unwrap();
        let text = document.create_text("x").unwrap();
        document.append_child(br, text).unwrap();
        assert_eq!(serialize_children(&document, br, true), "");
        assert_eq!(serialize_node(&document, br, true), "<br>");
        let image = document.create_element("image").unwrap();
        assert_eq!(serialize_node(&document, image, true), "<image></image>");
        // As deep as the parser's cap allows, and a sibling after it.
        let source = format!("{}x{}<i>y</i>", "<b>".repeat(600), "</b>".repeat(600));
        let (document, body) = parsed_body(&source);
        let markup = serialize_children(&document, body, true);
        assert!(markup.starts_with("<b><b>") && markup.ends_with("</b></b><i>y</i>"));
        let (again, body_again) = parsed_body(&markup);
        assert_eq!(serialize_children(&again, body_again, true), markup);
    }

    /// The first element child of `node` named `tag`.
    fn child(document: &Document, node: NodeId, tag: &str) -> NodeId {
        document
            .children(node)
            .find(|&n| document.tag(n) == Some(tag))
            .unwrap_or_else(|| panic!("a {tag}"))
    }

    #[test]
    fn declarative_shadow_roots_attach_to_their_hosts_and_serialize_back() {
        // HTML section 13.2.6.4.4: a `template` with a valid
        // `shadowrootmode` attaches a shadow root to its parent, its
        // contents the root's children; a second one on the same host
        // stays a template; an invalid mode stays a template too.
        let (document, _) = crate::parse_detached(
            "<div id=host><template shadowrootmode=open shadowrootdelegatesfocus \
             shadowrootclonable shadowrootserializable shadowrootslotassignment=manual>\
             <slot></slot><p>in</p></template><template shadowrootmode=closed>again</template>\
             <span>light</span></div><div id=bad><template shadowrootmode=other>t</template></div>",
            true,
        );
        let body = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("body"))
            .expect("a body");
        let host = child(&document, body, "div");
        let root = document.shadow_root(host).expect("attached");
        let init = document.shadow_init(root).unwrap();
        assert_eq!(init.mode, dom::ShadowRootMode::Open);
        assert!(init.delegates_focus && init.clonable && init.serializable);
        assert_eq!(init.slot_assignment, dom::SlotAssignmentMode::Manual);
        assert!(document.is_declarative_shadow(root));
        let tags: Vec<_> = document
            .children(root)
            .filter_map(|n| document.tag(n))
            .collect();
        assert_eq!(tags, ["slot", "p"]);
        let light: Vec<_> = document
            .children(host)
            .filter_map(|n| document.tag(n))
            .collect();
        assert_eq!(light, ["template", "span"], "the second template stays");
        let bad = document.next_sibling(host).unwrap();
        assert!(document.shadow_root(bad).is_none());
        // getHTML({serializableShadowRoots: true}): the root as a template
        // first, its attributes in the reference browser's order.
        let shadows = ShadowRoots {
            serializable: true,
            roots: &[],
        };
        assert_eq!(
            serialize_children_with(&document, body, true, shadows)
                .split("<template shadowrootmode=\"closed\"")
                .next(),
            Some(
                "<div id=\"host\"><template shadowrootmode=\"open\" shadowrootdelegatesfocus=\"\" \
                 shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" \
                 shadowrootclonable=\"\"><slot></slot><p>in</p></template>"
            )
        );
        // Without the option, and for a root neither serializable nor
        // named, only the light tree.
        assert!(!serialize_children(&document, body, true).contains("shadowrootmode=\"open\""));
    }

    #[test]
    fn fragment_parsing_attaches_no_shadow_root_unless_asked() {
        let mut document = Document::new();
        let container = document.create_element("div").unwrap();
        let markup = "<div><template shadowrootmode=open><p>x</p></template></div>";
        crate::parse_fragment(
            &mut document,
            container,
            (&dom::Namespace::Html, "div"),
            false,
            dom::MAX_DEPTH,
            markup,
        );
        let div = child(&document, container, "div");
        assert!(document.shadow_root(div).is_none(), "innerHTML: no opt-in");
        assert_eq!(
            serialize_children(&document, container, false),
            "<div><template shadowrootmode=\"open\"><p>x</p></template></div>"
        );
        // setHTMLUnsafe's opt-in.
        let other = document.create_element("div").unwrap();
        crate::parse_fragment_with(
            &mut document,
            other,
            (&dom::Namespace::Html, "div"),
            (false, true),
            dom::MAX_DEPTH,
            markup,
        );
        let div = child(&document, other, "div");
        let root = document.shadow_root(div).expect("attached");
        assert_eq!(
            document
                .children(root)
                .filter_map(|n| document.tag(n))
                .collect::<Vec<_>>(),
            ["p"]
        );
        // A root named in `shadowRoots` is written though not serializable.
        let roots = [root];
        let shadows = ShadowRoots {
            serializable: false,
            roots: &roots,
        };
        assert_eq!(
            serialize_children_with(&document, other, false, shadows),
            "<div><template shadowrootmode=\"open\"><p>x</p></template></div>"
        );
    }
}
