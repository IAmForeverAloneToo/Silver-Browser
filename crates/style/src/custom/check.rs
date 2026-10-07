//! What a value with substitution functions in it, or a custom property's
//! value, must be where it is parsed (M21.1, M21.3, M21.5): every
//! substitution function valid by its grammar, and the whole a
//! `<declaration-value>?` (Variables 1 §2.1, §3).

use css::Token;

use super::attr;
use super::{Function, closer, is_custom_ident, opens, substitution_function};

/// Where a substitution function's argument is, as [`check`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Argument {
    /// Nothing but white space yet.
    Empty,
    /// Begun with something other than a `{`.
    Plain,
    /// Begun with a `{}` block, which is open.
    Wrapped,
    /// Its `{}` block closed: only white space may follow.
    Unwrapped,
}

/// A substitution function open, as [`check`] reads it.
#[derive(Debug, Clone, Copy)]
struct Open {
    function: Function,
    /// Where its first argument begins among the tokens.
    start: usize,
    /// In the first argument, not the fallback.
    first: bool,
    argument: Argument,
    /// Whether the first argument's `{}` block holds anything but white
    /// space, or `env()`'s first argument has its name.
    named: bool,
    /// Whether a substitution function is in the first argument, which is
    /// then read only once it is substituted.
    dynamic: bool,
}

impl Open {
    fn new(function: Function, start: usize) -> Self {
        Open {
            function,
            start,
            first: true,
            argument: Argument::Empty,
            named: false,
            dynamic: false,
        }
    }

    /// Takes `token`, which is neither white space, a comma, a `;` nor a
    /// `!`, at the top level of the argument; says whether that is
    /// allowed.
    fn content(&mut self, token: &Token) -> bool {
        if self.function == Function::Env && self.first {
            // `<custom-ident> <integer [0,∞]>*`, read as written.
            return match token {
                Token::Ident(name) if !self.named && is_custom_ident(name) => {
                    self.named = true;
                    true
                }
                Token::Number {
                    value,
                    is_integer: true,
                } => self.named && *value >= 0.0,
                _ => false,
            };
        }
        match (self.argument, token) {
            (Argument::Empty, Token::OpenCurly) => {
                self.argument = Argument::Wrapped;
                true
            }
            (Argument::Empty, _) => {
                self.argument = Argument::Plain;
                true
            }
            // A `{}` block after other values, or anything after one.
            (Argument::Plain, Token::OpenCurly) | (Argument::Unwrapped, _) => false,
            (Argument::Plain, _) => true,
            (Argument::Wrapped, _) => unreachable!("the block takes its tokens"),
        }
    }

    /// Takes a comma at `at` at the top level of the argument.
    fn comma(&mut self, tokens: &[Token], at: usize) -> bool {
        if self.first {
            if !self.first_is_whole(tokens, at) {
                return false;
            }
            self.first = false;
            self.argument = Argument::Empty;
            return true;
        }
        // The fallback takes commas, Variables 1 §3 says, unless it is
        // wrapped.
        match self.argument {
            Argument::Empty | Argument::Plain => {
                self.argument = Argument::Plain;
                true
            }
            Argument::Wrapped | Argument::Unwrapped => false,
        }
    }

    /// Whether the first argument, which ends at `end`, is whole: not
    /// empty, wrapped or not; `env()`'s with its name; and `attr()`'s,
    /// when nothing is substituted into it, an `<attr-name>` and an
    /// `<attr-type>`, as the web-platform-tests and the reference browser
    /// have it, where the draft reads it only once it is substituted.
    fn first_is_whole(&self, tokens: &[Token], end: usize) -> bool {
        let filled = match self.argument {
            Argument::Plain => true,
            Argument::Unwrapped => self.named,
            Argument::Empty | Argument::Wrapped => false,
        };
        match self.function {
            Function::Env => self.named,
            Function::Var => filled,
            Function::Attr => {
                filled && (self.dynamic || attr::Spec::parse(&tokens[self.start..end]).is_some())
            }
        }
    }

    /// Whether the function may end at `end`.
    fn close(&self, tokens: &[Token], end: usize) -> bool {
        !self.first || self.first_is_whole(tokens, end)
    }
}

/// What opened a block, as [`check`] reads it.
#[derive(Debug, Clone, Copy)]
enum Block {
    /// A function or `(`, `[` or `{` of no meaning here.
    Plain(u8),
    /// A substitution function.
    Function(Open),
    /// The `{}` block an argument of the substitution function around it
    /// is wrapped in.
    Wrapper,
}

impl Block {
    fn closer(&self) -> u8 {
        match self {
            Block::Plain(closer) => *closer,
            Block::Function(_) => b')',
            Block::Wrapper => b'}',
        }
    }
}

/// Whether `tokens` are valid where they are parsed: every substitution
/// function in them valid by its grammar — `var()` and `attr()` by their
/// argument grammars, their arguments `<declaration-value>`s, wrapped in
/// `{}` or not, the first not empty, and `attr()`'s first argument by its
/// own grammar when nothing is substituted into it; `env()` with its
/// first argument a `<custom-ident>` and integers no less than 0 — and
/// the whole a `<declaration-value>`: no bad string or URL, no closer
/// without its block, and no `;` or `!` at its top level. That is a
/// custom property's value (Variables 1 §2.1), and the reference browser
/// holds any other property's value with a substitution function in it
/// to it too where it is parsed, so that `var(--a) !important
/// !important` or `var(--a) )` is dropped there (variable-reference-30).
/// One pass over the tokens, however deep the functions nest, each
/// `attr()`'s first argument read once more. A block a value leaves open
/// at its end is closed there, as CSS Syntax 3 closes it.
pub(crate) fn check(tokens: &[Token]) -> bool {
    let mut open: Vec<Block> = Vec::new();
    // Where the substitution functions open are in `open`, the innermost
    // last: inside one, the rules of a `<declaration-value>` hold.
    let mut functions: Vec<usize> = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        match token {
            Token::String { bad: true, .. } | Token::Url { bad: true, .. } => return false,
            _ => {}
        }
        if let Some(closer) = closer(token) {
            if open.last().map(Block::closer) != Some(closer) {
                // A closer with no block of its kind open.
                return false;
            }
            match open.pop().expect("a block is open") {
                Block::Function(function) => {
                    functions.pop();
                    if !function.close(tokens, at) {
                        return false;
                    }
                }
                Block::Wrapper => {
                    if let Some(Block::Function(function)) = open.last_mut() {
                        function.argument = Argument::Unwrapped;
                    }
                }
                Block::Plain(_) => {}
            }
            continue;
        }
        let depth = open.len();
        match open.last_mut() {
            // At the top level of a substitution function's argument.
            Some(Block::Function(function)) => match token {
                Token::Whitespace => {}
                Token::Comma => {
                    if !function.comma(tokens, at) {
                        return false;
                    }
                }
                Token::Semicolon | Token::Delim('!') => return false,
                _ => {
                    if !function.content(token) {
                        return false;
                    }
                }
            },
            // Inside an argument's `{}` wrapper, whose rules are the
            // argument's.
            Some(Block::Wrapper) => match token {
                Token::Semicolon | Token::Delim('!') => return false,
                Token::Whitespace => {}
                _ => {
                    if let Some(Block::Function(function)) = open.get_mut(depth - 2) {
                        function.named = true;
                    }
                }
            },
            Some(Block::Plain(_)) => {}
            None => {
                if matches!(token, Token::Semicolon | Token::Delim('!')) {
                    return false;
                }
            }
        }
        if let Some(closer) = opens(token) {
            let wrapper = matches!(
                open.last(),
                Some(Block::Function(Open {
                    argument: Argument::Wrapped,
                    ..
                }))
            ) && closer == b'}';
            let block = match substitution_function(token) {
                Some(function) => {
                    // A substitution function in another's first argument
                    // makes that argument known only once substituted.
                    if let Some(&place) = functions.last()
                        && let Block::Function(around) = &mut open[place]
                        && around.first
                    {
                        around.dynamic = true;
                    }
                    functions.push(open.len());
                    Block::Function(Open::new(function, at + 1))
                }
                None if wrapper => Block::Wrapper,
                None => Block::Plain(closer),
            };
            open.push(block);
        }
    }
    // The blocks still open close at the end.
    while let Some(block) = open.pop() {
        match block {
            Block::Function(function) => {
                if !function.close(tokens, tokens.len()) {
                    return false;
                }
            }
            Block::Wrapper => {
                if let Some(Block::Function(function)) = open.last_mut() {
                    function.argument = Argument::Unwrapped;
                }
            }
            Block::Plain(_) => {}
        }
    }
    true
}
