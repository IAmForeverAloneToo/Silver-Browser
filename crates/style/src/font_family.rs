//! `font-family`'s values (CSS Fonts 4 section 2.1): a list of family
//! names and generic families, kept as the reference browser writes it.
//! A generic family is its keyword, lowercase; a name is written as
//! identifiers where it reads back as the same name, else as a string,
//! so that a name like `"serif"` stays a name. The text crate reads the
//! list back in that form ([`text`'s `families`]).

use css::Token;

/// The generic family keywords (section 2.1.3), which a family name
/// written unquoted cannot be.
pub const GENERIC_FAMILIES: [&str; 11] = [
    "serif",
    "sans-serif",
    "cursive",
    "fantasy",
    "monospace",
    "system-ui",
    "math",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
];

/// The generic families `generic()` names (section 2.1.3).
const GENERIC_FUNCTIONS: [&str; 4] = ["fangsong", "kai", "khmer-mul", "nastaliq"];

/// The words no `<custom-ident>` is: the CSS-wide keywords and `default`.
fn reserved(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "revert-rule" | "default"
    )
}

/// One entry of a family list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Family {
    /// A generic family, its keyword lowercase (`generic(kai)` whole).
    Generic(String),
    /// A family's name.
    Name(String),
}

/// A `<family-name>`: a string, or identifiers separated by white space
/// (joined by one space), none of them a CSS-wide keyword alone nor a
/// generic family first. `None` for anything else.
pub fn parse_name(tokens: &[Token]) -> Option<String> {
    let words: Vec<&Token> = tokens.iter().filter(|t| **t != Token::Whitespace).collect();
    match words.as_slice() {
        [Token::String { value, bad: false }] if !value.is_empty() => Some(value.clone()),
        [] => None,
        idents => {
            let names: Option<Vec<&str>> = idents
                .iter()
                .map(|t| match t {
                    Token::Ident(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let names = names?;
            if names.len() == 1 && reserved(names[0]) {
                return None;
            }
            Some(names.join(" "))
        }
    }
}

/// A `font-family` list; `None` when it is not one.
pub fn parse_list(tokens: &[Token]) -> Option<Vec<Family>> {
    let mut out = Vec::new();
    for part in tokens.split(|t| *t == Token::Comma) {
        let words: Vec<&Token> = part.iter().filter(|t| **t != Token::Whitespace).collect();
        let family = match words.as_slice() {
            [Token::Ident(word)]
                if GENERIC_FAMILIES.contains(&word.to_ascii_lowercase().as_str()) =>
            {
                Family::Generic(word.to_ascii_lowercase())
            }
            [Token::Function(f), Token::Ident(word), Token::CloseParen]
                if f.eq_ignore_ascii_case("generic")
                    && GENERIC_FUNCTIONS.contains(&word.to_ascii_lowercase().as_str()) =>
            {
                Family::Generic(format!("generic({})", word.to_ascii_lowercase()))
            }
            [Token::Ident(first), _, ..]
                if GENERIC_FAMILIES.contains(&first.to_ascii_lowercase().as_str()) =>
            {
                return None;
            }
            _ => Family::Name(parse_name(part)?),
        };
        out.push(family);
    }
    (!out.is_empty()).then_some(out)
}

/// A family's name as the reference browser writes it: identifiers when
/// each word is one and the whole reads back as this name, else a
/// string.
pub fn write_name(name: &str) -> String {
    let plain = !name.is_empty()
        && name.split(' ').all(is_identifier)
        && !(GENERIC_FAMILIES.contains(&name.to_ascii_lowercase().as_str()) || reserved(name));
    if plain {
        return name.to_owned();
    }
    string(name)
}

/// `text` as a CSS string, as CSSOM serializes one: in double quotes,
/// a quote and a backslash escaped, a control character by its code.
pub fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\{:x} ", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A list as the reference browser writes it, the entries parted by
/// `, `.
pub fn write_list(families: &[Family]) -> String {
    families
        .iter()
        .map(|family| match family {
            Family::Generic(word) => word.clone(),
            Family::Name(name) => write_name(name),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `word` is an identifier as written, needing no escape.
fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    let name_start = |c: char| c.is_ascii_alphabetic() || c == '_' || !c.is_ascii();
    let name = |c: char| name_start(c) || c.is_ascii_digit() || c == '-';
    match chars.next() {
        Some('-') => match chars.next() {
            Some(c) if name_start(c) || c == '-' => chars.all(name),
            _ => false,
        },
        Some(c) if name_start(c) => chars.all(name),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(css: &str) -> Option<String> {
        parse_list(&css::tokenize(css)).map(|l| write_list(&l))
    }

    #[test]
    fn lists_are_names_and_generics() {
        assert_eq!(list("Serif").as_deref(), Some("serif"));
        assert_eq!(list("\"Serif\"").as_deref(), Some("\"Serif\""));
        assert_eq!(
            list("\"New Century Schoolbook\", serif").as_deref(),
            Some("New Century Schoolbook, serif")
        );
        assert_eq!(
            list("'21st Century', fantasy").as_deref(),
            Some("\"21st Century\", fantasy")
        );
        assert_eq!(
            list("\"inherit\", \"serif\"").as_deref(),
            Some("\"inherit\", \"serif\"")
        );
        assert_eq!(list("Arial,Arial").as_deref(), Some("Arial, Arial"));
        assert_eq!(list("\"Arial,\"").as_deref(), Some("\"Arial,\""));
        assert_eq!(list("A   B").as_deref(), Some("A B"));
        assert_eq!(list("\"A  B\"").as_deref(), Some("\"A  B\""));
        assert_eq!(list("generic(KAI), a").as_deref(), Some("generic(kai), a"));
        assert_eq!(
            list("\"Not Initial!\"").as_deref(),
            Some("\"Not Initial!\"")
        );
        for bad in [
            "cursive serif",
            "Red/Black, sans-serif",
            "\"Lucida\" Grande, sans-serif",
            "Ahem!, sans-serif",
            "Hawaii 5-0, sans-serif",
            "FirstName, Initial, LastName",
            "FirstName, LastName, revert-layer",
            "default, FirstName",
            "a,",
            ",a",
            "",
            "generic(foo)",
            "\"\"",
        ] {
            assert_eq!(list(bad), None, "{bad}");
        }
    }
}
