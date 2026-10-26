//! GLSL ES's tokens (GLSL ES 1.00 and 3.00, sections 3.1 to 3.6): what
//! the preprocessor reads a line into, and the parser takes after it.

use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// An integer literal's text (decimal, octal or hexadecimal, with
    /// its `u` when it has one), read by the parser.
    Int(String),
    /// A floating-point literal's text.
    Float(String),
    Punct(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

impl Token {
    /// The token's text, as an error names it.
    pub fn text(&self) -> String {
        match &self.tok {
            Tok::Ident(text) | Tok::Int(text) | Tok::Float(text) => text.clone(),
            Tok::Punct(text) => (*text).to_owned(),
        }
    }

    pub fn is(&self, punct: &str) -> bool {
        matches!(&self.tok, Tok::Punct(p) if *p == punct)
    }

    pub fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(name) => Some(name),
            _ => None,
        }
    }
}

/// The punctuators, the longer before those they begin with.
const PUNCTS: &[&str] = &[
    "<<=", ">>=", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "^^", "+=", "-=",
    "*=", "/=", "%=", "&=", "^=", "|=", "##", "(", ")", "[", "]", "{", "}", ".", ",", ";", ":",
    "?", "+", "-", "*", "/", "%", "<", ">", "&", "|", "^", "!", "~", "=", "#",
];

/// Whether `c` is one of GLSL ES's characters (section 3.1) outside a
/// comment: WebGL refuses a shader with another.
pub fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            ' ' | '\t'
                | '\u{b}'
                | '\u{c}'
                | '\r'
                | '\n'
                | '_'
                | '.'
                | '+'
                | '-'
                | '/'
                | '*'
                | '%'
                | '<'
                | '>'
                | '['
                | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '^'
                | '|'
                | '&'
                | '~'
                | '='
                | '!'
                | ':'
                | ';'
                | ','
                | '?'
                | '#'
        )
}

/// The tokens of `text`, a line on `line`, pushed to `out`.
pub fn lex(text: &str, line: u32, out: &mut Vec<Token>) -> Result<(), Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == 0x0b || c == 0x0c {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(text[start..i].to_owned()),
                line,
            });
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            let (tok, end) = number(text, i, line)?;
            i = end;
            debug_assert!(i > start);
            out.push(Token { tok, line });
            continue;
        }
        let rest = &text[i..];
        match PUNCTS.iter().find(|p| rest.starts_with(**p)) {
            Some(p) => {
                out.push(Token {
                    tok: Tok::Punct(p),
                    line,
                });
                i += p.len();
            }
            None => {
                let c = rest.chars().next().unwrap_or(' ');
                return Err(Error::new(line, &c.to_string(), "invalid character"));
            }
        }
    }
    Ok(())
}

/// A number at `start`: its token and where it ends.
fn number(text: &str, start: usize, line: u32) -> Result<(Tok, usize), Error> {
    let bytes = text.as_bytes();
    let mut i = start;
    // Hexadecimal.
    if bytes[i] == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X')) {
        i += 2;
        while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
            i += 1;
        }
        if matches!(bytes.get(i), Some(b'u' | b'U')) {
            i += 1;
        }
        let end = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i != end || end == start + 2 {
            return Err(Error::new(line, &text[start..i], "invalid number"));
        }
        return Ok((Tok::Int(text[start..end].to_owned()), end));
    }
    let mut float = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') {
        float = true;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            float = true;
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            return Err(Error::new(line, &text[start..j], "invalid number"));
        }
    }
    // A float's `f`, an integer's `u` (GLSL ES 3.00; the parser refuses
    // them in 1.00).
    if (float && matches!(bytes.get(i), Some(b'f' | b'F')))
        || (!float && matches!(bytes.get(i), Some(b'u' | b'U')))
    {
        i += 1;
    }
    let end = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i != end {
        return Err(Error::new(line, &text[start..i], "invalid number"));
    }
    let literal = text[start..end].to_owned();
    Ok((
        if float {
            Tok::Float(literal)
        } else {
            Tok::Int(literal)
        },
        end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        lex(text, 1, &mut out).unwrap();
        out.iter().map(Token::text).collect()
    }

    #[test]
    fn a_line_is_read_into_names_numbers_and_punctuators() {
        assert_eq!(
            tokens("vec4 a=b<<=2+.5e1*0x1Fu;x++"),
            [
                "vec4", "a", "=", "b", "<<=", "2", "+", ".5e1", "*", "0x1Fu", ";", "x", "++"
            ]
        );
        assert_eq!(tokens("1.0f 3u 1e-3"), ["1.0f", "3u", "1e-3"]);
        let mut out = Vec::new();
        assert!(lex("1x", 1, &mut out).is_err());
        assert!(lex("a $ b", 1, &mut out).is_err());
    }
}
