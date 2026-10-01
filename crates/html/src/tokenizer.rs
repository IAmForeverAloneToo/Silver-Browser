//! WHATWG-style HTML tokenizer (pragmatic subset).
//!
//! Splits source text into [`Token`]s: tags with attributes, text runs
//! with character references decoded, comments, and doctypes. Deviations
//! from the full specification, on purpose:
//!
//! * Named character references cover the HTML 4 set (the Latin-1
//!   range, the Greek letters, the arrows, mathematical and typographic
//!   symbols) plus `&apos;` and `&euro;`, not the full HTML5 table of
//!   some two thousand entries. The spec's rule for the Latin-1 and
//!   `amp`/`lt`/`gt`/`quot` names, which may appear without a `;`, is
//!   followed, including the exception inside attribute values that
//!   keeps `?a=1&copy=2` a query string.
//! * Anything malformed degrades to text or a bogus comment instead of
//!   producing spec-accurate parse errors.
//! * CDATA sections and `<script>`/`<style>` raw-text elements are
//!   tokenized like ordinary elements for now.
//!
//! The input is preprocessed as the spec's input stream is: CRLF and a
//! lone CR become LF before tokenizing, so text nodes never carry `\r`.
//!
//! The tokenizer never panics on any input, and its work is linear in
//! the input: every scan consumes what it looks at.

use std::collections::HashSet;

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A doctype declaration, e.g. `<!DOCTYPE html>`.
    Doctype(String),
    /// An opening tag.
    StartTag {
        /// Lowercased tag name.
        name: String,
        /// Attributes with lowercased names, order preserved.
        attrs: Vec<(String, String)>,
        /// Whether the tag ended with `/>`.
        self_closing: bool,
    },
    /// A closing tag.
    EndTag(String),
    /// Character data with character references decoded.
    Text(String),
    /// A comment (including bogus comments).
    Comment(String),
}

/// Splits HTML source into tokens.
pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    /// Set by a start tag whose content is text up to its end tag (the
    /// spec's RAWTEXT, RCDATA and PLAINTEXT states): the next call
    /// reads that text as one token.
    raw_text: Option<RawText>,
}

/// How an element's content is read as text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawText {
    /// Up to the end tag, no character references (`style`, `script`,
    /// `xmp`, `iframe`, `noembed`, `noframes`).
    Raw(String),
    /// Up to the end tag, character references decoded (`textarea`,
    /// `title`).
    Escapable(String),
    /// To the end of the input (`plaintext`).
    ToEnd,
}

/// The raw-text state `name`'s start tag switches to, if any.
fn raw_text_for(name: &str) -> Option<RawText> {
    match name {
        "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" => {
            Some(RawText::Raw(name.to_string()))
        }
        "textarea" | "title" => Some(RawText::Escapable(name.to_string())),
        "plaintext" => Some(RawText::ToEnd),
        _ => None,
    }
}

impl Tokenizer {
    /// Creates a tokenizer over `source`, normalising newlines the way
    /// the spec's input-stream preprocessing does (CRLF and CR to LF).
    pub fn new(source: &str) -> Self {
        let mut input = Vec::with_capacity(source.len());
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                input.push('\n');
            } else {
                input.push(c);
            }
        }
        Tokenizer {
            input,
            pos: 0,
            raw_text: None,
        }
    }

    /// Returns the next token, or `None` at end of input.
    ///
    /// Every call consumes at least one character before returning a
    /// token, so iteration always terminates.
    pub fn next_token(&mut self) -> Option<Token> {
        if self.pos >= self.input.len() {
            return None;
        }
        if let Some(mode) = self.raw_text.take() {
            let text = match &mode {
                RawText::Raw(name) => self.read_raw_until_end_tag(name),
                RawText::Escapable(name) => {
                    decode_entities(&self.read_raw_until_end_tag(name), false)
                }
                RawText::ToEnd => {
                    let rest: String = self.input[self.pos..].iter().collect();
                    self.pos = self.input.len();
                    rest
                }
            };
            if !text.is_empty() {
                return Some(Token::Text(text));
            }
            // Empty content: the end tag follows, read as any other.
            if self.pos >= self.input.len() {
                return None;
            }
        }
        if self.peek() == Some('<')
            && let Some(token) = self.read_markup_declaration()
        {
            if let Token::StartTag { name, .. } = &token {
                self.raw_text = raw_text_for(name);
            }
            return Some(token);
        }
        // A '<' that starts nothing markup-ish is read as literal text.
        Some(self.read_text())
    }

    /// Reads everything up to `</name` (ASCII case-insensitive) followed
    /// by white space, `/` or `>`, or to the end of the input, leaving
    /// the end tag to be read next. The spec's RAWTEXT and RCDATA
    /// states, less the script-data escape dance for `<!--` inside a
    /// script, which this engine does not run anyway.
    fn read_raw_until_end_tag(&mut self, name: &str) -> String {
        let start = self.pos;
        let name: Vec<char> = name.chars().collect();
        while self.pos < self.input.len() {
            if self.input[self.pos] == '<'
                && self.input.get(self.pos + 1) == Some(&'/')
                && name.iter().enumerate().all(|(i, c)| {
                    self.input
                        .get(self.pos + 2 + i)
                        .is_some_and(|d| d.eq_ignore_ascii_case(c))
                })
                && matches!(
                    self.input.get(self.pos + 2 + name.len()),
                    None | Some('\t' | '\n' | '\u{c}' | ' ' | '/' | '>')
                )
            {
                break;
            }
            self.pos += 1;
        }
        self.input[start..self.pos].iter().collect()
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn take_while(&mut self, pred: impl Fn(char) -> bool) -> String {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if pred(c)) {
            self.pos += 1;
        }
        self.input[start..self.pos].iter().collect()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn matches_ahead(&self, at: usize, pattern: &str) -> bool {
        pattern
            .chars()
            .enumerate()
            .all(|(i, c)| self.input.get(at + i) == Some(&c))
    }

    /// Reads a text run up to the next real tag start, decoding
    /// character references.
    fn read_text(&mut self) -> Token {
        let mut raw = String::new();
        while let Some(c) = self.peek() {
            if c == '<' && self.tag_starts_ahead() {
                break;
            }
            self.pos += 1;
            raw.push(c);
        }
        Token::Text(decode_entities(&raw, false))
    }

    /// True when the '<' at the current position begins something
    /// markup-like (tag, end tag, comment, doctype, or a `<?` bogus
    /// comment). Must agree with [`Tokenizer::read_markup_declaration`]
    /// on every case, or a text run could end on a '<' that then reads
    /// as text again: an empty token and no progress.
    fn tag_starts_ahead(&self) -> bool {
        match self.input.get(self.pos + 1) {
            Some(c) => c.is_ascii_alphabetic() || *c == '/' || *c == '!' || *c == '?',
            None => false, // trailing '<' is text
        }
    }

    /// Attempts to read a tag, end tag, comment, or doctype at the
    /// current '<'. Returns `None` when the '<' starts no markup.
    fn read_markup_declaration(&mut self) -> Option<Token> {
        let after = self.input.get(self.pos + 1).copied()?;
        match after {
            '!' => {
                if self.matches_ahead(self.pos + 2, "--") {
                    Some(self.read_comment())
                } else {
                    Some(self.read_bang())
                }
            }
            // The spec's tag-open state: `<?` is a bogus comment (an XML
            // declaration at the top of an XHTML page is the usual one).
            '?' => Some(self.read_bogus_comment()),
            '/' => match self.input.get(self.pos + 2).copied() {
                Some(c) if c.is_ascii_alphabetic() => Some(self.read_end_tag()),
                // `</` at the very end of the input is text, per the
                // spec's eof-before-tag-name rule.
                None => {
                    self.pos += 2;
                    Some(Token::Text("</".to_string()))
                }
                _ => Some(self.read_bogus_comment()),
            },
            c if c.is_ascii_alphabetic() => Some(self.read_start_tag()),
            _ => None,
        }
    }

    /// Reads a `<!-- ... -->` comment (unterminated comments end at EOF).
    /// The abrupt forms `<!-->` and `<!--->` are empty comments, and
    /// `--!>` closes a comment as `-->` does, per the spec's comment
    /// states; without that, one of them would swallow the rest of the
    /// page.
    fn read_comment(&mut self) -> Token {
        self.pos += 4; // "<!--"
        if self.peek() == Some('>') {
            self.pos += 1;
            return Token::Comment(String::new());
        }
        if self.matches_ahead(self.pos, "->") {
            self.pos += 2;
            return Token::Comment(String::new());
        }
        let mut content = String::new();
        while self.pos < self.input.len() {
            if self.matches_ahead(self.pos, "-->") {
                self.pos += 3;
                return Token::Comment(content);
            }
            if self.matches_ahead(self.pos, "--!>") {
                self.pos += 4;
                return Token::Comment(content);
            }
            content.push(self.input[self.pos]);
            self.pos += 1;
        }
        Token::Comment(content)
    }

    /// Reads `<!...>`: a doctype, or a bogus comment when it is not one.
    fn read_bang(&mut self) -> Token {
        self.pos += 2; // "<!"
        let mut content = String::new();
        while self.pos < self.input.len() {
            if self.peek() == Some('>') {
                self.pos += 1;
                break;
            }
            content.push(self.input[self.pos]);
            self.pos += 1;
        }
        if let Some(rest) = content.trim().to_lowercase().strip_prefix("doctype") {
            Token::Doctype(rest.trim().to_string())
        } else {
            Token::Comment(content)
        }
    }

    /// Reads a bogus comment: `</` not followed by a letter, or `<?`.
    fn read_bogus_comment(&mut self) -> Token {
        self.pos += 2; // "</" or "<?"
        let mut content = String::new();
        while self.pos < self.input.len() {
            if self.peek() == Some('>') {
                self.pos += 1;
                break;
            }
            content.push(self.input[self.pos]);
            self.pos += 1;
        }
        Token::Comment(content)
    }

    /// Reads an end tag; anything after the name is discarded (browsers
    /// drop attributes on end tags too).
    fn read_end_tag(&mut self) -> Token {
        self.pos += 2; // "</"
        let name = self.take_while(is_tag_name_char).to_lowercase();
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == '>' {
                break;
            }
        }
        Token::EndTag(name)
    }

    /// Reads a start tag and its attributes. The first of two attributes
    /// with the same name wins; the names seen so far are kept in a set,
    /// so a tag with a hundred thousand attributes costs a hundred
    /// thousand lookups, not a scan of the list for each.
    fn read_start_tag(&mut self) -> Token {
        self.pos += 1; // "<"
        let name = self.take_while(is_tag_name_char).to_lowercase();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut self_closing = false;
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => break, // EOF mid-tag
                Some('>') => {
                    self.pos += 1;
                    break;
                }
                Some('/') => {
                    self.pos += 1;
                    if self.peek() == Some('>') {
                        self.pos += 1;
                        self_closing = true;
                        break;
                    }
                    // Stray '/' — ignore (e.g. `<a / b>`).
                }
                _ => {
                    let (aname, avalue) = self.read_attribute();
                    if !aname.is_empty() && seen.insert(aname.clone()) {
                        attrs.push((aname, avalue));
                    }
                }
            }
        }
        Token::StartTag {
            name,
            attrs,
            self_closing,
        }
    }

    /// Reads one attribute (name, and value when `=` follows).
    fn read_attribute(&mut self) -> (String, String) {
        let name = self
            .take_while(|c| !c.is_whitespace() && c != '=' && c != '>' && c != '/')
            .to_lowercase();
        self.skip_whitespace();
        if self.peek() != Some('=') {
            return (name, String::new());
        }
        self.pos += 1; // '='
        self.skip_whitespace();
        let value = match self.peek() {
            Some(q @ ('\'' | '"')) => {
                self.pos += 1;
                let v = self.take_while(|c| c != q);
                if self.peek() == Some(q) {
                    self.pos += 1;
                }
                v
            }
            _ => self.take_while(|c| !c.is_whitespace() && c != '>'),
        };
        (name, decode_entities(&value, true))
    }
}

/// Characters allowed in a tag name.
fn is_tag_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '.'
}

/// Tokenizes all of `source` (convenience for tests and debugging).
pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokenizer = Tokenizer::new(source);
    let mut out = Vec::new();
    while let Some(token) = tokenizer.next_token() {
        out.push(token);
    }
    out
}

/// Decodes character references (`&amp;`, `&#65;`, `&#x41;`, and the
/// legacy forms without a `;`) in `s`. Unknown or malformed references
/// are left as literal text. `in_attribute` selects the spec's
/// attribute-value exception for the no-`;` forms.
fn decode_entities(s: &str, in_attribute: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&'
            && let Some((ch, consumed)) = decode_reference(&chars[i + 1..], in_attribute)
        {
            out.push(ch);
            i += 1 + consumed; // '&' plus the reference
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Tries to decode a reference at the start of `rest` (what follows an
/// '&'). Returns the decoded char plus how many chars were consumed.
fn decode_reference(rest: &[char], in_attribute: bool) -> Option<(char, usize)> {
    match rest.first() {
        Some('#') => decode_numeric_reference(rest),
        Some(_) => decode_named_reference(rest, in_attribute),
        None => None,
    }
}

/// The spec's numeric character reference states: digits with or
/// without a closing `;`, then the code point normalised — zero, a
/// surrogate, anything past U+10FFFF, or a number too large to hold
/// becomes U+FFFD, and the C1 control range maps through windows-1252,
/// which is what `&#146;` on an old page means.
fn decode_numeric_reference(rest: &[char]) -> Option<(char, usize)> {
    let (hex, start) = if matches!(rest.get(1), Some('x') | Some('X')) {
        (true, 2)
    } else {
        (false, 1)
    };
    let mut i = start;
    while i < rest.len()
        && (if hex {
            rest[i].is_ascii_hexdigit()
        } else {
            rest[i].is_ascii_digit()
        })
    {
        i += 1;
    }
    if i == start {
        return None; // `&#` or `&#x` with no digits stays literal
    }
    let digits: String = rest[start..i].iter().collect();
    let radix = if hex { 16 } else { 10 };
    // An overflowing number is "greater than 0x10FFFF" as far as the
    // spec cares: U+FFFD, same as any out-of-range value.
    let code = u32::from_str_radix(&digits, radix).unwrap_or(u32::MAX);
    let consumed = if matches!(rest.get(i), Some(';')) {
        i + 1
    } else {
        i
    };
    Some((normalise_code_point(code), consumed))
}

/// The replacement a numeric reference's code point gets before it is
/// inserted (the spec's numeric-character-reference-end state).
fn normalise_code_point(code: u32) -> char {
    let mapped = match code {
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        other => other,
    };
    match mapped {
        0 | 0xD800..=0xDFFF => '\u{FFFD}',
        c => char::from_u32(c).unwrap_or('\u{FFFD}'),
    }
}

/// The spec's named character reference state: the longest table entry
/// that matches wins. With a `;` the whole name must be known; without
/// one only the legacy names (the Latin-1 range and `amp`/`lt`/`gt`/
/// `quot`) decode, and inside an attribute value not even those when
/// an `=` or an alphanumeric follows, so that `?a=1&copy=2` survives.
fn decode_named_reference(rest: &[char], in_attribute: bool) -> Option<(char, usize)> {
    let mut i = 0;
    while i < rest.len() && rest[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let run: String = rest[..i].iter().collect();
    if matches!(rest.get(i), Some(';'))
        && let Some(ch) = named_reference(&run)
    {
        return Some((ch, i + 1)); // include the ';'
    }
    // No `;`, or an unknown name before one: the longest legacy prefix.
    let longest = i.min(LONGEST_LEGACY_NAME);
    for len in (1..=longest).rev() {
        let prefix: String = rest[..len].iter().collect();
        let Some(ch) = legacy_reference(&prefix) else {
            continue;
        };
        if in_attribute
            && let Some(next) = rest.get(len)
            && (*next == '=' || next.is_ascii_alphanumeric())
        {
            return None;
        }
        return Some((ch, len));
    }
    None
}

/// The longest name in [`legacy_reference`]'s set.
const LONGEST_LEGACY_NAME: usize = 6;

/// A name that the spec's table also carries without a `;`: the
/// Latin-1 range (the HTML 2 era names) and `amp`/`lt`/`gt`/`quot`,
/// with their upper-case spellings.
fn legacy_reference(name: &str) -> Option<char> {
    match name {
        "AMP" | "amp" => Some('&'),
        "LT" | "lt" => Some('<'),
        "GT" | "gt" => Some('>'),
        "QUOT" | "quot" => Some('"'),
        "COPY" => Some('\u{a9}'),
        "REG" => Some('\u{ae}'),
        _ => {
            let ch = named_reference(name)?;
            (0xa0..=0xff).contains(&(ch as u32)).then_some(ch)
        }
    }
}

/// The named character reference subset this tokenizer understands:
/// the HTML 4 table (Latin-1, Greek, arrows, mathematical and
/// typographic symbols), `&apos;` and `&euro;`.
fn named_reference(name: &str) -> Option<char> {
    NAMED_REFERENCES
        .binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| NAMED_REFERENCES[i].1)
}

/// Sorted by name (ASCII order), for the binary search above; a test
/// checks the order.
static NAMED_REFERENCES: &[(&str, char)] = &[
    ("AElig", '\u{c6}'),
    ("AMP", '&'),
    ("Aacute", '\u{c1}'),
    ("Acirc", '\u{c2}'),
    ("Agrave", '\u{c0}'),
    ("Alpha", '\u{391}'),
    ("Aring", '\u{c5}'),
    ("Atilde", '\u{c3}'),
    ("Auml", '\u{c4}'),
    ("Beta", '\u{392}'),
    ("COPY", '\u{a9}'),
    ("Ccedil", '\u{c7}'),
    ("Chi", '\u{3a7}'),
    ("Dagger", '\u{2021}'),
    ("Delta", '\u{394}'),
    ("ETH", '\u{d0}'),
    ("Eacute", '\u{c9}'),
    ("Ecirc", '\u{ca}'),
    ("Egrave", '\u{c8}'),
    ("Epsilon", '\u{395}'),
    ("Eta", '\u{397}'),
    ("Euml", '\u{cb}'),
    ("GT", '>'),
    ("Gamma", '\u{393}'),
    ("Iacute", '\u{cd}'),
    ("Icirc", '\u{ce}'),
    ("Igrave", '\u{cc}'),
    ("Iota", '\u{399}'),
    ("Iuml", '\u{cf}'),
    ("Kappa", '\u{39a}'),
    ("LT", '<'),
    ("Lambda", '\u{39b}'),
    ("Mu", '\u{39c}'),
    ("Ntilde", '\u{d1}'),
    ("Nu", '\u{39d}'),
    ("OElig", '\u{152}'),
    ("Oacute", '\u{d3}'),
    ("Ocirc", '\u{d4}'),
    ("Ograve", '\u{d2}'),
    ("Omega", '\u{3a9}'),
    ("Omicron", '\u{39f}'),
    ("Oslash", '\u{d8}'),
    ("Otilde", '\u{d5}'),
    ("Ouml", '\u{d6}'),
    ("Phi", '\u{3a6}'),
    ("Pi", '\u{3a0}'),
    ("Prime", '\u{2033}'),
    ("Psi", '\u{3a8}'),
    ("QUOT", '"'),
    ("REG", '\u{ae}'),
    ("Rho", '\u{3a1}'),
    ("Scaron", '\u{160}'),
    ("Sigma", '\u{3a3}'),
    ("THORN", '\u{de}'),
    ("Tau", '\u{3a4}'),
    ("Theta", '\u{398}'),
    ("Uacute", '\u{da}'),
    ("Ucirc", '\u{db}'),
    ("Ugrave", '\u{d9}'),
    ("Upsilon", '\u{3a5}'),
    ("Uuml", '\u{dc}'),
    ("Xi", '\u{39e}'),
    ("Yacute", '\u{dd}'),
    ("Yuml", '\u{178}'),
    ("Zeta", '\u{396}'),
    ("aacute", '\u{e1}'),
    ("acirc", '\u{e2}'),
    ("acute", '\u{b4}'),
    ("aelig", '\u{e6}'),
    ("agrave", '\u{e0}'),
    ("alefsym", '\u{2135}'),
    ("alpha", '\u{3b1}'),
    ("amp", '&'),
    ("and", '\u{2227}'),
    ("ang", '\u{2220}'),
    ("apos", '\''),
    ("aring", '\u{e5}'),
    ("asymp", '\u{2248}'),
    ("atilde", '\u{e3}'),
    ("auml", '\u{e4}'),
    ("bdquo", '\u{201e}'),
    ("beta", '\u{3b2}'),
    ("brvbar", '\u{a6}'),
    ("bull", '\u{2022}'),
    ("cap", '\u{2229}'),
    ("ccedil", '\u{e7}'),
    ("cedil", '\u{b8}'),
    ("cent", '\u{a2}'),
    ("chi", '\u{3c7}'),
    ("circ", '\u{2c6}'),
    ("clubs", '\u{2663}'),
    ("cong", '\u{2245}'),
    ("copy", '\u{a9}'),
    ("crarr", '\u{21b5}'),
    ("cup", '\u{222a}'),
    ("curren", '\u{a4}'),
    ("dArr", '\u{21d3}'),
    ("dagger", '\u{2020}'),
    ("darr", '\u{2193}'),
    ("deg", '\u{b0}'),
    ("delta", '\u{3b4}'),
    ("diams", '\u{2666}'),
    ("divide", '\u{f7}'),
    ("eacute", '\u{e9}'),
    ("ecirc", '\u{ea}'),
    ("egrave", '\u{e8}'),
    ("empty", '\u{2205}'),
    ("emsp", '\u{2003}'),
    ("ensp", '\u{2002}'),
    ("epsilon", '\u{3b5}'),
    ("equiv", '\u{2261}'),
    ("eta", '\u{3b7}'),
    ("eth", '\u{f0}'),
    ("euml", '\u{eb}'),
    ("euro", '\u{20ac}'),
    ("exist", '\u{2203}'),
    ("fnof", '\u{192}'),
    ("forall", '\u{2200}'),
    ("frac12", '\u{bd}'),
    ("frac14", '\u{bc}'),
    ("frac34", '\u{be}'),
    ("frasl", '\u{2044}'),
    ("gamma", '\u{3b3}'),
    ("ge", '\u{2265}'),
    ("gt", '>'),
    ("hArr", '\u{21d4}'),
    ("harr", '\u{2194}'),
    ("hearts", '\u{2665}'),
    ("hellip", '\u{2026}'),
    ("iacute", '\u{ed}'),
    ("icirc", '\u{ee}'),
    ("iexcl", '\u{a1}'),
    ("igrave", '\u{ec}'),
    ("image", '\u{2111}'),
    ("infin", '\u{221e}'),
    ("int", '\u{222b}'),
    ("iota", '\u{3b9}'),
    ("iquest", '\u{bf}'),
    ("isin", '\u{2208}'),
    ("iuml", '\u{ef}'),
    ("kappa", '\u{3ba}'),
    ("lArr", '\u{21d0}'),
    ("lambda", '\u{3bb}'),
    ("lang", '\u{27e8}'),
    ("laquo", '\u{ab}'),
    ("larr", '\u{2190}'),
    ("lceil", '\u{2308}'),
    ("ldquo", '\u{201c}'),
    ("le", '\u{2264}'),
    ("lfloor", '\u{230a}'),
    ("lowast", '\u{2217}'),
    ("loz", '\u{25ca}'),
    ("lrm", '\u{200e}'),
    ("lsaquo", '\u{2039}'),
    ("lsquo", '\u{2018}'),
    ("lt", '<'),
    ("macr", '\u{af}'),
    ("mdash", '\u{2014}'),
    ("micro", '\u{b5}'),
    ("middot", '\u{b7}'),
    ("minus", '\u{2212}'),
    ("mu", '\u{3bc}'),
    ("nabla", '\u{2207}'),
    ("nbsp", '\u{a0}'),
    ("ndash", '\u{2013}'),
    ("ne", '\u{2260}'),
    ("ni", '\u{220b}'),
    ("not", '\u{ac}'),
    ("notin", '\u{2209}'),
    ("nsub", '\u{2284}'),
    ("ntilde", '\u{f1}'),
    ("nu", '\u{3bd}'),
    ("oacute", '\u{f3}'),
    ("ocirc", '\u{f4}'),
    ("oelig", '\u{153}'),
    ("ograve", '\u{f2}'),
    ("oline", '\u{203e}'),
    ("omega", '\u{3c9}'),
    ("omicron", '\u{3bf}'),
    ("oplus", '\u{2295}'),
    ("or", '\u{2228}'),
    ("ordf", '\u{aa}'),
    ("ordm", '\u{ba}'),
    ("oslash", '\u{f8}'),
    ("otilde", '\u{f5}'),
    ("otimes", '\u{2297}'),
    ("ouml", '\u{f6}'),
    ("para", '\u{b6}'),
    ("part", '\u{2202}'),
    ("permil", '\u{2030}'),
    ("perp", '\u{22a5}'),
    ("phi", '\u{3c6}'),
    ("pi", '\u{3c0}'),
    ("piv", '\u{3d6}'),
    ("plusmn", '\u{b1}'),
    ("pound", '\u{a3}'),
    ("prime", '\u{2032}'),
    ("prod", '\u{220f}'),
    ("prop", '\u{221d}'),
    ("psi", '\u{3c8}'),
    ("quot", '"'),
    ("rArr", '\u{21d2}'),
    ("radic", '\u{221a}'),
    ("rang", '\u{27e9}'),
    ("raquo", '\u{bb}'),
    ("rarr", '\u{2192}'),
    ("rceil", '\u{2309}'),
    ("rdquo", '\u{201d}'),
    ("real", '\u{211c}'),
    ("reg", '\u{ae}'),
    ("rfloor", '\u{230b}'),
    ("rho", '\u{3c1}'),
    ("rlm", '\u{200f}'),
    ("rsaquo", '\u{203a}'),
    ("rsquo", '\u{2019}'),
    ("sbquo", '\u{201a}'),
    ("scaron", '\u{161}'),
    ("sdot", '\u{22c5}'),
    ("sect", '\u{a7}'),
    ("shy", '\u{ad}'),
    ("sigma", '\u{3c3}'),
    ("sigmaf", '\u{3c2}'),
    ("sim", '\u{223c}'),
    ("spades", '\u{2660}'),
    ("sub", '\u{2282}'),
    ("sube", '\u{2286}'),
    ("sum", '\u{2211}'),
    ("sup", '\u{2283}'),
    ("sup1", '\u{b9}'),
    ("sup2", '\u{b2}'),
    ("sup3", '\u{b3}'),
    ("supe", '\u{2287}'),
    ("szlig", '\u{df}'),
    ("tau", '\u{3c4}'),
    ("there4", '\u{2234}'),
    ("theta", '\u{3b8}'),
    ("thetasym", '\u{3d1}'),
    ("thinsp", '\u{2009}'),
    ("thorn", '\u{fe}'),
    ("tilde", '\u{2dc}'),
    ("times", '\u{d7}'),
    ("trade", '\u{2122}'),
    ("uArr", '\u{21d1}'),
    ("uacute", '\u{fa}'),
    ("uarr", '\u{2191}'),
    ("ucirc", '\u{fb}'),
    ("ugrave", '\u{f9}'),
    ("uml", '\u{a8}'),
    ("upsih", '\u{3d2}'),
    ("upsilon", '\u{3c5}'),
    ("uuml", '\u{fc}'),
    ("weierp", '\u{2118}'),
    ("xi", '\u{3be}'),
    ("yacute", '\u{fd}'),
    ("yen", '\u{a5}'),
    ("yuml", '\u{ff}'),
    ("zeta", '\u{3b6}'),
    ("zwj", '\u{200d}'),
    ("zwnj", '\u{200c}'),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Token {
        Token::Text(s.to_string())
    }

    #[test]
    fn plain_text_roundtrips() {
        assert_eq!(tokenize("hello world"), vec![text("hello world")]);
    }

    #[test]
    fn start_tag_with_attributes() {
        let tokens = tokenize(r#"<p class="intro" data-kind='x' hidden>"#);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "p".into(),
                attrs: vec![
                    ("class".into(), "intro".into()),
                    ("data-kind".into(), "x".into()),
                    ("hidden".into(), String::new()),
                ],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn self_closing_tags() {
        let expected = vec![Token::StartTag {
            name: "br".into(),
            attrs: vec![],
            self_closing: true,
        }];
        assert_eq!(tokenize("<br/>"), expected);
        assert_eq!(tokenize("<br />"), expected);
    }

    #[test]
    fn end_tags_are_lowercased() {
        assert_eq!(tokenize("</DIV>"), vec![Token::EndTag("div".into())]);
    }

    #[test]
    fn comments_and_doctype() {
        assert_eq!(tokenize("<!-- hi -->"), vec![Token::Comment(" hi ".into())]);
        assert_eq!(
            tokenize("<!DOCTYPE html>"),
            vec![Token::Doctype("html".into())]
        );
    }

    #[test]
    fn abruptly_closed_comments_do_not_swallow_the_page() {
        // The spec's abrupt-closing-of-empty-comment and the `--!>` form.
        for source in ["<!-->after", "<!--->after", "<!-- x --!>after"] {
            let tokens = tokenize(source);
            assert_eq!(tokens.len(), 2, "{source:?}: {tokens:?}");
            assert_eq!(tokens[1], text("after"), "{source:?}");
        }
        assert_eq!(tokenize("<!--->"), vec![Token::Comment(String::new())]);
    }

    #[test]
    fn question_mark_markup_is_a_dropped_bogus_comment() {
        assert_eq!(
            tokenize(r#"<?xml version="1.0" encoding="UTF-8"?><p>hi"#),
            vec![
                Token::Comment(r#"xml version="1.0" encoding="UTF-8"?"#.into()),
                Token::StartTag {
                    name: "p".into(),
                    attrs: vec![],
                    self_closing: false,
                },
                text("hi"),
            ]
        );
    }

    #[test]
    fn carriage_returns_are_normalised_to_newlines() {
        assert_eq!(tokenize("a\r\nb\rc\n"), vec![text("a\nb\nc\n")]);
        assert_eq!(
            tokenize("<p\r\nclass=x>"),
            vec![Token::StartTag {
                name: "p".into(),
                attrs: vec![("class".into(), "x".into())],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn stray_lt_is_text() {
        assert_eq!(tokenize("a < b and 5 < 6"), vec![text("a < b and 5 < 6")]);
        assert_eq!(tokenize("trailing <"), vec![text("trailing <")]);
    }

    #[test]
    fn character_references_decode() {
        assert_eq!(
            tokenize("Fish &amp; Chips &lt;3 &#65;&#x42;&nbsp;!"),
            vec![text("Fish & Chips <3 AB\u{a0}!")]
        );
        // Unknown or malformed references stay literal.
        assert_eq!(
            tokenize("&nope; &#; &#x; &"),
            vec![text("&nope; &#; &#x; &")]
        );
    }

    #[test]
    fn html4_named_references_decode() {
        assert_eq!(
            tokenize(
                "caf&eacute; &rsquo;quoted&rsquo; &euro;5 &bull; &trade; &laquo;x&raquo; &alpha;"
            ),
            vec![text(
                "caf\u{e9} \u{2019}quoted\u{2019} \u{20ac}5 \u{2022} \u{2122} \u{ab}x\u{bb} \u{3b1}"
            )]
        );
    }

    #[test]
    fn legacy_references_decode_without_a_semicolon() {
        assert_eq!(
            tokenize("&amp &lt &gt &copy 2024 &nbsp&reg;&eacute!"),
            vec![text("& < > \u{a9} 2024 \u{a0}\u{ae}\u{e9}!")]
        );
        // Only the legacy names: these need their `;`.
        assert_eq!(
            tokenize("&apos &ndash &rsquo &hellip"),
            vec![text("&apos &ndash &rsquo &hellip")]
        );
        // The longest legacy prefix wins, as in the spec's table walk.
        assert_eq!(tokenize("&copy2024"), vec![text("\u{a9}2024")]);
    }

    #[test]
    fn numeric_references_decode_without_a_semicolon_and_are_normalised() {
        assert_eq!(tokenize("&#65 &#x42 &#65;"), vec![text("A B A")]);
        // windows-1252 for the C1 range: the curly apostrophe of old pages.
        assert_eq!(
            tokenize("it&#146;s &#147;q&#148;"),
            vec![text("it\u{2019}s \u{201c}q\u{201d}")]
        );
        // Zero, surrogates, out of range and overflow all become U+FFFD.
        assert_eq!(
            tokenize("&#0;&#xD800;&#x110000;&#99999999999999999999;"),
            vec![text("\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}")]
        );
    }

    #[test]
    fn attribute_values_decode_references() {
        assert_eq!(
            tokenize(r#"<a title="A &amp; B">"#),
            vec![Token::StartTag {
                name: "a".into(),
                attrs: vec![("title".into(), "A & B".into())],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn attribute_values_keep_query_strings_intact() {
        // The spec's exception: a legacy name without `;` followed by
        // `=` or an alphanumeric is not a reference inside an attribute.
        assert_eq!(
            tokenize(r#"<a href="?x=1&amp=2&copy=3&notit=4&lt5&amp;y">"#),
            vec![Token::StartTag {
                name: "a".into(),
                attrs: vec![("href".into(), "?x=1&amp=2&copy=3&notit=4&lt5&y".into())],
                self_closing: false,
            }]
        );
        // In text the same input does decode.
        assert_eq!(
            tokenize("?x=1&amp=2&copy=3&notit=4"),
            vec![text("?x=1&=2\u{a9}=3\u{ac}it=4")]
        );
    }

    #[test]
    fn duplicate_attributes_keep_first() {
        assert_eq!(
            tokenize(r#"<p id="a" id="b">"#),
            vec![Token::StartTag {
                name: "p".into(),
                attrs: vec![("id".into(), "a".into())],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn a_tag_with_very_many_attributes_tokenizes_in_linear_time() {
        // 60k distinct attributes on one tag: the duplicate check used to
        // scan the attribute list for each, which took tens of seconds.
        let mut source = String::from("<a");
        for i in 0..60_000 {
            source.push_str(&format!(" a{i}"));
        }
        source.push('>');
        let tokens = tokenize(&source);
        let Token::StartTag { attrs, .. } = &tokens[0] else {
            panic!("expected a start tag");
        };
        assert_eq!(attrs.len(), 60_000);
    }

    #[test]
    fn eof_edge_cases() {
        assert_eq!(tokenize(""), Vec::<Token>::new());
        assert_eq!(tokenize("<"), vec![text("<")]);
        // The spec's eof-before-tag-name rule: `</` at EOF is text.
        assert_eq!(tokenize("</"), vec![text("</")]);
        assert_eq!(tokenize("a</"), vec![text("a"), text("</")]);
        assert_eq!(tokenize("<!"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<!--"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<?"), vec![Token::Comment(String::new())]);
    }

    #[test]
    fn named_reference_table_is_sorted_for_the_binary_search() {
        for pair in NAMED_REFERENCES.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "{} must sort before {}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    #[test]
    fn token_stream_of_a_small_document() {
        assert_eq!(
            tokenize("<h1>Hi</h1>"),
            vec![
                Token::StartTag {
                    name: "h1".into(),
                    attrs: vec![],
                    self_closing: false,
                },
                text("Hi"),
                Token::EndTag("h1".into()),
            ]
        );
    }
}
