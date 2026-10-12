//! Serializing HTML fragments (HTML section 13.3, M25): a node's
//! children as markup, what `innerHTML` and `outerHTML` read. Iterative
//! at any depth.

use dom::{Document, Namespace, NodeData, NodeId};

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
    let mut out = String::new();
    let void = document.tag(node).is_some_and(|tag| VOID.contains(&tag));
    if !void {
        write_children(document, node, scripting, &mut out);
    }
    out
}

/// The markup of `node` itself, with its children's: what `outerHTML`
/// reads.
pub fn serialize_node(document: &Document, node: NodeId, scripting: bool) -> String {
    let mut out = String::new();
    match document.data(node) {
        Some(NodeData::Element { tag, attrs, ns }) => {
            start_tag(tag, attrs, &mut out);
            if !(*ns == Namespace::Html && VOID.contains(&tag.as_str())) {
                write_children(document, node, scripting, &mut out);
                end_tag(tag, &mut out);
            }
        }
        Some(NodeData::Text(text)) => escape(text, false, &mut out),
        Some(NodeData::Document) | None => write_children(document, node, scripting, &mut out),
    }
    out
}

/// Writes the children of `node`, and theirs, in tree order: each
/// element's start tag on the way down, its end tag on the way up.
fn write_children(document: &Document, node: NodeId, scripting: bool, out: &mut String) {
    let raw = |parent: NodeId| {
        document
            .tag(parent)
            .is_some_and(|tag| RAW_TEXT.contains(&tag) || (tag == "noscript" && scripting))
    };
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
                start_tag(tag, attrs, out);
                // Only HTML's void elements have no end tag: a foreign
                // element empty or not has one (`<rect></rect>`).
                if !(*ns == Namespace::Html && VOID.contains(&tag.as_str())) {
                    if document.first_child(at).is_some() {
                        descend = true;
                    } else {
                        end_tag(tag, out);
                    }
                }
            }
            _ => {}
        }
        if descend && let Some(child) = document.first_child(at) {
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

fn start_tag(tag: &str, attrs: &[(String, String)], out: &mut String) {
    out.push('<');
    out.push_str(tag);
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
}
