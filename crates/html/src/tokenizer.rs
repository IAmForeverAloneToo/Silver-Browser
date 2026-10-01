//! WHATWG-style HTML tokenizer (pragmatic subset).
//!
//! Splits source text into [`Token`]s: tags with attributes, text runs
//! with character references decoded, comments, and doctypes. Deviations
//! from the full specification, on purpose:
//!
//! * Named character references cover the common set, not the full
//!   HTML5 table (~2000 entries).
//! * Anything malformed degrades to text or a bogus comment instead of
//!   producing spec-accurate parse errors.
//! * CDATA sections and `<script>`/`<style>` raw-text elements are
//!   tokenized like ordinary elements for now.
//!
//! The tokenizer never panics on any input.

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
}

impl Tokenizer {
    /// Creates a tokenizer over `source`.
    pub fn new(source: &str) -> Self {
        Tokenizer {
            input: source.chars().collect(),
            pos: 0,
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
        if self.peek() == Some('<')
            && let Some(token) = self.read_markup_declaration()
        {
            return Some(token);
        }
        // A '<' that starts nothing markup-ish is read as literal text.
        Some(self.read_text())
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
        Token::Text(decode_entities(&raw))
    }

    /// True when the '<' at the current position begins something
    /// markup-like (tag, end tag, comment, or doctype).
    fn tag_starts_ahead(&self) -> bool {
        match self.input.get(self.pos + 1) {
            Some(c) => c.is_ascii_alphabetic() || *c == '/' || *c == '!',
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
            '/' => match self.input.get(self.pos + 2).copied() {
                Some(c) if c.is_ascii_alphabetic() => Some(self.read_end_tag()),
                _ => Some(self.read_bogus_comment()),
            },
            c if c.is_ascii_alphabetic() => Some(self.read_start_tag()),
            _ => None,
        }
    }

    /// Reads a `<!-- ... -->` comment (unterminated comments end at EOF).
    fn read_comment(&mut self) -> Token {
        self.pos += 4; // "<!--"
        let mut content = String::new();
        while self.pos < self.input.len() {
            if self.matches_ahead(self.pos, "-->") {
                self.pos += 3;
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

    /// Reads a `</...>` that does not precede a letter (bogus comment).
    fn read_bogus_comment(&mut self) -> Token {
        self.pos += 2; // "</"
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

    /// Reads a start tag and its attributes.
    fn read_start_tag(&mut self) -> Token {
        self.pos += 1; // "<"
        let name = self.take_while(is_tag_name_char).to_lowercase();
        let mut attrs: Vec<(String, String)> = Vec::new();
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
                    if !aname.is_empty() && !attrs.iter().any(|(k, _)| *k == aname) {
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
        (name, decode_entities(&value))
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

/// Decodes character references (`&amp;`, `&#65;`, `&#x41;`) in `s`.
/// Unknown or malformed references are left as literal text.
fn decode_entities(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&'
            && let Some((ch, consumed)) = decode_reference(&chars[i + 1..])
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
fn decode_reference(rest: &[char]) -> Option<(char, usize)> {
    match rest.first() {
        Some('#') => {
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
            if i > start && matches!(rest.get(i), Some(';')) {
                let digits: String = rest[start..i].iter().collect();
                let radix = if hex { 16 } else { 10 };
                if let Ok(code) = u32::from_str_radix(&digits, radix)
                    && let Some(ch) = char::from_u32(code)
                {
                    return Some((ch, i + 1)); // include the ';'
                }
            }
            None
        }
        Some(_) => {
            let mut i = 0;
            while i < rest.len() && rest[i].is_ascii_alphanumeric() {
                i += 1;
            }
            if i > 0 && matches!(rest.get(i), Some(';')) {
                let name: String = rest[..i].iter().collect();
                if let Some(ch) = named_reference(&name) {
                    return Some((ch, i + 1)); // include the ';'
                }
            }
            None
        }
        None => None,
    }
}

/// The named character reference subset this tokenizer understands.
fn named_reference(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "copy" => Some('\u{a9}'),
        "reg" => Some('\u{ae}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "hellip" => Some('\u{2026}'),
        _ => None,
    }
}

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
    fn eof_edge_cases() {
        assert_eq!(tokenize(""), Vec::<Token>::new());
        assert_eq!(tokenize("<"), vec![text("<")]);
        assert_eq!(tokenize("</"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<!"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<!--"), vec![Token::Comment(String::new())]);
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
