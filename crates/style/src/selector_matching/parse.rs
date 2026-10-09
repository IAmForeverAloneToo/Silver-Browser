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
//!
//! A selector may end in a pseudo-element (M16), in the rule's own list
//! only: selectors-4 makes one invalid inside `:not()`, `:is()`,
//! `:where()` and `of S`. After it may come the user-action
//! pseudo-classes, and `:is()`, `:where()` and `:not()` of them
//! (§3.6.3), and nothing else, not even a combinator. Names are
//! compared in ASCII lowercase: Unicode lowercasing turns some
//! non-ASCII letters into ASCII ones (the Kelvin sign into `k`).

use crate::ParseError;
use css::Token;

use super::language::is_well_formed_range;
use super::{
    AttrOp, CaseFlag, Combinator, CompoundSelector, PseudoClass, PseudoElement,
    PseudoElementSelector, Selector, SelectorList, SimpleSelector, Specificity,
};

/// How deep `:not()`, `:is()`, `:where()` and `of S` may nest.
pub const MAX_NESTING: usize = 32;

/// Parses a comma-separated selector list from prelude tokens. A
/// trailing comma, an empty member or an unbalanced parenthesis is an
/// error (selectors-4 §5.1: `<complex-selector>#`, no empty members).
pub fn parse_selector_list(tokens: &[Token]) -> Result<SelectorList, ParseError> {
    let mut c = Cursor::new(tokens);
    let list = parse_list(&mut c, 0, Scope::Rule, false, false)?;
    c.skip_ws();
    if !c.at_end() {
        return Err(ParseError::InvalidSelector);
    }
    Ok(list)
}

/// Whether `@supports selector(...)` holds for its argument (CSS
/// Conditional 4): one complex selector, which the parser takes without
/// the forgiving rules of `:is()` and `:where()`, so that a part it
/// does not know anywhere in the selector makes the answer false, nor
/// the `::-webkit-` pseudo-elements it takes only to match nothing.
/// What the parser takes otherwise, the matcher knows.
pub fn supports_selector(tokens: &[Token]) -> bool {
    let mut c = Cursor::new(tokens);
    c.strict = true;
    let Ok(list) = parse_list(&mut c, 0, Scope::Rule, false, false) else {
        return false;
    };
    c.skip_ws();
    c.at_end() && list.0.len() == 1
}

/// Where a selector list stands, which decides what its selectors may
/// hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The rule's own list: a selector may end in a pseudo-element.
    Rule,
    /// Inside `:not()`, `:is()`, `:where()` or `of S`, where a
    /// pseudo-element is invalid (selectors-4 §4).
    Nested,
    /// Inside `:not()`, `:is()` or `:where()` after a pseudo-element,
    /// whose restrictions pass to their arguments (§3.6.3): each
    /// selector is one compound of the pseudo-classes allowed there.
    State,
}

/// A peekable cursor over a token slice.
struct Cursor<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// No list is forgiving, as in `@supports selector()`.
    strict: bool,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Cursor {
            tokens,
            pos: 0,
            strict: false,
        }
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

    /// Whether the cursor is where a list member ends: at a comma, at
    /// the end, or, `in_parens`, at the `)` closing the list.
    fn at_member_end(&self, in_parens: bool) -> bool {
        match self.peek() {
            None | Some(Token::Comma) => true,
            Some(Token::CloseParen) => in_parens,
            _ => false,
        }
    }
}

/// Parses a selector list at the cursor: complex selectors separated by
/// top-level commas, ending at the end of the tokens or, when
/// `in_parens`, before the `)` that closes the enclosing function
/// (left for the caller to consume). With `forgiving`, a member that
/// fails to parse, or is followed by anything but the end of its
/// member, is skipped rather than failing the list, and the list may
/// come out empty.
fn parse_list(
    c: &mut Cursor,
    depth: usize,
    scope: Scope,
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
        let member = parse_complex(c, depth, scope).and_then(|selector| {
            c.skip_ws();
            if c.at_member_end(in_parens) {
                Ok(selector)
            } else {
                Err(ParseError::InvalidSelector)
            }
        });
        match member {
            Ok(selector) => selectors.push(selector),
            Err(e) => {
                if !forgiving {
                    return Err(e);
                }
                c.pos = start;
                skip_member(c);
            }
        }
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
fn parse_complex(c: &mut Cursor, depth: usize, scope: Scope) -> Result<Selector, ParseError> {
    c.skip_ws();
    if scope == Scope::State {
        return parse_state_selector(c, depth);
    }
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut specificity = Specificity::default();
    let mut pseudo_element = None;
    loop {
        let compound = parse_compound(c, depth, scope).ok_or(ParseError::InvalidSelector)?;
        specificity += compound.specificity;
        compounds.push(compound.selector);
        if compound.pseudo_element.is_some() {
            // The pseudo-element ends the selector: it has no
            // descendants or siblings for a combinator to reach, and
            // whatever follows it is left for the list, which takes
            // anything but a comma or its end for an error.
            pseudo_element = compound.pseudo_element;
            break;
        }
        let had_ws = c.skip_ws();
        if let Some(comb) = peek_combinator(c) {
            c.advance();
            c.skip_ws();
            combinators.push(comb);
        } else if had_ws && starts_compound(c.peek()) {
            combinators.push(Combinator::Descendant);
        } else {
            break;
        }
    }
    Ok(Selector {
        compounds,
        combinators,
        pseudo_element,
        specificity,
    })
}

/// Parses one selector of a list inside `:not()`, `:is()` or `:where()`
/// after a pseudo-element: a compound of the pseudo-classes allowed
/// there, nothing else.
fn parse_state_selector(c: &mut Cursor, depth: usize) -> Result<Selector, ParseError> {
    let mut specificity = Specificity::default();
    let states = parse_states(c, depth, &mut specificity).ok_or(ParseError::InvalidSelector)?;
    if states.is_empty() {
        return Err(ParseError::InvalidSelector);
    }
    Ok(Selector {
        compounds: vec![CompoundSelector {
            simple: states
                .into_iter()
                .map(SimpleSelector::PseudoClass)
                .collect(),
        }],
        combinators: Vec::new(),
        pseudo_element: None,
        specificity,
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

/// One compound selector as parsed.
struct Compound {
    /// Its simple selectors.
    selector: CompoundSelector,
    /// Their specificity, the pseudo-element's and its states' with it.
    specificity: Specificity,
    /// The pseudo-element that ends it, and so the selector, if any.
    pseudo_element: Option<PseudoElementSelector>,
}

/// Parses one compound selector (no combinators), which in the rule's
/// own list may end in a pseudo-element; a compound that is a
/// pseudo-element alone stands for `*` and it (selectors-4 §3.6.2).
fn parse_compound(c: &mut Cursor, depth: usize, scope: Scope) -> Option<Compound> {
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
                simple.push(SimpleSelector::Type(name.to_ascii_lowercase()));
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
                let (name, op, value, case) = parse_attribute(c)?;
                spec.1 += 1;
                simple.push(SimpleSelector::Attribute {
                    name,
                    op,
                    value,
                    case,
                });
                continue; // parse_attribute consumed through `]`
            }
            Token::Colon => {
                match parse_pseudo(c, &mut spec, depth, Scope::Nested)? {
                    Pseudo::Class(pc) => simple.push(SimpleSelector::PseudoClass(pc)),
                    Pseudo::Element(element) => {
                        if scope != Scope::Rule {
                            return None;
                        }
                        let states = parse_states(c, depth, &mut spec)?;
                        if simple.is_empty() {
                            simple.push(SimpleSelector::Universal);
                        }
                        return Some(Compound {
                            selector: CompoundSelector { simple },
                            specificity: spec,
                            pseudo_element: Some(PseudoElementSelector { element, states }),
                        });
                    }
                }
                continue; // parse_pseudo consumed its tokens
            }
            _ => break,
        }
        c.advance();
    }
    if simple.is_empty() {
        return None;
    }
    Some(Compound {
        selector: CompoundSelector { simple },
        specificity: spec,
        pseudo_element: None,
    })
}

/// Parses the pseudo-classes after a pseudo-element, or in a list
/// inside one of them: the user-action ones, and `:is()`, `:where()` and
/// `:not()` of them (selectors-4 §3.6.3). `None` when another
/// pseudo-class or a pseudo-element is there; anything else after them
/// is left for the list, which takes it for an error.
fn parse_states(c: &mut Cursor, depth: usize, spec: &mut Specificity) -> Option<Vec<PseudoClass>> {
    let mut states = Vec::new();
    while matches!(c.peek(), Some(Token::Colon)) {
        match parse_pseudo(c, spec, depth, Scope::State)? {
            Pseudo::Class(
                pc @ (PseudoClass::Hover
                | PseudoClass::Focus
                | PseudoClass::FocusVisible
                | PseudoClass::FocusWithin
                | PseudoClass::Active
                | PseudoClass::Not(_)
                | PseudoClass::Is(_)
                | PseudoClass::Where(_)),
            ) => states.push(pc),
            _ => return None,
        }
    }
    Some(states)
}

/// Parses an attribute selector; the cursor is at `[`.
fn parse_attribute(c: &mut Cursor) -> Option<(String, AttrOp, String, CaseFlag)> {
    c.advance(); // '['
    c.skip_ws();
    let name = match c.peek() {
        Some(Token::Ident(name)) => name.to_ascii_lowercase(),
        _ => return None,
    };
    c.advance();
    c.skip_ws();
    let op = match c.peek() {
        Some(Token::CloseSquare) => {
            c.advance();
            return Some((name, AttrOp::Exists, String::new(), CaseFlag::Unset));
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
    // The optional case flag (§6.3), an `i` or an `s` in either case.
    let mut case = CaseFlag::Unset;
    if let Some(Token::Ident(flag)) = c.peek() {
        if flag.eq_ignore_ascii_case("i") {
            case = CaseFlag::I;
        } else if flag.eq_ignore_ascii_case("s") {
            case = CaseFlag::S;
        } else {
            return None;
        }
        c.advance();
        c.skip_ws();
    }
    if !matches!(c.peek(), Some(Token::CloseSquare)) {
        return None;
    }
    c.advance();
    Some((name, op, value, case))
}

/// What a `:` begins.
enum Pseudo {
    Class(PseudoClass),
    Element(PseudoElement),
}

/// Parses a pseudo-class or a pseudo-element; the cursor is at `:`.
/// Updates `spec` with the specificity contribution (selectors-4 §17:
/// `:not()`/`:is()` count the max of their arguments, `:where()` counts
/// zero, `:nth-child(… of S)` one pseudo-class and the max of S, a
/// pseudo-element one type selector). `inner` is the scope of the
/// lists inside `:not()`, `:is()` and `:where()`.
fn parse_pseudo(
    c: &mut Cursor,
    spec: &mut Specificity,
    depth: usize,
    inner: Scope,
) -> Option<Pseudo> {
    c.advance(); // ':'
    match c.peek() {
        Some(Token::Colon) => {
            c.advance();
            // `::name`; no functional pseudo-element (`::part()`,
            // `::-webkit-x()`) is one this engine knows.
            let Some(Token::Ident(name)) = c.peek() else {
                return None;
            };
            let element = pseudo_element(name)?;
            // Conditional 4: valid, but not supported.
            if c.strict && element == PseudoElement::WebkitPrefixed {
                return None;
            }
            c.advance();
            *spec += Specificity(0, 0, 1);
            Some(Pseudo::Element(element))
        }
        Some(Token::Ident(name)) => {
            let name = name.to_ascii_lowercase();
            // CSS 2's four pseudo-elements keep their one-colon form,
            // and are never taken for pseudo-classes (selectors-4 §3.6.1).
            if let Some(element) = legacy_pseudo_element(&name) {
                c.advance();
                *spec += Specificity(0, 0, 1);
                return Some(Pseudo::Element(element));
            }
            let pc = match name.as_str() {
                "first-child" => PseudoClass::FirstChild,
                "last-child" => PseudoClass::LastChild,
                "only-child" => PseudoClass::OnlyChild,
                "first-of-type" => PseudoClass::FirstOfType,
                "last-of-type" => PseudoClass::LastOfType,
                "only-of-type" => PseudoClass::OnlyOfType,
                "empty" => PseudoClass::Empty,
                "root" => PseudoClass::Root,
                "scope" => PseudoClass::Scope,
                // Dynamic: parseable, but never matched until M26.
                "hover" => PseudoClass::Hover,
                "focus" => PseudoClass::Focus,
                "focus-visible" => PseudoClass::FocusVisible,
                "focus-within" => PseudoClass::FocusWithin,
                "active" => PseudoClass::Active,
                "visited" => PseudoClass::Visited,
                "link" => PseudoClass::Link,
                "any-link" => PseudoClass::AnyLink,
                "target" => PseudoClass::Target,
                "disabled" => PseudoClass::Disabled,
                "enabled" => PseudoClass::Enabled,
                "checked" => PseudoClass::Checked,
                "required" => PseudoClass::Required,
                "optional" => PseudoClass::Optional,
                _ => return None,
            };
            c.advance();
            *spec += Specificity(0, 1, 0);
            Some(Pseudo::Class(pc))
        }
        Some(Token::Function(name)) => {
            let name = name.to_ascii_lowercase();
            c.advance();
            let pc = match name.as_str() {
                "not" => {
                    let list = parse_list(c, depth + 1, inner, true, false).ok()?;
                    *spec += max_specificity(&list);
                    PseudoClass::Not(list)
                }
                "is" => {
                    let list = parse_list(c, depth + 1, inner, true, !c.strict).ok()?;
                    *spec += max_specificity(&list);
                    PseudoClass::Is(list)
                }
                "where" => {
                    let list = parse_list(c, depth + 1, inner, true, !c.strict).ok()?;
                    PseudoClass::Where(list) // zero specificity by design
                }
                "nth-child" => {
                    let (a, b, of) = parse_nth(c, true, depth)?;
                    *spec += Specificity(0, 1, 0);
                    if let Some(list) = &of {
                        *spec += max_specificity(list);
                    }
                    PseudoClass::NthChild { a, b, of }
                }
                "lang" => {
                    *spec += Specificity(0, 1, 0);
                    PseudoClass::Lang(parse_language_ranges(c)?)
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
            Some(Pseudo::Class(pc))
        }
        _ => None,
    }
}

/// The pseudo-element `::name` names, if this engine knows it.
fn pseudo_element(name: &str) -> Option<PseudoElement> {
    let name = name.to_ascii_lowercase();
    if let Some(element) = legacy_pseudo_element(&name) {
        return Some(element);
    }
    match name.as_str() {
        "marker" => Some(PseudoElement::Marker),
        "placeholder" => Some(PseudoElement::Placeholder),
        "selection" => Some(PseudoElement::Selection),
        // selectors-4 Appendix B: valid, and matching nothing.
        _ if name.starts_with("-webkit-") => Some(PseudoElement::WebkitPrefixed),
        _ => None,
    }
}

/// The pseudo-elements CSS 2 wrote with one colon, by their ASCII
/// lowercase names.
fn legacy_pseudo_element(name: &str) -> Option<PseudoElement> {
    match name {
        "before" => Some(PseudoElement::Before),
        "after" => Some(PseudoElement::After),
        "first-letter" => Some(PseudoElement::FirstLetter),
        "first-line" => Some(PseudoElement::FirstLine),
        _ => None,
    }
}

/// Parses `:lang()`'s argument at the cursor (just after the function
/// token), stopping before the closing `)`: one or more language
/// ranges, each an identifier or a string, separated by commas
/// (selectors-4 §7.2). The ranges kept are those that can match
/// anything, ASCII lowercased; see [`PseudoClass::Lang`].
fn parse_language_ranges(c: &mut Cursor) -> Option<Vec<String>> {
    let mut ranges = Vec::new();
    loop {
        c.skip_ws();
        let range = match c.peek() {
            Some(Token::Ident(range))
            | Some(Token::String {
                value: range,
                bad: false,
            }) => range,
            _ => return None,
        };
        c.advance();
        if is_well_formed_range(range) {
            ranges.push(range.to_ascii_lowercase());
        }
        c.skip_ws();
        if !matches!(c.peek(), Some(Token::Comma)) {
            return Some(ranges);
        }
        c.advance();
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
    let list = parse_list(c, depth + 1, Scope::Nested, true, false).ok()?;
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
                case: CaseFlag::Unset,
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
                    case: CaseFlag::Unset,
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
                case: CaseFlag::Unset,
            }]
        );
    }

    #[test]
    fn parses_case_flags() {
        // Three states: none, `i` and `s`, each flag in either case.
        let case = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::Attribute { case, value, .. } => {
                assert_eq!(value, "B", "{sel}: the value is kept as written");
                *case
            }
            other => panic!("expected an attribute, got {other:?}"),
        };
        assert_eq!(case("[a=B]"), CaseFlag::Unset);
        assert_eq!(case("[a=B i]"), CaseFlag::I);
        assert_eq!(case("[a=B I]"), CaseFlag::I);
        assert_eq!(case("[a=B s]"), CaseFlag::S);
        assert_eq!(case("[a='B'S]"), CaseFlag::S);
        assert_eq!(case("[a^=B i ]"), CaseFlag::I);
        // One flag, after a value, and no other letter.
        err("[a=B i s]");
        err("[a i]");
        err("[a=B ii]");
        // The long s, which Unicode case folding makes `s`, is not one.
        err("[a=B \u{17F}]");
    }

    #[test]
    fn attribute_name_is_lowercased() {
        assert_eq!(
            ok("[DATA-X=y]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "data-x".into(),
                op: AttrOp::Equals,
                value: "y".into(),
                case: CaseFlag::Unset,
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
                case: CaseFlag::Unset,
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
    fn parses_the_m16_pseudo_classes() {
        for (src, expected) in [
            (":any-link", PseudoClass::AnyLink),
            (":target", PseudoClass::Target),
            (":disabled", PseudoClass::Disabled),
            (":Enabled", PseudoClass::Enabled),
            (":CHECKED", PseudoClass::Checked),
            (":required", PseudoClass::Required),
            (":optional", PseudoClass::Optional),
            (":focus-visible", PseudoClass::FocusVisible),
            (":focus-within", PseudoClass::FocusWithin),
        ] {
            assert_eq!(
                ok(src).0[0].compounds[0].simple,
                vec![SimpleSelector::PseudoClass(expected)],
                "{src}"
            );
            assert_eq!(spec(src), Specificity(0, 1, 0), "{src}");
        }
        err(":any-link()");
        err(":checked(x)");
    }

    #[test]
    fn lang_takes_identifiers_and_strings_and_keeps_the_well_formed() {
        let ranges = |sel: &str| match &ok(sel).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::Lang(ranges)) => ranges.clone(),
            other => panic!("expected :lang, got {other:?}"),
        };
        assert_eq!(ranges(":lang(en)"), ["en"]);
        assert_eq!(
            ranges(":LANG( EN-us ,'fr', \\*-CH,\"*-Latn\" , '' )"),
            ["en-us", "fr", "*-ch", "*-latn", ""]
        );
        // Ranges that are not well-formed match nothing, and are left out.
        assert_eq!(ranges(":lang('de-*-DE', de)"), ["de"]);
        assert!(ranges(":lang(åå)").is_empty());
        assert!(ranges(":lang('en_US')").is_empty());
        assert_eq!(spec(":lang(en, fr)"), Specificity(0, 1, 0));
        for bad in [
            ":lang()",
            ":lang( )",
            ":lang(en,)",
            ":lang(,en)",
            ":lang(en fr)",
            ":lang(*)",
            ":lang(1)",
            ":lang(en",
            ":lang(url(x))",
            ":lang(f(x))",
            ":lang",
        ] {
            err(bad);
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

    // ---- pseudo-elements (M16) --------------------------------------------

    /// The selector's pseudo-element and the states after it.
    fn ending(sel: &str) -> Option<PseudoElementSelector> {
        let list = ok(sel);
        assert_eq!(list.0.len(), 1, "{sel}");
        list.0[0].pseudo_element.clone()
    }

    fn element(sel: &str) -> PseudoElement {
        ending(sel)
            .unwrap_or_else(|| panic!("{sel} ends in a pseudo-element"))
            .element
    }

    #[test]
    fn parses_every_pseudo_element() {
        for (src, expected) in [
            ("::before", PseudoElement::Before),
            ("::after", PseudoElement::After),
            ("::first-letter", PseudoElement::FirstLetter),
            ("::first-line", PseudoElement::FirstLine),
            ("::marker", PseudoElement::Marker),
            ("::placeholder", PseudoElement::Placeholder),
            ("::selection", PseudoElement::Selection),
            // CSS 2's one-colon forms of the four.
            (":before", PseudoElement::Before),
            (":after", PseudoElement::After),
            (":first-letter", PseudoElement::FirstLetter),
            (":first-line", PseudoElement::FirstLine),
            // Names are ASCII case-insensitive.
            ("::BEFORE", PseudoElement::Before),
            (":First-Line", PseudoElement::FirstLine),
        ] {
            assert_eq!(element(src), expected, "{src}");
            assert_eq!(element(&format!("p{src}")), expected, "p{src}");
        }
        assert_eq!(ending("p"), None);
        assert_eq!(ending("p:hover"), None);
    }

    #[test]
    fn a_pseudo_element_alone_stands_for_the_universal_selector() {
        // selectors-4 §3.6.2: `::first-line` is `*::first-line`, and
        // `div ::before` is `div *::before`.
        let selector = &ok("::before").0[0];
        assert_eq!(
            selector.compounds[0].simple,
            vec![SimpleSelector::Universal]
        );
        let selector = &ok("div ::before").0[0];
        assert_eq!(selector.combinators, vec![Combinator::Descendant]);
        assert_eq!(
            selector.compounds[1].simple,
            vec![SimpleSelector::Universal]
        );
        assert_eq!(
            selector.pseudo_element.as_ref().unwrap().element,
            PseudoElement::Before
        );
        let selector = &ok("ul > li.a:hover::marker").0[0];
        assert_eq!(selector.combinators, vec![Combinator::Child]);
        assert_eq!(
            selector.compounds[1].simple,
            vec![
                SimpleSelector::Type("li".into()),
                SimpleSelector::Class("a".into()),
                SimpleSelector::PseudoClass(PseudoClass::Hover),
            ]
        );
        assert_eq!(
            selector.pseudo_element.as_ref().unwrap().element,
            PseudoElement::Marker
        );
    }

    #[test]
    fn only_the_css2_pseudo_elements_take_one_colon() {
        err(":marker");
        err(":placeholder");
        err(":selection");
        err(":-webkit-scrollbar");
    }

    #[test]
    fn webkit_pseudo_elements_are_valid_and_match_nothing_but_others_stay_invalid() {
        // selectors-4 Appendix B: any `::-webkit-` name that is not a
        // function, matched ASCII case-insensitively.
        for src in [
            "::-webkit-scrollbar",
            "div::-webkit-scrollbar-thumb:hover",
            "::-WEBKIT-Inner-Spin-Button",
            "::-webkit-",
        ] {
            assert_eq!(
                ending(src).unwrap().element,
                PseudoElement::WebkitPrefixed,
                "{src}"
            );
        }
        // A list holding one keeps its other selectors.
        assert_eq!(ok(".a, ::-webkit-scrollbar").0.len(), 2);
        err("::-webkit-x()");
        err("::-moz-selection");
        err("::-webkitx");
        err("::backdrop");
        err("::part(x)");
        err("::slotted(p)");
        err("::nope");
        err(":: before");
        err("::");
        err("::'before'");
    }

    #[test]
    fn a_pseudo_element_ends_the_selector() {
        for src in [
            "p::before span",
            "p::before > span",
            "p::before + p",
            "p::before ~ p",
            "p::before.a",
            "p::before#x",
            "p::before[x]",
            "p::before*",
            "p::beforediv",
            "::before div",
            "::before::after",
            "::before:after",
            "::first-line::first-letter",
            "::before::-webkit-x",
            // White space then `:hover` is a descendant compound.
            "p::before :hover",
        ] {
            err(src);
        }
        // A comma ends it too, and white space before one.
        assert_eq!(ok("p::before , .a").0.len(), 2);
    }

    #[test]
    fn user_action_pseudo_classes_and_logical_ones_of_them_may_follow() {
        let states = |src: &str| ending(src).expect(src).states;
        assert_eq!(states("::before:hover"), vec![PseudoClass::Hover]);
        assert_eq!(
            states("::after:focus:active"),
            vec![PseudoClass::Focus, PseudoClass::Active]
        );
        assert_eq!(
            states("::first-line:focus-visible"),
            vec![PseudoClass::FocusVisible]
        );
        assert_eq!(
            states("::selection:FOCUS-WITHIN"),
            vec![PseudoClass::FocusWithin]
        );
        assert_eq!(states(":before:hover").len(), 1);
        // §3.6.3: the logical pseudo-classes pass the restriction to
        // their arguments.
        assert!(matches!(
            states("::before:not(:hover, :focus)").as_slice(),
            [PseudoClass::Not(list)] if list.0.len() == 2
        ));
        assert!(matches!(
            states("::before:where(:is(:active:hover))").as_slice(),
            [PseudoClass::Where(list)] if list.0.len() == 1
        ));
        // `:is()` forgives an argument that is invalid there; `:not()`
        // does not.
        assert!(matches!(
            states("::before:is(:hover, .a, :first-child, p)").as_slice(),
            [PseudoClass::Is(list)] if list.0.len() == 1
        ));
        assert!(matches!(
            states("::before:is(.a)").as_slice(),
            [PseudoClass::Is(list)] if list.0.is_empty()
        ));
        assert!(matches!(
            states("::before:is(:hover.a, :focus[x], :active)").as_slice(),
            [PseudoClass::Is(list)] if list.0.len() == 1
        ));
        for src in [
            "::before:first-child",
            "::before:empty",
            "::before:root",
            "::before:visited",
            "::before:link",
            "::before:nth-child(1)",
            "::before:not(.a)",
            "::before:not(:hover .a)",
            "::before:not(:hover.a)",
            "::before:not(:focus::after)",
            "::before:hover.a",
            "::before:hover[x]",
            "::before:hover p",
            "::before:not(:hover > :focus)",
            "::before:not(:hover, ::after)",
            "::before:not(:first-child)",
            "::before:not()",
        ] {
            err(src);
        }
    }

    #[test]
    fn pseudo_elements_are_invalid_inside_logical_pseudo_classes() {
        err(":not(::before)");
        err(":not(:before)");
        err(":not(p::after)");
        err(":nth-child(1 of ::before)");
        err(":nth-child(1 of :first-line)");
        // Forgiven, and so dropped, by `:is()` and `:where()`.
        let list = ok(":is(:after, .a)");
        let SimpleSelector::PseudoClass(PseudoClass::Is(inner)) = &list.0[0].compounds[0].simple[0]
        else {
            panic!("expected :is");
        };
        assert_eq!(inner.0.len(), 1);
        assert!(inner.0[0].pseudo_element.is_none());
        let list = ok(":where(p::before)");
        let SimpleSelector::PseudoClass(PseudoClass::Where(inner)) =
            &list.0[0].compounds[0].simple[0]
        else {
            panic!("expected :where");
        };
        assert!(inner.0.is_empty());
    }

    #[test]
    fn a_forgiving_list_drops_a_member_with_anything_after_it() {
        // selectors-4 §4.2 parses each member whole; a member that parses
        // to a point and then meets junk is dropped, not the list.
        let members = |src: &str| match &ok(src).0[0].compounds[0].simple[0] {
            SimpleSelector::PseudoClass(PseudoClass::Is(l) | PseudoClass::Where(l)) => l.0.len(),
            other => panic!("expected :is or :where, got {other:?}"),
        };
        assert_eq!(members(":is(.a | .b, .c)"), 1);
        assert_eq!(members(":where(.a ::before, .b, .c)"), 2);
        assert_eq!(members(":is(.a .b)"), 1);
        err(":not(.a | .b)");
        err(":not(.a | .b, .c)");
    }

    #[test]
    fn names_are_compared_in_ascii_lowercase() {
        // The Kelvin sign is Unicode-lowercased to `k`; ASCII lowercasing
        // leaves it, so `:lin\u{212A}` is not `:link`, and the type
        // selector `\u{212A}BD` does not name `kbd`.
        err(":lin\u{212A}");
        err("::mar\u{212A}er");
        assert_eq!(
            ok("\u{212A}BD").0[0].compounds[0].simple,
            vec![SimpleSelector::Type("\u{212A}bd".into())]
        );
        assert_eq!(
            ok("[\u{212A}EY=x]").0[0].compounds[0].simple,
            vec![SimpleSelector::Attribute {
                name: "\u{212A}ey".into(),
                op: AttrOp::Equals,
                value: "x".into(),
                case: CaseFlag::Unset,
            }]
        );
        assert_eq!(
            ok("ÉTÉ").0[0].compounds[0].simple,
            vec![SimpleSelector::Type("ÉtÉ".into())]
        );
        assert!(parse(":NOT(.a):Hover::BEFORE").is_ok());
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
    fn specificity_of_nth_child_of_s_adds_the_most_specific_of_s() {
        // selectors-4 §17: one pseudo-class plus the most specific
        // selector in S.
        assert_eq!(spec(":nth-child(2n+1 of .a)"), Specificity(0, 2, 0));
        assert_eq!(spec(":nth-child(2n of #a, .b)"), Specificity(1, 1, 0));
        assert_eq!(spec("li:nth-child(odd of .x .y)"), Specificity(0, 3, 1));
        assert_eq!(spec(":nth-child(1 of :where(#a))"), Specificity(0, 1, 0));
        assert_eq!(spec(":nth-child(2n+1)"), Specificity(0, 1, 0));
        assert_eq!(spec(":nth-last-child(1)"), Specificity(0, 1, 0));
    }

    #[test]
    fn specificity_of_a_pseudo_element_is_a_type_selector() {
        assert_eq!(spec("::before"), Specificity(0, 0, 1));
        assert_eq!(spec("p::before"), Specificity(0, 0, 2));
        assert_eq!(spec("p:before"), Specificity(0, 0, 2));
        assert_eq!(spec(".a::after:hover"), Specificity(0, 2, 1));
        assert_eq!(spec("#x ::marker"), Specificity(1, 0, 1));
        assert_eq!(spec("::-webkit-scrollbar"), Specificity(0, 0, 1));
        assert_eq!(spec("::before:not(:hover, :focus)"), Specificity(0, 1, 1));
        assert_eq!(spec("::before:where(:hover)"), Specificity(0, 0, 1));
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
                case: CaseFlag::Unset,
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
                case: CaseFlag::Unset,
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
