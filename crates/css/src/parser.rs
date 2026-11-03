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
//!   (`@font-face`, `@page`, `@property`, `@counter-style`,
//!   `@font-palette-values`, and `@font-feature-values`' blocks, which
//!   it holds as its nested rules).
//! * Declarations missing their colon are skipped to the next `;`.
//! * Unclosed blocks at EOF close gracefully.
//!
//! A style rule's block is read by CSS Syntax 3's "consume a block's
//! contents", as CSS Nesting 1 has it: declarations, nested style rules
//! and the conditional group rules nesting allows (`@media`,
//! `@supports`, `@container`, `@layer` and `@scope`), in any order. The
//! declarations before the first rule are the rule's own; each run of
//! them after a rule is a [`Rule::Declarations`] among its nested rules
//! (Nesting 1 section 3.3), as every declaration in a group rule nested
//! in a style rule, or in an `@scope` rule, is. Something that does not
//! read as a declaration is read again as a nested rule, a `;` ending
//! it as an error. A keyframe's block holds declarations alone.
//!
//! Preludes (selector text, at-rule conditions) are kept as token
//! vectors — the selector engine (future `style` crate) consumes them
//! directly, the same way Blink keeps selector tokens.
//!
//! Two things keep hostile input linear and bounded. Blocks nest at
//! most [`MAX_NESTING`] deep: past that, a known at-rule's block is kept
//! raw like an unknown one's, and a style rule's is skipped, so a
//! megabyte of `@media{` or `a{` cannot recurse the parser off its
//! stack. And a `}` in a nested rule's prelude ends that rule and the
//! enclosing block rather than being swallowed into the prelude.

use std::collections::VecDeque;

use crate::tokenizer::{Token, tokenize_with_spans};

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
    /// A nested declarations rule (CSS Nesting 1 section 3.3): a run of
    /// declarations among a style rule's nested rules after one of
    /// them, or in a group rule nested in a style rule or in an `@scope`
    /// rule, which the CSSOM shows as `CSSNestedDeclarations`.
    Declarations(Vec<Declaration>),
}

/// A qualified rule (`selector { declarations }`).
#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedRule {
    /// The selector text as tokens (leading/trailing whitespace trimmed).
    pub prelude: Vec<Token>,
    /// The declarations in the block, those before its first nested
    /// rule.
    pub declarations: Vec<Declaration>,
    /// The rules nested in its block (CSS Nesting 1), in order: style
    /// rules, the group rules nesting allows, and the runs of
    /// declarations after a rule as [`Rule::Declarations`]. A
    /// keyframe's is empty.
    pub rules: Vec<Rule>,
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

/// Token stream with one-token reconsumption, which knows where each
/// token was in the source.
struct TokenStream {
    iter: std::iter::Enumerate<std::vec::IntoIter<Token>>,
    pending: VecDeque<(Token, usize)>,
    /// The number of the token last read.
    last: usize,
    /// Each token's start and end in `source`.
    spans: Vec<(usize, usize)>,
    source: Vec<char>,
}

impl TokenStream {
    fn new(input: &str) -> Self {
        let spanned = tokenize_with_spans(input);
        TokenStream {
            iter: spanned.tokens.into_iter().enumerate(),
            pending: VecDeque::new(),
            last: 0,
            spans: spanned.spans,
            source: spanned.source,
        }
    }

    fn next(&mut self) -> Option<Token> {
        let (n, token) = match self.pending.pop_front() {
            Some((token, n)) => (n, token),
            None => self.iter.next()?,
        };
        self.last = n;
        Some(token)
    }

    /// Puts back the token last read.
    fn reconsume(&mut self, token: Token) {
        self.pending.push_front((token, self.last));
    }

    /// Puts back `tokens`, read in that order, each with its number: CSS
    /// Syntax 3's "restore a mark", for what was read as a declaration
    /// and is to be read again as a rule.
    fn restore(&mut self, tokens: Vec<(Token, usize)>) {
        for token in tokens.into_iter().rev() {
            self.pending.push_front(token);
        }
    }

    /// The source text of tokens `first` to `last`.
    fn text(&self, first: usize, last: usize) -> String {
        let (Some(&(start, _)), Some(&(_, end))) = (self.spans.get(first), self.spans.get(last))
        else {
            return String::new();
        };
        self.source
            .get(start..end)
            .map_or_else(String::new, |chars| chars.iter().collect())
    }
}

/// Parses a stylesheet. Always recovers; see the module docs.
pub fn parse_stylesheet(input: &str) -> Result<Stylesheet, crate::ParseError> {
    let mut stream = TokenStream::new(input);
    Ok(Stylesheet {
        rules: parse_rules(&mut stream, true, 0, false),
    })
}

/// Parses a `style` attribute's contents (declaration list only).
pub fn parse_style_attribute(input: &str) -> Result<Vec<Declaration>, crate::ParseError> {
    let mut stream = TokenStream::new(input);
    Ok(parse_declarations(&mut stream, 0, false, false))
}

/// Parses `input` as a style rule's block contents are read (CSS Syntax
/// 3's "parse a block's contents"): its rules, with the runs of
/// declarations among them as [`Rule::Declarations`]. What the CSSOM
/// inserts into a style rule, or into a group rule nested in one, is
/// read so. A `}` with no block open closes nothing and is passed over.
pub fn parse_block_contents(input: &str) -> Vec<Rule> {
    let mut stream = TokenStream::new(input);
    let mut rules = Vec::new();
    loop {
        rules.extend(parse_contents(&mut stream, 0, Context::Rule));
        if stream.next().is_none() {
            return rules;
        }
    }
}

/// At-rule names whose blocks contain nested rules: a keyframes rule's
/// are its keyframes, a selector list of offsets and a block of
/// declarations each (CSS Animations 1 section 3, M22); a feature values
/// rule's are its feature value blocks (CSS Fonts 4 section 6.10).
const NESTED_RULE_AT: [&str; 7] = [
    "media",
    "supports",
    "layer",
    "container",
    "keyframes",
    "-webkit-keyframes",
    "font-feature-values",
];

/// At-rule names that may be nested in a style rule (CSS Nesting 1
/// section 3.2, the conditional group rules and `@layer`), whose blocks
/// are then read as a style rule's contents are, declarations and all;
/// an `@scope` rule's block always is (Cascade 6 section 2.1).
const NESTED_GROUP_AT: [&str; 5] = ["media", "supports", "layer", "container", "scope"];

/// At-rule names whose blocks contain declarations.
const DECLARATION_AT: [&str; 5] = [
    "font-face",
    "page",
    "property",
    "counter-style",
    "font-palette-values",
];

/// The feature value blocks of `@font-feature-values` (CSS Fonts 4
/// section 6.10.1), whose blocks contain declarations too.
const FEATURE_VALUE_BLOCKS: [&str; 7] = [
    "stylistic",
    "historical-forms",
    "styleset",
    "character-variant",
    "swash",
    "ornaments",
    "annotation",
];

/// How deep blocks may nest before a block is kept raw rather than
/// parsed. Real sheets nest a handful of levels; the parser recurses
/// once per level, and this is what bounds that recursion.
pub const MAX_NESTING: usize = 128;

/// Where a block's contents are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// A rule list, at the top level or in a group rule there: no
    /// declarations, a rule's `}` its block's end only.
    Top,
    /// An `@scope` rule's block, or a group rule's in one: declarations
    /// and every at-rule.
    Scope,
    /// A style rule's block, or a group rule's nested in one: the
    /// declarations, and the at-rules nesting allows.
    Rule,
}

/// What a qualified rule's block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    /// A style rule's: declarations and nested rules.
    Style,
    /// A keyframe's: declarations alone.
    Keyframe,
}

/// Parses rules until EOF (top level) or an upcoming `}` (nested).
/// `depth` is how many blocks enclose this rule list; `keyframes` says
/// it is a keyframes rule's, whose rules are keyframes.
fn parse_rules(stream: &mut TokenStream, top: bool, depth: usize, keyframes: bool) -> Vec<Rule> {
    let body = if keyframes {
        Body::Keyframe
    } else {
        Body::Style
    };
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
                rules.push(Rule::At(parse_at_rule(
                    stream,
                    name,
                    depth,
                    Context::Top,
                    !top,
                )));
            }
            Some(first) => {
                if let Some(rule) = parse_qualified_rule(stream, first, !top, depth, body, false) {
                    rules.push(Rule::Qualified(rule));
                }
            }
        }
    }
}

/// Parses one at-rule; the name has been consumed. `context` says where
/// its block is; `nested`, that the at-rule is in a block, whose `}`
/// ends it (CSS Syntax 3 section 5.5.2): a group rule's rule list, a
/// style rule's block.
fn parse_at_rule(
    stream: &mut TokenStream,
    name: String,
    depth: usize,
    context: Context,
    nested: bool,
) -> AtRule {
    let mut prelude = Vec::new();
    let mut block = None;
    let mut paren_depth = 0i32;
    loop {
        match stream.next() {
            None => break,
            Some(Token::Semicolon) if paren_depth == 0 => break,
            // A `}` ends an at-rule nested in a block, the block's own
            // `}` left for it (CSS Syntax 3 section 5.5.2).
            Some(Token::CloseCurly) if nested && paren_depth == 0 => {
                stream.reconsume(Token::CloseCurly);
                break;
            }
            Some(Token::OpenCurly) if paren_depth == 0 => {
                block = Some(parse_at_block(stream, &name, depth, context));
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
fn parse_at_block(stream: &mut TokenStream, name: &str, depth: usize, context: Context) -> Block {
    let lname = name.to_lowercase();
    if depth >= MAX_NESTING {
        // Too deep to parse: kept whole, like an unknown at-rule's block.
        return Block::Raw(take_raw_block(stream));
    }
    if NESTED_GROUP_AT.contains(&lname.as_str()) && (context != Context::Top || lname == "scope") {
        let inside = match context {
            Context::Rule => Context::Rule,
            _ => Context::Scope,
        };
        let rules = parse_contents(stream, depth + 1, inside);
        consume_close_curly(stream);
        Block::Rules(rules)
    } else if NESTED_RULE_AT.contains(&lname.as_str()) {
        let keyframes = lname.ends_with("keyframes");
        let rules = parse_rules(stream, false, depth + 1, keyframes);
        consume_close_curly(stream);
        Block::Rules(rules)
    } else if DECLARATION_AT.contains(&lname.as_str())
        || FEATURE_VALUE_BLOCKS.contains(&lname.as_str())
    {
        // An `@property` rule's `initial-value` is any value, as a custom
        // property's is (Properties and Values API section 4); a feature
        // value block's names are the author's, their case kept (CSS Fonts
        // 4 section 6.10.1).
        let declarations = parse_declarations(
            stream,
            depth + 1,
            lname == "property",
            FEATURE_VALUE_BLOCKS.contains(&lname.as_str()),
        );
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
/// caller, which ends its block on it), or, when `semicolon` stops it,
/// when a `;` does, which is left for the caller too. A style rule's
/// block too deep to read is passed over, the rule kept empty.
fn parse_qualified_rule(
    stream: &mut TokenStream,
    first: Token,
    nested: bool,
    depth: usize,
    body: Body,
    semicolon: bool,
) -> Option<QualifiedRule> {
    let mut prelude = Vec::new();
    let mut paren_depth = 0i32;
    let mut pending = Some(first);
    loop {
        match pending.take().or_else(|| stream.next()) {
            None => return None, // EOF before '{': discard the rule
            Some(Token::OpenCurly) if paren_depth == 0 => break,
            Some(Token::CloseCurly) if nested && paren_depth == 0 => {
                stream.reconsume(Token::CloseCurly);
                return None;
            }
            Some(Token::Semicolon) if semicolon && paren_depth == 0 => {
                stream.reconsume(Token::Semicolon);
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
    let (declarations, rules) = match body {
        Body::Keyframe => (
            parse_declarations(stream, depth + 1, false, false),
            Vec::new(),
        ),
        Body::Style if depth + 1 >= MAX_NESTING => {
            let _ = take_raw_block(stream);
            return Some(QualifiedRule {
                prelude,
                declarations: Vec::new(),
                rules: Vec::new(),
            });
        }
        Body::Style => {
            let mut rules = parse_contents(stream, depth + 1, Context::Rule);
            // The declarations before the first rule are the rule's own.
            let declarations = match rules.first() {
                Some(Rule::Declarations(_)) => match rules.remove(0) {
                    Rule::Declarations(declarations) => declarations,
                    _ => unreachable!("the first rule is a run of declarations"),
                },
                _ => Vec::new(),
            };
            (declarations, rules)
        }
    };
    consume_close_curly(stream);
    Some(QualifiedRule {
        prelude,
        declarations,
        rules,
    })
}

/// Parses a block's contents (CSS Syntax 3's "consume a block's
/// contents") until `}` (reconsumed for the caller) or EOF: its
/// declarations, its nested style rules and its at-rules, in a style
/// rule's block those nesting allows, each run of declarations before a
/// rule a [`Rule::Declarations`]. A rule read, kept or not, and an
/// at-rule end a run, as the draft says. `depth` is the nesting of the
/// block, `context` where it is.
fn parse_contents(stream: &mut TokenStream, depth: usize, context: Context) -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut run = Vec::new();
    loop {
        match stream.next() {
            None => break,
            Some(Token::CloseCurly) => {
                stream.reconsume(Token::CloseCurly);
                break;
            }
            Some(Token::Whitespace)
            | Some(Token::Semicolon)
            | Some(Token::Cdo)
            | Some(Token::Cdc) => {}
            Some(Token::AtKeyword(name)) => {
                end_run(&mut rules, &mut run);
                let at = parse_at_rule(stream, name, depth, context, context != Context::Top);
                // A statement, `@layer a;` among them, is no group rule.
                if context != Context::Rule
                    || (NESTED_GROUP_AT.contains(&at.name.to_ascii_lowercase().as_str())
                        && at.block.is_some())
                {
                    rules.push(Rule::At(at));
                }
            }
            Some(Token::Ident(name)) => match parse_declaration(stream, name, true, false) {
                Attempt::Declaration(declaration) => run.push(declaration),
                Attempt::Nothing => {}
                Attempt::Rule(tokens) => {
                    stream.restore(tokens);
                    if let Some(first) = stream.next() {
                        nested_rule(stream, first, depth, &mut rules, &mut run);
                    }
                }
            },
            Some(first) => nested_rule(stream, first, depth, &mut rules, &mut run),
        }
    }
    end_run(&mut rules, &mut run);
    rules
}

/// Ends a run of declarations: puts it among `rules`, if it holds any.
fn end_run(rules: &mut Vec<Rule>, run: &mut Vec<Declaration>) {
    if !run.is_empty() {
        rules.push(Rule::Declarations(std::mem::take(run)));
    }
}

/// Reads a style rule nested in a block, `first` its prelude's first
/// token, a `;` ending it as an error; one read ends the run of
/// declarations before it.
fn nested_rule(
    stream: &mut TokenStream,
    first: Token,
    depth: usize,
    rules: &mut Vec<Rule>,
    run: &mut Vec<Declaration>,
) {
    if let Some(rule) = parse_qualified_rule(stream, first, true, depth, Body::Style, true) {
        end_run(rules, run);
        rules.push(Rule::Qualified(rule));
    }
}

/// Parses declarations until `}` (reconsumed for the caller) or EOF.
/// `depth` is the nesting of the block the declarations are in; `blocks`
/// says a value may hold a `{}` block, as a custom property's always may;
/// `keep_case`, that the names are kept as written.
fn parse_declarations(
    stream: &mut TokenStream,
    depth: usize,
    blocks: bool,
    keep_case: bool,
) -> Vec<Declaration> {
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
                let _ = parse_at_rule(stream, name, depth, Context::Rule, true);
            }
            Some(Token::Ident(name)) => {
                let written = keep_case.then(|| name.clone());
                if let Attempt::Declaration(mut declaration) =
                    parse_declaration(stream, name, false, blocks)
                {
                    if let Some(written) = written {
                        declaration.name = written;
                    }
                    declarations.push(declaration);
                }
            }
            Some(other) => skip_until_delimiter(stream, other),
        }
    }
}

/// What reading a declaration came to.
enum Attempt {
    Declaration(Declaration),
    /// Nothing: a declaration passed over.
    Nothing,
    /// Not a declaration but, in a style rule's block, perhaps a nested
    /// rule: the tokens read, each with its number, to be read again.
    Rule(Vec<(Token, usize)>),
}

/// Parses one declaration; the property name has been consumed. A `{}`
/// block in the value of any property but a custom one means this was a
/// nested rule, not a declaration (`a:hover { ... }`): in a style rule's
/// block, where `rules` is set, the tokens read are given back to be
/// read again as a rule, as they are when no colon follows the name
/// (`div { ... }`); elsewhere the block is consumed to its `}` and the
/// declaration dropped, so what follows is parsed afresh. A custom
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
fn parse_declaration(stream: &mut TokenStream, name: String, rules: bool, blocks: bool) -> Attempt {
    let name_at = stream.last;
    // White space may stand between the name and the colon (CSS Syntax 3
    // section 5.4.6).
    let mut spaces = Vec::new();
    let mut next = stream.next();
    while next == Some(Token::Whitespace) {
        spaces.push(stream.last);
        next = stream.next();
    }
    // The name and the white space read, should they be read again.
    let read = |name: String, spaces: &[usize]| {
        let mut read = vec![(Token::Ident(name), name_at)];
        read.extend(spaces.iter().map(|&n| (Token::Whitespace, n)));
        read
    };
    match next {
        Some(Token::Colon) => {}
        None => return Attempt::Nothing,
        Some(t) if rules => {
            // No colon: a nested rule's prelude, perhaps.
            let mut tokens = read(name, &spaces);
            tokens.push((t, stream.last));
            return Attempt::Rule(tokens);
        }
        Some(t) => {
            // Missing colon: skip to the next ';' or '}'.
            skip_until_delimiter(stream, t);
            return Attempt::Nothing;
        }
    }
    let colon_at = stream.last;
    let custom = name.starts_with("--");
    // The value's tokens, each with its number.
    let mut value: Vec<(Token, usize)> = Vec::new();
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
                Token::OpenCurly if !custom && !blocks && rules => {
                    let mut tokens = read(name, &spaces);
                    tokens.push((Token::Colon, colon_at));
                    tokens.extend(value);
                    tokens.push((Token::OpenCurly, stream.last));
                    return Attempt::Rule(tokens);
                }
                Token::OpenCurly if !custom && !blocks => {
                    let _ = take_raw_block(stream);
                    return Attempt::Nothing;
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
        value.push((token, stream.last));
    }
    let (mut value, numbers): (Vec<Token>, Vec<usize>) = value.into_iter().unzip();
    let leading = value
        .iter()
        .take_while(|t| matches!(t, Token::Whitespace))
        .count();
    let important = strip_important(&mut value);
    trim_ws(&mut value);
    // A `unicode-range` is read from its text (CSS Syntax 3 section
    // 7.1): its tokens have lost what the text said.
    if name.eq_ignore_ascii_case("unicode-range")
        && !value.is_empty()
        && let (Some(&first), Some(&last)) =
            (numbers.get(leading), numbers.get(leading + value.len() - 1))
        && let Some(ranges) = crate::unicode_range::read_list(&stream.text(first, last))
    {
        value = crate::unicode_range::tokens(&ranges);
    }
    Attempt::Declaration(Declaration {
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
    use crate::tokenizer::tokenize;

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
        // `@font-palette-values` holds descriptors too (CSS Fonts 4
        // section 9.1).
        let sheet = parse_stylesheet("@font-palette-values --p { base-palette: 1; x: y }").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        let Some(Block::Declarations(decls)) = &at.block else {
            panic!("expected a declarations block");
        };
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "base-palette");
        // `@font-feature-values` holds its feature value blocks, each of
        // declarations (section 6.10).
        let sheet =
            parse_stylesheet("@font-feature-values f { @swash { Ab: 1; b: 2 } @x { c: 3 } }")
                .unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        let Some(Block::Rules(blocks)) = &at.block else {
            panic!("expected its rules");
        };
        let [Rule::At(swash), Rule::At(other)] = blocks.as_slice() else {
            panic!("expected two blocks");
        };
        assert!(
            matches!(&swash.block, Some(Block::Declarations(d)) if d.len() == 2 && d[0].name == "Ab")
        );
        assert!(matches!(&other.block, Some(Block::Raw(_))));
        // In a block's rule list, a `}` ends an at-rule with it (CSS
        // Syntax 3 section 5.5.2).
        let sheet = parse_stylesheet("@font-feature-values f { @x } @swash { a: 1 } }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected an at-rule");
        };
        assert!(matches!(&at.block, Some(Block::Rules(inner)) if inner.len() == 1));
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
        // CSS nesting: the `& b { ... }` block is a nested rule,
        // `color: blue` after it a run of declarations after it, and `p`
        // is still a rule.
        let sheet =
            parse_stylesheet("a { & b { color: red } color: blue } p { color: green }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(a.prelude, vec![Token::Ident("a".into())]);
        assert!(a.declarations.is_empty());
        let [Rule::Qualified(b), Rule::Declarations(after)] = a.rules.as_slice() else {
            panic!("expected a rule and declarations: {:?}", a.rules);
        };
        assert_eq!(b.prelude, tokenize("& b"));
        assert_eq!(b.declarations[0].value, vec![Token::Ident("red".into())]);
        assert_eq!(after[0].value, vec![Token::Ident("blue".into())]);
        let Rule::Qualified(p) = &sheet.rules[1] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(p.prelude, vec![Token::Ident("p".into())]);
        assert_eq!(p.declarations[0].value, vec![Token::Ident("green".into())]);

        // A block in a declaration's value makes it a rule too.
        let sheet = parse_stylesheet("a { x: y { z } color: blue } p { color: green }").unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        let [Rule::Qualified(xy), Rule::Declarations(_)] = a.rules.as_slice() else {
            panic!("expected a rule and declarations: {:?}", a.rules);
        };
        assert_eq!(xy.prelude, tokenize("x: y"));
        // A `style` attribute nests nothing: the block is passed over.
        let declarations = parse_style_attribute("x: y { z } color: blue").unwrap();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "color");
    }

    #[test]
    fn a_style_rules_block_holds_declarations_and_nested_rules_in_order() {
        // CSS Nesting 1: declarations before the first rule are the
        // rule's own, each run after a rule a nested declarations rule;
        // a name without a colon, a `:` and a `{}` block read as a
        // selector; a group rule nested holds declarations too.
        let sheet = parse_stylesheet(
            "a { color: red; div { z-index: 1 } width: 1px; height: 2px; \
             p:hover { } @media screen { top: 0; .x { } } left: 3px }",
        )
        .unwrap();
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(a.declarations.len(), 1);
        let kinds: Vec<String> = a
            .rules
            .iter()
            .map(|rule| match rule {
                Rule::Qualified(q) if q.prelude == tokenize("div") => "rule div".to_owned(),
                Rule::Qualified(q) if q.prelude == tokenize("p:hover") => "rule p:hover".to_owned(),
                Rule::Qualified(q) => format!("rule {:?}", q.prelude),
                Rule::At(at) => format!("@{}", at.name),
                Rule::Declarations(list) => format!("{} declarations", list.len()),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "rule div",
                "2 declarations",
                "rule p:hover",
                "@media",
                "1 declarations"
            ]
        );
        let Rule::At(media) = &a.rules[3] else {
            panic!("expected @media");
        };
        let Some(Block::Rules(inside)) = &media.block else {
            panic!("expected a block of rules");
        };
        assert!(matches!(
            inside.as_slice(),
            [Rule::Declarations(_), Rule::Qualified(_)]
        ));
        // At the top level `@media` holds rules alone, as before.
        let sheet = parse_stylesheet("@media screen { top: 0; .x { } }").unwrap();
        let Rule::At(media) = &sheet.rules[0] else {
            panic!("expected @media");
        };
        let Some(Block::Rules(inside)) = &media.block else {
            panic!("expected a block of rules");
        };
        assert!(matches!(inside.as_slice(), [Rule::Qualified(_)]));
        // `@scope` holds declarations at the top level too.
        let sheet = parse_stylesheet("@scope (.a) { color: red; .b { } }").unwrap();
        let Rule::At(scope) = &sheet.rules[0] else {
            panic!("expected @scope");
        };
        assert!(matches!(
            &scope.block,
            Some(Block::Rules(inside))
                if matches!(inside.as_slice(), [Rule::Declarations(_), Rule::Qualified(_)])
        ));
    }

    #[test]
    fn nested_error_recovery_ends_at_a_semicolon_or_the_block() {
        // A nested rule's prelude ends, as an error, at a `;` or at the
        // block's `}`; an at-rule nesting does not allow is dropped; a
        // `{}` with no prelude is a rule whose selector is empty.
        let sheet = parse_stylesheet(
            "a { error3234; z-index: 1; @font-face { x: y } { foo } z-index: 2; nope } b { }",
        )
        .unwrap();
        assert_eq!(sheet.rules.len(), 2);
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!("expected a qualified rule");
        };
        assert_eq!(a.declarations.len(), 1);
        let [Rule::Qualified(empty), Rule::Declarations(after)] = a.rules.as_slice() else {
            panic!("expected a rule and declarations: {:?}", a.rules);
        };
        assert!(empty.prelude.is_empty());
        assert_eq!(after.len(), 1);
        // A keyframe's block holds declarations alone.
        let sheet = parse_stylesheet("@keyframes k { from { & { } opacity: 0 } }").unwrap();
        let Rule::At(at) = &sheet.rules[0] else {
            panic!("expected @keyframes");
        };
        let Some(Block::Rules(frames)) = &at.block else {
            panic!("expected keyframes");
        };
        let Rule::Qualified(from) = &frames[0] else {
            panic!("expected a keyframe");
        };
        assert!(from.rules.is_empty());
        assert_eq!(from.declarations.len(), 1);
    }

    #[test]
    fn scope_blocks_keep_every_at_rule_and_property_values_their_blocks() {
        // An `@scope` rule at the top level is no style rule: its block
        // keeps the at-rules nesting leaves out, statements among them.
        let sheet = parse_stylesheet("@scope (.a) { @keyframes k { } @layer x; .b { @layer y; } }")
            .unwrap();
        let Rule::At(scope) = &sheet.rules[0] else {
            panic!("expected @scope");
        };
        let Some(Block::Rules(inside)) = &scope.block else {
            panic!("expected a block of rules");
        };
        assert!(
            matches!(inside.as_slice(), [Rule::At(_), Rule::At(_), Rule::Qualified(b)] if b.rules.is_empty())
        );
        // An `@property` rule's descriptors keep a `{}` block, as a custom
        // property's value does.
        let sheet =
            parse_stylesheet("@property --x { syntax: '*'; initial-value: foo(){} }").unwrap();
        let Rule::At(property) = &sheet.rules[0] else {
            panic!("expected @property");
        };
        let Some(Block::Declarations(list)) = &property.block else {
            panic!("expected declarations");
        };
        assert_eq!(list[1].value, tokenize("foo(){}"));
    }

    #[test]
    fn block_contents_read_as_the_cssom_inserts_them() {
        let rules = parse_block_contents("z-index: 3; & .b { } color: red");
        assert!(matches!(
            rules.as_slice(),
            [
                Rule::Declarations(_),
                Rule::Qualified(_),
                Rule::Declarations(_)
            ]
        ));
        assert_eq!(parse_block_contents("} a { }").len(), 1);
    }

    #[test]
    fn style_rules_nested_past_the_cap_are_passed_over() {
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let source = "a{".repeat(20_000) + &"}".repeat(20_000) + "b{color:red}";
                let sheet = parse_stylesheet(&source).unwrap();
                let mut depth = 0;
                let mut rules = &sheet.rules;
                while let [Rule::Qualified(rule), ..] = rules.as_slice() {
                    depth += 1;
                    rules = &rule.rules;
                }
                assert_eq!(depth, MAX_NESTING);
                assert_eq!(sheet.rules.len(), 2, "the rule after is read");
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
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
