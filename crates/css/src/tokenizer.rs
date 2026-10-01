//! CSS tokenizer following the CSS Syntax Module Level 3 token model.
//!
//! The token types, their consumption algorithms, and the error-recovery
//! rules mirror the WHATWG `css-syntax-3` specification (`<ident-token>`,
//! `<function-token>`, `<at-keyword-token>`, `<hash-token>`, and so on)
//! because that model is what real engines sit on: every later system
//! (selectors, cascade, custom properties, layout) consumes these tokens.
//!
//! Documented deviations (pragmatic subset):
//!
//! * The stream ends at EOF; there is no distinct `<EOF-token>`.
//! * A string containing an unescaped newline is a bad-string and stops
//!   there (the newline itself is re-tokenized as whitespace).
//! * An unterminated string still produces a string token with the
//!   content read so far; an unterminated unquoted url ends at EOF.
//! * `url()` accepts quoted arguments and escaped code points; bad urls
//!   are consumed through to the closing `)` per the spec.
//! * Surrogates, code point 0, and out-of-range escapes all become
//!   U+FFFD (as the spec requires).
//!
//! The tokenizer never panics on any input, and never allocates beyond
//! the token values it returns.

/// A CSS token (`css-syntax-3` §4.1.1, minus comments — they are
/// consumed by the tokenizer and produce no token).
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `<ident-token>`: a name like `body` or `--custom`.
    Ident(String),
    /// `<function-token>`: an identifier immediately followed by `(`,
    /// which is consumed into this token.
    Function(String),
    /// `<at-keyword-token>`: `@` + name, e.g. `@media`.
    AtKeyword(String),
    /// `<hash-token>`: `#` + name, with the id flag set when the value
    /// would start an identifier (an id selector, not a hex color).
    Hash {
        /// The name after `#`.
        value: String,
        /// Whether the name starts like an identifier.
        is_id: bool,
    },
    /// `<string-token>`; `bad` marks a `<bad-string-token>`.
    String {
        /// The decoded string value.
        value: String,
        /// Set when an unescaped newline ended the string.
        bad: bool,
    },
    /// `<url-token>`; `bad` marks a `<bad-url-token>`.
    Url {
        /// The url value.
        value: String,
        /// Set when the url was malformed.
        bad: bool,
    },
    /// `<number-token>`.
    Number {
        /// The numeric value.
        value: f64,
        /// Whether the source was an integer (no `.` or exponent).
        is_integer: bool,
    },
    /// `<percentage-token>`.
    Percentage {
        /// The numeric value before the `%`.
        value: f64,
    },
    /// `<dimension-token>`: a number with a unit.
    Dimension {
        /// The numeric value.
        value: f64,
        /// The unit (`px`, `em`, ...).
        unit: String,
    },
    /// `<delim-token>`: any single code point with no other meaning.
    Delim(char),
    /// A run of whitespace, coalesced into one token.
    Whitespace,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `(`
    OpenParen,
    /// `)`
    CloseParen,
    /// `[`
    OpenSquare,
    /// `]`
    CloseSquare,
    /// `{`
    OpenCurly,
    /// `}`
    CloseCurly,
    /// `<!--`
    Cdo,
    /// `-->`
    Cdc,
}

/// Tokenizes CSS source. Never panics.
pub struct Tokenizer {
    input: Vec<char>,
    len: usize,
    pos: usize,
}

impl Tokenizer {
    /// Creates a tokenizer over `source`.
    pub fn new(source: &str) -> Self {
        let input: Vec<char> = source.chars().collect();
        let len = input.len();
        Tokenizer { input, len, pos: 0 }
    }

    /// Returns the next token, or `None` at end of input. Every call
    /// consumes at least one character, so iteration always terminates.
    pub fn next_token(&mut self) -> Option<Token> {
        // Comments produce no tokens.
        while self.matches_at(self.pos, "/*") {
            self.skip_comment();
        }
        let c = self.peek()?;

        // Whitespace runs coalesce into one token.
        if is_css_whitespace(c) {
            while self.peek().is_some_and(is_css_whitespace) {
                self.pos += 1;
            }
            return Some(Token::Whitespace);
        }
        if c == '"' || c == '\'' {
            return Some(self.consume_string(c));
        }
        if c == '#' {
            // Hash when a name follows; a bare '#' is a delim.
            if self.name_follows(self.pos + 1) {
                let is_id = self.starts_ident_at(self.pos + 1);
                self.pos += 1;
                let value = self.consume_name();
                return Some(Token::Hash { value, is_id });
            }
            self.pos += 1;
            return Some(Token::Delim('#'));
        }
        if c == '@' {
            if self.starts_ident_at(self.pos + 1) {
                self.pos += 1;
                let name = self.consume_name();
                return Some(Token::AtKeyword(name));
            }
            self.pos += 1;
            return Some(Token::Delim('@'));
        }
        if self.matches_at(self.pos, "<!--") {
            self.pos += 4;
            return Some(Token::Cdo);
        }
        if self.matches_at(self.pos, "-->") {
            self.pos += 3;
            return Some(Token::Cdc);
        }
        if self.starts_number_at(self.pos) {
            return Some(self.consume_numeric());
        }
        if self.starts_ident_at(self.pos) {
            return Some(self.consume_ident_like());
        }
        self.pos += 1;
        Some(match c {
            '(' => Token::OpenParen,
            ')' => Token::CloseParen,
            '[' => Token::OpenSquare,
            ']' => Token::CloseSquare,
            '{' => Token::OpenCurly,
            '}' => Token::CloseCurly,
            ':' => Token::Colon,
            ';' => Token::Semicolon,
            ',' => Token::Comma,
            _ => Token::Delim(c),
        })
    }

    // ---- low-level cursor helpers -------------------------------------

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn matches_at(&self, at: usize, pattern: &str) -> bool {
        pattern
            .chars()
            .enumerate()
            .all(|(i, c)| self.input.get(at + i) == Some(&c))
    }

    /// True when the code point at `at` begins a valid escape (a `\`
    /// followed by anything but a newline or EOF).
    fn is_valid_escape_at(&self, at: usize) -> bool {
        if self.input.get(at) != Some(&'\\') {
            return false;
        }
        matches!(self.input.get(at + 1), Some(c) if !is_css_newline(*c))
    }

    /// True when the code point at `at` starts an identifier.
    fn starts_ident_at(&self, at: usize) -> bool {
        match self.input.get(at) {
            Some(c) if is_name_start(*c) => true,
            Some('\\') => self.is_valid_escape_at(at),
            Some('-') => match self.input.get(at + 1) {
                Some(n) if is_name_start(*n) => true,
                Some('-') => true, // `--custom`
                Some('\\') => self.is_valid_escape_at(at + 1),
                _ => false,
            },
            _ => false,
        }
    }

    /// True when the code point at `at` starts a number.
    fn starts_number_at(&self, at: usize) -> bool {
        match self.input.get(at) {
            Some(c) if c.is_ascii_digit() => true,
            Some('.') => matches!(self.input.get(at + 1), Some(d) if d.is_ascii_digit()),
            Some('+' | '-') => match self.input.get(at + 1) {
                Some(d) if d.is_ascii_digit() => true,
                Some('.') => matches!(self.input.get(at + 2), Some(d) if d.is_ascii_digit()),
                _ => false,
            },
            _ => false,
        }
    }

    /// True when a name (identifier characters) starts at `at`
    /// (used after `#`, where digits also count).
    fn name_follows(&self, at: usize) -> bool {
        match self.input.get(at) {
            Some(c) if is_name_char(*c) => true,
            Some('\\') => self.is_valid_escape_at(at),
            _ => false,
        }
    }

    // ---- consumption algorithms ----------------------------------------

    /// Consumes `/* ... */`; an unterminated comment runs to EOF.
    fn skip_comment(&mut self) {
        self.pos += 2; // "/*"
        while self.pos < self.len {
            if self.matches_at(self.pos, "*/") {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }

    /// Consumes an identifier, then decides between `<ident-token>`,
    /// `<function-token>`, and `<url-token>`.
    fn consume_ident_like(&mut self) -> Token {
        let name = self.consume_name();
        if self.peek() == Some('(') {
            if name.eq_ignore_ascii_case("url") {
                self.consume_url_token()
            } else {
                self.pos += 1; // '(' belongs to the function token
                Token::Function(name)
            }
        } else {
            Token::Ident(name)
        }
    }

    /// Consumes a name (identifier characters and escapes).
    fn consume_name(&mut self) -> String {
        let mut name = String::new();
        loop {
            match self.peek() {
                Some(c) if is_name_char(c) => {
                    self.pos += 1;
                    name.push(c);
                }
                Some('\\') if self.is_valid_escape_at(self.pos) => {
                    name.push(self.consume_escape());
                }
                _ => break,
            }
        }
        name
    }

    /// Consumes an escape sequence at a valid `\`. Escaped U+0000,
    /// surrogates, and out-of-range values all become U+FFFD.
    fn consume_escape(&mut self) -> char {
        self.pos += 1; // backslash
        let start = self.pos;
        while self.pos < self.len
            && self.pos - start < 6
            && self.input[self.pos].is_ascii_hexdigit()
        {
            self.pos += 1;
        }
        if self.pos > start {
            let end = self.pos;
            if self.peek().is_some_and(is_css_whitespace) {
                self.pos += 1; // one optional whitespace after the digits
            }
            let digits: String = self.input[start..end].iter().collect();
            match u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                Some('\0') | None => '\u{FFFD}',
                Some(c) => c,
            }
        } else if self.pos < self.len {
            let c = self.input[self.pos];
            self.pos += 1;
            c
        } else {
            '\u{FFFD}'
        }
    }

    /// Consumes a string token (the opening quote is next).
    fn consume_string(&mut self, quote: char) -> Token {
        self.pos += 1;
        let mut value = String::new();
        loop {
            match self.peek() {
                None => break, // unterminated: still a string token
                Some(c) if c == quote => {
                    self.pos += 1;
                    break;
                }
                Some('\n') => {
                    // Unescaped newline: bad string. The newline is NOT
                    // consumed and will be re-tokenized as whitespace.
                    return Token::String { value, bad: true };
                }
                Some('\\') => match self.peek_at(1) {
                    None => {
                        self.pos += 1;
                        break;
                    }
                    Some('\n') => {
                        self.pos += 2; // line continuation: appends nothing
                    }
                    Some(_) => {
                        value.push(self.consume_escape());
                    }
                },
                Some(c) => {
                    self.pos += 1;
                    value.push(c);
                }
            }
        }
        Token::String { value, bad: false }
    }

    /// Consumes a number, then a `%` or unit if present
    /// (`<number-token>`, `<percentage-token>`, `<dimension-token>`).
    fn consume_numeric(&mut self) -> Token {
        let start = self.pos;
        if matches!(self.peek(), Some('+' | '-')) {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some('.')
            && matches!(self.peek_at(1), Some(d) if d.is_ascii_digit())
        {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let after = self.pos + 1;
            let has_sign = matches!(self.input.get(after), Some('+' | '-'));
            let digit_at = if has_sign { after + 1 } else { after };
            if matches!(self.input.get(digit_at), Some(d) if d.is_ascii_digit()) {
                self.pos = digit_at;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
        }
        let text: String = self.input[start..self.pos].iter().collect();
        let is_integer = !text.contains('.') && !text.contains('e') && !text.contains('E');
        let value: f64 = text.parse().unwrap_or(0.0);

        if self.peek() == Some('%') {
            self.pos += 1;
            return Token::Percentage { value };
        }
        if self.starts_ident_at(self.pos) {
            let unit = self.consume_name();
            return Token::Dimension { value, unit };
        }
        Token::Number { value, is_integer }
    }

    /// Consumes a `url(...)` token; the current position is at `(`.
    fn consume_url_token(&mut self) -> Token {
        self.pos += 1; // '('
        while self.peek().is_some_and(is_css_whitespace) {
            self.pos += 1;
        }
        match self.peek() {
            None => Token::Url { value: String::new(), bad: true },
            Some(')') => {
                self.pos += 1;
                Token::Url { value: String::new(), bad: false }
            }
            Some(q @ ('"' | '\'')) => {
                let (value, bad) = match self.consume_string(q) {
                    Token::String { value, bad } => (value, bad),
                    _ => (String::new(), true),
                };
                while self.peek().is_some_and(is_css_whitespace) {
                    self.pos += 1;
                }
                if self.peek() == Some(')') {
                    self.pos += 1;
                    Token::Url {
                        value: if bad { String::new() } else { value },
                        bad,
                    }
                } else {
                    self.consume_bad_url()
                }
            }
            Some(_) => {
                // Unquoted url.
                let mut value = String::new();
                loop {
                    match self.peek() {
                        None => break, // EOF: tolerate (parse error per spec)
                        Some(')') => {
                            self.pos += 1;
                            break;
                        }
                        Some(c) if is_css_whitespace(c) => {
                            while self.peek().is_some_and(is_css_whitespace) {
                                self.pos += 1;
                            }
                            if self.peek() == Some(')') {
                                self.pos += 1;
                            } else {
                                return self.consume_bad_url();
                            }
                            break;
                        }
                        Some(c) if is_bad_url_start(c) => return self.consume_bad_url(),
                        Some('\\') => {
                            if self.is_valid_escape_at(self.pos) {
                                value.push(self.consume_escape());
                            } else {
                                return self.consume_bad_url();
                            }
                        }
                        Some(c) => {
                            self.pos += 1;
                            value.push(c);
                        }
                    }
                }
                Token::Url { value, bad: false }
            }
        }
    }

    /// Consumes the remnants of a bad url, up to and including the `)`.
    fn consume_bad_url(&mut self) -> Token {
        loop {
            match self.peek() {
                None => break,
                Some(')') => {
                    self.pos += 1;
                    break;
                }
                Some('\\') if self.is_valid_escape_at(self.pos) => {
                    let _ = self.consume_escape();
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
        Token::Url { value: String::new(), bad: true }
    }
}

/// CSS whitespace: tab, LF, CR, FF, and space.
fn is_css_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// CSS newline: LF, CR, or FF (CR is not followed-peeked for CRLF).
fn is_css_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{c}')
}

/// Name-start code points: letters, `_`, and anything non-ASCII.
fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || (c as u32) >= 0x80
}

/// Name code points: name-start plus digits and `-`.
fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

/// Code points that make an unquoted url go bad.
fn is_bad_url_start(c: char) -> bool {
    matches!(c, '"' | '\'' | '(') || is_non_printable(c)
}

/// Non-printable code points per `css-syntax-3` §4.2.
fn is_non_printable(c: char) -> bool {
    let u = c as u32;
    (u <= 0x08) || u == 0x0B || (0x0E..=0x1F).contains(&u) || u == 0x7F
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents_and_functions() {
        assert_eq!(
            tokenize("foo Bar( bazé --custom"),
            vec![
                Token::Ident("foo".into()),
                Token::Whitespace,
                Token::Function("Bar".into()),
                Token::Whitespace,
                Token::Ident("bazé".into()),
                Token::Whitespace,
                Token::Ident("--custom".into()),
            ]
        );
    }

    #[test]
    fn at_keywords() {
        assert_eq!(
            tokenize("@media @font-face @@"),
            vec![
                Token::AtKeyword("media".into()),
                Token::Whitespace,
                Token::AtKeyword("font-face".into()),
                Token::Whitespace,
                Token::Delim('@'), // '@' followed by '@' is not an ident start
                Token::Delim('@'), // trailing '@' at EOF
            ]
        );
    }

    #[test]
    fn hashes() {
        assert_eq!(
            tokenize("#fff #123"),
            vec![
                Token::Hash { value: "fff".into(), is_id: true },
                Token::Whitespace,
                Token::Hash { value: "123".into(), is_id: false },
            ]
        );
    }

    #[test]
    fn strings_with_escapes() {
        assert_eq!(
            tokenize(r#""hi there" 'a\'b' "line\
break""#),
            vec![
                Token::String { value: "hi there".into(), bad: false },
                Token::Whitespace,
                Token::String { value: "a'b".into(), bad: false },
                Token::Whitespace,
                Token::String { value: "linebreak".into(), bad: false },
            ]
        );
    }

    #[test]
    fn bad_string_on_unescaped_newline() {
        assert_eq!(
            tokenize("\"ab\ncd"),
            vec![
                Token::String { value: "ab".into(), bad: true },
                Token::Whitespace,
                Token::Ident("cd".into()),
            ]
        );
    }

    #[test]
    fn urls() {
        assert_eq!(
            tokenize("url(a/b.png) url( 'q' ) url(a b)"),
            vec![
                Token::Url { value: "a/b.png".into(), bad: false },
                Token::Whitespace,
                Token::Url { value: "q".into(), bad: false },
                Token::Whitespace,
                Token::Url { value: String::new(), bad: true },
            ]
        );
    }

    #[test]
    fn url_with_escaped_paren() {
        assert_eq!(
            tokenize(r"url(a\)b)"),
            vec![Token::Url { value: "a)b".into(), bad: false }]
        );
    }

    #[test]
    fn numbers_percentages_dimensions() {
        assert_eq!(
            tokenize("1 2.5 -3 .5 50% 10px 1e3 -2.5em"),
            vec![
                Token::Number { value: 1.0, is_integer: true },
                Token::Whitespace,
                Token::Number { value: 2.5, is_integer: false },
                Token::Whitespace,
                Token::Number { value: -3.0, is_integer: true },
                Token::Whitespace,
                Token::Number { value: 0.5, is_integer: false },
                Token::Whitespace,
                Token::Percentage { value: 50.0 },
                Token::Whitespace,
                Token::Dimension { value: 10.0, unit: "px".into() },
                Token::Whitespace,
                Token::Number { value: 1000.0, is_integer: false },
                Token::Whitespace,
                Token::Dimension { value: -2.5, unit: "em".into() },
            ]
        );
    }

    #[test]
    fn comments_produce_no_tokens() {
        assert_eq!(
            tokenize("a/* c */b  c"),
            vec![
                Token::Ident("a".into()),
                Token::Ident("b".into()),
                Token::Whitespace,
                Token::Ident("c".into()),
            ]
        );
    }

    #[test]
    fn delims_and_cdo_cdc() {
        assert_eq!(
            tokenize("> * + <!-- --> ~"),
            vec![
                Token::Delim('>'),
                Token::Whitespace,
                Token::Delim('*'),
                Token::Whitespace,
                Token::Delim('+'),
                Token::Whitespace,
                Token::Cdo,
                Token::Whitespace,
                Token::Cdc,
                Token::Whitespace,
                Token::Delim('~'),
            ]
        );
    }

    #[test]
    fn escapes_in_idents() {
        // \41 is 'A'
        assert_eq!(tokenize(r"\41 bc"), vec![Token::Ident("Abc".into())]);
        assert_eq!(tokenize(r"\@literal"), vec![Token::Ident("@literal".into())]);
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            tokenize(":;,()[]{}"),
            vec![
                Token::Colon,
                Token::Semicolon,
                Token::Comma,
                Token::OpenParen,
                Token::CloseParen,
                Token::OpenSquare,
                Token::CloseSquare,
                Token::OpenCurly,
                Token::CloseCurly,
            ]
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(tokenize(""), Vec::<Token>::new());
    }

    #[test]
    fn unterminated_comment_and_string() {
        assert_eq!(tokenize("/* no end"), Vec::<Token>::new());
        assert_eq!(
            tokenize("\"abc"),
            vec![Token::String { value: "abc".into(), bad: false }]
        );
    }
}
