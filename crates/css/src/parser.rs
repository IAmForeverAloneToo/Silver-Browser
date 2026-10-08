//! CSS stylesheet parsing: tokens in, [`Stylesheet`] out.
//!
//! Follows the `css-syntax-3` parsing entry points (§5) with full error
//! recovery: malformed input degrades, never fails. In particular:
//!
//! * Unknown at-rules are kept structurally as a raw token block
//!   ([`Block::Raw`]) — browsers never drop unknown rules at parse time,
//!   which is what keeps old engines rendering new pages.
//! * Known at-rules parse into nested rules (`@media`, `@supports`,
//!   `@layer`, `@container`, `@scope`, and `@keyframes` with the
//!   Compatibility Standard's `@-webkit-keyframes`, whose rules'
//!   preludes are keyframe selectors) or declaration blocks
//!   (`@font-face`, `@page`, `@property`, `@counter-style`).
//! * Declarations missing their colon are skipped to the next `;`.
//! * Unclosed blocks at EOF close gracefully.
//!
//! Preludes (selector text, at-rule conditions) are kept as token
//! vectors — the selector engine (future `style` crate) consumes them
//! directly, the same way Blink keeps selector tokens.
//!
//! Two things keep hostile input linear and bounded. Blocks nest at
//! most [`MAX_NESTING`] deep: past that, a known at-rule's block is kept
//! raw like an unknown one's, so a megabyte of `@media{` cannot recurse
//! the parser off its stack. And a `{}` block met where a declaration
//! was expected (CSS nesting, which every shipping browser parses) is
//! consumed to its matching `}` and dropped, so the rule after it is
//! still seen; a `}` in a nested rule's prelude ends that rule and the
//! enclosing block rather than being swallowed into the prelude.

use std::collections::VecDeque;

use crate::tokenizer::{Token, tokenize};

/// A parsed stylesheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stylesheet {
    /// Top-level rules, in source order.
    pub rules: Vec<Rule>,
}

/// One rule in a stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// A qualified rule: selector prelude + declaration block.
    Qualified(QualifiedRule),
    /// An at-rule: `@name prelude { block }` or `@name prelude;`.
    At(AtRule),
}

/// A qualified rule (`selector { declarations }`).
#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedRule {
    /// The selector text as tokens (leading/trailing whitespace trimmed).
    pub prelude: Vec<Token>,
    /// The declarations in the block.
    pub declarations: Vec<Declaration>,
}

/// An at-rule.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    /// The at-rule name (e.g. `media`), without the `@`.
    pub name: String,
    /// The prelude before the `;` or `{`, as tokens.
    pub prelude: Vec<Token>,
    /// The block, when the rule had one.
    pub block: Option<Block>,
}

/// The body of an at-rule block.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A declaration block, e.g. `@font-face { ... }`.
    Declarations(Vec<Declaration>),
    /// A nested rule list, e.g. `@media { ... }`.
    Rules(Vec<Rule>),
    /// An unknown at-rule's raw block, kept for forward compatibility.
    Raw(Vec<Token>),
}

/// One `name: value` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// The property's name, ASCII-lowercased; a custom property's (two
    /// hyphens and more) as written, its case meaning something.
    pub name: String,
    /// The value as tokens (whitespace trimmed).
    pub value: Vec<Token>,
    /// Whether the value ended with `!important`.
    pub important: bool,
}

/// Token stream with one-token reconsumption.
struct TokenStream {
    iter: std::vec::IntoIter<Token>,
    pending: VecDeque<Token>,
}

impl TokenStream {
    fn new(tokens: Vec<Token>) -> Self {
        TokenStream {
            iter: tokens.into_iter(),
            pending: VecDeque::new(),
        }
    }

    fn next(&mut self) -> Option<Token> {
        self.pending.pop_front().or_else(|| self.iter.next())
    }

    fn reconsume(&mut self, token: Token) {
        self.pending.push_front(token);
    }
}

/// Parses a stylesheet. Always recovers; see the module docs.
pub fn parse_stylesheet(input: &str) -> Result<Stylesheet, crate::ParseError> {
    let mut stream = TokenStream::new(tokenize(input));
    Ok(Stylesheet {
        rules: parse_rules(&mut stream, true, 0),
    })
}

/// Parses a `style` attribute's contents (declaration list only).
pub fn parse_style_attribute(input: &str) -> Result<Vec<Declaration>, crate::ParseError> {
    let mut stream = TokenStream::new(tokenize(input));
    Ok(parse_declarations(&mut stream, 0))
}

/// At-rule names whose blocks contain nested rules: a keyframes rule's
/// are its keyframes, a selector list of offsets and a block of
/// declarations each (CSS Animations 1 §3, M22).
const NESTED_RULE_AT: [&str; 7] = [
    "media",
    "supports",
    "layer",
    "container",
    "scope",
    "keyframes",
    "-webkit-keyframes",
];

/// At-rule names whose blocks contain declarations.
const DECLARATION_AT: [&str; 4] = ["font-face", "page", "property", "counter-style"];

/// How deep blocks may nest before a block is kept raw rather than
/// parsed. Real sheets nest a handful of levels; the parser recurses
/// once per level, and this is what bounds that recursion.
pub const MAX_NESTING: usize = 128;

/// Parses rules until EOF (top level) or an upcoming `}` (nested).
/// `depth` is how many blocks enclose this rule list.
fn parse_rules(stream: &mut TokenStream, top: bool, depth: usize) -> Vec<Rule> {
    let mut rules = Vec::new();
    loop {
        match stream.next() {
            None => return rules,
            Some(Token::Whitespace)
            | Some(Token::Cdo)
            | Some(Token::Cdc)
            | Some(Token::Semicolon) => {
                continue;
            }
            Some(Token::CloseCurly) => {
                if top {
                    continue; // stray '}': ignored at top level
                }
                stream.reconsume(Token::CloseCurly);
                return rules;
            }
            Some(Token::AtKeyword(name)) => {
                rules.push(Rule::At(parse_at_rule(stream, name, depth)));
            }
            Some(first) => {
                if let Some(rule) = parse_qualified_rule(stream, first, !top, depth) {
                    rules.push(Rule::Qualified(rule));
                }
            }
        }
    }
}

/// Parses one at-rule; the name has been consumed.
fn parse_at_rule(stream: &mut TokenStream, name: String, depth: usize) -> AtRule {
    let mut prelude = Vec::new();
    let mut block = None;
    let mut paren_depth = 0i32;
    loop {
        match stream.next() {
            None => break,
            Some(Token::Semicolon) if paren_depth == 0 => break,
            Some(Token::OpenCurly) if paren_depth == 0 => {
                block = Some(parse_at_block(stream, &name, depth));
                break;
            }
            Some(t @ (Token::OpenParen | Token::OpenSquare)) => {
                paren_depth += 1;
                prelude.push(t);
            }
            Some(t @ (Token::CloseParen | Token::CloseSquare)) => {
                paren_depth = (paren_depth - 1).max(0);
                prelude.push(t);
            }
            Some(t) => prelude.push(t),
        }
    }
    trim_ws(&mut prelude);
    AtRule {
        name,
        prelude,
        block,
    }
}

/// Parses the body of an at-rule block; the `{` has been consumed.
/// `depth` is the nesting of the block being opened.
fn parse_at_block(stream: &mut TokenStream, name: &str, depth: usize) -> Block {
    let lname = name.to_lowercase();
    if depth >= MAX_NESTING {
        // Too deep to parse: kept whole, like an unknown at-rule's block.
        return Block::Raw(take_raw_block(stream));
    }
    if NESTED_RULE_AT.contains(&lname.as_str()) {
        let rules = parse_rules(stream, false, depth + 1);
        consume_close_curly(stream);
        Block::Rules(rules)
    } else if DECLARATION_AT.contains(&lname.as_str()) {
        let declarations = parse_declarations(stream, depth + 1);
        consume_close_curly(stream);
        Block::Declarations(declarations)
    } else {
        // Unknown at-rule: keep the raw block for forward compatibility.
        Block::Raw(take_raw_block(stream))
    }
}

/// Parses a qualified rule; `first` is the first prelude token.
/// Returns `None` when EOF arrived before the `{`, or, in a nested rule
/// list, when a `}` ends the enclosing block first (the spec's "consume
/// a qualified rule" with `nested` set: the `}` is reconsumed for the
/// caller, which ends its block on it).
fn parse_qualified_rule(
    stream: &mut TokenStream,
    first: Token,
    nested: bool,
    depth: usize,
) -> Option<QualifiedRule> {
    let mut prelude = vec![first];
    let mut paren_depth = 0i32;
    loop {
        match stream.next() {
            None => return None, // EOF before '{': discard the rule
            Some(Token::OpenCurly) if paren_depth == 0 => break,
            Some(Token::CloseCurly) if nested && paren_depth == 0 => {
                stream.reconsume(Token::CloseCurly);
                return None;
            }
            Some(t @ (Token::OpenParen | Token::OpenSquare)) => {
                paren_depth += 1;
                prelude.push(t);
            }
            Some(t @ (Token::CloseParen | Token::CloseSquare)) => {
                paren_depth = (paren_depth - 1).max(0);
                prelude.push(t);
            }
            Some(t) => prelude.push(t),
        }
    }
    trim_ws(&mut prelude);
    let declarations = parse_declarations(stream, depth + 1);
    consume_close_curly(stream);
    Some(QualifiedRule {
        prelude,
        declarations,
    })
}

/// Parses declarations until `}` (reconsumed for the caller) or EOF.
/// `depth` is the nesting of the block the declarations are in.
fn parse_declarations(stream: &mut TokenStream, depth: usize) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    loop {
        match stream.next() {
            None => return declarations,
            Some(Token::CloseCurly) => {
                stream.reconsume(Token::CloseCurly);
                return declarations;
            }
            Some(Token::Whitespace)
            | Some(Token::Semicolon)
            | Some(Token::Cdo)
            | Some(Token::Cdc) => {
                continue;
            }
            Some(Token::AtKeyword(name)) => {
                // At-rules inside declaration blocks are consumed and
                // ignored for now.
                let _ = parse_at_rule(stream, name, depth);
            }
            Some(Token::Ident(name)) => {
                if let Some(declaration) = parse_declaration(stream, name) {
                    declarations.push(declaration);
                }
            }
            Some(other) => skip_until_delimiter(stream, other),
        }
    }
}

/// Parses one declaration; the property name has been consumed. A `{}`
/// block in the value means this was a nested rule, not a declaration
/// (`a { color: red } b { ... }` without a `;` reads as `color: red }
/// b {`-nothing, but `& b { ... }` reads as a declaration named `&`...
/// no: as the `&` delim, which [`skip_until_delimiter`] handles; the
/// case here is `x: y { ... }`): the block is consumed to its `}` and
/// the declaration dropped, so what follows is parsed afresh. A custom
/// property (`--*`, M21) is the exception: its value may hold a `{}`
/// block, as CSS Syntax 3 §5.5.5 says, kept with its tokens.
///
/// The value is read by CSS Syntax 3's component values: a function, a
/// `(`, a `[` and a custom property's `{` open a block that only its own
/// closer ends, so a `;` or `}` inside `var(…)` or `calc(…)` is part of
/// the value, and a `)` with no block open to close is a token like any
/// other.
///
/// The name is ASCII-lowercased, CSS's property names being ASCII
/// case-insensitive, except a custom property's, which is
/// case-sensitive (Variables 1 §2).
fn parse_declaration(stream: &mut TokenStream, name: String) -> Option<Declaration> {
    // White space may stand between the name and the colon (CSS Syntax 3
    // section 5.4.6).
    let mut next = stream.next();
    while next == Some(Token::Whitespace) {
        next = stream.next();
    }
    match next {
        Some(Token::Colon) => {}
        None => return None,
        Some(t) => {
            // Missing colon: skip to the next ';' or '}'.
            skip_until_delimiter(stream, t);
            return None;
        }
    }
    let custom = name.starts_with("--");
    let mut value = Vec::new();
    // The closer of each block open around the token being read,
    // innermost last.
    let mut open = Vec::new();
    while let Some(token) = stream.next() {
        if open.is_empty() {
            match token {
                Token::Semicolon => break,
                Token::CloseCurly => {
                    stream.reconsume(Token::CloseCurly);
                    break;
                }
                Token::OpenCurly if !custom => {
                    let _ = take_raw_block(stream);
                    return None;
                }
                _ => {}
            }
        }
        match token {
            Token::OpenParen | Token::Function(_) => open.push(Closer::Paren),
            Token::OpenSquare => open.push(Closer::Square),
            Token::OpenCurly => open.push(Closer::Curly),
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly
                if open.last() == Some(&Closer::of(&token)) =>
            {
                open.pop();
            }
            _ => {}
        }
        value.push(token);
    }
    let important = strip_important(&mut value);
    trim_ws(&mut value);
    Some(Declaration {
        name: if custom {
            name
        } else {
            name.to_ascii_lowercase()
        },
        value,
        important,
    })
}

/// What ends a block of component values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closer {
    Paren,
    Square,
    Curly,
}

impl Closer {
    /// The closer `token` is, which must be a closing token.
    fn of(token: &Token) -> Closer {
        match token {
            Token::CloseSquare => Closer::Square,
            Token::CloseCurly => Closer::Curly,
            _ => Closer::Paren,
        }
    }
}

/// Strips a trailing `!important` (whitespace-tolerant, case-insensitive).
fn strip_important(value: &mut Vec<Token>) -> bool {
    let mut end = value.len();
    while end > 0 && matches!(value[end - 1], Token::Whitespace) {
        end -= 1;
    }
    if end == 0 {
        return false;
    }
    if let Token::Ident(s) = &value[end - 1]
        && s.eq_ignore_ascii_case("important")
    {
        let mut i = end - 1;
        while i > 0 && matches!(value[i - 1], Token::Whitespace) {
            i -= 1;
        }
        if i > 0 && matches!(value[i - 1], Token::Delim('!')) {
            value.truncate(i - 1);
            return true;
        }
    }
    false
}

/// Skips tokens until `;`, `}` (reconsumed), or EOF, staying balanced
/// inside functions, `(...)` and `[...]`. A `{` at the top level is a
/// nested rule's block (`& b { ... }` inside a declaration list): it is
/// consumed to its matching `}` and the skip ends there, so the
/// declaration after the block is read as one.
fn skip_until_delimiter(stream: &mut TokenStream, first: Token) {
    let mut depth = 0i32;
    let mut pending = Some(first);
    while let Some(token) = pending.take().or_else(|| stream.next()) {
        match token {
            Token::OpenParen | Token::OpenSquare | Token::Function(_) => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = (depth - 1).max(0),
            Token::Semicolon if depth == 0 => return,
            Token::CloseCurly if depth == 0 => {
                stream.reconsume(Token::CloseCurly);
                return;
            }
            Token::OpenCurly if depth == 0 => {
                let _ = take_raw_block(stream);
                return;
            }
            _ => {}
        }
    }
}

/// Takes a raw block (unknown at-rule) up to the matching `}` at depth
/// zero, exclusive of that final `}`.
fn take_raw_block(stream: &mut TokenStream) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut depth = 0i32;
    loop {
        match stream.next() {
            None => return tokens,
            Some(Token::CloseCurly) if depth == 0 => return tokens,
            Some(Token::OpenCurly) => {
                depth += 1;
                tokens.push(Token::OpenCurly);
            }
            Some(Token::CloseCurly) => {
                depth -= 1;
                tokens.push(Token::CloseCurly);
            }
            Some(t) => tokens.push(t),
        }
    }
}

/// Consumes the `}` that ends a block, if present.
fn consume_close_curly(stream: &mut TokenStream) {
    match stream.next() {
        Some(Token::CloseCurly) | None => {}
        Some(t) => stream.reconsume(t),
    }
}

/// Trims leading/trailing whitespace tokens. Comments split a run of
/// whitespace into several tokens, so a value can start with thousands
/// of them; they go in one drain, not one `remove(0)` each.
fn trim_ws(tokens: &mut Vec<Token>) {
    let leading = tokens
        .iter()
        .take_while(|t| matches!(t, Token::Whitespace))
        .count();
    tokens.drain(..leading);
    while matches!(tokens.last(), Some(Token::Whitespace)) {
        tokens.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_qualified_rule() {
        let sheet = parse_stylesheet("h1 { color: red; }").expect("never fails");
        assert_eq!(sheet.rules.len(), 1);
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.prelude, vec![Token::Ident("h1".into())]);
        assert_eq!(rule.declarations.len(), 1);
        let d = &rule.declarations[0];
        assert_eq!(d.name, "color");
        assert_eq!(d.value, vec![Token::Ident("red".into())]);
        assert!(!d.important);
    }

    #[test]
    fn important_multiple_declarations_and_functions() {
        let sheet = parse_stylesheet("p { margin: 0 auto !important; FONT-SIZE: 12px; }").unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.declarations.len(), 2);
        assert_eq!(rule.declarations[0].name, "margin");
        assert!(rule.declarations[0].important);
        assert_eq!(
            rule.declarations[0].value,
            vec![
                Token::Number {
                    value: 0.0,
                    is_integer: true
                },
                Token::Whitespace,
                Token::Ident("auto".into()),
            ]
        );
        assert_eq!(rule.declarations[1].name, "font-size");
        assert!(!rule.declarations[1].important);
        assert_eq!(
            rule.declarations[1].value,
            vec![Token::Dimension {
                value: 12.0,
                unit: "px".into()
            }]
        );
    }

    #[test]
    fn function_values_keep_parens_balanced() {
        let sheet = parse_stylesheet("a { width: calc(100% - 10px) }").unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(
            rule.declarations[0].value,
            vec![
                Token::Function("calc".into()),
                Token::Percentage { value: 100.0 },
                Token::Whitespace,
                Token::Delim('-'),
                Token::Whitespace,
                Token::Dimension {
                    value: 10.0,
                    unit: "px".into()
                },
                Token::CloseParen,
            ]
        );
    }

    #[test]
    fn at_rule_with_nested_rules() {
        let sheet = parse_stylesheet("@media (min-width: 600px) { .c { color: blue } }").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        assert_eq!(at.name, "media");
        assert_eq!(
            at.prelude,
            vec![
                Token::OpenParen,
                Token::Ident("min-width".into()),
                Token::Colon,
                Token::Whitespace,
                Token::Dimension {
                    value: 600.0,
                    unit: "px".into()
                },
                Token::CloseParen,
            ]
        );
        let Some(Block::Rules(rules)) = &at.block else {
            panic!("expected a rules block");
        };
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn at_rule_without_block() {
        let sheet = parse_stylesheet("@import url(x.css);").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        assert_eq!(at.name, "import");
        assert!(at.block.is_none());
        assert_eq!(
            at.prelude,
            vec![Token::Url {
                value: "x.css".into(),
                bad: false
            }]
        );
    }

    #[test]
    fn unknown_at_rule_kept_raw_with_nested_braces() {
        let sheet = parse_stylesheet("@-moz-x { a: b { nested } }").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        assert_eq!(at.name, "-moz-x");
        let Some(Block::Raw(tokens)) = &at.block else {
            panic!("expected a raw block");
        };
        // The nested braces are preserved; the closing outer `}` is not.
        assert!(tokens.contains(&Token::OpenCurly));
        assert!(tokens.contains(&Token::CloseCurly));
    }

    #[test]
    fn keyframes_blocks_hold_rules_of_offsets_and_declarations() {
        for name in ["keyframes", "-webkit-keyframes", "KeyFrames"] {
            let sheet = parse_stylesheet(&format!(
                "@{name} spin {{ from {{ opacity: 0 }} 50%, 75% {{ opacity: .5 !important }} to {{}} }}"
            ))
            .unwrap();
            let Rule::At(at) = &sheet.rules[0] else {
                panic!("expected an at-rule");
            };
            assert_eq!(at.prelude, vec![Token::Ident("spin".into())]);
            let Some(Block::Rules(keyframes)) = &at.block else {
                panic!("expected a block of rules for @{name}");
            };
            assert_eq!(keyframes.len(), 3);
            let Rule::Qualified(half) = &keyframes[1] else {
                panic!("expected a keyframe");
            };
            assert_eq!(
                half.prelude,
                vec![
                    Token::Percentage { value: 50.0 },
                    Token::Comma,
                    Token::Whitespace,
                    Token::Percentage { value: 75.0 }
                ]
            );
            assert_eq!(half.declarations.len(), 1);
            assert!(half.declarations[0].important);
        }
    }

    #[test]
    fn font_face_declaration_block() {
        let sheet = parse_stylesheet("@font-face { font-family: X; src: url(f.woff); }").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        let Some(Block::Declarations(decls)) = &at.block else {
            panic!("expected a declarations block");
        };
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "font-family");
        assert_eq!(decls[1].name, "src");
    }

    #[test]
    fn error_recovery_skips_bad_declarations() {
        let sheet = parse_stylesheet("h1 { color } p { color: red }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(h1) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert!(h1.declarations.is_empty());
        let Rule::Qualified(p) = &sheet.rules[1] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(p.declarations.len(), 1);
    }

    #[test]
    fn white_space_may_come_before_the_colon() {
        let sheet = parse_stylesheet("p { color : red; width\n:\t10px } a { color } ").unwrap();
        let Rule::Qualified(p) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        let names: Vec<&str> = p.declarations.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["color", "width"]);
        let declarations = parse_style_attribute("height  : 5px").unwrap();
        assert_eq!(declarations[0].name, "height");
    }

    #[test]
    fn unclosed_block_at_eof() {
        let sheet = parse_stylesheet("h1 { color: red").unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.declarations.len(), 1);
    }

    #[test]
    fn selector_prelude_can_contain_parens() {
        let sheet = parse_stylesheet("a:not(.x) { color: red }").unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(
            rule.prelude,
            vec![
                Token::Ident("a".into()),
                Token::Colon,
                Token::Function("not".into()),
                Token::Delim('.'),
                Token::Ident("x".into()),
                Token::CloseParen,
            ]
        );
    }

    #[test]
    fn a_nested_rule_inside_a_declaration_block_does_not_eat_the_next_rule() {
        // CSS nesting: the `& b { ... }` block is skipped, `color: blue`
        // after it is still a declaration of `a`, and `p` is still a rule.
        let sheet =
            parse_stylesheet("a { & b { color: red } color: blue } p { color: green }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(a.prelude, vec![Token::Ident("a".into())]);
        assert_eq!(a.declarations.len(), 1);
        assert_eq!(a.declarations[0].value, vec![Token::Ident("blue".into())]);
        let Rule::Qualified(p) = &sheet.rules[1] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(p.prelude, vec![Token::Ident("p".into())]);
        assert_eq!(p.declarations[0].value, vec![Token::Ident("green".into())]);

        // A block in a declaration's value is the same case.
        let sheet = parse_stylesheet("a { x: y { z } color: blue } p { color: green }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn a_close_curly_in_a_nested_prelude_ends_the_enclosing_block() {
        let sheet =
            parse_stylesheet("@media screen { .x { color: red; } .y } p { color: green }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        let Some(Block::Rules(rules)) = &at.block else {
            panic!("expected a rules block");
        };
        assert_eq!(rules.len(), 1, "the dangling `.y` is dropped, nothing else");
        let Rule::Qualified(p) = &sheet.rules[1] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(p.prelude, vec![Token::Ident("p".into())]);
    }

    #[test]
    fn nesting_past_the_cap_is_kept_raw_rather_than_recursed_into() {
        // 20k nested `@media{` used to overflow the stack; on a small
        // stack even a few thousand would. Past MAX_NESTING the block is
        // raw, so the depth of parsing is bounded whatever the input.
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let source = "@media{".repeat(20_000) + "a{color:red}" + &"}".repeat(20_000);
                let sheet = parse_stylesheet(&source).unwrap();
                let mut depth = 0;
                let mut rules = &sheet.rules;
                loop {
                    let [Rule::At(at)] = rules.as_slice() else {
                        panic!("one at-rule per level");
                    };
                    match &at.block {
                        Some(Block::Rules(inner)) => {
                            depth += 1;
                            rules = inner;
                        }
                        Some(Block::Raw(_)) => break,
                        other => panic!("unexpected block {other:?}"),
                    }
                }
                assert_eq!(depth, MAX_NESTING);
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn many_whitespace_tokens_before_a_value_are_trimmed_in_one_pass() {
        // Comments split whitespace into separate tokens; trimming
        // used to remove them from the front one at a time.
        let source = format!("a{{color:{}red}}", "/**/ ".repeat(100_000));
        let sheet = parse_stylesheet(&source).unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.declarations[0].value, vec![Token::Ident("red".into())]);
    }

    #[test]
    fn a_function_holds_its_semicolons_and_braces() {
        // M21: `;` and `}` inside a function are part of the value, as
        // CSS Syntax 3's component values have them; the declaration
        // after it is read as one.
        let decls = parse_style_attribute("--x: f(a;b}c); color: red; width: calc(1px;2px)")
            .expect("never fails");
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["--x", "color", "width"]);
        assert_eq!(decls[0].value, tokenize("f(a;b}c)"));
        assert_eq!(decls[2].value, tokenize("calc(1px;2px)"));
        // A `)` with no block open is a token; it closes nothing.
        let decls = parse_style_attribute("color: red) ; width: 1px").expect("never fails");
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].value, tokenize("red)"));
        // An invalid declaration skipped is skipped past a function too.
        let decls = parse_style_attribute("x f(a;b); color: red").expect("never fails");
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["color"]);
    }

    #[test]
    fn a_custom_property_keeps_its_case_and_its_braces() {
        // Variables 1 §2: a custom property's name is case-sensitive,
        // every other is ASCII case-insensitive; and CSS Syntax 3 lets a
        // custom property's value hold a `{}` block, which drops any
        // other declaration as a nested rule.
        let sheet = parse_stylesheet("a { --MyVar: {a: b; c} 1; COLOR: red; Width: x { y } }")
            .expect("never fails");
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        let names: Vec<&str> = rule.declarations.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["--MyVar", "color"]);
        assert_eq!(rule.declarations[0].value, tokenize("{a: b; c} 1"));
        // A name's other letters are lowercased by ASCII's rules alone.
        let decls = parse_style_attribute("COLOR\\212A: red").expect("never fails");
        assert_eq!(decls[0].name, "color\u{212a}");
    }

    #[test]
    fn style_attribute_parsing() {
        let decls = parse_style_attribute("color: red; width: 10px").unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[1].name, "width");
        assert_eq!(
            decls[1].value,
            vec![Token::Dimension {
                value: 10.0,
                unit: "px".into()
            }]
        );
    }
}
