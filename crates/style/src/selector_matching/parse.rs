//! Selector parsing: `css::Token` slices in, [`Selector`] out.
//!
//! Consumes the prelude token vectors the `css` crate stores on
//! qualified rules. A failed parse is a hard error here; per
//! css-syntax-3, the *caller* drops the whole rule containing the bad
//! selector — that policy lives in the engine (M5), not in this parser.
//!
//! The parser works in place over the borrowed tokens with one cursor,
//! including inside `:not()`, `:is()`, `:where()` and `:nth-child(… of
//! S)`: nothing is copied per nesting level, so a selector that nests
//! those a thousand deep costs a thousand frames of work, not a
//! thousand copies of itself. Nesting past [`MAX_NESTING`] levels is a
//! parse error, which bounds the recursion; no real stylesheet comes
//! near it. `:is()` and `:where()` take a forgiving selector list
//! (selectors-4 §4.2): a member that does not parse is dropped and the
//! rest stand, where `:not()` and the rule's own list fail whole.

use crate::ParseError;
use css::Token;

use super::{
    AttrOp, Combinator, CompoundSelector, PseudoClass, Selector, SelectorList, SimpleSelector,
    Specificity,
};

/// How deep `:not()`, `:is()`, `:where()` and `of S` may nest.
pub const MAX_NESTING: usize = 32;

/// Parses a comma-separated selector list from prelude tokens. A
/// trailing comma, an empty member or an unbalanced parenthesis is an
/// error (selectors-4 §5.1: `<complex-selector>#`, no empty members).
pub fn parse_selector_list(tokens: &[Token]) -> Result<SelectorList, ParseError> {
    let mut c = Cursor::new(tokens);
    let list = parse_list(&mut c, 0, false, false)?;
    c.skip_ws();
    if !c.at_end() {
        return Err(ParseError::InvalidSelector);
    }
    Ok(list)
}

/// A peekable cursor over a token slice.
struct Cursor<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Cursor { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// Skips whitespace; returns whether any was skipped.
    fn skip_ws(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        self.pos > start
    }
}

/// Parses a selector list at the cursor: complex selectors separated by
/// top-level commas, ending at the end of the tokens or, when
/// `in_parens`, before the `)` that closes the enclosing function
/// (left for the caller to consume). With `forgiving`, a member that
/// fails to parse is skipped rather than failing the list, and the
/// list may come out empty.
fn parse_list(
    c: &mut Cursor,
    depth: usize,
    in_parens: bool,
    forgiving: bool,
) -> Result<SelectorList, ParseError> {
    if depth > MAX_NESTING {
        return Err(ParseError::InvalidSelector);
    }
    let mut selectors = Vec::new();
    loop {
        c.skip_ws();
        let start = c.pos;
        match parse_complex(c, depth) {
            Ok(selector) => selectors.push(selector),
            Err(e) => {
                if !forgiving {
                    return Err(e);
                }
                c.pos = start;
                skip_member(c);
            }
        }
        c.skip_ws();
        match c.peek() {
            Some(Token::Comma) => {
                c.advance();
                c.skip_ws();
                let at_list_end =
                    c.at_end() || (in_parens && matches!(c.peek(), Some(Token::CloseParen)));
                if at_list_end {
                    if forgiving {
                        break; // `:is(.a,)`: the empty member is dropped
                    }
                    return Err(ParseError::InvalidSelector);
                }
            }
            Some(Token::CloseParen) if in_parens => break,
            None => break,
            _ => return Err(ParseError::InvalidSelector),
        }
    }
    if selectors.is_empty() && !forgiving {
        return Err(ParseError::InvalidSelector);
    }
    Ok(SelectorList(selectors))
}

/// Skips a forgiving list's bad member: up to, not including, the next
/// top-level comma or the `)` that closes the list.
fn skip_member(c: &mut Cursor) {
    let mut depth = 0i32;
    while let Some(t) = c.peek() {
        match t {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseSquare => depth -= 1,
            Token::CloseParen => {
                if depth == 0 {
                    return;
                }
                depth -= 1;
            }
            Token::Comma if depth == 0 => return,
            _ => {}
        }
        c.advance();
    }
}

/// Parses one complex selector at the cursor, stopping before a
/// top-level comma, a closing `)`, or the end.
fn parse_complex(c: &mut Cursor, depth: usize) -> Result<Selector, ParseError> {
    c.skip_ws();
    let (first, mut spec) = parse_compound(c, depth).ok_or(ParseError::InvalidSelector)?;
    let mut compounds = vec![first];
    let mut combinators = Vec::new();
    loop {
        let had_ws = c.skip_ws();
        if let Some(comb) = peek_combinator(c) {
            c.advance();
            c.skip_ws();
            let (next, s) = parse_compound(c, depth).ok_or(ParseError::InvalidSelector)?;
            spec += s;
            combinators.push(comb);
            compounds.push(next);
        } else if had_ws && starts_compound(c.peek()) {
            let (next, s) = parse_compound(c, depth).ok_or(ParseError::InvalidSelector)?;
            spec += s;
            combinators.push(Combinator::Descendant);
            compounds.push(next);
        } else {
            break;
        }
    }
    Ok(Selector {
        compounds,
        combinators,
        specificity: spec,
    })
}

/// The combinator at the cursor, if any.
fn peek_combinator(c: &Cursor) -> Option<Combinator> {
    match c.peek() {
        Some(Token::Delim('>')) => Some(Combinator::Child),
        Some(Token::Delim('+')) => Some(Combinator::NextSibling),
        Some(Token::Delim('~')) => Some(Combinator::LaterSibling),
        _ => None,
    }
}

/// Whether a token can begin a compound selector.
fn starts_compound(t: Option<&Token>) -> bool {
    matches!(
        t,
        Some(
            Token::Ident(_)
                | Token::Delim('*')
                | Token::Delim('.')
                | Token::Hash { .. }
                | Token::OpenSquare
                | Token::Colon
        )
    )
}

/// Parses one compound selector (no combinators). Returns the compound
/// and its specificity contribution.
fn parse_compound(c: &mut Cursor, depth: usize) -> Option<(CompoundSelector, Specificity)> {
    let mut simple: Vec<SimpleSelector> = Vec::new();
    let mut spec = Specificity::default();
    let mut seen_type = false;
    while let Some(t) = c.peek() {
        match t {
            Token::Ident(name) => {
                // A type selector must be first in the compound.
                if seen_type || !simple.is_empty() {
                    return None;
                }
                seen_type = true;
                simple.push(SimpleSelector::Type(name.to_lowercase()));
                spec.2 += 1;
            }
            Token::Delim('*') => {
                if seen_type || !simple.is_empty() {
                    return None;
                }
                seen_type = true;
                simple.push(SimpleSelector::Universal);
                // Universal adds no specificity.
            }
            Token::Hash { value, is_id: true } => {
                simple.push(SimpleSelector::Id(value.clone()));
                spec.0 += 1;
            }
            // '#123' — a hash that is not an id is never a valid selector.
            Token::Hash { .. } => return None,
            Token::Delim('.') => {
                c.advance();
                match c.peek() {
                    Some(Token::Ident(name)) => {
                        simple.push(SimpleSelector::Class(name.clone()));
                        spec.1 += 1;
                    }
                    _ => return None,
                }
            }
            // Namespaces are unsupported (documented subset).
            Token::Delim('|') => return None,
            Token::OpenSquare => {
                let (name, op, value, case_insensitive) = parse_attribute(c)?;
                spec.1 += 1;
                simple.push(SimpleSelector::Attribute {
                    name,
                    op,
                    value,
                    case_insensitive,
                });
                continue; // parse_attribute consumed through `]`
            }
            Token::Colon => {
                let pseudo = parse_pseudo(c, &mut spec, depth)?;
                simple.push(SimpleSelector::PseudoClass(pseudo));
                continue; // parse_pseudo consumed its tokens
            }
            _ => break,
        }
        c.advance();
    }
    if simple.is_empty() {
        return None;
    }
    Some((CompoundSelector { simple }, spec))
}

/// Parses an attribute selector; the cursor is at `[`.
fn parse_attribute(c: &mut Cursor) -> Option<(String, AttrOp, String, bool)> {
    c.advance(); // '['
    c.skip_ws();
    let name = match c.peek() {
        Some(Token::Ident(name)) => name.to_lowercase(),
        _ => return None,
    };
    c.advance();
    c.skip_ws();
    let op = match c.peek() {
        Some(Token::CloseSquare) => {
            c.advance();
            return Some((name, AttrOp::Exists, String::new(), false));
        }
        Some(Token::Delim('=')) => {
            c.advance();
            AttrOp::Equals
        }
        Some(&Token::Delim(op @ ('~' | '|' | '^' | '$' | '*'))) => {
            c.advance();
            if !matches!(c.peek(), Some(Token::Delim('='))) {
                return None;
            }
            c.advance();
            match op {
                '~' => AttrOp::IncludeSpace,
                '|' => AttrOp::Dash,
                '^' => AttrOp::Prefix,
                '$' => AttrOp::Suffix,
                _ => AttrOp::Substring,
            }
        }
        _ => return None,
    };
    c.skip_ws();
    // The value: a string or an identifier (selectors-4 §6.1). A bare
    // number (`[tabindex=0]`) is not in the grammar; browsers drop the
    // rule, and so does this.
    let value = match c.peek() {
        Some(Token::String { value, bad: false }) => {
            let v = value.clone();
            c.advance();
            v
        }
        Some(Token::Ident(s)) => {
            let v = s.clone();
            c.advance();
            v
        }
        _ => return None,
    };
    c.skip_ws();
    // Optional case flag (`i` or `s`).
    let mut case_insensitive = false;
    if let Some(Token::Ident(flag)) = c.peek() {
        match flag.as_str() {
            "i" | "I" => {
                case_insensitive = true;
                c.advance();
                c.skip_ws();
            }
            "s" | "S" => {
                c.advance();
                c.skip_ws();
            }
            _ => {}
        }
    }
    if !matches!(c.peek(), Some(Token::CloseSquare)) {
        return None;
    }
    c.advance();
    Some((name, op, value, case_insensitive))
}

/// Parses a pseudo-class; the cursor is at `:`. Updates `spec` with the
/// specificity contribution (selectors-4 §17: `:not()`/`:is()` count
/// the max of their arguments; `:where()` counts zero).
fn parse_pseudo(c: &mut Cursor, spec: &mut Specificity, depth: usize) -> Option<PseudoClass> {
    c.advance(); // ':'
    match c.peek() {
        // Pseudo-elements (`::before` etc.) are the M16 milestone.
        Some(Token::Colon) => None,
        Some(Token::Ident(name)) => {
            let pc = match name.to_lowercase().as_str() {
                "first-child" => PseudoClass::FirstChild,
                "last-child" => PseudoClass::LastChild,
                "only-child" => PseudoClass::OnlyChild,
                "first-of-type" => PseudoClass::FirstOfType,
                "last-of-type" => PseudoClass::LastOfType,
                "only-of-type" => PseudoClass::OnlyOfType,
                "empty" => PseudoClass::Empty,
                "root" => PseudoClass::Root,
                // Dynamic: parseable, but never matched until M26.
                "hover" => PseudoClass::Hover,
                "focus" => PseudoClass::Focus,
                "active" => PseudoClass::Active,
                "visited" => PseudoClass::Visited,
                "link" => PseudoClass::Link,
                _ => return None,
            };
            c.advance();
            *spec += Specificity(0, 1, 0);
            Some(pc)
        }
        Some(Token::Function(name)) => {
            let name = name.to_lowercase();
            c.advance();
            let pc = match name.as_str() {
                "not" => {
                    let list = parse_list(c, depth + 1, true, false).ok()?;
                    *spec += max_specificity(&list);
                    PseudoClass::Not(list)
                }
                "is" => {
                    let list = parse_list(c, depth + 1, true, true).ok()?;
                    *spec += max_specificity(&list);
                    PseudoClass::Is(list)
                }
                "where" => {
                    let list = parse_list(c, depth + 1, true, true).ok()?;
                    PseudoClass::Where(list) // zero specificity by design
                }
                "nth-child" => {
                    *spec += Specificity(0, 1, 0);
                    let (a, b, of) = parse_nth(c, true, depth)?;
                    PseudoClass::NthChild { a, b, of }
                }
                "nth-last-child" => {
                    *spec += Specificity(0, 1, 0);
                    // `of S` is M4 scope only for `:nth-child` (roadmap
                    // M4.5); `NthLastChild` has no `of` field, so reject
                    // it here rather than silently drop it.
                    let (a, b, _of) = parse_nth(c, false, depth)?;
                    PseudoClass::NthLastChild { a, b }
                }
                "nth-of-type" => {
                    *spec += Specificity(0, 1, 0);
                    let (a, b, _) = parse_nth(c, false, depth)?;
                    PseudoClass::NthOfType { a, b }
                }
                "nth-last-of-type" => {
                    *spec += Specificity(0, 1, 0);
                    let (a, b, _) = parse_nth(c, false, depth)?;
                    PseudoClass::NthLastOfType { a, b }
                }
                _ => return None,
            };
            // The function's closing `)`.
            if !matches!(c.peek(), Some(Token::CloseParen)) {
                return None;
            }
            c.advance();
            Some(pc)
        }
        _ => None,
    }
}

/// Parses `An+B [of S]` at the cursor (just after the function token),
/// stopping before the closing `)`. `allow_of` selects the nth-child
/// forms (of-type never takes `of S` per selectors-4).
fn parse_nth(
    c: &mut Cursor,
    allow_of: bool,
    depth: usize,
) -> Option<(i32, i32, Option<Box<SelectorList>>)> {
    // The An+B part: everything up to a top-level `of` or the `)` (a
    // handful of tokens, copied; the selector list after `of` is not).
    let mut head: Vec<Token> = Vec::new();
    let mut of = false;
    while let Some(t) = c.peek() {
        match t {
            Token::CloseParen => break,
            Token::Ident(s) if s.eq_ignore_ascii_case("of") => {
                of = true;
                c.advance();
                break;
            }
            Token::Function(_) | Token::OpenParen | Token::OpenSquare => return None,
            _ => head.push(t.clone()),
        }
        c.advance();
    }
    let (a, b) = parse_an_b(&head)?;
    if !of {
        return Some((a, b, None));
    }
    if !allow_of {
        return None;
    }
    let list = parse_list(c, depth + 1, true, false).ok()?;
    Some((a, b, Some(Box::new(list))))
}

/// Parses the An+B microgrammar (selectors-4 §4, css-syntax-3 §8.2):
/// `odd`, `even`, integers, `n`, `-n`, `2n`, `2n+1`, `-n+3`, with
/// whitespace tolerated between the parts (the token stream keeps it).
fn parse_an_b(tokens: &[Token]) -> Option<(i32, i32)> {
    let t: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    // A leading explicit '+' never changes anything. It only ever shows
    // up as its own token in front of a bare ident (`+n`) — a digit
    // right after it would already have fused into a signed number at
    // the tokenizer level (see below) — so strip it up front and let
    // everything else see one shape.
    let t: &[&Token] = match t.as_slice() {
        [Token::Delim('+'), rest @ ..] => rest,
        slice => slice,
    };
    match t {
        [Token::Ident(s)] if s.eq_ignore_ascii_case("odd") => return Some((2, 1)),
        [Token::Ident(s)] if s.eq_ignore_ascii_case("even") => return Some((2, 0)),
        [Token::Number { value, .. }] => return Some((0, f64_to_i32(*value)?)),
        _ => {}
    }
    // The `An` part: a bare `n`/`-n` ident, or a signed dimension whose
    // unit starts with `n` — either may carry a *fused* "-digits" suffix
    // supplying `B` directly, per the tokenizer quirk below. The checks
    // are on bytes: an ident can start with a multi-byte character, and
    // slicing a `str` at byte 2 would panic inside it.
    let (a, name_rest) = match t.first()? {
        Token::Ident(s) if matches!(s.as_bytes(), [b'-', b'n' | b'N', ..]) => (-1, &s[2..]),
        Token::Ident(s) => (1, strip_n_prefix(s)?),
        Token::Dimension { value, unit } => (f64_to_i32(*value)?, strip_n_prefix(unit)?),
        _ => return None,
    };
    let rest = &t[1..];
    if name_rest.is_empty() {
        // No fused suffix: an optional separate signed B follows. A sign
        // immediately followed by a digit (no whitespace between them,
        // e.g. "2n+1") tokenizes as one signed Number — the css
        // tokenizer only leaves the sign as its own Delim when source
        // whitespace forced it apart (the last two arms below), and
        // then the number after it must be signless (§8.2's
        // `<signless-integer>`): `n - -1` is not `n+1`.
        return match rest {
            [] => Some((a, 0)),
            [Token::Number { value, .. }] => Some((a, f64_to_i32(*value)?)),
            [Token::Delim('+'), Token::Number { value, .. }] if *value >= 0.0 => {
                Some((a, f64_to_i32(*value)?))
            }
            [Token::Delim('-'), Token::Number { value, .. }] if *value >= 0.0 => {
                Some((a, f64_to_i32(*value)?.checked_neg()?))
            }
            _ => None,
        };
    }
    // A fused "-digits" suffix (from an unspaced "-B": `-` is a
    // name-continuation code point, so e.g. "n-1" or "2n-1" tokenizes as
    // ONE ident/dimension, name/unit `"n-1"`, rather than splitting the
    // way an unspaced "+B" does). It supplies B directly, so nothing may
    // follow it. The bare `n-` form (`2n- 1`) is §8.2's
    // `<ndash-dimension> <signless-integer>`: the digits follow as a
    // number of their own.
    match (name_rest, rest) {
        ("-", [Token::Number { value, .. }]) if *value >= 0.0 => {
            Some((a, f64_to_i32(*value)?.checked_neg()?))
        }
        (_, []) => Some((a, trailing_minus_digits(name_rest)?)),
        _ => None,
    }
}

/// Strips a single leading `n`/`N`, if present.
fn strip_n_prefix(s: &str) -> Option<&str> {
    match s.as_bytes().first() {
        Some(b) if b.eq_ignore_ascii_case(&b'n') => Some(&s[1..]),
        _ => None,
    }
}

/// `"-123"` -> `Some(-123)`; anything else (including plain `""`, which
/// callers check for separately) -> `None`.
fn trailing_minus_digits(s: &str) -> Option<i32> {
    let digits = s.strip_prefix('-')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i32>().ok().map(|d| -d)
}

/// Integral f64 to i32, or None for fractional/out-of-range values.
fn f64_to_i32(v: f64) -> Option<i32> {
    if v.fract() != 0.0 || v < i32::MIN as f64 || v > i32::MAX as f64 {
        None
    } else {
        Some(v as i32)
    }
}

/// The max specificity in a list (for `:is()`/`:not()`).
fn max_specificity(list: &SelectorList) -> Specificity {
    list.0
        .iter()
        .map(|s| s.specificity)
        .max()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a selector string (through the css tokenizer, like a
    /// real rule prelude).
    fn parse(s: &str) -> Result<SelectorList, ParseError> {
        parse_selector_list(&css::tokenize(s))
    }

    fn ok(s: &str) -> SelectorList {
        parse(s).expect("selector must parse")
    }

    fn err(s: &str) {
        assert!(
            parse(s).is_err(),
            "selector must NOT parse: {s:?} -> {:?}",
            parse(s)
        );
    }

    // ---- simple selectors ------------------------------------------------

    #[test]
    fn parses_type() {
        assert_eq!(
            ok("div").0[0].compounds[0].simple,
            vec![SimpleSelector::Type("div".into())]
        );
    }

    #[test]
    fn type_selectors_are_lowercased() {
        assert_eq!(
            ok("DIV").0[0].compounds[0].simple,
            vec![SimpleSelector::Type("div".into())]
        );
    }

    #[test]
    fn parses_universal() {
        assert_eq!(
            ok("*").0[0].compounds[0].simple,
            vec![SimpleSelector::Universal]
        );
    }

    #[test]
    fn parses_class() {
        assert_eq!(
            ok(".card").0[0].compounds[0].simple,
            vec![SimpleSelector::Class("card".into())]
        );
    }

    #[test]
    fn parses_id() {
        assert_eq!(
            ok("#main").0[0].compounds[0].simple,
            vec![SimpleSelector::Id("main".into())]
        );
    }

    #[test]
    fn parses_compound() {
        assert_eq!(
            ok("div.card#main").0[0].compounds[0].simple,
            vec![
                SimpleSelector::Type("div".into()),
                SimpleSelector::Class("card".into()),
                SimpleSelector::Id("main".into()),
            ]
        );
    }

    #[test]
    fn hash_without_id_flag_is_invalid() {
        err("#123");
    }

    #[test]
    fn dot_must_be_followed_by_ident() {
        err(".");
        err(".5");
    }

    #[test]
    fn universal_must_come_first() {
        err(".a*");
        err("*div");
    }

    #[test]
    fn namespaces_are_unsupported() {
        err("ns|div");
        err("|div");
        err("*|div");
    }

    // ---- attributes -------------------------------------------------------

    #[test]
    fn parses_attribute_exists() {
        assert_eq!(
            ok("[disabled]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "disabled".into(),
                op: AttrOp::Exists,
                value: String::new(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn parses_all_attribute_operators() {
        for (src, op) in [
            ("[a=b]", AttrOp::Equals),
            ("[a~=b]", AttrOp::IncludeSpace),
            ("[a|=b]", AttrOp::Dash),
            ("[a^=b]", AttrOp::Prefix),
            ("[a$=b]", AttrOp::Suffix),
            ("[a*=b]", AttrOp::Substring),
        ] {
            assert_eq!(
                ok(src).0[0].compounds[0].simple,
                vec![SimpleSelector::Attribute {
                    name: "a".into(),
                    op,
                    value: "b".into(),
                    case_insensitive: false,
                }],
                "operator mismatch for {src}"
            );
        }
    }

    #[test]
    fn parses_quoted_attribute_values() {
        assert_eq!(
            ok(r#"[href="https://x"]"#).0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "href".into(),
                op: AttrOp::Equals,
                value: "https://x".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn parses_case_flags() {
        assert_eq!(
            ok("[a=b i]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "a".into(),
                op: AttrOp::Equals,
                value: "b".into(),
                case_insensitive: true,
            }]
        );
        assert_eq!(
            ok("[a=B s]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "a".into(),
                op: AttrOp::Equals,
                value: "B".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn attribute_name_is_lowercased() {
        assert_eq!(
            ok("[DATA-X=y]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "data-x".into(),
                op: AttrOp::Equals,
                value: "y".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn numeric_attribute_values_are_not_in_the_grammar() {
        // selectors-4 §6.1: the value is a string or an identifier.
        // `[tabindex=0]` throws in every browser's querySelector.
        err("[tabindex=0]");
        err("[data-n=1e3]");
        assert_eq!(
            ok("[tabindex='0']").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "tabindex".into(),
                op: AttrOp::Equals,
                value: "0".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn bad_attributes_error() {
        err("[=b]");
        err("[a=]");
        err("[a~b]");
        err("[a");
        err("[a=b");
        err("[a=b x]");
    }

    // ---- combinators ------------------------------------------------------

    #[test]
    fn parses_descendant() {
        let s = &ok("div p").0[0];
        assert_eq!(s.combinators, vec![Combinator::Descendant]);
        assert_eq!(s.compounds.len(), 2);
    }

    #[test]
    fn parses_child() {
        assert_eq!(ok("div > p").0[0].combinators, vec![Combinator::Child]);
        assert_eq!(ok("div>p").0[0].combinators, vec![Combinator::Child]);
        assert_eq!(ok("div> p").0[0].combinators, vec![Combinator::Child]);
        assert_eq!(ok("div >p").0[0].combinators, vec![Combinator::Child]);
    }

    #[test]
    fn parses_siblings() {
        assert_eq!(ok("a + b").0[0].combinators, vec![Combinator::NextSibling]);
        assert_eq!(ok("a ~ b").0[0].combinators, vec![Combinator::LaterSibling]);
    }

    #[test]
    fn parses_chained_combinators() {
        let s = &ok("a b > c + d ~ e").0[0];
        assert_eq!(
            s.combinators,
            vec![
                Combinator::Descendant,
                Combinator::Child,
                Combinator::NextSibling,
                Combinator::LaterSibling,
            ]
        );
        assert_eq!(s.compounds.len(), 5);
    }

    #[test]
    fn leading_or_trailing_combinators_error() {
        err("> a");
        err("a >");
        err("+ a");
        err("~ a");
        err("a ~");
    }

    #[test]
    fn empty_selector_errors() {
        err("");
        err("   ");
    }

    // ---- pseudo-classes ----------------------------------------------------

    #[test]
    fn parses_structural_pseudo_classes() {
        for (src, expected) in [
            (":first-child", PseudoClass::FirstChild),
            (":last-child", PseudoClass::LastChild),
            (":only-child", PseudoClass::OnlyChild),
            (":first-of-type", PseudoClass::FirstOfType),
            (":last-of-type", PseudoClass::LastOfType),
            (":only-of-type", PseudoClass::OnlyOfType),
            (":empty", PseudoClass::Empty),
            (":root", PseudoClass::Root),
            (":hover", PseudoClass::Hover),
            (":focus", PseudoClass::Focus),
            (":active", PseudoClass::Active),
            (":visited", PseudoClass::Visited),
            (":link", PseudoClass::Link),
        ] {
            assert_eq!(
                ok(src).0[0].compounds[0].simple,
                vec![SimpleSelector::PseudoClass(expected)],
                "mismatch for {src}"
            );
        }
    }

    #[test]
    fn pseudo_classes_are_case_insensitive() {
        assert_eq!(
            ok(":FIRST-CHILD").0[0].compounds[0].simple,
            vec![SimpleSelector::PseudoClass(PseudoClass::FirstChild)]
        );
    }

    #[test]
    fn unknown_pseudo_class_errors() {
        err(":hoverish");
        err(":2px");
        err(":");
    }

    #[test]
    fn pseudo_elements_error_for_now() {
        err("::before");
        err("a::after");
    }

    #[test]
    fn parses_not_is_where() {
        let list = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(pc) => match pc {
                PseudoClass::Not(l) | PseudoClass::Is(l) | PseudoClass::Where(l) => l.clone(),
                other => panic!("expected logical, got {other:?}"),
            },
            other => panic!("expected pseudo, got {other:?}"),
        };
        assert_eq!(list(":not(.a)").0.len(), 1);
        assert_eq!(list(":is(a, .b, #c)").0.len(), 3);
        assert_eq!(list(":where(a b, c)").0.len(), 2);
        // Nested logicals: `:not`'s list holds exactly one complex
        // selector — ":is(a, b)" — the nesting is inside *that* one
        // selector, not a top-level comma in `:not`'s own list.
        assert_eq!(list(":not(:is(a, b))").0.len(), 1);
    }

    #[test]
    fn empty_not_errors_but_empty_is_and_where_are_valid() {
        err(":not()");
        // selectors-4 §4.2: a forgiving list may be empty; it then
        // matches nothing (and `:where()` has no specificity to count).
        assert!(parse(":is()").is_ok());
        assert!(parse(":where()").is_ok());
    }

    #[test]
    fn parses_nth_arguments() {
        let nth = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => (*a, *b),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert_eq!(nth(":nth-child(odd)"), (2, 1));
        assert_eq!(nth(":nth-child(even)"), (2, 0));
        assert_eq!(nth(":nth-child(5)"), (0, 5));
        assert_eq!(nth(":nth-child(n)"), (1, 0));
        assert_eq!(nth(":nth-child(-n)"), (-1, 0));
        assert_eq!(nth(":nth-child(2n)"), (2, 0));
        assert_eq!(nth(":nth-child(2n+1)"), (2, 1));
        assert_eq!(nth(":nth-child(-n+3)"), (-1, 3));
        assert_eq!(nth(":nth-child(n+3)"), (1, 3));
        assert_eq!(nth(":nth-child(0n+2)"), (0, 2));
        assert_eq!(nth(":nth-child(+2n+1)"), (2, 1));
        // Whitespace inside An+B is tolerated.
        assert_eq!(nth(":nth-child(2n + 1)"), (2, 1));
    }

    #[test]
    fn nth_case_insensitive_units() {
        let nth = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => (*a, *b),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert_eq!(nth(":nth-child(2N+1)"), (2, 1));
        assert_eq!(nth(":nth-child(ODD)"), (2, 1));
    }

    #[test]
    fn bad_nth_errors() {
        err(":nth-child()");
        err(":nth-child(2.5)");
        err(":nth-child(2n+)");
        err(":nth-child(n b)");
        err(":nth-child(of .a)");
    }

    #[test]
    fn parses_nth_of_s() {
        let of = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { of, .. }) => of.is_some(),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert!(of(":nth-child(2n+1 of .item)"));
        assert!(of(":nth-child(odd of li, .x)"));
        assert!(!of(":nth-child(2n+1)"));
        // of-type takes no `of`.
        err(":nth-of-type(2n of .a)");
    }

    // ---- selector lists ----------------------------------------------------

    #[test]
    fn parses_comma_lists() {
        let list = ok("a, .b, #c");
        assert_eq!(list.0.len(), 3);
        assert_eq!(ok("a , b").0.len(), 2);
        // A trailing comma is an empty member: the whole list is invalid
        // (selectors-4 §5.1), so the rule is dropped as browsers drop it.
        err("a , b,");
        err("a,,b");
    }

    #[test]
    fn empty_list_segments_error() {
        err("a,,b");
        err(",a");
    }

    #[test]
    fn commas_inside_functions_do_not_split() {
        let list = ok(":is(a, b), c");
        assert_eq!(list.0.len(), 2);
    }

    // ---- specificity --------------------------------------------------------

    fn spec(s: &str) -> Specificity {
        ok(s).0[0].specificity
    }

    #[test]
    fn specificity_counts() {
        assert_eq!(spec("div"), Specificity(0, 0, 1));
        assert_eq!(spec("*"), Specificity(0, 0, 0));
        assert_eq!(spec(".a"), Specificity(0, 1, 0));
        assert_eq!(spec("#a"), Specificity(1, 0, 0));
        assert_eq!(spec("[a]"), Specificity(0, 1, 0));
        assert_eq!(spec(":hover"), Specificity(0, 1, 0));
        assert_eq!(spec("div.a#b"), Specificity(1, 1, 1));
        assert_eq!(spec("div p"), Specificity(0, 0, 2));
        assert_eq!(spec("#a .b"), Specificity(1, 1, 0));
    }

    #[test]
    fn specificity_of_where_is_zero() {
        assert_eq!(spec(":where(#a, .b)"), Specificity(0, 0, 0));
    }

    #[test]
    fn specificity_of_is_is_max_of_arguments() {
        assert_eq!(spec(":is(#a, .b)"), Specificity(1, 0, 0));
        assert_eq!(spec(":is(.b, #a)"), Specificity(1, 0, 0));
        assert_eq!(spec(":not(.b)"), Specificity(0, 1, 0));
    }

    #[test]
    fn specificity_of_nth_counts_as_one_class() {
        assert_eq!(spec(":nth-child(2n+1 of .a)"), Specificity(0, 1, 0));
    }

    #[test]
    fn specificity_ordering_is_lexicographic() {
        assert!(Specificity(0, 1, 0) > Specificity(0, 0, 99));
        assert!(Specificity(1, 0, 0) > Specificity(0, 99, 99));
    }

    // ---- An+B unit checks ---------------------------------------------------

    #[test]
    fn an_b_edge_forms() {
        assert_eq!(parse_an_b(&css::tokenize("3")), Some((0, 3)));
        assert_eq!(parse_an_b(&css::tokenize("+7")), Some((0, 7)));
        assert_eq!(parse_an_b(&css::tokenize("-2n-1")), Some((-2, -1)));
        assert_eq!(parse_an_b(&css::tokenize("n-2")), Some((1, -2)));
        assert_eq!(parse_an_b(&css::tokenize("0n")), Some((0, 0)));
        assert_eq!(parse_an_b(&css::tokenize("junk")), None);
        assert_eq!(parse_an_b(&css::tokenize("")), None);
    }

    // The unspaced "-B" fused-token shape is exactly where the earlier
    // implementation broke (`-` is a name-continuation code point, so it
    // fuses into the preceding ident/dimension rather than tokenizing
    // separately) — extra direct coverage here, beyond an_b_edge_forms.
    #[test]
    fn an_b_fused_minus_forms() {
        assert_eq!(parse_an_b(&css::tokenize("n-1")), Some((1, -1)));
        assert_eq!(parse_an_b(&css::tokenize("2n-1")), Some((2, -1)));
        assert_eq!(parse_an_b(&css::tokenize("-n-1")), Some((-1, -1)));
        assert_eq!(parse_an_b(&css::tokenize("N-1")), Some((1, -1))); // case-insensitive
        assert_eq!(parse_an_b(&css::tokenize("10n-100")), Some((10, -100)));
        // A fused "-digits" suffix leaves nothing to follow.
        assert_eq!(parse_an_b(&css::tokenize("n-1 1")), None);
    }

    // The "+n" bare form (no coefficient): '+' is not a name-continuation
    // code point, so it tokenizes as its own `Delim`, separate from the
    // `n` ident — unlike the "-n" form, which fuses into one token.
    #[test]
    fn an_b_bare_plus_n() {
        assert_eq!(parse_an_b(&css::tokenize("+n")), Some((1, 0)));
        assert_eq!(parse_an_b(&css::tokenize("+n+2")), Some((1, 2)));
        assert_eq!(parse_an_b(&css::tokenize("+n-2")), Some((1, -2)));
    }

    #[test]
    fn an_b_malformed_units_reject() {
        assert_eq!(parse_an_b(&css::tokenize("2px")), None);
        assert_eq!(parse_an_b(&css::tokenize("nx")), None);
        assert_eq!(parse_an_b(&css::tokenize("n2")), None); // no hyphen: not "n-digits"
        assert_eq!(parse_an_b(&css::tokenize("-nx")), None);
    }

    // End-to-end: the unspaced-minus fusion also has to work through the
    // full `:nth-child()` parse, not just the `parse_an_b` unit.
    #[test]
    fn nth_child_unspaced_minus_end_to_end() {
        let nth = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => (*a, *b),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert_eq!(nth(":nth-child(2n-1)"), (2, -1));
        assert_eq!(nth(":nth-child(n-1)"), (1, -1));
    }

    #[test]
    fn nth_last_child_rejects_of() {
        // Unlike `:nth-child`, `:nth-last-child` has no `of S` support in
        // this engine (roadmap M4.5 scopes `of S` to nth-child only) — it
        // must be a hard parse error, not a silently-dropped filter.
        err(":nth-last-child(1 of .x)");
    }

    #[test]
    fn nth_of_type_variants_reject_of() {
        err(":nth-of-type(2n of .a)");
        err(":nth-last-of-type(1 of .a)");
    }

    #[test]
    fn deeply_nested_logical_selectors_parse() {
        assert!(ok(":not(:is(:not(.a)))").0.len() == 1);
        let s = ":is(:not(.a), :where(.b, .c))";
        assert_eq!(ok(s).0.len(), 1);
    }

    #[test]
    fn specificity_of_fully_loaded_compound() {
        // One id, two classes (one via `.x`, one via an attribute-style
        // pseudo-class contribution), one type: (1, 2, 1).
        assert_eq!(spec("div#a.b:hover"), Specificity(1, 2, 1));
    }

    #[test]
    fn trailing_garbage_after_valid_selector_errors() {
        err("div)");
        err("div]");
        err("div{");
    }

    #[test]
    fn repeated_combinators_error() {
        err("div >> p");
        err("div ++ p");
        err("div > > p");
    }

    #[test]
    fn unbalanced_pseudo_class_parens_error() {
        err(":not(.a");
        err(":is(a, b");
        err(":nth-child(2n+1");
    }

    #[test]
    fn attribute_value_with_internal_whitespace_requires_quotes() {
        // An unquoted attribute value can't contain whitespace at all —
        // the css tokenizer would end the value at the first space.
        assert_eq!(
            ok(r#"[title="hello world"]"#).0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "title".into(),
                op: AttrOp::Equals,
                value: "hello world".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn attribute_name_with_digits_and_hyphens() {
        assert_eq!(
            ok("[aria-level2=x]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "aria-level2".into(),
                op: AttrOp::Equals,
                value: "x".into(),
                case_insensitive: false,
            }]
        );
    }

    #[test]
    fn multiple_classes_on_one_compound() {
        assert_eq!(
            ok(".a.b.c").0[0].compounds[0].simple,
            vec![
                SimpleSelector::Class("a".into()),
                SimpleSelector::Class("b".into()),
                SimpleSelector::Class("c".into()),
            ]
        );
    }

    #[test]
    fn combinators_tolerate_extra_internal_whitespace() {
        assert_eq!(ok("div   >   p").0[0].combinators, vec![Combinator::Child]);
        assert_eq!(
            ok("  div   p  ").0[0].combinators,
            vec![Combinator::Descendant]
        );
    }

    #[test]
    fn nth_child_negative_a_only() {
        let nth = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => (*a, *b),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert_eq!(nth(":nth-child(-2n)"), (-2, 0));
    }

    #[test]
    fn pseudo_class_function_args_tolerate_surrounding_whitespace() {
        assert_eq!(
            ok(":not( .a )").0[0].compounds[0].simple[0],
            ok(":not(.a)").0[0].compounds[0].simple[0]
        );
        assert_eq!(
            ok(":nth-child( 2n + 1 )").0[0].compounds[0].simple[0],
            ok(":nth-child(2n+1)").0[0].compounds[0].simple[0]
        );
    }

    #[test]
    fn nth_child_of_filter_accepts_a_comma_separated_filter() {
        let of_len = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { of: Some(list), .. }) => {
                list.0.len()
            }
            other => panic!("expected nth-child with `of`, got {other:?}"),
        };
        assert_eq!(of_len(":nth-child(1 of .a, .b, #c)"), 3);
    }

    #[test]
    fn pseudo_class_name_is_ascii_case_insensitive_for_functions() {
        // Function names, like bare pseudo-class names, are
        // case-insensitive (selectors-4 keywords are ASCII case-insensitive).
        assert!(
            ok(":NOT(.a)").0[0].compounds[0].simple[0]
                == ok(":not(.a)").0[0].compounds[0].simple[0]
        );
        assert!(matches!(
            &ok(":NTH-CHILD(2n+1)").0[0].compounds[0].simple[0],
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a: 2, b: 1, .. })
        ));
    }

    // ---- hostile input and the review's findings ---------------------------

    #[test]
    fn nested_logical_pseudo_classes_are_bounded_and_not_copied() {
        // 5000 nested `:not(` used to clone the remaining tokens at every
        // level (quadratic memory) and recurse once per level; now the
        // depth cap rejects it on a small stack, and a nesting within
        // the cap still parses.
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let deep = format!("{}.a{}", ":not(".repeat(5_000), ")".repeat(5_000));
                err(&deep);
                let within = format!(
                    "{}.a{}",
                    ":not(".repeat(MAX_NESTING),
                    ")".repeat(MAX_NESTING)
                );
                assert!(parse(&within).is_ok());
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn an_b_with_a_multibyte_character_is_rejected_not_a_panic() {
        err(":nth-child(n日)");
        err(":nth-child(aé)");
        err(":nth-child(日)");
        err(":nth-child(€n)");
    }

    #[test]
    fn an_b_signs_must_be_followed_by_a_signless_integer() {
        // css-syntax-3 §8.2: after a sign delim only a signless integer;
        // `n - -2147483648` used to overflow on negation.
        err(":nth-child(n - -2147483648)");
        err(":nth-child(n - -1)");
        // `n + +1` cannot be told from `n + 1`: the css tokenizer keeps
        // no sign flag on a number token, so a doubled plus is accepted.
    }

    #[test]
    fn an_b_ndash_dimension_then_signless_integer() {
        let nth = |s: &str| match &ok(s).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => (*a, *b),
            other => panic!("expected nth-child, got {other:?}"),
        };
        assert_eq!(nth(":nth-child(2n- 1)"), (2, -1));
        assert_eq!(nth(":nth-child(n- 1)"), (1, -1));
        assert_eq!(nth(":nth-child(-n- 1)"), (-1, -1));
        assert_eq!(nth(":nth-child(2n -1)"), (2, -1));
    }

    #[test]
    fn is_and_where_are_forgiving_but_not_is_not() {
        // selectors-4 §4.2: a member that fails to parse is dropped.
        let list = ok(":is(.a, ::before)");
        let SimpleSelector::PseudoClass(PseudoClass::Is(inner)) = &list.0[0].compounds[0].simple[0]
        else {
            panic!("expected :is");
        };
        assert_eq!(inner.0.len(), 1);
        let list = ok(":where(.a, :nope, .b)");
        let SimpleSelector::PseudoClass(PseudoClass::Where(inner)) =
            &list.0[0].compounds[0].simple[0]
        else {
            panic!("expected :where");
        };
        assert_eq!(inner.0.len(), 2);
        // All invalid is valid and matches nothing.
        let list = ok(":is(::before)");
        let SimpleSelector::PseudoClass(PseudoClass::Is(inner)) = &list.0[0].compounds[0].simple[0]
        else {
            panic!("expected :is");
        };
        assert!(inner.0.is_empty());
        // `:not()` fails whole.
        err(":not(.a, ::before)");
        err(":not()");
    }
}
