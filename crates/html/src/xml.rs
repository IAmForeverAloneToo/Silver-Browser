//! The XML parser (M32, `docs/design/svg.md`): bytes to a
//! `dom::Document`, as XML 1.0 (Fifth Edition) with Namespaces in XML
//! 1.0 reads them, non-validating, for SVG documents and images.
//!
//! The text is decoded by its byte order mark, else by the encoding its
//! declaration names, else as UTF-8, and its line ends taken to line
//! feeds. Elements go into the document in the namespace their prefix,
//! or the default namespace, is bound to by the `xmlns` attributes in
//! scope (the `xml` prefix bound from the start); attributes keep their
//! qualified names, normalized as section 3.3.3 says; character data
//! becomes text, adjacent text joined, and a CDATA section a text node
//! of its own that is one (D67). Comments,
//! processing instructions and the document type declaration are nodes
//! (D58), but for those in the internal subset; what is in an HTML
//! `template` goes in its contents. The internal subset's general
//! entities are expanded where they are referenced, their markup parsed
//! as content, with the five predefined ones and character references;
//! an external subset or entity is never fetched, and a reference to an
//! external entity in content stands for nothing.
//!
//! There is no recovery: the first error a well-formed document has not
//! ends the parse with its line and column, as the reference browser's
//! parser does. Bounds keep a hostile document linear: elements nest at
//! most [`MAX_XML_DEPTH`] deep; entities expand to at most
//! [`MAX_ENTITY_EXPANSION`] characters, and to no more than
//! [`MAX_ENTITY_FACTOR`] times the document's own length (the billion
//! laughs is an error, as in the reference browser), nested at most
//! [`MAX_ENTITY_NESTING`] deep; an entity that refers to itself is an
//! error.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use dom::{Document, Namespace, NodeData, NodeId, XML_NAMESPACE, XMLNS_NAMESPACE};
use encoding::Encoding;

/// The most elements open at once.
pub const MAX_XML_DEPTH: usize = 512;
/// The most characters entities may expand to in a document.
pub const MAX_ENTITY_EXPANSION: usize = 1 << 20;
/// How many times the document's own length its entities may expand to.
pub const MAX_ENTITY_FACTOR: usize = 10;
/// How deep entities may refer to entities.
pub const MAX_ENTITY_NESTING: usize = 32;
/// The expansion every document may have whatever its length.
const EXPANSION_SLACK: usize = 4096;

/// Why a document is not well formed, and where: the line and column,
/// counted from 1, of the place in the document the parser stopped at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    /// The line.
    pub line: u32,
    /// The column, in characters.
    pub column: u32,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "error on line {} at column {}: {}",
            self.line, self.column, self.message
        )
    }
}

/// The text of an XML document's `bytes`: decoded by a byte order mark,
/// else by the encoding the XML declaration names, else as UTF-8; an
/// encoding this engine cannot decode is UTF-8 too.
pub fn decode_xml(bytes: &[u8]) -> String {
    decode_xml_with(bytes, None).0
}

/// The text of an XML document's `bytes` and the encoding it was read
/// in: a byte order mark's, else `transport`'s (the `Content-Type`
/// charset), else the XML declaration's, else UTF-8; one this engine
/// cannot decode is decoded as UTF-8 and named as it is.
pub fn decode_xml_with(bytes: &[u8], transport: Option<Encoding>) -> (String, Encoding) {
    if let Some((encoding, _)) = encoding::bom_sniff(bytes) {
        return (encoding::decode(bytes, encoding).0, encoding);
    }
    let named = transport
        .or_else(|| declared_encoding(bytes))
        .unwrap_or(encoding::UTF_8);
    let with = if named.can_decode() {
        named
    } else {
        encoding::UTF_8
    };
    (encoding::decode(bytes, with).0, named)
}

/// Whether an XML document shows as a document rather than as its
/// source: it has an element of the XHTML, SVG or MathML namespace, or a
/// `xml-stylesheet` processing instruction, as the reference browser
/// decides.
pub fn has_style_information(document: &Document) -> bool {
    document
        .subtree(document.root())
        .any(|node| match document.data(node) {
            Some(dom::NodeData::Element { ns, .. }) => !matches!(ns, Namespace::Other(_)),
            Some(dom::NodeData::ProcessingInstruction { target, .. }) => {
                &**target == "xml-stylesheet"
            }
            _ => false,
        })
}

/// The reference browser's report of the error an XML document's parse
/// stopped at, put first in its root element: a `parsererror` element
/// saying so above what was parsed before it. A document with no root
/// element is given an HTML one; an SVG root is moved into the body of
/// an HTML one.
pub fn insert_error_block(document: &mut Document, error: &XmlError) {
    let _ = error_block(document, error);
}

fn error_block(document: &mut Document, error: &XmlError) -> Result<(), dom::MutationError> {
    let root = document.root();
    let html = |document: &mut Document, tag: &str| {
        document.create_element_ns(Namespace::Html, tag, Vec::new())
    };
    let text = |document: &mut Document, parent: NodeId, data: &str| {
        let node = document.create_text(data)?;
        document.append_new(parent, node)
    };
    let mut parent = document.document_element();
    let svg = parent.filter(|&node| document.namespace(node) == Some(&Namespace::Svg));
    if parent.is_none() || svg.is_some() {
        if let Some(svg) = svg {
            document.remove_child(root, svg)?;
        }
        let root_element = html(document, "html")?;
        document.append_new(root, root_element)?;
        if svg.is_some() {
            let head = html(document, "head")?;
            document.append_new(root_element, head)?;
            let style = html(document, "style")?;
            document.append_new(head, style)?;
            text(
                document,
                style,
                "html, body { height: 100% } parsererror + svg { width: 100%; height: 100% }",
            )?;
        }
        let body = html(document, "body")?;
        document.append_new(root_element, body)?;
        if let Some(svg) = svg {
            document.append_child(body, svg)?;
        }
        parent = Some(body);
    }
    let Some(parent) = parent else {
        return Ok(());
    };
    let report = document.create_element_ns(
        Namespace::Html,
        "parsererror",
        vec![(
            "style".to_owned(),
            "display: block; white-space: pre; border: 2px solid #c77; padding: 0 1em 0 1em; \
             margin: 1em; background-color: #fdd; color: black"
                .to_owned(),
        )],
    )?;
    let heading = html(document, "h3")?;
    document.append_new(report, heading)?;
    text(
        document,
        heading,
        "This page contains the following errors:",
    )?;
    let fixed = document.create_element_ns(
        Namespace::Html,
        "div",
        vec![(
            "style".to_owned(),
            "font-family:monospace;font-size:12px".to_owned(),
        )],
    )?;
    document.append_new(report, fixed)?;
    text(document, fixed, &format!("{error}\n"))?;
    let heading = html(document, "h3")?;
    document.append_new(report, heading)?;
    text(
        document,
        heading,
        "Below is a rendering of the page up to the first error.",
    )?;
    match document.first_child(parent) {
        Some(first) => document.insert_before(parent, report, Some(first)),
        None => document.append_child(parent, report),
    }
}

/// The encoding an XML declaration at the start of `bytes` names.
fn declared_encoding(bytes: &[u8]) -> Option<Encoding> {
    let head = bytes.get(..bytes.len().min(1024))?;
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&head[..end]).ok()?;
    let at = decl.find("encoding")?;
    let rest = decl[at + "encoding".len()..]
        .trim_start()
        .strip_prefix('=')?;
    let rest = rest.trim_start();
    let quote = rest.chars().next().filter(|&c| c == '"' || c == '\'')?;
    let value = rest[1..].split(quote).next()?;
    Encoding::for_label(value)
}

/// Parses an XML document's `bytes` (see the module doc).
pub fn parse_xml(bytes: &[u8]) -> Result<Document, XmlError> {
    parse_xml_text(&decode_xml(bytes))
}

/// Parses an XML document's decoded `text`.
pub fn parse_xml_text(text: &str) -> Result<Document, XmlError> {
    let mut parser = XmlParser::new(text);
    let mut document = Document::new();
    while parser.parse(&mut document, usize::MAX)? != XmlStop::Done {}
    Ok(document)
}

/// Parses `text` as a document of its own (`DOMParser`'s, D67): the
/// document as far as the parse went, and the error it stopped at, if
/// it is not well formed, for the caller to report in it.
pub fn parse_xml_detached(text: &str) -> (Document, Option<XmlError>) {
    let mut parser = XmlParser::new(text);
    let mut document = Document::new();
    loop {
        match parser.parse(&mut document, usize::MAX) {
            Ok(XmlStop::Done) => return (document, None),
            Ok(_) => {}
            Err(error) => return (document, Some(error)),
        }
    }
}

/// The name the container of a fragment's parse goes by: no end tag can
/// name it.
const FRAGMENT_ROOT: &str = "\u{0}fragment";

/// Parses `markup` as an XML fragment (DOM Parsing's "fragment parsing
/// algorithm" for an XML document, `innerHTML` and its kin) into the
/// children of `container`, a detached element its caller made, in the
/// context of element `context`: its namespace the default one, and the
/// prefixes its and its ancestors' `xmlns` attributes declare bound; at
/// most `max_depth` elements open, the container counted. Not well
/// formed, it is an error, and `container` may hold part of it.
pub fn parse_xml_fragment(
    document: &mut Document,
    container: NodeId,
    context: NodeId,
    markup: &str,
    max_depth: usize,
) -> Result<(), XmlError> {
    let mut bindings = vec![Binding {
        prefix: Some("xml".to_owned()),
        uri: Some(XML_NAMESPACE.to_owned()),
    }];
    // Outermost first, so that the innermost declaration wins.
    let mut scope: Vec<NodeId> = document.ancestors(context).collect();
    scope.reverse();
    scope.push(context);
    for element in scope {
        for (name, value) in document.attrs(element).unwrap_or_default() {
            let prefix = match name.as_str() {
                "xmlns" => None,
                name => match name.strip_prefix("xmlns:") {
                    Some(prefix) => Some(prefix.to_owned()),
                    None => continue,
                },
            };
            bindings.push(Binding {
                prefix,
                uri: (!value.is_empty()).then(|| value.clone()),
            });
        }
    }
    // The context's own namespace is the default, unless it declares
    // another.
    if let Some((ns, tag)) = document.element_name(context)
        && !tag.contains(':')
        && document.attr(context, "xmlns").is_none()
    {
        bindings.push(Binding {
            prefix: None,
            uri: ns.uri().map(str::to_owned),
        });
    }
    let text = markup.replace("\r\n", "\n").replace('\r', "\n");
    let mut parser = Parser {
        document: std::mem::take(document),
        entities: HashMap::new(),
        expanded: 0,
        budget: (text.len() * MAX_ENTITY_FACTOR + EXPANSION_SLACK).min(MAX_ENTITY_EXPANSION),
        entity_stack: Vec::new(),
        open: vec![Open {
            node: container,
            name: FRAGMENT_ROOT.to_owned(),
            bindings: bindings.len(),
            line: 1,
        }],
        bindings,
        root: Root::Inside,
        reference: None,
        scripts: VecDeque::new(),
        xhtml_entities: false,
        max_depth,
        custom: CustomParse::default(),
    };
    let mut c = Cursor::new(&text);
    let result = parser
        .content(&mut c)
        .and_then(|()| match parser.open.len() {
            1 => Ok(()),
            _ => parser.finished(&c),
        });
    *document = parser.document;
    result
}

/// The XML serialization of `node`'s children (DOM Parsing section 3.2,
/// what `innerHTML` reads in an XML document): each child serialized in
/// no namespace's context, a template's contents standing for its
/// children, the markup required to be well formed; `None` when it is
/// not, which `innerHTML` throws for.
pub fn serialize_xml_children(document: &Document, node: NodeId) -> Option<String> {
    let mut serializer = XmlSerializer::new(document, true);
    let node = document.template_content(node).unwrap_or(node);
    for child in document.children(node) {
        serializer.node(child, None, &PrefixMap::new())?;
    }
    Some(serializer.out)
}

/// The XML serialization of `node` itself (what `outerHTML` reads in an
/// XML document), required to be well formed; `None` when it is not.
pub fn serialize_xml_node(document: &Document, node: NodeId) -> Option<String> {
    xml_serialization(document, node, true)
}

/// DOM Parsing section 3.2's "produce an XML serialization" of `node`:
/// its markup, prefixes declared or made up where its elements' and
/// attributes' namespaces need them (`XMLSerializer.serializeToString`,
/// with `require_well_formed` off); `None` when it is required to be
/// well formed and is not.
pub fn xml_serialization(
    document: &Document,
    node: NodeId,
    require_well_formed: bool,
) -> Option<String> {
    let mut serializer = XmlSerializer::new(document, require_well_formed);
    if require_well_formed
        && document.is_document(node)
        && document.document_element_of(node).is_none()
    {
        return None;
    }
    serializer.node(node, None, &PrefixMap::new())?;
    Some(serializer.out)
}

/// HTML's void elements, which XML writes as `<br />` when empty.
const XML_VOID: [&str; 19] = [
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr",
];

/// A namespace prefix map (DOM Parsing section 3.2.1.1): each namespace,
/// none among them, with the prefixes bound to it, in the order they
/// were; the `xml` prefix bound from the start.
#[derive(Clone)]
struct PrefixMap(Vec<(Option<String>, Vec<String>)>);

impl PrefixMap {
    fn new() -> PrefixMap {
        PrefixMap(vec![(
            Some(XML_NAMESPACE.to_owned()),
            vec!["xml".to_owned()],
        )])
    }

    fn prefixes(&self, ns: Option<&str>) -> Option<&Vec<String>> {
        self.0
            .iter()
            .find(|(uri, _)| uri.as_deref() == ns)
            .map(|(_, prefixes)| prefixes)
    }

    /// Whether `prefix` is bound to `ns` ("found").
    fn has(&self, ns: Option<&str>, prefix: &str) -> bool {
        self.prefixes(ns)
            .is_some_and(|prefixes| prefixes.iter().any(|p| p == prefix))
    }

    fn add(&mut self, ns: Option<&str>, prefix: &str) {
        match self.0.iter_mut().find(|(uri, _)| uri.as_deref() == ns) {
            Some((_, prefixes)) => prefixes.push(prefix.to_owned()),
            None => self
                .0
                .push((ns.map(str::to_owned), vec![prefix.to_owned()])),
        }
    }

    /// "Retrieving a preferred prefix string": `preferred` if it is bound
    /// to `ns`, else the last prefix bound to it.
    fn preferred(&self, preferred: Option<&str>, ns: Option<&str>) -> Option<String> {
        let prefixes = self.prefixes(ns)?;
        prefixes
            .iter()
            .find(|p| Some(p.as_str()) == preferred)
            .or(prefixes.last())
            .cloned()
    }
}

/// What an element's start tag leaves its children: its qualified name
/// for its end tag, and the namespace and prefix map they are serialized
/// with; `None` when it closed itself.
type Opened = Option<(String, Option<String>, PrefixMap)>;

/// The state of one XML serialization: the markup so far, the made up
/// prefixes' index, and whether the markup must be well formed.
struct XmlSerializer<'a> {
    document: &'a Document,
    require_well_formed: bool,
    prefix_index: u32,
    out: String,
}

impl<'a> XmlSerializer<'a> {
    fn new(document: &'a Document, require_well_formed: bool) -> XmlSerializer<'a> {
        XmlSerializer {
            document,
            require_well_formed,
            prefix_index: 1,
            out: String::new(),
        }
    }

    /// DOM Parsing's new prefix for a namespace: `ns{index}`, bound to
    /// `ns` in `map`.
    fn new_prefix(&mut self, map: &mut PrefixMap, ns: Option<&str>) -> String {
        let prefix = format!("ns{}", self.prefix_index);
        self.prefix_index += 1;
        map.add(ns, &prefix);
        prefix
    }

    /// The XML serialization algorithm (section 3.2.1) of `node` with the
    /// context namespace `ns` and the prefix map `map`, iteratively: an
    /// element's children are serialized with the namespace and map its
    /// start tag left, then its end tag written; `None` when the markup
    /// must be well formed and is not.
    fn node(&mut self, node: NodeId, ns: Option<&str>, map: &PrefixMap) -> Option<()> {
        enum Step {
            Node(NodeId, Option<String>, Rc<PrefixMap>),
            End(String),
        }
        let document = self.document;
        let mut steps = vec![Step::Node(
            node,
            ns.map(str::to_owned),
            Rc::new(map.clone()),
        )];
        while let Some(step) = steps.pop() {
            let (at, ns, map) = match step {
                Step::End(name) => {
                    self.out.push_str("</");
                    self.out.push_str(&name);
                    self.out.push('>');
                    continue;
                }
                Step::Node(at, ns, map) => (at, ns, map),
            };
            match document.data(at) {
                Some(NodeData::Element { .. }) => {
                    let Some((name, inherited, map)) = self.start_tag(at, ns.as_deref(), &map)?
                    else {
                        continue;
                    };
                    let map = Rc::new(map);
                    steps.push(Step::End(name));
                    let inner = match document.element_name(at) {
                        Some((Namespace::Html, "template")) => {
                            document.template_content(at).unwrap_or(at)
                        }
                        _ => at,
                    };
                    let children: Vec<NodeId> = document.children(inner).collect();
                    for child in children.into_iter().rev() {
                        steps.push(Step::Node(child, inherited.clone(), map.clone()));
                    }
                }
                Some(NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot(_)) => {
                    let children: Vec<NodeId> = document.children(at).collect();
                    for child in children.into_iter().rev() {
                        steps.push(Step::Node(child, ns.clone(), map.clone()));
                    }
                }
                Some(NodeData::Text(text)) if document.is_cdata(at) => {
                    if self.require_well_formed && text.contains("]]>") {
                        return None;
                    }
                    self.out.push_str("<![CDATA[");
                    self.out.push_str(text);
                    self.out.push_str("]]>");
                }
                Some(NodeData::Text(text)) => {
                    if self.require_well_formed && !text.chars().all(is_char) {
                        return None;
                    }
                    escape_xml(text, false, &mut self.out);
                }
                Some(NodeData::Comment(text)) => {
                    if self.require_well_formed
                        && (!text.chars().all(is_char)
                            || text.contains("--")
                            || text.ends_with('-'))
                    {
                        return None;
                    }
                    self.out.push_str("<!--");
                    self.out.push_str(text);
                    self.out.push_str("-->");
                }
                Some(NodeData::ProcessingInstruction { target, data }) => {
                    if self.require_well_formed
                        && (target.contains(':')
                            || target.eq_ignore_ascii_case("xml")
                            || !is_xml_name(target)
                            || !data.chars().all(is_char)
                            || data.contains("?>"))
                    {
                        return None;
                    }
                    self.out.push_str("<?");
                    self.out.push_str(target);
                    self.out.push(' ');
                    self.out.push_str(data);
                    self.out.push_str("?>");
                }
                Some(NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                }) => {
                    if self.require_well_formed
                        && (!public_id.chars().all(is_pubid_char)
                            || !system_id.chars().all(is_char)
                            || (system_id.contains('"') && system_id.contains('\'')))
                    {
                        return None;
                    }
                    self.out.push_str("<!DOCTYPE ");
                    self.out.push_str(name);
                    if !public_id.is_empty() {
                        self.out.push_str(" PUBLIC \"");
                        self.out.push_str(public_id);
                        self.out.push('"');
                    } else if !system_id.is_empty() {
                        self.out.push_str(" SYSTEM");
                    }
                    if !system_id.is_empty() {
                        self.out.push_str(" \"");
                        self.out.push_str(system_id);
                        self.out.push('"');
                    }
                    self.out.push('>');
                }
                None => {}
            }
        }
        Some(())
    }

    /// An element's start tag ("serializing an Element node", steps 1 to
    /// 17), written with its attributes: what it leaves its children;
    /// `None` when it must be well formed and is not.
    fn start_tag(
        &mut self,
        element: NodeId,
        inherited: Option<&str>,
        map: &PrefixMap,
    ) -> Option<Opened> {
        let document = self.document;
        let (namespace, _) = document.element_name(element)?;
        let local = document.local_name(element)?;
        let prefix = document.element_prefix(element);
        let ns = namespace.uri();
        if self.require_well_formed && (local.contains(':') || !is_xml_name(local)) {
            return None;
        }
        let mut map = map.clone();
        let mut local_prefixes: Vec<(String, Option<String>)> = Vec::new();
        let local_default = self.record_namespaces(element, &mut map, &mut local_prefixes);
        // The local default namespace as the one inherited: none for an
        // empty declaration, nothing for one of the XML namespace.
        let declared = local_default
            .as_ref()
            .filter(|uri| uri.as_str() != XML_NAMESPACE)
            .map(|uri| (!uri.is_empty()).then(|| uri.clone()));
        let mut inherited = inherited.map(str::to_owned);
        let mut ignore_default = false;
        let qualified;
        self.out.push('<');
        if inherited.as_deref() == ns {
            ignore_default = local_default.is_some();
            qualified = match ns {
                Some(XML_NAMESPACE) => format!("xml:{local}"),
                _ => local.to_owned(),
            };
            self.out.push_str(&qualified);
        } else {
            let mut candidate = map.preferred(prefix, ns);
            if prefix == Some("xmlns") {
                if self.require_well_formed {
                    return None;
                }
                candidate = prefix.map(str::to_owned);
            }
            if let Some(candidate) = candidate {
                qualified = format!("{candidate}:{local}");
                if let Some(default) = declared {
                    inherited = default;
                }
                self.out.push_str(&qualified);
            } else if let Some(prefix) = prefix {
                let prefix = if local_prefixes.iter().any(|(p, _)| p == prefix) {
                    self.new_prefix(&mut map, ns)
                } else {
                    map.add(ns, prefix);
                    prefix.to_owned()
                };
                qualified = format!("{prefix}:{local}");
                self.out.push_str(&qualified);
                self.out.push_str(" xmlns:");
                self.out.push_str(&prefix);
                self.out.push_str("=\"");
                self.attribute_value(ns.unwrap_or(""))?;
                self.out.push('"');
                if let Some(default) = declared {
                    inherited = default;
                }
            } else if local_default.as_deref() != Some(ns.unwrap_or("")) {
                ignore_default = true;
                qualified = local.to_owned();
                inherited = ns.map(str::to_owned);
                self.out.push_str(&qualified);
                self.out.push_str(" xmlns=\"");
                self.attribute_value(ns.unwrap_or(""))?;
                self.out.push('"');
            } else {
                qualified = local.to_owned();
                inherited = ns.map(str::to_owned);
                self.out.push_str(&qualified);
            }
        }
        self.attributes(element, &mut map, &local_prefixes, ignore_default)?;
        let empty = document.first_child(element).is_none();
        if empty && *namespace == Namespace::Html && XML_VOID.contains(&local) {
            self.out.push_str(" />");
            return Some(None);
        }
        if empty && *namespace != Namespace::Html {
            self.out.push_str("/>");
            return Some(None);
        }
        self.out.push('>');
        Some(Some((qualified, inherited, map)))
    }

    /// "Recording the namespace information" (section 3.2.1.1.2): the
    /// element's `xmlns:` declarations put in `map` and `local_prefixes`;
    /// its `xmlns` declaration's value, if it has one.
    fn record_namespaces(
        &self,
        element: NodeId,
        map: &mut PrefixMap,
        local_prefixes: &mut Vec<(String, Option<String>)>,
    ) -> Option<String> {
        let document = self.document;
        let mut default = None;
        for (i, (name, value)) in document.attrs(element)?.iter().enumerate() {
            if document.attribute_namespace(element, i) != Some(XMLNS_NAMESPACE) {
                continue;
            }
            let Some((_, prefix)) = name.split_once(':') else {
                default = Some(value.clone());
                continue;
            };
            if value == XML_NAMESPACE {
                continue;
            }
            let ns = (!value.is_empty()).then_some(value.as_str());
            if map.has(ns, prefix) {
                continue;
            }
            map.add(ns, prefix);
            local_prefixes.push((prefix.to_owned(), ns.map(str::to_owned)));
        }
        default
    }

    /// "XML serialization of the attributes" (section 3.2.1.1.3).
    fn attributes(
        &mut self,
        element: NodeId,
        map: &mut PrefixMap,
        local_prefixes: &[(String, Option<String>)],
        ignore_default: bool,
    ) -> Option<()> {
        let document = self.document;
        let attrs = document.attrs(element)?;
        let own = document.namespace(element).and_then(Namespace::uri);
        let mut seen: Vec<(Option<&str>, &str)> = Vec::new();
        for (i, (name, value)) in attrs.iter().enumerate() {
            let ns = document.attribute_namespace(element, i);
            let local = dom::attribute_local_name(ns, name);
            // An `xmlns` of no namespace (`setAttribute`'s) declares a
            // default namespace to a reader: one that is not the
            // element's, or that its start tag made or left out, is
            // dropped, as the reference browser drops it.
            if ns.is_none()
                && local == "xmlns"
                && (ignore_default || value.as_str() != own.unwrap_or(""))
            {
                continue;
            }
            let prefix = match ns {
                Some(_) => name.split_once(':').map(|(prefix, _)| prefix),
                None => None,
            };
            if self.require_well_formed && seen.contains(&(ns, local)) {
                return None;
            }
            seen.push((ns, local));
            let mut candidate = None;
            if let Some(ns) = ns {
                candidate = map.preferred(prefix, Some(ns));
                if ns == XMLNS_NAMESPACE {
                    // A declaration the element's start tag made, or that
                    // says what is already so, is left out.
                    let declared = local_prefixes
                        .iter()
                        .find(|(p, _)| p == local)
                        .map(|(_, uri)| uri.as_deref().unwrap_or(""));
                    if value == XML_NAMESPACE
                        || (prefix.is_none() && ignore_default)
                        || (prefix.is_some() && declared != Some(value.as_str()))
                    {
                        continue;
                    }
                    // A prefix's declaration cannot undeclare it.
                    if self.require_well_formed
                        && (value == XMLNS_NAMESPACE || (prefix.is_some() && value.is_empty()))
                    {
                        return None;
                    }
                    if prefix == Some("xmlns") {
                        candidate = Some("xmlns".to_owned());
                    }
                } else if candidate.is_none() {
                    let made = self.new_prefix(map, Some(ns));
                    self.out.push_str(" xmlns:");
                    self.out.push_str(&made);
                    self.out.push_str("=\"");
                    self.attribute_value(ns)?;
                    self.out.push('"');
                    candidate = Some(made);
                }
            }
            self.out.push(' ');
            if let Some(candidate) = candidate {
                self.out.push_str(&candidate);
                self.out.push(':');
            }
            if self.require_well_formed
                && (local.contains(':')
                    || !is_xml_name(local)
                    || (local == "xmlns" && ns.is_none()))
            {
                return None;
            }
            self.out.push_str(local);
            self.out.push_str("=\"");
            self.attribute_value(value)?;
            self.out.push('"');
        }
        Some(())
    }

    /// "Serializing an attribute value": escaped; `None` when it must be
    /// well formed and holds what XML's `Char` does not.
    fn attribute_value(&mut self, value: &str) -> Option<()> {
        if self.require_well_formed && !value.chars().all(is_char) {
            return None;
        }
        escape_xml(value, true, &mut self.out);
        Some(())
    }
}

/// Whether `name` is XML's `Name`.
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_name_start) && chars.all(is_name_char)
}

/// XML's `PubidChar`.
fn is_pubid_char(c: char) -> bool {
    matches!(c, ' ' | '\r' | '\n' | 'a'..='z' | 'A'..='Z' | '0'..='9')
        || "-'()+,./:=?;!*#@$_%".contains(c)
}

/// XML's escaping: `&`, `<` and `>`, and in an attribute value `"` and
/// the white space a parser would normalize away.
fn escape_xml(text: &str, attribute: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\t' if attribute => out.push_str("&#x9;"),
            '\n' if attribute => out.push_str("&#xA;"),
            '\r' if attribute => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

/// Why [`XmlParser::parse`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlStop {
    /// It parsed the items it was allowed.
    Slice,
    /// An HTML or SVG `script` element was just closed: the caller
    /// prepares it before the parse goes on, as XML's parser runs a
    /// script it inserts. `line` is the line its start tag ended on.
    Script { element: NodeId, line: u32 },
    /// The next start tag makes a custom element a definition is known
    /// for (D70): the caller constructs it and gives it to the parser
    /// ([`XmlParser::provide_element`]) before the parse goes on, as
    /// HTML's parser has it ([`crate::Stop::Construct`]).
    Construct,
    /// What the parser did queued custom element reactions, which the
    /// caller runs before the parse goes on.
    Reactions,
    /// The document is parsed to its end.
    Done,
}

/// An XML parser over a whole document's text that builds into a
/// [`Document`] a slice at a time, stopping after each script element
/// it closes: an XML document's scripts run as its parser meets them.
pub struct XmlParser {
    text: String,
    /// Where the parse stands in `text`.
    pos: usize,
    line: u32,
    column: u32,
    started: bool,
    done: bool,
    parser: Parser,
}

impl XmlParser {
    /// A parser of the decoded `text`, a byte order mark and carriage
    /// returns taken away.
    pub fn new(text: &str) -> XmlParser {
        let text = text
            .strip_prefix('\u{feff}')
            .unwrap_or(text)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let budget = (text.len() * MAX_ENTITY_FACTOR + EXPANSION_SLACK).min(MAX_ENTITY_EXPANSION);
        XmlParser {
            text,
            pos: 0,
            line: 1,
            column: 1,
            started: false,
            done: false,
            parser: Parser {
                document: Document::new(),
                entities: HashMap::new(),
                expanded: 0,
                budget,
                entity_stack: Vec::new(),
                open: Vec::new(),
                bindings: vec![Binding {
                    prefix: Some("xml".to_owned()),
                    uri: Some(XML_NAMESPACE.to_owned()),
                }],
                root: Root::Before,
                reference: None,
                scripts: VecDeque::new(),
                xhtml_entities: false,
                max_depth: MAX_XML_DEPTH,
                custom: CustomParse::default(),
            },
        }
    }

    /// Whether the parse is over, at the end or at an error.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Whether the elements a custom element definition is known for are
    /// constructed as they are made, stopping at [`XmlStop::Construct`]
    /// and [`XmlStop::Reactions`] (D70): off until asked, as a parser
    /// whose caller runs no script's has none.
    pub fn set_custom_elements(&mut self, construct: bool) {
        self.parser.custom.construct = construct;
    }

    /// The local name and `is` value of the custom element the parse
    /// stopped at [`XmlStop::Construct`] for.
    pub fn custom_element(&self) -> Option<(&str, Option<&str>)> {
        self.parser
            .custom
            .held
            .as_ref()
            .map(|held| (held.local.as_str(), held.is.as_deref()))
    }

    /// The prefix of the qualified name of the tag the parse stopped at
    /// [`XmlStop::Construct`] for, if it has one.
    pub fn custom_prefix(&self) -> Option<&str> {
        self.parser
            .custom
            .held
            .as_ref()
            .and_then(|held| held.prefix.as_deref())
    }

    /// Gives the parser the element a definition's constructor made for
    /// the start tag it stopped at, which it takes as that tag's.
    pub fn provide_element(&mut self, element: NodeId) {
        self.parser.custom.provided = Some(element);
    }

    /// Parses at most `slice` items of content (a tag, a run of text, a
    /// comment...) into `document`, which must be the one this parser
    /// has built into from the start; the first error ends the parse,
    /// what was built before it left in the document.
    pub fn parse(&mut self, document: &mut Document, slice: usize) -> Result<XmlStop, XmlError> {
        if self.done {
            return Ok(XmlStop::Done);
        }
        std::mem::swap(&mut self.parser.document, document);
        let mut c = Cursor {
            text: &self.text,
            pos: self.pos,
            line: self.line,
            column: self.column,
        };
        let parser = &mut self.parser;
        let mut result = if self.started {
            Ok(())
        } else {
            self.started = true;
            parser.document.set_xml_document(true);
            parser.prolog(&mut c)
        };
        let mut stop = XmlStop::Slice;
        if result.is_ok() {
            match parser.content_slice(&mut c, slice) {
                Ok(Some(found)) => stop = found,
                Ok(None) => {}
                Err(error) => result = Err(error),
            }
        }
        if result.is_ok() && stop == XmlStop::Slice && c.at_end() {
            result = parser.finished(&c);
            stop = XmlStop::Done;
        }
        (self.pos, self.line, self.column) = (c.pos, c.line, c.column);
        std::mem::swap(&mut self.parser.document, document);
        match result {
            Ok(()) => {
                self.done = stop == XmlStop::Done;
                Ok(stop)
            }
            Err(error) => {
                self.done = true;
                Err(error)
            }
        }
    }
}

/// An entity the internal subset declares.
#[derive(Debug, Clone)]
enum Entity {
    /// Its replacement text, its character references already replaced.
    Internal(String),
    /// One whose text is outside the document, never fetched.
    External,
}

/// A namespace prefix bound in scope: `None` for the default namespace,
/// and its URI, `None` where `xmlns=""` unbinds the default.
#[derive(Debug, Clone)]
struct Binding {
    prefix: Option<String>,
    uri: Option<String>,
}

/// An open element: its node, its qualified name, and how many bindings
/// were in scope before it.
#[derive(Debug)]
struct Open {
    node: NodeId,
    name: String,
    bindings: usize,
    /// The line its start tag ended on.
    line: u32,
}

/// Where the root element stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    Before,
    Inside,
    After,
}

struct Parser {
    document: Document,
    entities: HashMap<String, Entity>,
    /// Characters entities have expanded to, and how many they may.
    expanded: usize,
    budget: usize,
    /// The entities being expanded, outermost first.
    entity_stack: Vec<String>,
    open: Vec<Open>,
    bindings: Vec<Binding>,
    root: Root,
    /// Where in the document the entity being expanded was referenced:
    /// what an error inside it reports.
    reference: Option<(u32, u32)>,
    /// The script elements closed and not yet reported, with the line
    /// each start tag ended on.
    scripts: VecDeque<(NodeId, u32)>,
    /// The doctype names one of XHTML's DTDs: HTML's named character
    /// references stand for what they do in HTML.
    xhtml_entities: bool,
    /// The most elements open at once: [`MAX_XML_DEPTH`], or fewer for a
    /// fragment going deep in its document.
    max_depth: usize,
    /// The custom elements a document's parse constructs (D70).
    custom: CustomParse,
}

/// What the XML parser keeps of the custom element it constructs (HTML
/// section 13.2.6.1, "create an element for a token", which an XML
/// document's parser follows with the synchronous custom elements flag
/// set, D70).
#[derive(Debug, Default)]
struct CustomParse {
    /// Whether it constructs them at all: a document's parse with scripts
    /// to run them.
    construct: bool,
    /// The start tag it stopped at, read ahead.
    held: Option<HeldTag>,
    /// The element constructed for it.
    provided: Option<NodeId>,
    /// Whether the provided element has the tag's attributes already, the
    /// reactions they queued run before it is inserted.
    attributes: bool,
}

/// A start tag as read: its name, its attributes, and whether it is an
/// empty-element tag.
type StartTag = (String, Vec<(String, String)>, bool);

/// A start tag read ahead for the custom element it makes.
#[derive(Debug)]
struct HeldTag {
    local: String,
    is: Option<String>,
    prefix: Option<String>,
    attrs: Vec<(String, String)>,
}

/// The public identifiers of the DTDs whose entities the reference
/// browser knows without fetching them: HTML's named references.
const XHTML_DTDS: [&str; 11] = [
    "-//W3C//DTD XHTML 1.0 Transitional//EN",
    "-//W3C//DTD XHTML 1.1//EN",
    "-//W3C//DTD XHTML 1.0 Strict//EN",
    "-//W3C//DTD XHTML 1.0 Frameset//EN",
    "-//W3C//DTD XHTML Basic 1.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0 plus SVG 1.1//EN",
    "-//W3C//DTD MathML 2.0//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.0//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.1//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.2//EN",
];

/// What HTML's named character reference `name;` stands for.
fn html_entity(name: &str) -> Option<&'static str> {
    let key = format!("{name};");
    crate::named_references::NAMED_REFERENCES
        .binary_search_by(|(n, _)| (*n).cmp(key.as_str()))
        .ok()
        .map(|at| crate::named_references::NAMED_REFERENCES[at].1)
}

/// A place in a text: its byte offset, and its line and column.
#[derive(Clone)]
struct Cursor<'a> {
    text: &'a str,
    pos: usize,
    line: u32,
    column: u32,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Cursor<'a> {
        Cursor {
            text,
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn skip(&mut self, s: &str) -> bool {
        if self.starts_with(s) {
            for _ in s.chars() {
                self.bump();
            }
            true
        } else {
            false
        }
    }

    /// Skips white space (XML's `S`); whether any was there.
    fn space(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(' ' | '\t' | '\n')) {
            self.bump();
        }
        self.pos > start
    }

    /// An error here, or, inside an entity, at its reference.
    fn error(&self, parser: &Parser, message: String) -> XmlError {
        let (line, column) = parser.reference.unwrap_or((self.line, self.column));
        XmlError {
            line,
            column,
            message,
        }
    }

    /// Takes the text up to `end`, and `end`; an error if it never comes.
    fn until(&mut self, parser: &Parser, end: &str, what: &str) -> Result<&'a str, XmlError> {
        let Some(at) = self.rest().find(end) else {
            return Err(self.error(parser, format!("{what} not terminated")));
        };
        let start = self.pos;
        let stop = self.pos + at;
        while self.pos < stop {
            self.bump();
        }
        self.skip(end);
        Ok(&self.text[start..stop])
    }

    /// A name (XML's `Name`), or an error.
    fn name(&mut self, parser: &Parser, what: &str) -> Result<&'a str, XmlError> {
        let start = self.pos;
        match self.peek() {
            Some(c) if is_name_start(c) => {
                self.bump();
            }
            _ => return Err(self.error(parser, format!("{what}: name expected"))),
        }
        while self.peek().is_some_and(is_name_char) {
            self.bump();
        }
        Ok(&self.text[start..self.pos])
    }
}

/// XML's `NameStartChar`.
fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML's `NameChar`.
fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// XML's `Char`: what a document may hold.
fn is_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

impl Parser {
    /// The prolog: the XML declaration, comments, processing
    /// instructions and white space, and the document type declaration.
    fn prolog(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        if c.starts_with("<?xml") && c.rest()[5..].starts_with([' ', '\t', '\n', '?']) {
            c.until(self, "?>", "XML declaration")?;
        }
        loop {
            c.space();
            if c.starts_with("<!--") {
                self.comment(c, true)?;
            } else if c.starts_with("<?") {
                self.processing_instruction(c, true)?;
            } else if c.starts_with("<!DOCTYPE") {
                self.doctype(c)?;
            } else {
                return Ok(());
            }
        }
    }

    /// A comment, made a node when `keep`.
    fn comment(&mut self, c: &mut Cursor, keep: bool) -> Result<(), XmlError> {
        c.skip("<!--");
        let body = c.until(self, "-->", "Comment")?;
        if body.contains("--") || body.ends_with('-') {
            return Err(c.error(self, "Double hyphen within comment".to_owned()));
        }
        if keep {
            let node = self.document.create_comment(body);
            self.insert(c, node)?;
        }
        Ok(())
    }

    /// A processing instruction, made a node when `keep`.
    fn processing_instruction(&mut self, c: &mut Cursor, keep: bool) -> Result<(), XmlError> {
        c.skip("<?");
        let target = c.name(self, "Processing instruction")?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(c.error(
                self,
                "XML declaration allowed only at the start of the document".to_owned(),
            ));
        }
        c.space();
        let data = c.until(self, "?>", "Processing instruction")?;
        if keep {
            let node = self.document.create_processing_instruction(target, data);
            self.insert(c, node)?;
        }
        Ok(())
    }

    /// Where the next node goes: in the open element, a template's
    /// contents for an HTML `template`, or the document.
    fn parent(&self) -> NodeId {
        self.open.last().map_or(self.document.root(), |open| {
            self.document
                .template_content(open.node)
                .unwrap_or(open.node)
        })
    }

    /// Appends the node just made where the next node goes.
    fn insert(
        &mut self,
        c: &Cursor,
        node: Result<NodeId, dom::MutationError>,
    ) -> Result<(), XmlError> {
        let parent = self.parent();
        node.and_then(|node| self.document.append_new(parent, node))
            .map_err(|_| c.error(self, "the document is too large".to_owned()))
    }

    /// `<!DOCTYPE name [external id] [internal subset]>`: the subset's
    /// general entities are kept, the rest skipped.
    fn doctype(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("<!DOCTYPE");
        if !c.space() {
            return Err(c.error(self, "Space required after '<!DOCTYPE'".to_owned()));
        }
        let name = c.name(self, "DOCTYPE")?;
        c.space();
        let (mut public_id, mut system_id) = ("", "");
        if c.starts_with("SYSTEM") || c.starts_with("PUBLIC") {
            (public_id, system_id) = self.external_id(c)?;
            c.space();
        }
        self.xhtml_entities = XHTML_DTDS.contains(&public_id);
        let node = self.document.create_doctype(name, public_id, system_id);
        if c.skip("[") {
            self.internal_subset(c)?;
            c.space();
        }
        if !c.skip(">") {
            return Err(c.error(self, "DOCTYPE improperly terminated".to_owned()));
        }
        self.insert(c, node)
    }

    /// `SYSTEM "..."` or `PUBLIC "..." "..."` (XML 1.0's `ExternalID`,
    /// whose system literal follows a public one, empty or not): the
    /// public and system identifiers.
    fn external_id<'a>(&mut self, c: &mut Cursor<'a>) -> Result<(&'a str, &'a str), XmlError> {
        let public = c.skip("PUBLIC");
        if !public {
            c.skip("SYSTEM");
        }
        c.space();
        let first = self.literal(c)?;
        if !public {
            return Ok(("", first));
        }
        c.space();
        Ok((first, self.literal(c)?))
    }

    /// A quoted literal's content.
    fn literal<'a>(&mut self, c: &mut Cursor<'a>) -> Result<&'a str, XmlError> {
        match c.bump() {
            Some('"') => c.until(self, "\"", "Literal"),
            Some('\'') => c.until(self, "'", "Literal"),
            _ => Err(c.error(self, "SystemLiteral \" or ' expected".to_owned())),
        }
    }

    fn internal_subset(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        loop {
            c.space();
            if c.skip("]") {
                return Ok(());
            }
            if c.at_end() {
                return Err(c.error(self, "internal subset not terminated".to_owned()));
            }
            if c.starts_with("<!--") {
                self.comment(c, false)?;
            } else if c.starts_with("<?") {
                self.processing_instruction(c, false)?;
            } else if c.skip("<!ENTITY") {
                self.entity_declaration(c)?;
            } else if c.starts_with("<!") {
                self.skip_declaration(c)?;
            } else if c.skip("%") {
                c.name(self, "PEReference")?;
                if !c.skip(";") {
                    return Err(c.error(self, "PEReference: expecting ';'".to_owned()));
                }
            } else {
                return Err(c.error(self, "internal subset: markup expected".to_owned()));
            }
        }
    }

    /// A markup declaration this parser does not keep (`ELEMENT`,
    /// `ATTLIST`, `NOTATION`), skipped to its `>`, quotes respected.
    fn skip_declaration(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("<!");
        loop {
            match c.bump() {
                Some('>') => return Ok(()),
                Some(q @ ('"' | '\'')) => {
                    c.until(self, if q == '"' { "\"" } else { "'" }, "Literal")?;
                }
                Some(_) => {}
                None => return Err(c.error(self, "declaration not terminated".to_owned())),
            }
        }
    }

    /// `<!ENTITY [%] name value-or-external-id [NDATA n]>`: a general
    /// entity kept, its character references replaced now (section
    /// 4.5); the first declaration of a name wins.
    fn entity_declaration(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        if !c.space() {
            return Err(c.error(self, "Space required after '<!ENTITY'".to_owned()));
        }
        let parameter = c.skip("%");
        if parameter {
            c.space();
        }
        let name = c.name(self, "Entity")?.to_owned();
        c.space();
        let entity = if matches!(c.peek(), Some('"' | '\'')) {
            let value = self.literal(c)?;
            Entity::Internal(self.replace_char_refs(c, value)?)
        } else {
            self.external_id(c)?;
            c.space();
            if c.skip("NDATA") {
                c.space();
                c.name(self, "NDATA")?;
            }
            Entity::External
        };
        c.space();
        if !c.skip(">") {
            return Err(c.error(self, "EntityDecl: entity not terminated".to_owned()));
        }
        if !parameter {
            self.entities.entry(name).or_insert(entity);
        }
        Ok(())
    }

    /// An entity value with its character references replaced.
    fn replace_char_refs(&mut self, c: &Cursor, value: &str) -> Result<String, XmlError> {
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(at) = rest.find("&#") {
            out.push_str(&rest[..at]);
            let after = &rest[at + 2..];
            let Some(end) = after.find(';') else {
                return Err(c.error(self, "CharRef: invalid decimal value".to_owned()));
            };
            out.push(self.char_ref(c, &after[..end])?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The character a reference's digits (`123` or `x7B`) name.
    fn char_ref(&self, c: &Cursor, digits: &str) -> Result<char, XmlError> {
        let value = match digits.strip_prefix('x') {
            Some(hex) if !hex.is_empty() && hex.chars().all(|d| d.is_ascii_hexdigit()) => {
                u32::from_str_radix(hex, 16).ok()
            }
            None if !digits.is_empty() && digits.chars().all(|d| d.is_ascii_digit()) => {
                digits.parse().ok()
            }
            _ => None,
        };
        value
            .and_then(char::from_u32)
            .filter(|&ch| is_char(ch))
            .ok_or_else(|| {
                c.error(
                    self,
                    format!("xmlParseCharRef: invalid xmlChar value {digits}"),
                )
            })
    }

    /// Content: the root element and the rest, or an entity's
    /// replacement text where it is referenced.
    fn content(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        while !c.at_end() {
            self.item(c)?;
        }
        Ok(())
    }

    /// One item of content: a tag, a comment, a CDATA section, a
    /// processing instruction, a reference or a run of character data.
    fn item(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        if c.starts_with("</") {
            self.end_tag(c)
        } else if c.starts_with("<!--") {
            self.comment(c, true)
        } else if c.starts_with("<![CDATA[") {
            if self.open.is_empty() {
                return Err(c.error(self, "Start tag expected, '<' not found".to_owned()));
            }
            c.skip("<![CDATA[");
            let text = c.until(self, "]]>", "CData section")?;
            self.check_chars(c, text)?;
            self.cdata(c, text)
        } else if c.starts_with("<?") {
            self.processing_instruction(c, true)
        } else if c.starts_with("<!") {
            Err(c.error(self, "StartTag: invalid element name".to_owned()))
        } else if c.starts_with("<") {
            self.start_tag(c)
        } else if c.starts_with("&") {
            self.reference(c)
        } else {
            self.char_data(c)
        }
    }

    /// Content at the document's level, at most `slice` items, stopping
    /// after a script element is closed: one an entity's text held, once
    /// the entity is expanded.
    fn content_slice(&mut self, c: &mut Cursor, slice: usize) -> Result<Option<XmlStop>, XmlError> {
        for _ in 0..slice {
            if let Some((element, line)) = self.scripts.pop_front() {
                return Ok(Some(XmlStop::Script { element, line }));
            }
            if c.at_end() {
                return Ok(None);
            }
            if self.custom.construct
                && let Some(stop) = self.custom_stop(c)
            {
                return Ok(Some(stop));
            }
            let item = self.item(c);
            if let Some(unused) = self.custom.provided.take()
                && self.document.parent(unused).is_none()
            {
                let _ = self.document.free(unused);
            }
            self.custom.held = None;
            self.custom.attributes = false;
            item?;
            if self.custom.construct && self.document.has_reactions() {
                return Ok(Some(XmlStop::Reactions));
            }
        }
        Ok(self
            .scripts
            .pop_front()
            .map(|(element, line)| XmlStop::Script { element, line }))
    }

    /// The checks at the document's end: no element left open, and a
    /// root element met.
    fn finished(&self, c: &Cursor) -> Result<(), XmlError> {
        if let Some(open) = self.open.last() {
            return Err(c.error(self, format!("Premature end of data in tag {}", open.name)));
        }
        if self.root == Root::Before {
            return Err(c.error(self, "Start tag expected, '<' not found".to_owned()));
        }
        Ok(())
    }

    /// Character data up to the next markup.
    fn char_data(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        let start = c.pos;
        while let Some(ch) = c.peek() {
            if ch == '<' || ch == '&' {
                break;
            }
            if c.starts_with("]]>") {
                return Err(c.error(self, "Sequence ']]>' not allowed in content".to_owned()));
            }
            if !is_char(ch) {
                return Err(c.error(self, format!("PCDATA invalid Char value {}", u32::from(ch))));
            }
            c.bump();
        }
        let text = &c.text[start..c.pos];
        if self.open.is_empty() {
            if text.chars().all(|ch| matches!(ch, ' ' | '\t' | '\n')) {
                return Ok(());
            }
            return Err(c.error(
                self,
                match self.root {
                    Root::After => "Extra content at the end of the document",
                    _ => "Start tag expected, '<' not found",
                }
                .to_owned(),
            ));
        }
        self.text(c, text)
    }

    fn check_chars(&self, c: &Cursor, text: &str) -> Result<(), XmlError> {
        match text.chars().find(|&ch| !is_char(ch)) {
            Some(ch) => Err(c.error(self, format!("PCDATA invalid Char value {}", u32::from(ch)))),
            None => Ok(()),
        }
    }

    /// Appends text to the open element, joining a text node that ends
    /// it.
    fn text(&mut self, c: &Cursor, text: &str) -> Result<(), XmlError> {
        if text.is_empty() {
            return Ok(());
        }
        if self.open.is_empty() {
            return Ok(());
        }
        let parent = self.parent();
        if let Some(last) = self.document.last_child(parent)
            && self.document.is_text(last)
            && !self.document.is_cdata(last)
        {
            return self
                .document
                .append_data(last, text)
                .map_err(|_| c.error(self, "the document is too large".to_owned()));
        }
        let node = self
            .document
            .create_text(text)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        self.document
            .append_new(parent, node)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))
    }

    /// A CDATA section, a node of its own in the open element, joined to
    /// no text beside it.
    fn cdata(&mut self, c: &Cursor, text: &str) -> Result<(), XmlError> {
        let parent = self.parent();
        let node = self
            .document
            .create_cdata(text)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        self.document
            .append_new(parent, node)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))
    }

    /// A reference in content: a character, a predefined entity, or a
    /// declared one, whose replacement text is parsed as content where
    /// it stands.
    fn reference(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        let at = (c.line, c.column);
        c.skip("&");
        if c.skip("#") {
            let digits = c.until(self, ";", "CharRef")?;
            let ch = self.char_ref(c, digits)?;
            if self.open.is_empty() {
                return Err(c.error(self, "Start tag expected, '<' not found".to_owned()));
            }
            return self.text(c, ch.encode_utf8(&mut [0; 4]));
        }
        let name = c.name(self, "EntityRef")?.to_owned();
        if !c.skip(";") {
            return Err(c.error(self, "EntityRef: expecting ';'".to_owned()));
        }
        if self.open.is_empty() {
            return Err(c.error(self, "Start tag expected, '<' not found".to_owned()));
        }
        if let Some(ch) = predefined(&name) {
            return self.text(c, ch);
        }
        let replacement = match self.entities.get(&name) {
            Some(Entity::Internal(text)) => text.clone(),
            Some(Entity::External) => return Ok(()),
            None => match html_entity(&name).filter(|_| self.xhtml_entities) {
                Some(text) => return self.text(c, text),
                None => return Err(c.error(self, format!("Entity '{name}' not defined"))),
            },
        };
        let (text, outer) = self.enter_entity(c, &name, replacement, at)?;
        let depth = self.open.len();
        let mut inner = Cursor::new(&text);
        let result = self.content(&mut inner).and_then(|()| {
            if self.open.len() == depth {
                Ok(())
            } else {
                Err(inner.error(self, format!("Entity '{name}' is not balanced content")))
            }
        });
        self.leave_entity(outer);
        result
    }

    /// Enters entity `name` referenced at `at`: an error if it refers to
    /// itself, nests too deep or expands past the budget.
    fn enter_entity(
        &mut self,
        c: &Cursor,
        name: &str,
        replacement: String,
        at: (u32, u32),
    ) -> Result<(String, Option<(u32, u32)>), XmlError> {
        if self.entity_stack.iter().any(|open| open == name) {
            return Err(c.error(self, "Detected an entity reference loop".to_owned()));
        }
        if self.entity_stack.len() >= MAX_ENTITY_NESTING {
            return Err(c.error(self, "Maximum entity nesting depth exceeded".to_owned()));
        }
        self.expanded += replacement.len();
        if self.expanded > self.budget {
            return Err(c.error(
                self,
                "Maximum entity amplification factor exceeded".to_owned(),
            ));
        }
        self.entity_stack.push(name.to_owned());
        let outer = self.reference;
        if self.reference.is_none() {
            self.reference = Some(at);
        }
        Ok((replacement, outer))
    }

    fn leave_entity(&mut self, outer: Option<(u32, u32)>) {
        self.entity_stack.pop();
        self.reference = outer;
    }

    /// A start tag: its attributes read and normalized, its namespace
    /// bindings taken, the element made in its namespace; an empty one
    /// closed at once.
    fn start_tag(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        if self.root == Root::After && self.open.is_empty() {
            return Err(c.error(self, "Extra content at the end of the document".to_owned()));
        }
        let (name, attrs, empty) = self.read_start_tag(c)?;
        self.open_element(c, name, attrs)?;
        if empty {
            return self.close_element(c);
        }
        Ok(())
    }

    /// Before a start tag that makes a custom element a definition is
    /// known for, the stop for its construction; once it is constructed,
    /// the tag's attributes given it and the stop for the reactions they
    /// queued, before it is inserted ("create an element for a token"
    /// steps 3 to 10). None for any other item, or a tag the parse
    /// errs at, which the item reports.
    fn custom_stop(&mut self, c: &Cursor) -> Option<XmlStop> {
        if let Some(held) = &self.custom.held {
            let element = self.custom.provided?;
            if self.custom.attributes {
                return None;
            }
            self.custom.attributes = true;
            for (name, value) in &held.attrs {
                let _ = self.document.set_attribute(element, name, value);
            }
            return self.document.has_reactions().then_some(XmlStop::Reactions);
        }
        if !c.starts_with("<") || c.starts_with("</") || c.starts_with("<!") || c.starts_with("<?")
        {
            return None;
        }
        // The tag read ahead, the entity budget left as it was.
        let mut ahead = c.clone();
        let expanded = self.expanded;
        let read = self.read_start_tag(&mut ahead);
        self.expanded = expanded;
        let (name, attrs, _) = read.ok()?;
        let declared = |key: &str| {
            attrs
                .iter()
                .find(|(attr, _)| attr == key)
                .map(|(_, value)| value.clone())
        };
        let uri = match name.split_once(':') {
            Some((prefix, _)) => {
                declared(&format!("xmlns:{prefix}")).or_else(|| self.bound(Some(prefix)))
            }
            None => declared("xmlns")
                .or_else(|| self.bound(None))
                .filter(|uri| !uri.is_empty()),
        };
        if Namespace::from_uri(uri.as_deref()) != Namespace::Html
            || declared("customelementregistry").is_some()
        {
            return None;
        }
        let local = local(&name).to_owned();
        let prefix = name.split_once(':').map(|(prefix, _)| prefix.to_owned());
        let is = declared("is");
        let registry = self.document.custom_element_registry(self.parent());
        self.document
            .look_up_custom_element(registry, &Namespace::Html, &local, is.as_deref())?;
        self.custom.held = Some(HeldTag {
            local,
            is,
            prefix,
            attrs,
        });
        Some(XmlStop::Construct)
    }

    /// A start tag read: its name, its attributes, and whether it is an
    /// empty-element tag.
    fn read_start_tag(&mut self, c: &mut Cursor) -> Result<StartTag, XmlError> {
        c.skip("<");
        let name = c.name(self, "StartTag")?.to_owned();
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let spaced = c.space();
            if c.skip("/>") {
                return Ok((name, attrs, true));
            }
            if c.skip(">") {
                return Ok((name, attrs, false));
            }
            if !spaced {
                return Err(c.error(self, "attributes construct error".to_owned()));
            }
            let attr = c.name(self, "AttValue")?.to_owned();
            c.space();
            if !c.skip("=") {
                return Err(c.error(
                    self,
                    format!("Specification mandates value for attribute {attr}"),
                ));
            }
            c.space();
            let value = self.attribute_value(c)?;
            if attrs.iter().any(|(existing, _)| *existing == attr) {
                return Err(c.error(self, format!("Attribute {attr} redefined")));
            }
            attrs.push((attr, value));
        }
    }

    /// A quoted attribute value, normalized (section 3.3.3): references
    /// replaced, white space a space each, a `<` an error.
    fn attribute_value(&mut self, c: &mut Cursor) -> Result<String, XmlError> {
        let quote = match c.bump() {
            Some(q @ ('"' | '\'')) => q,
            _ => return Err(c.error(self, "AttValue: \" or ' expected".to_owned())),
        };
        let mut out = String::new();
        loop {
            match c.peek() {
                None => return Err(c.error(self, "AttValue: ' expected".to_owned())),
                Some(ch) if ch == quote => {
                    c.bump();
                    return Ok(out);
                }
                Some('<') => {
                    return Err(c.error(
                        self,
                        "Unescaped '<' not allowed in attributes values".to_owned(),
                    ));
                }
                Some('&') => {
                    let at = (c.line, c.column);
                    c.bump();
                    self.attribute_reference(c, at, &mut out)?;
                }
                Some(' ' | '\t' | '\n') => {
                    c.bump();
                    out.push(' ');
                }
                Some(ch) if !is_char(ch) => {
                    return Err(c.error(
                        self,
                        format!("invalid character in attribute value {}", u32::from(ch)),
                    ));
                }
                Some(ch) => {
                    c.bump();
                    out.push(ch);
                }
            }
        }
    }

    /// A reference in an attribute value, its replacement normalized into
    /// `out`; the cursor is after the `&`.
    fn attribute_reference(
        &mut self,
        c: &mut Cursor,
        at: (u32, u32),
        out: &mut String,
    ) -> Result<(), XmlError> {
        if c.skip("#") {
            let digits = c.until(self, ";", "CharRef")?;
            out.push(self.char_ref(c, digits)?);
            return Ok(());
        }
        let name = c.name(self, "EntityRef")?.to_owned();
        if !c.skip(";") {
            return Err(c.error(self, "EntityRef: expecting ';'".to_owned()));
        }
        if let Some(ch) = predefined(&name) {
            out.push_str(ch);
            return Ok(());
        }
        let replacement = match self.entities.get(&name) {
            Some(Entity::Internal(text)) => text.clone(),
            Some(Entity::External) => {
                return Err(c.error(
                    self,
                    format!("Attribute references external entity '{name}'"),
                ));
            }
            None => match html_entity(&name).filter(|_| self.xhtml_entities) {
                Some(text) => {
                    out.push_str(text);
                    return Ok(());
                }
                None => return Err(c.error(self, format!("Entity '{name}' not defined"))),
            },
        };
        let (text, outer) = self.enter_entity(c, &name, replacement, at)?;
        let mut inner = Cursor::new(&text);
        let result = (|| {
            while let Some(ch) = inner.peek() {
                match ch {
                    '<' => {
                        return Err(inner.error(
                            self,
                            "'<' in entity is not allowed in attributes values".to_owned(),
                        ));
                    }
                    '&' => {
                        let at = (inner.line, inner.column);
                        inner.bump();
                        self.attribute_reference(&mut inner, at, out)?;
                    }
                    ' ' | '\t' | '\n' => {
                        inner.bump();
                        out.push(' ');
                    }
                    _ => {
                        inner.bump();
                        out.push(ch);
                    }
                }
            }
            Ok(())
        })();
        self.leave_entity(outer);
        result
    }

    /// Opens element `name` with `attrs`: its `xmlns` attributes bind
    /// prefixes for it and its content; its prefix and its attributes'
    /// must be bound.
    fn open_element(
        &mut self,
        c: &Cursor,
        name: String,
        attrs: Vec<(String, String)>,
    ) -> Result<(), XmlError> {
        if self.open.len() >= self.max_depth {
            return Err(c.error(
                self,
                format!("Excessive depth in document: {}", self.max_depth),
            ));
        }
        let bindings = self.bindings.len();
        for (attr, value) in &attrs {
            let prefix = if attr == "xmlns" {
                None
            } else if let Some(prefix) = attr.strip_prefix("xmlns:") {
                // A declaration's prefix is a name (Namespaces in XML 1.0
                // section 3, `PrefixedAttName`).
                if prefix.is_empty() {
                    return Err(c.error(self, "Failed to parse QName 'xmlns:'".to_owned()));
                }
                if value.is_empty() {
                    return Err(c.error(
                        self,
                        format!("xmlns:{prefix}: Empty XML namespace is not allowed"),
                    ));
                }
                if prefix == "xmlns" || (prefix == "xml") != (value == XML_NAMESPACE) {
                    return Err(c.error(
                        self,
                        format!("xmlns:{prefix}: reserved prefix or namespace"),
                    ));
                }
                Some(prefix.to_owned())
            } else {
                continue;
            };
            if value == XMLNS_NAMESPACE {
                return Err(c.error(
                    self,
                    "xmlns: the XMLNS namespace may not be bound".to_owned(),
                ));
            }
            self.bindings.push(Binding {
                prefix,
                uri: (!value.is_empty()).then(|| value.clone()),
            });
        }
        let ns = match name.split_once(':') {
            Some((prefix, _)) => match self.bound(Some(prefix)) {
                Some(uri) => Namespace::from_uri(Some(&uri)),
                None => {
                    return Err(c.error(
                        self,
                        format!(
                            "Namespace prefix {prefix} on {} is not defined",
                            local(&name)
                        ),
                    ));
                }
            },
            None => Namespace::from_uri(self.bound(None).as_deref()),
        };
        // Each attribute's prefix bound, and no two of one expanded name.
        let mut expanded: Vec<(Option<String>, &str)> = Vec::new();
        for (attr, _) in &attrs {
            let key = match attr.split_once(':') {
                Some(("xmlns", _)) => (Some(XMLNS_NAMESPACE.to_owned()), local(attr)),
                Some((prefix, local_name)) => {
                    match self.bound(Some(prefix)) {
                        Some(uri) => (Some(uri), local_name),
                        None => {
                            return Err(c.error(self, format!("Namespace prefix {prefix} for {local_name} on {} is not defined", local(&name))));
                        }
                    }
                }
                None if attr == "xmlns" => (Some(XMLNS_NAMESPACE.to_owned()), attr.as_str()),
                None => (None, attr.as_str()),
            };
            if expanded.contains(&key) {
                return Err(c.error(self, format!("Namespaced Attribute {} redefined", key.1)));
            }
            expanded.push(key);
        }
        let namespaces: Vec<Option<String>> = expanded.into_iter().map(|(ns, _)| ns).collect();
        let parent = self.parent();
        // The element a definition constructed for the tag (D70) takes
        // its attributes, then its prefix, and goes in only where it may.
        let provided = self.custom.provided.take().filter(|&id| {
            ns == Namespace::Html && self.document.local_name(id) == Some(local(&name))
        });
        let node = match provided {
            Some(id) => {
                if !self.custom.attributes {
                    for (attr, value) in &attrs {
                        let _ = self.document.set_attribute(id, attr, value);
                    }
                }
                if let Some((prefix, _)) = name.split_once(':') {
                    let _ = self.document.set_element_prefix(id, prefix);
                }
                id
            }
            None => {
                let registry_null = ns == Namespace::Html
                    && attrs
                        .iter()
                        .any(|(attr, _)| attr == "customelementregistry");
                let node = self
                    .document
                    .create_element_ns(ns, &name, attrs)
                    .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
                self.document.mark_prefixed(node);
                // `customelementregistry`: it has no registry, nor what goes
                // in it, until one is given it (D70).
                if registry_null {
                    self.document.keep_custom_element_registry_null(node);
                }
                node
            }
        };
        // Each attribute in the namespace its prefix is bound to.
        self.document
            .set_attribute_namespaces(node, namespaces)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        let inserted = match provided {
            Some(_) => self.document.append_child(parent, node),
            None => self.document.append_new(parent, node),
        };
        inserted.map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        if self.open.is_empty() {
            self.root = Root::Inside;
        }
        self.open.push(Open {
            node,
            name,
            bindings,
            line: c.line,
        });
        Ok(())
    }

    /// The URI `prefix` (`None` for the default namespace) is bound to.
    fn bound(&self, prefix: Option<&str>) -> Option<String> {
        self.bindings
            .iter()
            .rev()
            .find(|binding| binding.prefix.as_deref() == prefix)
            .and_then(|binding| binding.uri.clone())
    }

    /// Closes the innermost open element.
    fn close_element(&mut self, c: &Cursor) -> Result<(), XmlError> {
        let Some(open) = self.open.pop() else {
            return Err(c.error(self, "unexpected end tag".to_owned()));
        };
        self.bindings.truncate(open.bindings);
        if self.open.is_empty() {
            self.root = Root::After;
        }
        let script = match self.document.element_name(open.node) {
            Some((Namespace::Html, name)) => name == "script",
            Some((Namespace::Svg, name)) => local(name) == "script",
            _ => false,
        };
        if script {
            self.scripts.push_back((open.node, open.line));
        }
        Ok(())
    }

    /// An end tag, which must name the open element.
    fn end_tag(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("</");
        let name = c.name(self, "EndTag")?.to_owned();
        c.space();
        if !c.skip(">") {
            return Err(c.error(self, "expected '>'".to_owned()));
        }
        match self.open.last() {
            Some(open) if open.name == name => self.close_element(c),
            Some(open) => Err(c.error(
                self,
                format!("Opening and ending tag mismatch: {} and {name}", open.name),
            )),
            None => Err(c.error(self, "Extra content at the end of the document".to_owned())),
        }
    }
}

/// The text of a predefined entity.
fn predefined(name: &str) -> Option<&'static str> {
    match name {
        "lt" => Some("<"),
        "gt" => Some(">"),
        "amp" => Some("&"),
        "apos" => Some("'"),
        "quot" => Some("\""),
        _ => None,
    }
}

/// A qualified name's local part.
fn local(name: &str) -> &str {
    dom::local_part(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::NodeData;

    const SVG: &str = " xmlns=\"http://www.w3.org/2000/svg\"";

    /// The tree as `ns local [attrs]` lines and `"text"`, indented.
    fn dump(source: &str) -> Vec<String> {
        let document = parse_xml_text(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut out = Vec::new();
        for node in document.subtree(document.root()) {
            let depth = document.ancestors(node).count().saturating_sub(1);
            let indent = "  ".repeat(depth);
            match document.data(node) {
                Some(NodeData::Element { tag, attrs, ns }) => {
                    let ns = match ns {
                        Namespace::Svg => "svg".to_owned(),
                        Namespace::Html => "html".to_owned(),
                        Namespace::MathMl => "math".to_owned(),
                        Namespace::Other(uri) => uri.as_deref().unwrap_or("none").to_owned(),
                    };
                    let attrs: Vec<String> =
                        attrs.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
                    out.push(
                        format!("{indent}{ns} {tag} {}", attrs.join(" "))
                            .trim_end()
                            .to_owned(),
                    );
                }
                Some(NodeData::Text(text)) => out.push(format!("{indent}{text:?}")),
                Some(NodeData::Comment(text)) => out.push(format!("{indent}<!--{text}-->")),
                Some(NodeData::ProcessingInstruction { target, data }) => {
                    out.push(format!("{indent}<?{target} {data}?>"));
                }
                _ => {}
            }
        }
        out
    }

    /// The error `source` gives.
    fn error(source: &str) -> String {
        match parse_xml_text(source) {
            Ok(_) => panic!("{source} parsed"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn an_svg_document_is_svgs_its_prefixes_bound_by_their_declarations() {
        assert_eq!(
            dump(&format!(
                "<svg{SVG} xmlns:xlink=\"http://www.w3.org/1999/xlink\"><rect width='1'/>\
                 <use xlink:href='#a'/><x:g xmlns:x='urn:x'><g xmlns=''/></x:g></svg>"
            )),
            [
                "svg svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
                "  svg rect width=\"1\"",
                "  svg use xlink:href=\"#a\"",
                "  urn:x x:g xmlns:x=\"urn:x\"",
                "    none g xmlns=\"\"",
            ]
        );
        // The prefix goes out of scope with its element.
        assert_eq!(
            error(&format!("<svg{SVG}><a xmlns:p='urn:p'/><p:b/></svg>")),
            "error on line 1 at column 67: Namespace prefix p on b is not defined"
        );
        assert!(
            error(&format!("<svg{SVG}><use xlink:href='#a'/></svg>"))
                .contains("Namespace prefix xlink for href on use is not defined")
        );
        // `xml:` is bound from the start.
        assert_eq!(
            dump(&format!("<svg{SVG} xml:space='preserve'/>"))[0],
            "svg svg xmlns=\"http://www.w3.org/2000/svg\" xml:space=\"preserve\""
        );
    }

    #[test]
    fn entities_and_references_expand_and_markup_in_them_is_parsed() {
        // The reference browser's answers (scratchpad m32/p6-xml.html).
        assert_eq!(
            dump(&format!(
                "<!DOCTYPE svg [<!ENTITY a \"AB\"><!ENTITY e \"<g id='i'>&a;</g>\">]>\
                 <svg{SVG}><text>&a;&amp;&#65;&#x42;&lt;</text>&e;</svg>"
            )),
            [
                "svg svg xmlns=\"http://www.w3.org/2000/svg\"",
                "  svg text",
                "    \"AB&AB<\"",
                "  svg g id=\"i\"",
                "    \"AB\"",
            ]
        );
        // Illustrator names its namespace by an entity.
        assert_eq!(
            dump(
                "<!DOCTYPE svg [<!ENTITY ns_svg \"http://www.w3.org/2000/svg\">]>\
                  <svg xmlns=\"&ns_svg;\"><rect/></svg>"
            )[1],
            "  svg rect"
        );
        assert!(
            error(&format!("<svg{SVG}><text>&nope;</text></svg>"))
                .ends_with("Entity 'nope' not defined")
        );
        assert!(
            error(&format!("<svg{SVG}><text>&nbsp;</text></svg>"))
                .ends_with("Entity 'nbsp' not defined")
        );
        assert!(
            error("<!DOCTYPE svg [<!ENTITY a \"&b;\"><!ENTITY b \"&a;\">]><svg>&a;</svg>")
                .ends_with("Detected an entity reference loop")
        );
        assert!(
            error("<!DOCTYPE svg [<!ENTITY e \"<g>\">]><svg>&e;</svg>")
                .contains("is not balanced content")
        );
        // An external subset is not fetched; an external entity stands
        // for nothing in content and is an error in an attribute.
        assert_eq!(
            dump(&format!(
                "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \
                 \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [<!ENTITY x SYSTEM \"x.txt\">]>\
                 <svg{SVG}>a&x;b</svg>"
            ))[1],
            "  \"ab\""
        );
        assert!(
            error(&format!(
                "<!DOCTYPE svg [<!ENTITY x SYSTEM \"x.txt\">]><svg{SVG} id='&x;'/>"
            ))
            .contains("external entity")
        );
    }

    #[test]
    fn the_billion_laughs_is_refused() {
        let mut source = String::from("<!DOCTYPE svg [<!ENTITY a \"aaaaaaaaaa\">");
        let names = ["a", "b", "c", "d", "e", "f", "g"];
        for pair in names.windows(2) {
            source.push_str(&format!(
                "<!ENTITY {} \"{}\">",
                pair[1],
                format!("&{};", pair[0]).repeat(10)
            ));
        }
        source.push_str(&format!("]><svg{SVG}><text>&g;</text></svg>"));
        assert!(error(&source).ends_with("Maximum entity amplification factor exceeded"));
        // Far below the ceiling, but ten times a small document's length
        // and more.
        let small = format!(
            "<!DOCTYPE svg [<!ENTITY a \"{}\">]><svg{SVG}>{}</svg>",
            "x".repeat(1000),
            "&a;".repeat(20)
        );
        assert!(error(&small).ends_with("Maximum entity amplification factor exceeded"));
        // A document using its entities moderately expands.
        assert_eq!(
            dump(&format!(
                "<!DOCTYPE svg [<!ENTITY a \"0123456789\">]><svg{SVG}>{}</svg>",
                "&a;".repeat(20)
            ))[1]
                .len(),
            2 + 200 + 2
        );
    }

    #[test]
    fn cdata_comments_instructions_and_the_declaration() {
        assert_eq!(
            dump(&format!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\
                 <?xml-stylesheet href=\"a.css\"?><!-- c --><svg{SVG}><style><![CDATA[a<b]]>\
                 </style><!-- d --><?pi x?></svg><!-- after -->\n"
            )),
            [
                "<?xml-stylesheet href=\"a.css\"?>",
                "<!-- c -->",
                "svg svg xmlns=\"http://www.w3.org/2000/svg\"",
                "  svg style",
                "    \"a<b\"",
                "  <!-- d -->",
                "  <?pi x?>",
                "<!-- after -->",
            ]
        );
        let document = parse_xml_text(&format!(
            "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x.dtd\" [<!-- no -->]><svg{SVG}/>"
        ))
        .unwrap();
        let doctype = document.doctype().expect("a doctype");
        assert_eq!(
            document.data(doctype),
            Some(&NodeData::DocumentType {
                name: "svg".into(),
                public_id: "-//W3C//DTD SVG 1.1//EN".into(),
                system_id: "x.dtd".into(),
            })
        );
        assert_eq!(
            document.children(document.root()).count(),
            2,
            "no comment from the subset"
        );
        // White space is kept as written; line ends are line feeds;
        // attribute values are normalized, a reference's newline kept.
        assert_eq!(
            dump(&format!(
                "<svg{SVG} id=\"a\r\nb\tc&#10;d\"><text>  a\r\n  b  </text></svg>"
            )),
            [
                "svg svg xmlns=\"http://www.w3.org/2000/svg\" id=\"a b c\\nd\"",
                "  svg text",
                "    \"  a\\n  b  \"",
            ]
        );
    }

    #[test]
    fn a_document_not_well_formed_fails_where_it_stops() {
        let cases = [
            (
                format!("<svg{SVG}><g><rect/></svg>"),
                "Opening and ending tag mismatch: g and svg",
            ),
            (
                format!("<svg{SVG}></g>"),
                "Opening and ending tag mismatch: svg and g",
            ),
            (
                format!("<svg{SVG} width='1' width='2'/>"),
                // As the reference browser words it, not the namespaced
                // check's words.
                ": Attribute width redefined",
            ),
            (format!("<svg{SVG} width=1/>"), "AttValue: \" or ' expected"),
            (
                format!("<svg{SVG} id='a<b'/>"),
                "Unescaped '<' not allowed in attributes values",
            ),
            (
                format!("<svg{SVG}/>junk"),
                "Extra content at the end of the document",
            ),
            (
                format!("<svg{SVG}/><svg{SVG}/>"),
                "Extra content at the end of the document",
            ),
            (format!("<svg{SVG}><g>"), "Premature end of data in tag g"),
            (String::new(), "Start tag expected, '<' not found"),
            ("text".to_owned(), "Start tag expected, '<' not found"),
            (
                format!("<svg{SVG}><!-- a -- b --></svg>"),
                "Double hyphen within comment",
            ),
            (
                format!("<svg{SVG}>a]]>b</svg>"),
                "Sequence ']]>' not allowed in content",
            ),
            (format!("<svg{SVG}>&#0;</svg>"), "invalid xmlChar value 0"),
            (
                format!("<svg{SVG} a:b='1' xmlns:a='urn:a' c:b='2' xmlns:c='urn:a'/>"),
                "Namespaced Attribute b redefined",
            ),
            (
                format!("<svg{SVG}><?xml version='1.0'?></svg>"),
                "XML declaration allowed only at the start",
            ),
        ];
        for (source, message) in cases {
            let error = error(&source);
            assert!(error.contains(message), "{source}: {error}");
        }
        // The line and column of the place it stopped.
        assert_eq!(
            error(&format!("<svg{SVG}>\n  <g>\n  </h>\n</svg>")),
            "error on line 3 at column 7: Opening and ending tag mismatch: g and h"
        );
    }

    #[test]
    fn the_bytes_are_decoded_by_their_mark_or_declaration() {
        let latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><svg><text>\xE9</text></svg>";
        let document = parse_xml(latin).unwrap();
        assert_eq!(document.text_content(document.root()), "\u{e9}");
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<svg><text>\u{e9}</text></svg>".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let document = parse_xml(&utf16).unwrap();
        assert_eq!(document.text_content(document.root()), "\u{e9}");
        let document = parse_xml(b"\xEF\xBB\xBF<svg>x</svg>").unwrap();
        assert_eq!(document.text_content(document.root()), "x");
    }

    #[test]
    fn elements_nest_no_deeper_than_the_bound() {
        let deep = format!(
            "<svg{SVG}>{}{}</svg>",
            "<g>".repeat(MAX_XML_DEPTH),
            "</g>".repeat(MAX_XML_DEPTH)
        );
        assert!(error(&deep).contains("Excessive depth in document"));
        let fits = format!(
            "<svg{SVG}>{}{}</svg>",
            "<g>".repeat(MAX_XML_DEPTH - 1),
            "</g>".repeat(MAX_XML_DEPTH - 1)
        );
        assert!(parse_xml_text(&fits).is_ok());
    }

    const XHTML: &str = " xmlns=\"http://www.w3.org/1999/xhtml\"";

    #[test]
    fn the_parser_stops_after_each_script_with_the_tree_as_it_stands() {
        let source = format!(
            "<html{XHTML}><body><p>a</p>\n<script>x()</script><p>b</p>\
             <svg xmlns='http://www.w3.org/2000/svg'><script/></svg></body></html>"
        );
        let mut parser = XmlParser::new(&source);
        let mut document = Document::new();
        let Ok(XmlStop::Script { element, line }) = parser.parse(&mut document, usize::MAX) else {
            panic!("the script's end tag");
        };
        assert_eq!(document.tag(element), Some("script"));
        assert_eq!(line, 2);
        assert!(document.is_xml_document());
        // Nothing after the script is in the tree yet.
        assert_eq!(document.text_content(document.root()), "a\nx()");
        let Ok(XmlStop::Script { element, .. }) = parser.parse(&mut document, usize::MAX) else {
            panic!("the SVG script");
        };
        assert_eq!(document.svg_tag(element), Some("script"));
        assert_eq!(parser.parse(&mut document, usize::MAX), Ok(XmlStop::Done));
        assert!(parser.is_done());
        assert_eq!(document.text_content(document.root()), "a\nx()b");
        // A slice at a time.
        let mut parser = XmlParser::new(&format!("<html{XHTML}><p>a</p><p>b</p></html>"));
        let mut document = Document::new();
        assert_eq!(parser.parse(&mut document, 2), Ok(XmlStop::Slice));
        assert_eq!(document.text_content(document.root()), "");
        while parser.parse(&mut document, 1) != Ok(XmlStop::Done) {}
        assert_eq!(document.text_content(document.root()), "ab");
    }

    #[test]
    fn an_xhtml_dtd_gives_htmls_named_references() {
        let doctype = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x.dtd\">";
        let document = parse_xml_text(&format!(
            "{doctype}<html{XHTML} title='&eacute;'>&nbsp;&copy;&amp;</html>"
        ))
        .unwrap();
        assert_eq!(
            document.text_content(document.root()),
            "\u{a0}\u{a9}&",
            "in content"
        );
        let html = document.document_element().unwrap();
        assert_eq!(document.attr(html, "title"), Some("\u{e9}"));
        // Without one, or with another, they are not defined.
        assert!(
            error(&format!("<html{XHTML}>&nbsp;</html>")).ends_with("Entity 'nbsp' not defined")
        );
        assert!(
            error(&format!(
                "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\" \"x\"><html{XHTML}>&nbsp;</html>"
            ))
            .ends_with("Entity 'nbsp' not defined")
        );
        // A name HTML has only without its semicolon is none.
        assert!(
            error(&format!("{doctype}<html{XHTML}>&nbspx;</html>"))
                .ends_with("Entity 'nbspx' not defined")
        );
    }

    #[test]
    fn an_error_is_reported_above_what_came_before_it() {
        let mut parser =
            XmlParser::new(&format!("<html{XHTML}><body><p>a</p><i></b></body></html>"));
        let mut document = Document::new();
        let error = parser.parse(&mut document, usize::MAX).unwrap_err();
        insert_error_block(&mut document, &error);
        let html = document.document_element().unwrap();
        let report = document.first_child(html).unwrap();
        assert_eq!(document.tag(report), Some("parsererror"));
        assert_eq!(
            document.text_content(report),
            "This page contains the following errors:error on line 1 at column 65: \
             Opening and ending tag mismatch: i and b\nBelow is a rendering of the page up \
             to the first error."
        );
        // What was parsed stays after it.
        assert_eq!(document.text_content(html).rsplit('.').next(), Some("a"));
        // No root element: one is made; an SVG root goes in its body.
        let mut document = Document::new();
        insert_error_block(&mut document, &error);
        let html = document.document_element().unwrap();
        assert_eq!(document.tag(html), Some("html"));
        let mut document = parse_xml_text(&format!("<svg{SVG}/>")).unwrap();
        insert_error_block(&mut document, &error);
        let html = document.document_element().unwrap();
        let body = document.last_child(html).unwrap();
        let tags: Vec<_> = document
            .children(body)
            .map(|n| document.local_name(n).unwrap_or("").to_owned())
            .collect();
        assert_eq!(tags, ["parsererror", "svg"]);
    }

    #[test]
    fn a_fragment_is_parsed_in_its_contexts_namespaces_and_written_back_as_xml() {
        let mut document = parse_xml_text(&format!(
            "<html{XHTML} xmlns:s='http://www.w3.org/2000/svg'><body/></html>"
        ))
        .unwrap();
        let body = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("body"))
            .unwrap();
        let container = document.create_element("html").unwrap();
        parse_xml_fragment(
            &mut document,
            container,
            body,
            "<P class='a'>x &amp; &lt;</P><br/><s:rect/><q xmlns=''>t</q><!--c-->",
            MAX_XML_DEPTH,
        )
        .unwrap();
        let children: Vec<_> = document.children(container).collect();
        assert_eq!(
            document.element_name(children[0]),
            Some((&Namespace::Html, "P"))
        );
        assert_eq!(document.svg_tag(children[2]), Some("rect"));
        assert_eq!(
            document.namespace(children[3]),
            Some(&Namespace::Other(None))
        );
        while let Some(child) = document.first_child(container) {
            document.remove_child(container, child).unwrap();
            document.append_child(body, child).unwrap();
        }
        // The children are serialized in no namespace's context, each
        // declaring the namespace it is in (DOM Parsing section 3.2).
        assert_eq!(
            serialize_xml_children(&document, body).unwrap(),
            "<P xmlns=\"http://www.w3.org/1999/xhtml\" class=\"a\">x &amp; &lt;</P>\
             <br xmlns=\"http://www.w3.org/1999/xhtml\" />\
             <s:rect xmlns:s=\"http://www.w3.org/2000/svg\"/><q>t</q><!--c-->"
        );
        let html = document.document_element().unwrap();
        assert!(serialize_xml_node(&document, html).unwrap().starts_with(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\" \
             xmlns:s=\"http://www.w3.org/2000/svg\"><body><P"
        ));
        // Not well formed, or with a prefix bound nowhere, it is refused.
        for markup in ["<p>", "</body>", "<x:y/>", "a&nbsp;b"] {
            let container = document.create_element("html").unwrap();
            assert!(
                parse_xml_fragment(&mut document, container, body, markup, MAX_XML_DEPTH).is_err(),
                "{markup}"
            );
        }
        // An empty element of another namespace closes itself, an HTML one
        // does not; an attribute's white space is kept by a reference.
        let div = document.create_element("div").unwrap();
        document.set_attribute(div, "title", "a\nb\"").unwrap();
        assert_eq!(
            serialize_xml_node(&document, div).unwrap(),
            "<div xmlns=\"http://www.w3.org/1999/xhtml\" title=\"a&#xA;b&quot;\"></div>"
        );
    }

    /// The document element of `source`, parsed.
    fn root_of(source: &str) -> (Document, NodeId) {
        let document = parse_xml_text(source).unwrap();
        let root = document.document_element().unwrap();
        (document, root)
    }

    fn serialized(document: &Document, node: NodeId) -> String {
        xml_serialization(document, node, false).unwrap()
    }

    #[test]
    fn the_serialization_declares_or_makes_up_the_prefixes_namespaces_need() {
        // DOM Parsing section 3.2: an attribute of a namespace bound to a
        // prefix takes the nearest, one of another one made up, `ns1`, `ns2`.
        let (mut document, root) =
            root_of("<r xmlns:x0='uri' xmlns:x2='uri'><b xmlns:x1='uri'/></r>");
        let b = document.first_child(root).unwrap();
        document
            .set_attribute_ns(b, Some("uri"), "name", "v")
            .unwrap();
        assert_eq!(
            serialized(&document, root),
            "<r xmlns:x0=\"uri\" xmlns:x2=\"uri\"><b xmlns:x1=\"uri\" x1:name=\"v\"/></r>"
        );
        let (mut document, root) = root_of("<root><child1/><child2/></root>");
        let (one, two) = (
            document.first_child(root).unwrap(),
            document.last_child(root).unwrap(),
        );
        document
            .set_attribute_ns(one, Some("u1"), "a", "1")
            .unwrap();
        document
            .set_attribute_ns(one, Some("u2"), "b", "2")
            .unwrap();
        document
            .set_attribute_ns(two, Some("u3"), "c", "3")
            .unwrap();
        assert_eq!(
            serialized(&document, root),
            "<root><child1 xmlns:ns1=\"u1\" ns1:a=\"1\" xmlns:ns2=\"u2\" ns2:b=\"2\"/>\
             <child2 xmlns:ns3=\"u3\" ns3:c=\"3\"/></root>"
        );
        // A default namespace is declared where an element's differs
        // from what it inherits, and reset for one of none; a prefix the
        // parser bound is the element's.
        let (document, root) = root_of("<p:a xmlns:p='u'><b xmlns='v'><c xmlns=''/></b></p:a>");
        assert_eq!(
            serialized(&document, root),
            "<p:a xmlns:p=\"u\"><b xmlns=\"v\"><c xmlns=\"\"/></b></p:a>"
        );
        // An `xmlns` of no namespace that is not the element's is dropped.
        let (mut document, root) = root_of("<package/>");
        document.set_attribute(root, "xmlns", "urn:opf").unwrap();
        assert_eq!(serialized(&document, root), "<package/>");
    }

    #[test]
    fn a_cdata_section_is_a_node_of_its_own_written_back_as_one() {
        let (document, root) = root_of("<a>x<![CDATA[<b> & ]]><![CDATA[]]>y</a>");
        let kids: Vec<NodeId> = document.children(root).collect();
        assert_eq!(kids.len(), 4, "text, two sections, text");
        assert!(!document.is_cdata(kids[0]) && document.is_cdata(kids[1]));
        assert_eq!(document.node_type(kids[1]), Some(4));
        assert_eq!(document.text(kids[2]), Some(""));
        assert_eq!(document.text_content(root), "x<b> & y");
        assert_eq!(
            serialized(&document, root),
            "<a>x<![CDATA[<b> & ]]><![CDATA[]]>y</a>"
        );
    }

    #[test]
    fn markup_required_well_formed_that_is_not_is_refused() {
        let mut document = Document::new();
        let p = document.create_element("p").unwrap();
        let comment = document.create_comment("a--b").unwrap();
        document.append_child(p, comment).unwrap();
        assert_eq!(serialize_xml_children(&document, p), None);
        assert_eq!(
            xml_serialization(&document, p, false).as_deref(),
            Some("<p xmlns=\"http://www.w3.org/1999/xhtml\"><!--a--b--></p>")
        );
        let q = document.create_element("q").unwrap();
        let text = document.create_text("\u{c}").unwrap();
        document.append_child(q, text).unwrap();
        assert_eq!(serialize_xml_node(&document, q), None);
        // A local name with a colon, an element `createElement` made.
        let colon = document.create_element("a:b").unwrap();
        assert_eq!(serialize_xml_node(&document, colon), None);
        // A document needs its element.
        let empty = Document::new();
        assert_eq!(xml_serialization(&empty, empty.root(), true), None);
    }

    #[test]
    fn a_document_parsed_alone_keeps_what_was_parsed_before_its_error() {
        let (document, error) = parse_xml_detached("<r><a>1</a><b>");
        let error = error.expect("not well formed");
        assert!(error.message.contains("b"), "{error}");
        let root = document.document_element().unwrap();
        assert_eq!(document.text_content(root), "1");
        // A public identifier needs a system literal; a declaration a
        // prefix.
        assert!(parse_xml_text("<!DOCTYPE html PUBLIC \"-//x\"><html/>").is_err());
        assert!(parse_xml_text("<!DOCTYPE html PUBLIC \"-//x\" \"\"><html/>").is_ok());
        assert!(parse_xml_text("<a xmlns:='urn:x'/>").is_err());
        // The parser keeps each attribute's namespace, and the element's
        // prefix.
        let (document, root) = root_of("<p:r xmlns:p='urn:p' p:a='1' xml:lang='en' b='2'/>");
        assert_eq!(document.attr_ns(root, Some("urn:p"), "a"), Some("1"));
        assert_eq!(
            document.attr_ns(root, Some(XML_NAMESPACE), "lang"),
            Some("en")
        );
        assert_eq!(
            document.attr_ns(root, Some(XMLNS_NAMESPACE), "p"),
            Some("urn:p")
        );
        assert_eq!(document.attr_ns(root, None, "b"), Some("2"));
        assert_eq!(document.element_prefix(root), Some("p"));
        assert_eq!(document.local_name(root), Some("r"));
    }

    #[test]
    fn an_xml_document_shows_as_one_with_elements_it_renders_or_a_style_sheet() {
        let shown = |source: &str| has_style_information(&parse_xml_text(source).unwrap());
        assert!(!shown("<root><a/></root>"));
        assert!(shown(&format!("<root><p{XHTML}/></root>")));
        assert!(shown(&format!("<svg{SVG}/>")));
        assert!(shown("<?xml-stylesheet href='a.css'?><root/>"));
        // The transport's encoding comes after a byte order mark.
        let (text, encoding) = decode_xml_with(
            b"<?xml version='1.0' encoding='utf-8'?><r>\xE9</r>",
            Some(encoding::WINDOWS_1252),
        );
        assert_eq!(encoding, encoding::WINDOWS_1252);
        assert!(text.contains('\u{e9}'));
        let (_, encoding) = decode_xml_with(b"\xEF\xBB\xBF<r/>", Some(encoding::WINDOWS_1252));
        assert_eq!(encoding, encoding::UTF_8);
    }
}
