//! What a script's `style` object reads and writes (M25.2,
//! `docs/design/dom-scripting.md`): the `style` attribute's declarations
//! with their values' text as written, each checked as the cascade would
//! take it; the bindings write the block back. CSSOM's serialization of
//! specified values is G50's: a value is kept as it was given, trimmed.

use css::Token;

/// One declaration of a declaration block, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    /// The property's name, ASCII-lowercased, but a custom property's.
    pub(crate) name: String,
    /// The value's text, trimmed, without `!important`.
    pub(crate) value: String,
    pub(crate) important: bool,
}

/// The declarations of `text`, a declaration block's body (a `style`
/// attribute's, or `cssText`), that the cascade would take: those whose
/// value is valid for a property it knows. A property declared again
/// takes the later's value, in the later's place.
pub(crate) fn declarations(text: &str) -> Vec<Declared> {
    let text = without_comments(text);
    let mut found: Vec<Declared> = Vec::new();
    for piece in split_top_level(&text, ';') {
        let Some((name, value)) = split_once_top_level(piece, ':') else {
            continue;
        };
        let name = name.trim_matches(is_space);
        if name.is_empty() || name.contains(is_space) {
            continue;
        }
        let name = canonical(name);
        let (value, important) = strip_important(value.trim_matches(is_space));
        if !valid(&name, value) {
            continue;
        }
        found.retain(|declared| declared.name != name);
        found.push(Declared {
            name,
            value: value.to_owned(),
            important,
        });
    }
    found
}

/// A property's name as a declaration block keeps it: an alias's
/// property, any other name lowercased, but a custom property's, whose
/// case is its own.
pub(crate) fn canonical(name: &str) -> String {
    if name.starts_with("--") {
        name.to_owned()
    } else {
        style::property_name(&name.to_ascii_lowercase()).to_owned()
    }
}

/// Whether `name` is a property the cascade knows (any takes `initial`).
pub(crate) fn known(name: &str) -> bool {
    valid(name, "initial")
}

/// Whether `value` is valid for `name`, as the cascade decides
/// (`@supports`'s test).
pub(crate) fn valid(name: &str, value: &str) -> bool {
    let tokens: Vec<Token> = css::tokenize(value);
    if tokens
        .iter()
        .all(|token| matches!(token, Token::Whitespace))
    {
        return false;
    }
    style::supports(name, &tokens)
}

fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// `value` without a final `!important` (any case, white space allowed
/// after the `!`), and whether it had one.
fn strip_important(value: &str) -> (&str, bool) {
    let Some(bang) = value.rfind('!') else {
        return (value, false);
    };
    let word = value[bang + 1..].trim_matches(is_space);
    if word.eq_ignore_ascii_case("important") {
        (value[..bang].trim_matches(is_space), true)
    } else {
        (value, false)
    }
}

/// `text` without its comments, those outside strings, which CSS's
/// tokenizer drops as it reads.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '/' && chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for inside in chars.by_ref() {
                    if last == '*' && inside == '/' {
                        break;
                    }
                    last = inside;
                }
            }
            None => {
                out.push(c);
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if c == '"' || c == '\'' {
                    quote = Some(c);
                }
            }
        }
    }
    out
}

/// The pieces of `text` between each `separator` that is outside
/// strings, comments, and brackets of every kind.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut scan = Scanner::default();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if scan.top_level(c, chars.peek().map(|&(_, next)| next)) && c == separator {
            pieces.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// `text` split at its first top-level `separator`.
fn split_once_top_level(text: &str, separator: char) -> Option<(&str, &str)> {
    let mut scan = Scanner::default();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if scan.top_level(c, chars.peek().map(|&(_, next)| next)) && c == separator {
            return Some((&text[..i], &text[i + c.len_utf8()..]));
        }
    }
    None
}

/// Where a scan of a block's text stands: in a string (its quote), in a
/// comment, after a backslash, and how deep in brackets.
#[derive(Default)]
struct Scanner {
    quote: Option<char>,
    comment: bool,
    escaped: bool,
    depth: usize,
    /// The comment's opening `/*`'s star, or its closing `*/`'s star,
    /// seen: the next character is not looked at again.
    skip: bool,
}

impl Scanner {
    /// Takes `c`, `next` after it; whether `c` is at the top level, out of
    /// strings, comments and brackets, and not escaped.
    fn top_level(&mut self, c: char, next: Option<char>) -> bool {
        if self.skip {
            self.skip = false;
            return false;
        }
        if self.escaped {
            self.escaped = false;
            return false;
        }
        if self.comment {
            if c == '*' && next == Some('/') {
                self.comment = false;
                self.skip = true;
            }
            return false;
        }
        if let Some(quote) = self.quote {
            match c {
                '\\' => self.escaped = true,
                c if c == quote => self.quote = None,
                _ => {}
            }
            return false;
        }
        match c {
            '\\' => {
                self.escaped = true;
                false
            }
            '"' | '\'' => {
                self.quote = Some(c);
                false
            }
            '/' if next == Some('*') => {
                self.comment = true;
                self.skip = true;
                false
            }
            '(' | '[' | '{' => {
                self.depth += 1;
                false
            }
            ')' | ']' | '}' => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            _ => self.depth == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block's text: each declaration as `name: value;`, ` !important`
    /// before the semicolon when it is, a space between them.
    fn serialize(declarations: &[Declared]) -> String {
        let mut out = String::new();
        for declared in declarations {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&declared.name);
            out.push_str(": ");
            out.push_str(&declared.value);
            if declared.important {
                out.push_str(" !important");
            }
            out.push(';');
        }
        out
    }

    /// The block's declarations, as name, value and priority.
    fn read(text: &str) -> Vec<(String, String, bool)> {
        declarations(text)
            .into_iter()
            .map(|d| (d.name, d.value, d.important))
            .collect()
    }

    #[test]
    fn a_block_is_read_as_the_cascade_would_take_it() {
        assert_eq!(
            read("color:blue;;margin-top:3px"),
            [
                ("color".into(), "blue".into(), false),
                ("margin-top".into(), "3px".into(), false)
            ]
        );
        // Invalid values and unknown properties go; a later one wins.
        assert_eq!(
            read("COLOR: red; bogus: 1; width: banana; width: 2px !IMPORTANT ; color: RED"),
            [
                ("width".into(), "2px".into(), true),
                ("color".into(), "RED".into(), false)
            ]
        );
        // Separators inside strings, comments and brackets are not.
        assert_eq!(
            read(
                "background-image: url(\"a;b\"); /* c: d; */ font-family: \"x;y\\\"\"; --My-Var: a(b;c) "
            ),
            [
                ("background-image".into(), "url(\"a;b\")".into(), false),
                ("font-family".into(), "\"x;y\\\"\"".into(), false),
                ("--My-Var".into(), "a(b;c)".into(), false)
            ]
        );
        assert_eq!(
            read("col/* x */or: red"),
            [("color".into(), "red".into(), false)]
        );
        assert_eq!(read("color"), []);
        assert_eq!(read("color:"), []);
        assert_eq!(read(": red"), []);
    }

    #[test]
    fn a_block_is_written_as_the_reference_browser_writes_it() {
        let block = declarations("color: red; width: 2px !important");
        assert_eq!(serialize(&block), "color: red; width: 2px !important;");
        assert_eq!(serialize(&[]), "");
        assert!(known("float") && known("-webkit-transform") && known("--x"));
        assert!(!known("bogus"));
        assert_eq!(canonical("-WebKit-Transform"), "transform");
        assert_eq!(canonical("--Mine"), "--Mine");
        assert_eq!(
            serialize(&declarations("-webkit-transform: none")),
            "transform: none;"
        );
    }
}
