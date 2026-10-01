//! CSS stylesheet parsing: tokens in, [`Stylesheet`] out.
//!
//! Follows the `css-syntax-3` parsing entry points (§5) with full error
//! recovery: malformed input degrades, never fails. In particular:
//!
//! * Unknown at-rules are kept structurally as a raw token block
//!   ([`Block::Raw`]) — browsers never drop unknown rules at parse time,
//!   which is what keeps old engines rendering new pages.
//! * Known at-rules parse into nested rules (`@media`, `@supports`,
//!   `@layer`, `@container`, `@scope`) or declaration blocks
//!   (`@font-face`, `@page`, `@property`, `@counter-style`).
//! * Declarations missing their colon are skipped to the next `;`.
//! * Unclosed blocks at EOF close gracefully.
//!
//! Preludes (selector text, at-rule conditions) are kept as token
//! vectors — the selector engine (future `style` crate) consumes them
//! directly, the same way Blink keeps selector tokens.

use std::collections::VecDeque;

use crate::tokenizer::{tokenize, Token};

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
    /// Lowercased property name.
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
        rules: parse_rules(&mut stream, true),
    })
}

/// Parses a `style` attribute's contents (declaration list only).
pub fn parse_style_attribute(input: &str) -> Result<Vec<Declaration>, crate::ParseError> {
    let mut stream = TokenStream::new(tokenize(input));
    Ok(parse_declarations(&mut stream))
}

/// At-rule names whose blocks contain nested rules.
const NESTED_RULE_AT: [&str; 5] = ["media", "supports", "layer", "container", "scope"];

/// At-rule names whose blocks contain declarations.
const DECLARATION_AT: [&str; 4] = ["font-face", "page", "property", "counter-style"];

/// Parses rules until EOF (top level) or an upcoming `}` (nested).
fn parse_rules(stream: &mut TokenStream, top: bool) -> Vec<Rule> {
    let mut rules = Vec::new();
    loop {
        match stream.next() {
            None => return rules,
            Some(Token::Whitespace) | Some(Token::Cdo) | Some(Token::Cdc) | Some(Token::Semicolon) => {
                continue;
            }
            Some(Token::CloseCurly) => {
                if top {
                    continue; // stray '}': ignored at top level
                }
                stream.reconsume(Token::CloseCurly);
                return rules;
            }
            Some(Token::AtKeyword(name)) => rules.push(Rule::At(parse_at_rule(stream, name))),
            Some(first) => {
                if let Some(rule) = parse_qualified_rule(stream, first) {
                    rules.push(Rule::Qualified(rule));
                }
            }
        }
    }
}

/// Parses one at-rule; the name has been consumed.
fn parse_at_rule(stream: &mut TokenStream, name: String) -> AtRule {
    let mut prelude = Vec::new();
    let mut block = None;
    let mut depth = 0i32;
    loop {
        match stream.next() {
            None => break,
            Some(Token::Semicolon) if depth == 0 => break,
            Some(Token::OpenCurly) if depth == 0 => {
                block = Some(parse_at_block(stream, &name));
                break;
            }
            Some(t @ (Token::OpenParen | Token::OpenSquare)) => {
                depth += 1;
                prelude.push(t);
            }
            Some(t @ (Token::CloseParen | Token::CloseSquare)) => {
                depth = (depth - 1).max(0);
                prelude.push(t);
            }
            Some(t) => prelude.push(t),
        }
    }
    trim_ws(&mut prelude);
    AtRule { name, prelude, block }
}

/// Parses the body of an at-rule block; the `{` has been consumed.
fn parse_at_block(stream: &mut TokenStream, name: &str) -> Block {
    let lname = name.to_lowercase();
    if NESTED_RULE_AT.contains(&lname.as_str()) {
        let rules = parse_rules(stream, false);
        consume_close_curly(stream);
        Block::Rules(rules)
    } else if DECLARATION_AT.contains(&lname.as_str()) {
        let declarations = parse_declarations(stream);
        consume_close_curly(stream);
        Block::Declarations(declarations)
    } else {
        // Unknown at-rule: keep the raw block for forward compatibility.
        Block::Raw(take_raw_block(stream))
    }
}

/// Parses a qualified rule; `first` is the first prelude token.
/// Returns `None` when EOF arrived before the `{`.
fn parse_qualified_rule(stream: &mut TokenStream, first: Token) -> Option<QualifiedRule> {
    let mut prelude = vec![first];
    let mut depth = 0i32;
    loop {
        match stream.next() {
            None => return None, // EOF before '{': discard the rule
            Some(Token::OpenCurly) if depth == 0 => break,
            Some(t @ (Token::OpenParen | Token::OpenSquare)) => {
                depth += 1;
                prelude.push(t);
            }
            Some(t @ (Token::CloseParen | Token::CloseSquare)) => {
                depth = (depth - 1).max(0);
                prelude.push(t);
            }
            Some(t) => prelude.push(t),
        }
    }
    trim_ws(&mut prelude);
    let declarations = parse_declarations(stream);
    consume_close_curly(stream);
    Some(QualifiedRule {
        prelude,
        declarations,
    })
}

/// Parses declarations until `}` (reconsumed for the caller) or EOF.
fn parse_declarations(stream: &mut TokenStream) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    loop {
        match stream.next() {
            None => return declarations,
            Some(Token::CloseCurly) => {
                stream.reconsume(Token::CloseCurly);
                return declarations;
            }
            Some(Token::Whitespace) | Some(Token::Semicolon) | Some(Token::Cdo) | Some(Token::Cdc) => {
                continue;
            }
            Some(Token::AtKeyword(name)) => {
                // At-rules inside declaration blocks are consumed and
                // ignored for now.
                let _ = parse_at_rule(stream, name);
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

/// Parses one declaration; the property name has been consumed.
fn parse_declaration(stream: &mut TokenStream, name: String) -> Option<Declaration> {
    match stream.next() {
        Some(Token::Colon) => {}
        None => return None,
        Some(t) => {
            // Missing colon: skip to the next ';' or '}'.
            skip_until_delimiter(stream, t);
            return None;
        }
    }
    let mut value = Vec::new();
    let mut depth = 0i32;
    loop {
        match stream.next() {
            None => break,
            Some(Token::Semicolon) if depth == 0 => break,
            Some(Token::CloseCurly) if depth == 0 => {
                stream.reconsume(Token::CloseCurly);
                break;
            }
            Some(t @ (Token::OpenParen | Token::OpenSquare)) => {
                depth += 1;
                value.push(t);
            }
            Some(t @ (Token::CloseParen | Token::CloseSquare)) => {
                depth = (depth - 1).max(0);
                value.push(t);
            }
            Some(t) => value.push(t),
        }
    }
    let important = strip_important(&mut value);
    trim_ws(&mut value);
    Some(Declaration {
        name: name.to_lowercase(),
        value,
        important,
    })
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
/// inside `(...)` and `[...]`.
fn skip_until_delimiter(stream: &mut TokenStream, first: Token) {
    let mut depth = 0i32;
    let mut pending = Some(first);
    while let Some(token) = pending.take().or_else(|| stream.next()) {
        match token {
            Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = (depth - 1).max(0),
            Token::Semicolon if depth == 0 => return,
            Token::CloseCurly if depth == 0 => {
                stream.reconsume(Token::CloseCurly);
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

/// Trims leading/trailing whitespace tokens.
fn trim_ws(tokens: &mut Vec<Token>) {
    while matches!(tokens.first(), Some(Token::Whitespace)) {
        tokens.remove(0);
    }
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
        let sheet =
            parse_stylesheet("p { margin: 0 auto !important; FONT-SIZE: 12px; }").unwrap();
        let Rule::Qualified(rule) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(rule.declarations.len(), 2);
        assert_eq!(rule.declarations[0].name, "margin");
        assert!(rule.declarations[0].important);
        assert_eq!(
            rule.declarations[0].value,
            vec![
                Token::Number { value: 0.0, is_integer: true },
                Token::Whitespace,
                Token::Ident("auto".into()),
            ]
        );
        assert_eq!(rule.declarations[1].name, "font-size");
        assert!(!rule.declarations[1].important);
        assert_eq!(
            rule.declarations[1].value,
            vec![Token::Dimension { value: 12.0, unit: "px".into() }]
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
                Token::Dimension { value: 10.0, unit: "px".into() },
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
                Token::Dimension { value: 600.0, unit: "px".into() },
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
            vec![Token::Url { value: "x.css".into(), bad: false }]
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
    fn style_attribute_parsing() {
        let decls = parse_style_attribute("color: red; width: 10px").unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[1].name, "width");
        assert_eq!(
            decls[1].value,
            vec![Token::Dimension { value: 10.0, unit: "px".into() }]
        );
    }
}
