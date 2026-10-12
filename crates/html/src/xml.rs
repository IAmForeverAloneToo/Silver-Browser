//! The XML parser (M32, `docs/design/svg.md`): bytes to a
//! `dom::Document`, as XML 1.0 (Fifth Edition) with Namespaces in XML
//! 1.0 reads them, non-validating, for SVG documents and images.
//!
//! The text is decoded by its byte order mark, else by the encoding its
//! declaration names, else as UTF-8, and its line ends taken to line
//! feeds. Elements go into the document in the namespace their prefix,
//! or the default namespace, is bound to by the `xmlns` attributes in
//! scope (the `xml` prefix bound from the start); attributes keep their
//! qualified names, normalized as section 3.3.3 says; character data and
//! CDATA sections become text, adjacent text joined. Comments,
//! processing instructions and the document type declaration leave no
//! node: the DOM keeps none yet (G51). The internal subset's general
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

use std::collections::HashMap;

use dom::{Document, Namespace, NodeId, XML_NAMESPACE, XMLNS_NAMESPACE};
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
    if let Some((encoding, _)) = encoding::bom_sniff(bytes) {
        return encoding::decode(bytes, encoding).0;
    }
    let declared = declared_encoding(bytes).unwrap_or(encoding::UTF_8);
    encoding::decode(bytes, declared).0
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
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut parser = Parser {
        document: Document::new(),
        entities: HashMap::new(),
        expanded: 0,
        budget: (text.len() * MAX_ENTITY_FACTOR + EXPANSION_SLACK).min(MAX_ENTITY_EXPANSION),
        entity_stack: Vec::new(),
        open: Vec::new(),
        bindings: vec![Binding {
            prefix: Some("xml".to_owned()),
            uri: Some(XML_NAMESPACE.to_owned()),
        }],
        root: Root::Before,
        reference: None,
    };
    let mut cursor = Cursor::new(&text);
    parser.prolog(&mut cursor)?;
    parser.content(&mut cursor)?;
    if let Some(open) = parser.open.last() {
        return Err(cursor.error(
            &parser,
            format!("Premature end of data in tag {}", open.name),
        ));
    }
    if parser.root == Root::Before {
        return Err(cursor.error(&parser, "Start tag expected, '<' not found".to_owned()));
    }
    Ok(parser.document)
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
}

/// A place in a text: its byte offset, and its line and column.
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
                self.comment(c)?;
            } else if c.starts_with("<?") {
                self.processing_instruction(c)?;
            } else if c.starts_with("<!DOCTYPE") {
                self.doctype(c)?;
            } else {
                return Ok(());
            }
        }
    }

    fn comment(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("<!--");
        let body = c.until(self, "-->", "Comment")?;
        if body.contains("--") || body.ends_with('-') {
            return Err(c.error(self, "Double hyphen within comment".to_owned()));
        }
        Ok(())
    }

    fn processing_instruction(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("<?");
        let target = c.name(self, "Processing instruction")?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(c.error(
                self,
                "XML declaration allowed only at the start of the document".to_owned(),
            ));
        }
        c.until(self, "?>", "Processing instruction")?;
        Ok(())
    }

    /// `<!DOCTYPE name [external id] [internal subset]>`: the subset's
    /// general entities are kept, the rest skipped.
    fn doctype(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        c.skip("<!DOCTYPE");
        if !c.space() {
            return Err(c.error(self, "Space required after '<!DOCTYPE'".to_owned()));
        }
        c.name(self, "DOCTYPE")?;
        c.space();
        if c.starts_with("SYSTEM") || c.starts_with("PUBLIC") {
            self.external_id(c)?;
            c.space();
        }
        if c.skip("[") {
            self.internal_subset(c)?;
            c.space();
        }
        if !c.skip(">") {
            return Err(c.error(self, "DOCTYPE improperly terminated".to_owned()));
        }
        Ok(())
    }

    /// `SYSTEM "..."` or `PUBLIC "..." "..."`.
    fn external_id(&mut self, c: &mut Cursor) -> Result<(), XmlError> {
        let public = c.skip("PUBLIC");
        if !public {
            c.skip("SYSTEM");
        }
        c.space();
        self.literal(c)?;
        if public {
            c.space();
            if matches!(c.peek(), Some('"' | '\'')) {
                self.literal(c)?;
            }
        }
        Ok(())
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
                self.comment(c)?;
            } else if c.starts_with("<?") {
                self.processing_instruction(c)?;
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
            if c.starts_with("</") {
                self.end_tag(c)?;
            } else if c.starts_with("<!--") {
                self.comment(c)?;
            } else if c.starts_with("<![CDATA[") {
                if self.open.is_empty() {
                    return Err(c.error(self, "Start tag expected, '<' not found".to_owned()));
                }
                c.skip("<![CDATA[");
                let text = c.until(self, "]]>", "CData section")?;
                self.check_chars(c, text)?;
                self.text(c, text)?;
            } else if c.starts_with("<?") {
                self.processing_instruction(c)?;
            } else if c.starts_with("<!") {
                return Err(c.error(self, "StartTag: invalid element name".to_owned()));
            } else if c.starts_with("<") {
                self.start_tag(c)?;
            } else if c.starts_with("&") {
                self.reference(c)?;
            } else {
                self.char_data(c)?;
            }
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
        let Some(open) = self.open.last() else {
            return Ok(());
        };
        let parent = open.node;
        if let Some(last) = self.document.last_child(parent)
            && self.document.is_text(last)
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
            None => return Err(c.error(self, format!("Entity '{name}' not defined"))),
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
        c.skip("<");
        let name = c.name(self, "StartTag")?.to_owned();
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let spaced = c.space();
            if c.skip("/>") {
                self.open_element(c, name, attrs)?;
                return self.close_element(c);
            }
            if c.skip(">") {
                return self.open_element(c, name, attrs);
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
            None => return Err(c.error(self, format!("Entity '{name}' not defined"))),
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
        if self.open.len() >= MAX_XML_DEPTH {
            return Err(c.error(
                self,
                format!("Excessive depth in document: {MAX_XML_DEPTH}"),
            ));
        }
        let bindings = self.bindings.len();
        for (attr, value) in &attrs {
            let prefix = if attr == "xmlns" {
                None
            } else if let Some(prefix) = attr.strip_prefix("xmlns:") {
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
                None => (None, attr.as_str()),
            };
            if expanded.contains(&key) {
                return Err(c.error(self, format!("Namespaced Attribute {} redefined", key.1)));
            }
            expanded.push(key);
        }
        let parent = self
            .open
            .last()
            .map_or(self.document.root(), |open| open.node);
        let node = self
            .document
            .create_element_ns(ns, &name, attrs)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        self.document
            .append_new(parent, node)
            .map_err(|_| c.error(self, "the document is too large".to_owned()))?;
        if self.open.is_empty() {
            self.root = Root::Inside;
        }
        self.open.push(Open {
            node,
            name,
            bindings,
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
                "svg svg xmlns=\"http://www.w3.org/2000/svg\"",
                "  svg style",
                "    \"a<b\"",
            ]
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
}
