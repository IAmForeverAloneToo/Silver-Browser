//! Custom properties and the arbitrary substitution functions `var()`
//! and `env()` (M21.1, M21.3; `docs/design/values.md`).
//!
//! A declaration whose name begins with two hyphens, `--` alone aside, is
//! a custom property (Variables 1 §2). Its value is any list of tokens
//! without a bad string, a bad URL, a closer with no block of its kind
//! open, or a `;` or `!` outside every block, and with each substitution
//! function in it valid by its grammar; otherwise the declaration is
//! dropped, as one invalid at parse time is. It applies to every element
//! and pseudo-element and is inherited. Its computed value is its value
//! with the substitution functions in it replaced, or the
//! guaranteed-invalid value, which `initial` gives too and which a name
//! absent from [`CustomProperties`] has.
//!
//! The substitution functions are replaced by Values 5's Appendix A.
//! `var(<declaration-value>, <declaration-value>?)` takes the custom
//! property its first argument names once that argument is itself
//! substituted, so `var(var(--name))` works, as the draft and the
//! web-platform-tests say and the reference browser refuses; the
//! fallback, everything after the first comma, is substituted only when
//! the property has the guaranteed-invalid value. Either argument may be
//! wrapped in `{}`, which is not part of it (Values 5 §2.4.1); one not
//! wrapped may hold no `{}` block of its own. `env(<custom-ident>
//! <integer [0,∞]>*, <declaration-value>?)` takes an environment variable
//! (Environment Variables 1 §2) or its fallback; its first argument is
//! read as written, never substituted, and checked where it is parsed.
//!
//! An element's own custom properties are computed name by name, each
//! one another needs before it, with the names being computed as the
//! guard of Appendix A: a name met again while it is being computed makes
//! every name from it to the newest cyclic, and a cyclic name's value is
//! the guaranteed-invalid value, a fallback in the cycle notwithstanding.
//! The walk keeps its own stack ([`Resolver::run`]), so a chain of ten
//! thousand properties each naming the next, or fallbacks nested ten
//! thousand deep, cost memory on the heap and no recursion.
//!
//! Substitution is bounded, as Values 5 §"Safely Handling Overly-Long
//! Substitution" asks, by what the tokens it makes weigh: each token its
//! size in memory and the bytes of the name, string or unit it carries.
//! No substitution — a function's result, or a value with its functions
//! replaced — may weigh more than [`MAX_SUBSTITUTION`]; one that would is
//! the guaranteed-invalid value, so thirty custom properties each
//! doubling the last end there and not at a billion tokens. And the
//! substitutions of one cascade together may weigh at most
//! [`MAX_CASCADE_SUBSTITUTION`]; past that every one is the
//! guaranteed-invalid value, a limit no page meets but one built to
//! exhaust the engine, which bounds the time and the memory a page can
//! spend on substitution however many elements it has.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::{Entry, RandomState};
use std::fmt;
use std::hash::BuildHasher;
use std::sync::{Arc, LazyLock};

use css::Token;

use crate::cascade::{CssWide, css_wide_keyword};

/// The most one substitution may weigh: a function's result, or a value
/// with its functions replaced, a megabyte of tokens — some thirty
/// thousand of them, or a string as long. The draft asks for a limit
/// "relatively high", custom properties of a kilobyte of text and more
/// having their uses.
pub const MAX_SUBSTITUTION: usize = 1 << 20;

/// The most the substitutions of one cascade may weigh together.
pub const MAX_CASCADE_SUBSTITUTION: usize = 256 << 20;

/// What `token` weighs against the bounds: its size in memory and the
/// bytes of the name, string or unit it carries.
fn weight(token: &Token) -> usize {
    std::mem::size_of::<Token>()
        + match token {
            Token::Ident(text) | Token::Function(text) | Token::AtKeyword(text) => text.len(),
            Token::Hash { value, .. } | Token::String { value, .. } | Token::Url { value, .. } => {
                value.len()
            }
            Token::Dimension { unit, .. } => unit.len(),
            _ => 0,
        }
}

/// Whether `name` is a custom property's: two hyphens and at least one
/// more code point (Variables 1 §2; `--` alone is reserved).
pub fn is_custom_property_name(name: &str) -> bool {
    name.len() > 2 && name.starts_with("--")
}

/// How many ways each level of [`CustomProperties`]' trie branches.
const FANOUT: usize = 64;

/// One branch of [`CustomProperties`]' trie.
type Branch = [Option<Leaf>; FANOUT];

/// The properties whose names fall in one leaf, sorted by name.
type Leaf = Arc<[(Arc<str>, Arc<[Token]>)]>;

/// The key the names are hashed with, chosen afresh in each process.
static KEY: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// Where `name` goes in the trie: a branch and a leaf in it.
fn slot(name: &str) -> (usize, usize) {
    let hash = KEY.hash_one(name);
    ((hash >> 58) as usize, ((hash >> 52) & 0x3f) as usize)
}

/// The computed custom properties of a style (M21.1): each one's value,
/// the tokens its declaration gave with the substitution functions in
/// them replaced. A name absent has the guaranteed-invalid value, every
/// custom property's initial value.
///
/// A style shares its parent's map unless its element declares a custom
/// property, and then copies it on write. The map is a trie of two
/// levels of 64 branches, by a hash of the name keyed afresh in each
/// process, so a copy made to change one property copies two arrays of
/// 64 pointers and the few properties that fall beside it, not the
/// hundreds or thousands a design system declares on the root; the key
/// unknown to it, a page cannot choose names that fall together.
#[derive(Clone)]
pub struct CustomProperties {
    branches: [Option<Arc<Branch>>; FANOUT],
    len: usize,
    /// The declarations this map was computed from, when an element
    /// whose winning custom property declarations are the same ones may
    /// share it (see [`Substitutions::compute`]).
    declared: Option<Declared>,
}

/// The winning custom property declarations of an element, by their
/// places in memory, sorted, in one set of stylesheets.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Declared {
    /// The stylesheets', [`crate::Stylesheets`]' own number.
    sheets: u64,
    declarations: Box<[usize]>,
}

impl Default for CustomProperties {
    fn default() -> Self {
        CustomProperties {
            branches: [const { None }; FANOUT],
            len: 0,
            declared: None,
        }
    }
}

impl CustomProperties {
    /// The value of the custom property `name`; `None` when it has the
    /// guaranteed-invalid value.
    pub fn get(&self, name: &str) -> Option<&Arc<[Token]>> {
        let (branch, leaf) = slot(name);
        let leaf = self.branches[branch].as_ref()?[leaf].as_ref()?;
        let index = leaf.binary_search_by(|(n, _)| (**n).cmp(name)).ok()?;
        Some(&leaf[index].1)
    }

    /// How many custom properties have a value.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no custom property has a value.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Each custom property with a value, in no order that means
    /// anything.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[Token])> {
        self.branches
            .iter()
            .flatten()
            .flat_map(|branch| branch.iter().flatten())
            .flat_map(|leaf| leaf.iter())
            .map(|(name, value)| (&**name, &**value))
    }

    /// Gives `name` the value `value`.
    fn insert(&mut self, name: &str, value: Arc<[Token]>) {
        let (branch, leaf) = slot(name);
        let branch = Arc::make_mut(
            self.branches[branch].get_or_insert_with(|| Arc::new([const { None }; FANOUT])),
        );
        let mut entries = branch[leaf].as_deref().unwrap_or_default().to_vec();
        match entries.binary_search_by(|(n, _)| (**n).cmp(name)) {
            Ok(index) => entries[index].1 = value,
            Err(index) => {
                entries.insert(index, (Arc::from(name), value));
                self.len += 1;
            }
        }
        branch[leaf] = Some(entries.into());
    }

    /// Gives `name` the guaranteed-invalid value.
    fn remove(&mut self, name: &str) {
        let (branch_at, leaf_at) = slot(name);
        let Some(branch) = &self.branches[branch_at] else {
            return;
        };
        let Some(leaf) = &branch[leaf_at] else {
            return;
        };
        let Ok(index) = leaf.binary_search_by(|(n, _)| (**n).cmp(name)) else {
            return;
        };
        let mut entries = leaf.to_vec();
        entries.remove(index);
        let branch = Arc::make_mut(self.branches[branch_at].as_mut().expect("present"));
        branch[leaf_at] = (!entries.is_empty()).then(|| entries.into());
        if branch.iter().all(Option::is_none) {
            self.branches[branch_at] = None;
        }
        self.len -= 1;
    }
}

impl PartialEq for CustomProperties {
    /// Whether the two give every custom property the same value; what
    /// they were computed from aside. A name falls in the same place in
    /// both, the key being the process's, so the tries compare place by
    /// place, and a branch or leaf they share at once.
    fn eq(&self, other: &Self) -> bool {
        fn same<T: ?Sized>(a: &Arc<T>, b: &Arc<T>, eq: impl FnOnce(&T, &T) -> bool) -> bool {
            Arc::ptr_eq(a, b) || eq(a, b)
        }
        let leaves = |a: &Leaf, b: &Leaf| {
            same(a, b, |a, b| {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((an, av), (bn, bv))| an == bn && same(av, bv, |av, bv| av == bv))
            })
        };
        let branches = |a: &Arc<Branch>, b: &Arc<Branch>| {
            same(a, b, |a, b| {
                a.iter().zip(b).all(|pair| match pair {
                    (None, None) => true,
                    (Some(a), Some(b)) => leaves(a, b),
                    _ => false,
                })
            })
        };
        self.len == other.len
            && self
                .branches
                .iter()
                .zip(&other.branches)
                .all(|pair| match pair {
                    (None, None) => true,
                    (Some(a), Some(b)) => branches(a, b),
                    _ => false,
                })
    }
}

impl fmt::Debug for CustomProperties {
    /// The properties sorted by name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(&str, &[Token])> = self.iter().collect();
        entries.sort_unstable_by_key(|&(name, _)| name);
        f.debug_map().entries(entries).finish()
    }
}

/// A substitution function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Var,
    Env,
}

/// The substitution function `token` opens, if it opens one: function
/// names are ASCII case-insensitive.
fn substitution_function(token: &Token) -> Option<Function> {
    match token {
        Token::Function(name) if name.eq_ignore_ascii_case("var") => Some(Function::Var),
        Token::Function(name) if name.eq_ignore_ascii_case("env") => Some(Function::Env),
        _ => None,
    }
}

/// Whether `tokens` hold a substitution function, at any depth.
pub(crate) fn has_substitution(tokens: &[Token]) -> bool {
    tokens.iter().any(|t| substitution_function(t).is_some())
}

/// The closer of the block `token` opens, a function or a `(`, `[` or
/// `{`, or `None` when it opens none.
fn opens(token: &Token) -> Option<u8> {
    match token {
        Token::Function(_) | Token::OpenParen => Some(b')'),
        Token::OpenSquare => Some(b']'),
        Token::OpenCurly => Some(b'}'),
        _ => None,
    }
}

/// The closer `token` is, if it is one.
fn closer(token: &Token) -> Option<u8> {
    match token {
        Token::CloseParen => Some(b')'),
        Token::CloseSquare => Some(b']'),
        Token::CloseCurly => Some(b'}'),
        _ => None,
    }
}

/// Whether `name` may be a `<custom-ident>`: not a CSS-wide keyword nor
/// `default` (Values 4 §4.2), ASCII case-insensitively.
fn is_custom_ident(name: &str) -> bool {
    ![
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ]
    .iter()
    .any(|keyword| name.eq_ignore_ascii_case(keyword))
}

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
    /// In the first argument, not the fallback.
    first: bool,
    argument: Argument,
    /// Whether the first argument's `{}` block holds anything but white
    /// space, or `env()`'s first argument has its name.
    named: bool,
}

impl Open {
    fn new(function: Function) -> Self {
        Open {
            function,
            first: true,
            argument: Argument::Empty,
            named: false,
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

    /// Takes a comma at the top level of the argument.
    fn comma(&mut self) -> bool {
        if self.first {
            if !self.first_is_whole() {
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

    /// Whether the first argument is not empty: `var()`'s, wrapped or
    /// not, and `env()`'s has its name.
    fn first_is_whole(&self) -> bool {
        match self.function {
            Function::Env => self.named,
            Function::Var => match self.argument {
                Argument::Plain => true,
                Argument::Unwrapped => self.named,
                Argument::Empty | Argument::Wrapped => false,
            },
        }
    }

    /// Whether the function may end here.
    fn close(&self) -> bool {
        !self.first || self.first_is_whole()
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
/// function in them valid by its grammar — `var()` by its argument
/// grammar, its arguments `<declaration-value>`s, wrapped in `{}` or not,
/// the first not empty; `env()` with its first argument a
/// `<custom-ident>` and integers no less than 0 — and, when `custom`,
/// the whole a custom property's value (Variables 1 §2.1). One pass over
/// the tokens, however deep the functions nest. A block a value leaves
/// open at its end is closed there, as CSS Syntax 3 closes it.
pub(crate) fn check(tokens: &[Token], custom: bool) -> bool {
    let mut open: Vec<Block> = Vec::new();
    // How many substitution functions are open: inside one, the rules
    // of a `<declaration-value>` hold.
    let mut functions = 0usize;
    for token in tokens {
        match token {
            Token::String { bad: true, .. } | Token::Url { bad: true, .. }
                if custom || functions > 0 =>
            {
                return false;
            }
            _ => {}
        }
        if let Some(closer) = closer(token) {
            if open.last().map(Block::closer) != Some(closer) {
                // A closer with no block of its kind open.
                if custom || functions > 0 {
                    return false;
                }
                continue;
            }
            match open.pop().expect("a block is open") {
                Block::Function(function) => {
                    functions -= 1;
                    if !function.close() {
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
                    if !function.comma() {
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
                if custom && matches!(token, Token::Semicolon | Token::Delim('!')) {
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
            open.push(match substitution_function(token) {
                Some(function) => {
                    functions += 1;
                    Block::Function(Open::new(function))
                }
                None if wrapper => Block::Wrapper,
                None => Block::Plain(closer),
            });
        }
    }
    // The blocks still open close at the end.
    while let Some(block) = open.pop() {
        match block {
            Block::Function(function) => {
                if !function.close() {
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

/// How one element's custom property declaration stands in the cascade.
pub(crate) struct Declaration<'t> {
    /// The property's name.
    pub(crate) name: &'t str,
    /// Its value's tokens.
    pub(crate) value: &'t [Token],
    /// Its place in memory, when it is a stylesheet's, which every
    /// element it applies to shares; `None` for an inline style's.
    pub(crate) identity: Option<usize>,
}

/// What became of a property's value with its substitution functions
/// replaced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Substituted {
    /// The value had a substitution function invalid by its grammar:
    /// the declaration is invalid at parse time and ignored.
    Ignored,
    /// The substitution gave the guaranteed-invalid value: the
    /// declaration is invalid at computed-value time.
    Invalid,
    /// The tokens it gave.
    Value(Arc<[Token]>),
}

/// What the substitutions of one cascade share: the stylesheets' number,
/// the weight they may still make, and what they have made already,
/// kept for the elements that need it again.
pub(crate) struct Substitutions {
    sheets: u64,
    remaining: Cell<usize>,
    /// The values of custom property declarations with no substitution
    /// function in them, by the declaration's place in memory: one
    /// list of tokens for every element that declares them.
    literals: RefCell<HashMap<usize, Arc<[Token]>>>,
    /// Values substituted, by the declaration's place in memory and the
    /// custom properties': the same declaration with the same custom
    /// properties substitutes to the same tokens, which the elements that
    /// share a map, nearly all of a page's, then substitute once. The
    /// map is held, so its place is not reused while the entry lives.
    values: RefCell<HashMap<(usize, usize), Kept>>,
}

/// A substitution kept for the elements that need it again: what it gave,
/// and the custom properties it was made with, held so that their place
/// is not another map's while the entry lives.
struct Kept {
    _custom: Option<Arc<CustomProperties>>,
    substituted: Substituted,
}

impl Substitutions {
    /// The substitutions of a cascade with the stylesheets numbered
    /// `sheets`, which owns every declaration with an identity it meets.
    pub(crate) fn new(sheets: u64) -> Self {
        Substitutions {
            sheets,
            remaining: Cell::new(MAX_CASCADE_SUBSTITUTION),
            literals: RefCell::default(),
            values: RefCell::default(),
        }
    }

    /// Takes `weight` from what the cascade may still substitute; once
    /// that is spent, every substitution fails.
    fn charge(&self, weight: usize) -> bool {
        match self.remaining.get().checked_sub(weight) {
            Some(rest) => {
                self.remaining.set(rest);
                true
            }
            None => {
                self.remaining.set(0);
                false
            }
        }
    }

    /// The tokens of `declaration`'s value, shared when it is a
    /// stylesheet's.
    fn literal(&self, declaration: &Declaration<'_>) -> Arc<[Token]> {
        let Some(identity) = declaration.identity else {
            return Arc::from(declaration.value);
        };
        Arc::clone(
            self.literals
                .borrow_mut()
                .entry(identity)
                .or_insert_with(|| Arc::from(declaration.value)),
        )
    }

    /// The custom properties of an element whose own custom property
    /// declarations are `declared`, in cascade order, the least
    /// precedent first, and whose parent's are `parent`.
    ///
    /// The winning declaration of each name is the last valid one: a
    /// CSS-wide keyword, or a value [`check`] takes; `revert` and
    /// `revert-layer`, which the cascade does not know (G7), are passed
    /// over as every property passes them over. `initial` gives the
    /// guaranteed-invalid value, `inherit` and `unset` the parent's.
    ///
    /// When the winning declarations are the very ones the parent's map
    /// was computed from, the element shares that map without computing
    /// it: computing the same declarations again over a map they made
    /// gives that map, the names they do not declare being the same. So a
    /// set of custom properties declared on `*` is computed once, not for
    /// every element. A map is `None` when no custom property has a
    /// value.
    pub(crate) fn compute(
        &self,
        declared: &[Declaration<'_>],
        parent: Option<&Arc<CustomProperties>>,
    ) -> Option<Arc<CustomProperties>> {
        enum Kind {
            Keyword(CssWide),
            Literal,
            Substituted,
        }
        let mut winners: Vec<(&Declaration<'_>, Kind)> = Vec::new();
        let mut places: HashMap<&str, usize> = HashMap::new();
        for declaration in declared {
            let kind = if let Some(keyword) = css_wide_keyword(declaration.value) {
                Kind::Keyword(keyword)
            } else if is_revert(declaration.value) || !check(declaration.value, true) {
                continue;
            } else if has_substitution(declaration.value) {
                Kind::Substituted
            } else {
                Kind::Literal
            };
            match places.entry(declaration.name) {
                Entry::Occupied(place) => winners[*place.get()] = (declaration, kind),
                Entry::Vacant(place) => {
                    place.insert(winners.len());
                    winners.push((declaration, kind));
                }
            }
        }
        if winners.is_empty() {
            return parent.cloned();
        }
        let fingerprint = winners
            .iter()
            .map(|(declaration, _)| declaration.identity)
            .collect::<Option<Vec<usize>>>()
            .map(|mut declarations| {
                declarations.sort_unstable();
                Declared {
                    sheets: self.sheets,
                    declarations: declarations.into(),
                }
            });
        if let (Some(fingerprint), Some(parent)) = (&fingerprint, parent)
            && parent.declared.as_ref() == Some(fingerprint)
        {
            return Some(Arc::clone(parent));
        }
        let inherited = parent.map(|p| &**p);
        let states = winners
            .iter()
            .map(|(declaration, kind)| match kind {
                Kind::Keyword(CssWide::Initial) => State::Done(None),
                Kind::Keyword(CssWide::Inherit | CssWide::Unset) => {
                    State::Done(inherited.and_then(|m| m.get(declaration.name)).cloned())
                }
                Kind::Literal => State::Done(Some(self.literal(declaration))),
                Kind::Substituted => State::Pending(declaration.value),
            })
            .collect();
        let mut resolver = Resolver {
            own: places,
            names: winners.iter().map(|(d, _)| d.name).collect(),
            states,
            active: Vec::new(),
            inherited,
            substitutions: self,
        };
        for index in 0..winners.len() {
            if matches!(resolver.states[index], State::Pending(_)) {
                resolver.run(Vec::new(), Frame::Property(index));
            }
        }
        let mut map = match parent {
            Some(parent) => CustomProperties::clone(parent),
            None => CustomProperties::default(),
        };
        for ((declaration, _), state) in winners.iter().zip(&resolver.states) {
            let State::Done(value) = state else {
                unreachable!("every property is computed")
            };
            match value {
                // An inherited value is in the map already.
                Some(value)
                    if inherited
                        .and_then(|m| m.get(declaration.name))
                        .is_some_and(|old| Arc::ptr_eq(old, value)) => {}
                Some(value) => map.insert(declaration.name, Arc::clone(value)),
                None => map.remove(declaration.name),
            }
        }
        map.declared = fingerprint;
        (!map.is_empty()).then(|| Arc::new(map))
    }

    /// `value`, a property's other than a custom one, with its
    /// substitution functions replaced against the element's custom
    /// properties `custom`; `identity` is the declaration's, when it is a
    /// stylesheet's.
    pub(crate) fn substitute(
        &self,
        value: &[Token],
        identity: Option<usize>,
        custom: Option<&Arc<CustomProperties>>,
    ) -> Substituted {
        let key =
            identity.map(|identity| (identity, custom.map_or(0, |m| Arc::as_ptr(m) as usize)));
        if let Some(key) = key
            && let Some(kept) = self.values.borrow().get(&key)
        {
            return kept.substituted.clone();
        }
        let substituted = if !check(value, false) {
            Substituted::Ignored
        } else {
            let mut resolver = Resolver {
                own: HashMap::new(),
                names: Vec::new(),
                states: Vec::new(),
                active: Vec::new(),
                inherited: custom.map(|m| &**m),
                substitutions: self,
            };
            let root = Frame::Value(Run::new(0, 0, value.len()));
            match resolver.run(vec![Source::new(value)], root) {
                Some(tokens) => Substituted::Value(tokens.into()),
                None => Substituted::Invalid,
            }
        };
        if let Some(key) = key {
            let kept = Kept {
                _custom: custom.cloned(),
                substituted: substituted.clone(),
            };
            self.values.borrow_mut().insert(key, kept);
        }
        substituted
    }
}

/// Whether `tokens` are the CSS-wide keyword `revert` or `revert-layer`,
/// which the cascade does not know yet (G7).
fn is_revert(tokens: &[Token]) -> bool {
    match tokens {
        [Token::Ident(word)] => {
            word.eq_ignore_ascii_case("revert") || word.eq_ignore_ascii_case("revert-layer")
        }
        _ => false,
    }
}

/// Whether the value `name: value` is valid where it is parsed, for a
/// custom property: a CSS-wide keyword the cascade knows, or a value
/// [`check`] takes.
pub(crate) fn is_valid_custom_declaration(value: &[Token]) -> bool {
    css_wide_keyword(value).is_some() || (!is_revert(value) && check(value, true))
}

/// The value of the environment variable `name` with `indices` indices
/// (Environment Variables 1 §2), as a desktop browser's window has them:
/// the safe area's insets, maximum or not, are `0px`, the window being a
/// rectangle; `preferred-text-scale` is 1, the engine scaling no text;
/// the viewport segments are not defined, the viewport never being
/// split. A variable given more or fewer indices than it has dimensions
/// is not defined either. These are the reference browser's answers.
fn environment(name: &str, indices: usize) -> Option<Arc<[Token]>> {
    static ZERO: LazyLock<Arc<[Token]>> = LazyLock::new(|| {
        Arc::from([Token::Dimension {
            value: 0.0,
            unit: "px".into(),
        }])
    });
    static ONE: LazyLock<Arc<[Token]>> = LazyLock::new(|| {
        Arc::from([Token::Number {
            value: 1.0,
            is_integer: true,
        }])
    });
    match (name, indices) {
        (
            "safe-area-inset-top"
            | "safe-area-inset-right"
            | "safe-area-inset-bottom"
            | "safe-area-inset-left"
            | "safe-area-max-inset-top"
            | "safe-area-max-inset-right"
            | "safe-area-max-inset-bottom"
            | "safe-area-max-inset-left",
            0,
        ) => Some(Arc::clone(&ZERO)),
        ("preferred-text-scale", 0) => Some(Arc::clone(&ONE)),
        _ => None,
    }
}

/// The name a `var()`'s first argument gives once substituted: one
/// identifier of two hyphens and more, white space around it allowed.
fn custom_property_name(tokens: &[Token]) -> Option<&str> {
    let mut tokens = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    match (tokens.next(), tokens.next()) {
        (Some(Token::Ident(name)), None) if is_custom_property_name(name) => Some(name),
        _ => None,
    }
}

/// Tokens being substituted, and where each block in them closes.
struct Source<'t> {
    tokens: &'t [Token],
    /// For each token that opens a block, a function or a `(`, `[` or
    /// `{`, the place of its closer, or the length of the tokens when it
    /// has none; nothing that means anything for the other tokens.
    closes: Vec<usize>,
}

impl<'t> Source<'t> {
    fn new(tokens: &'t [Token]) -> Self {
        let mut closes = vec![tokens.len(); tokens.len()];
        let mut open: Vec<(usize, u8)> = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            if let Some(closer) = opens(token) {
                open.push((at, closer));
            } else if let Some(closer) = closer(token)
                && open.last().is_some_and(|&(_, c)| c == closer)
            {
                let (opener, _) = open.pop().expect("a block is open");
                closes[opener] = at;
            }
        }
        Source { tokens, closes }
    }

    /// Where the block opened at `at` closes.
    fn close(&self, at: usize) -> usize {
        self.closes[at]
    }

    /// The first comma between `start` and `end` outside every block.
    fn top_level_comma(&self, start: usize, end: usize) -> Option<usize> {
        let mut at = start;
        while at < end {
            match &self.tokens[at] {
                Token::Comma => return Some(at),
                token if opens(token).is_some() => at = self.close(at) + 1,
                _ => at += 1,
            }
        }
        None
    }

    /// `start..end` without the white space at its ends.
    fn trim(&self, mut start: usize, mut end: usize) -> Span {
        while start < end && matches!(self.tokens[start], Token::Whitespace) {
            start += 1;
        }
        while end > start && matches!(self.tokens[end - 1], Token::Whitespace) {
            end -= 1;
        }
        (start, end)
    }

    /// The argument between `start` and `end`: trimmed, and, when it is
    /// wrapped in `{}`, what is inside them (Values 5 §2.4.1); `None`
    /// when anything follows its wrapper.
    fn argument(&self, start: usize, end: usize) -> Option<Span> {
        let (start, end) = self.trim(start, end);
        if start < end && matches!(self.tokens[start], Token::OpenCurly) {
            let close = self.close(start);
            if close + 1 < end {
                return None;
            }
            return Some(self.trim(start + 1, close.min(end)));
        }
        Some((start, end))
    }

    /// The arguments of the `var()` whose tokens run from `start` to
    /// `end`: the first, not empty, and the fallback, when a comma is
    /// there.
    fn var_arguments(&self, start: usize, end: usize) -> Option<Arguments> {
        let comma = self.top_level_comma(start, end);
        let first = self.argument(start, comma.unwrap_or(end))?;
        if first.0 == first.1 {
            return None;
        }
        let fallback = match comma {
            Some(comma) => Some(self.argument(comma + 1, end)?),
            None => None,
        };
        Some((first, fallback))
    }

    /// The arguments of the `env()` whose tokens run from `start` to
    /// `end`: the variable's name, how many indices follow it, and the
    /// fallback, when a comma is there.
    fn env_arguments(&self, start: usize, end: usize) -> Option<Environment<'t>> {
        let comma = self.top_level_comma(start, end);
        let mut name = None;
        let mut indices = 0;
        for token in &self.tokens[start..comma.unwrap_or(end)] {
            match token {
                Token::Whitespace => {}
                Token::Ident(n) if name.is_none() && is_custom_ident(n) => name = Some(n.as_str()),
                Token::Number {
                    value,
                    is_integer: true,
                } if name.is_some() && *value >= 0.0 => indices += 1,
                _ => return None,
            }
        }
        let fallback = match comma {
            Some(comma) => Some(self.argument(comma + 1, end)?),
            None => None,
        };
        Some(Environment {
            name: name?,
            indices,
            fallback,
        })
    }
}

/// A run of a source's tokens: where it begins and where it ends.
type Span = (usize, usize);

/// A function's first argument and its fallback.
type Arguments = (Span, Option<Span>);

/// What an `env()`'s arguments say: the variable's name, how many
/// indices follow it, and the fallback.
struct Environment<'t> {
    name: &'t str,
    indices: usize,
    fallback: Option<Span>,
}

/// Where an element's custom property stands while its custom properties
/// are computed.
enum State<'t> {
    /// Declared with substitution functions; not computed yet.
    Pending(&'t [Token]),
    /// Being computed by the frame at `frame` on the stack. `cycle` is
    /// where the lowest cycle found through it begins on the stack, when
    /// one is: see [`Resolver::var`].
    Active { frame: usize, cycle: Option<usize> },
    /// Computed: its value, or `None` for the guaranteed-invalid value.
    Done(Option<Arc<[Token]>>),
}

/// A step of the substitution.
enum Frame {
    /// Computing the element's own custom property at this place among
    /// the resolver's.
    Property(usize),
    /// Substituting tokens.
    Value(Run),
}

/// What a frame waits for, the function it is replacing ending at `end`.
#[derive(Clone, Copy)]
enum Waiting {
    /// `var()`'s first argument substituted, the name it gives.
    Name { end: usize, fallback: Option<Span> },
    /// The element's custom property at `index`, which `var()` names,
    /// computed.
    Property {
        end: usize,
        index: usize,
        fallback: Option<Span>,
    },
    /// The fallback substituted.
    Fallback { end: usize },
}

/// What a frame gave the one under it.
enum Returned {
    /// A run's tokens and their weight, or `None` for the
    /// guaranteed-invalid value.
    Value(Option<(Vec<Token>, usize)>),
    /// A custom property computed, which the frame under reads.
    Property,
}

/// Tokens being substituted: a run of a source's from `pos` to `end`, and
/// what they have given so far.
struct Run {
    source: usize,
    pos: usize,
    end: usize,
    out: Vec<Token>,
    weight: usize,
    /// Set once the run has met the guaranteed-invalid value; the rest of
    /// it is still read, for the cycles in it, but not kept.
    invalid: bool,
    waiting: Option<Waiting>,
}

impl Run {
    fn new(source: usize, start: usize, end: usize) -> Self {
        Run {
            source,
            pos: start,
            end,
            out: Vec::new(),
            weight: 0,
            invalid: false,
            waiting: None,
        }
    }

    /// Adds `weight` to the run's, within the bounds; the run becomes the
    /// guaranteed-invalid value when it would go past one.
    fn grow(&mut self, weight: usize, substitutions: &Substitutions) -> bool {
        self.weight += weight;
        if self.weight > MAX_SUBSTITUTION || !substitutions.charge(weight) {
            self.fail();
            return false;
        }
        true
    }

    fn fail(&mut self) {
        self.invalid = true;
        self.out = Vec::new();
    }

    /// Copies `token`, one of the run's own.
    fn push(&mut self, token: &Token, substitutions: &Substitutions) {
        if !self.invalid && self.grow(weight(token), substitutions) {
            self.out.push(token.clone());
        }
    }

    /// Copies a function's result, `tokens`.
    fn splice(&mut self, tokens: &[Token], substitutions: &Substitutions) {
        if self.invalid {
            return;
        }
        let weight = tokens.iter().map(weight).sum();
        if weight > MAX_SUBSTITUTION {
            self.fail();
        } else if self.grow(weight, substitutions) {
            self.out.extend_from_slice(tokens);
        }
    }

    /// Takes a function's result, `tokens`, which weigh `weight` and were
    /// charged when they were copied.
    fn append(&mut self, tokens: Vec<Token>, weight: usize) {
        if self.invalid {
            return;
        }
        self.weight += weight;
        if self.weight > MAX_SUBSTITUTION {
            self.fail();
        } else {
            self.out.extend(tokens);
        }
    }

    /// What the run gave, the white space at its ends left out, and its
    /// weight; `None` for the guaranteed-invalid value.
    fn finish(self) -> Option<(Vec<Token>, usize)> {
        if self.invalid {
            return None;
        }
        let mut out = self.out;
        while matches!(out.last(), Some(Token::Whitespace)) {
            out.pop();
        }
        let leading = out
            .iter()
            .take_while(|t| matches!(t, Token::Whitespace))
            .count();
        out.drain(..leading);
        Some((out, self.weight))
    }
}

/// The substitution of one element's values: its own custom properties,
/// declared, being computed or computed, and the custom properties it
/// has otherwise, its parent's or, once its own are computed, its own.
struct Resolver<'t, 's> {
    /// The places of the element's own custom properties among `states`,
    /// by name.
    own: HashMap<&'t str, usize>,
    /// Their names, by their places.
    names: Vec<&'t str>,
    states: Vec<State<'t>>,
    /// The places among `states` of the properties being computed, the
    /// newest last.
    active: Vec<usize>,
    inherited: Option<&'s CustomProperties>,
    substitutions: &'s Substitutions,
}

impl<'t> Resolver<'t, '_> {
    /// Runs the substitution from `root` over `sources` to its end: the
    /// root run's tokens, or `None` for the guaranteed-invalid value; a
    /// root custom property leaves its value in its state.
    ///
    /// The frames are kept on a stack of their own: a run that meets a
    /// function whose result needs another run or a custom property
    /// computed pushes itself and that, and is resumed with what it gave.
    fn run(&mut self, mut sources: Vec<Source<'t>>, root: Frame) -> Option<Vec<Token>> {
        let mut stack = vec![root];
        let mut returned: Option<Returned> = None;
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Property(index) => match returned.take() {
                    None => {
                        let State::Pending(tokens) = self.states[index] else {
                            unreachable!("a custom property is computed once")
                        };
                        self.states[index] = State::Active {
                            frame: stack.len(),
                            cycle: None,
                        };
                        self.active.push(index);
                        sources.push(Source::new(tokens));
                        stack.push(Frame::Property(index));
                        stack.push(Frame::Value(Run::new(sources.len() - 1, 0, tokens.len())));
                    }
                    Some(Returned::Value(value)) => {
                        let State::Active { cycle, .. } = self.states[index] else {
                            unreachable!("a custom property computed was active")
                        };
                        self.active.pop();
                        // The cycle reaches the property under this one
                        // when it begins at or below it.
                        if let (Some(begin), Some(&under)) = (cycle, self.active.last())
                            && let State::Active { frame, cycle } = &mut self.states[under]
                            && begin <= *frame
                        {
                            *cycle = Some(cycle.map_or(begin, |c| c.min(begin)));
                        }
                        let value = value.filter(|_| cycle.is_none());
                        let value = value.and_then(|(tokens, _)| self.settle(index, tokens));
                        self.states[index] = State::Done(value);
                        returned = Some(Returned::Property);
                    }
                    Some(Returned::Property) => unreachable!("a property waits for its value"),
                },
                Frame::Value(mut run) => match self.step(&mut run, returned.take(), &sources) {
                    Some(child) => {
                        stack.push(Frame::Value(run));
                        stack.push(child);
                    }
                    None => returned = Some(Returned::Value(run.finish())),
                },
            }
        }
        match returned {
            Some(Returned::Value(value)) => value.map(|(tokens, _)| tokens),
            _ => None,
        }
    }

    /// The computed value of the element's custom property at `index`,
    /// whose declared value substituted to `tokens`. A lone CSS-wide
    /// keyword is that keyword, as if it were declared (Values 5 Appendix
    /// A): `initial` the guaranteed-invalid value, `inherit` and `unset`
    /// the inherited value. `revert` and `revert-layer`, which the
    /// cascade does not know (G7), are `unset`, which is what they give a
    /// custom property no stylesheet of an earlier origin declares.
    fn settle(&self, index: usize, tokens: Vec<Token>) -> Option<Arc<[Token]>> {
        let inherited = || {
            self.inherited
                .and_then(|m| m.get(self.names[index]))
                .cloned()
        };
        match css_wide_keyword(&tokens) {
            Some(CssWide::Initial) => None,
            Some(CssWide::Inherit | CssWide::Unset) => inherited(),
            None if is_revert(&tokens) => inherited(),
            None => Some(tokens.into()),
        }
    }

    /// Takes the run on, with what the frame it waited for `returned`:
    /// to its end, or to a function whose result needs another frame,
    /// which it returns.
    fn step(
        &mut self,
        run: &mut Run,
        returned: Option<Returned>,
        sources: &[Source<'t>],
    ) -> Option<Frame> {
        let source = &sources[run.source];
        if let Some(returned) = returned
            && let Some(child) = self.resume(run, returned)
        {
            return Some(child);
        }
        while run.pos < run.end {
            let at = run.pos;
            let token = &source.tokens[at];
            let Some(function) = substitution_function(token) else {
                run.push(token, self.substitutions);
                run.pos += 1;
                continue;
            };
            let end = source.close(at).min(run.end);
            let child = match function {
                Function::Var => match source.var_arguments(at + 1, end) {
                    None => self.replace(run, None, end, None),
                    Some(((start, stop), fallback)) => {
                        let first = &source.tokens[start..stop];
                        if let Some(name) = custom_property_name(first) {
                            self.var(run, name, end, fallback)
                        } else if has_substitution(first) {
                            run.waiting = Some(Waiting::Name { end, fallback });
                            Some(Frame::Value(Run::new(run.source, start, stop)))
                        } else {
                            // No substitution can make it a name.
                            self.replace(run, None, end, fallback)
                        }
                    }
                },
                Function::Env => match source.env_arguments(at + 1, end) {
                    None => self.replace(run, None, end, None),
                    Some(env) => {
                        self.replace(run, environment(env.name, env.indices), end, env.fallback)
                    }
                },
            };
            if child.is_some() {
                return child;
            }
        }
        None
    }

    /// Resumes the run with what the frame it waited for `returned`.
    fn resume(&mut self, run: &mut Run, returned: Returned) -> Option<Frame> {
        match (run.waiting.take(), returned) {
            (Some(Waiting::Name { end, fallback }), Returned::Value(value)) => {
                match value
                    .as_ref()
                    .and_then(|(tokens, _)| custom_property_name(tokens))
                {
                    Some(name) => self.var(run, name, end, fallback),
                    None => self.replace(run, None, end, fallback),
                }
            }
            (
                Some(Waiting::Property {
                    end,
                    index,
                    fallback,
                }),
                Returned::Property,
            ) => {
                let State::Done(value) = &self.states[index] else {
                    unreachable!("the property waited for is computed")
                };
                let value = value.clone();
                self.replace(run, value, end, fallback)
            }
            (Some(Waiting::Fallback { end }), Returned::Value(value)) => {
                match value {
                    Some((tokens, weight)) => run.append(tokens, weight),
                    None => run.fail(),
                }
                run.pos = end + 1;
                None
            }
            _ => unreachable!("a run resumes with what it waits for"),
        }
    }

    /// Replaces the `var()` ending at `end` that names `name`: by the
    /// custom property's value, once it is computed.
    fn var(
        &mut self,
        run: &mut Run,
        name: &str,
        end: usize,
        fallback: Option<Span>,
    ) -> Option<Frame> {
        let value = match self.own.get(name).copied() {
            Some(index) => match &self.states[index] {
                State::Pending(_) => {
                    run.waiting = Some(Waiting::Property {
                        end,
                        index,
                        fallback,
                    });
                    return Some(Frame::Property(index));
                }
                State::Active { frame, .. } => {
                    // A cycle: every property from this one to the
                    // newest is in it. It is marked on the newest, which
                    // hands it to the one under it as it finishes, and
                    // that on, down to this one: a mark costs the same
                    // however deep the stack.
                    let begin = *frame;
                    let newest = *self.active.last().expect("this one is active");
                    if let State::Active { cycle, .. } = &mut self.states[newest] {
                        *cycle = Some(cycle.map_or(begin, |c| c.min(begin)));
                    }
                    None
                }
                State::Done(value) => value.clone(),
            },
            None => self.inherited.and_then(|m| m.get(name)).cloned(),
        };
        self.replace(run, value, end, fallback)
    }

    /// Replaces the function ending at `end` by `value`, or, when that is
    /// the guaranteed-invalid value, by its fallback substituted, or by
    /// the guaranteed-invalid value when it has none.
    fn replace(
        &mut self,
        run: &mut Run,
        value: Option<Arc<[Token]>>,
        end: usize,
        fallback: Option<Span>,
    ) -> Option<Frame> {
        match (value, fallback) {
            (Some(value), _) => run.splice(&value, self.substitutions),
            (None, Some((start, stop))) => {
                run.waiting = Some(Waiting::Fallback { end });
                return Some(Frame::Value(Run::new(run.source, start, stop)));
            }
            (None, None) => run.fail(),
        }
        run.pos = end + 1;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        css::tokenize(source)
            .into_iter()
            .filter(|t| !matches!(t, Token::Whitespace))
            .collect()
    }

    /// A declaration of a stylesheet's, as the cascade hands one over,
    /// its tokens trimmed as the css crate trims them.
    struct Declared {
        name: String,
        value: Vec<Token>,
    }

    fn declared(source: &str) -> Vec<Declared> {
        css::parse_style_attribute(source)
            .expect("never fails")
            .into_iter()
            .map(|d| Declared {
                name: d.name,
                value: d.value,
            })
            .collect()
    }

    fn compute_with(
        substitutions: &Substitutions,
        source: &[Declared],
        parent: Option<&Arc<CustomProperties>>,
    ) -> Option<Arc<CustomProperties>> {
        let declarations: Vec<Declaration<'_>> = source
            .iter()
            .filter(|d| is_custom_property_name(&d.name))
            .map(|d| Declaration {
                name: &d.name,
                value: &d.value,
                identity: Some(std::ptr::from_ref(d) as usize),
            })
            .collect();
        substitutions.compute(&declarations, parent)
    }

    fn compute(source: &str) -> Option<Arc<CustomProperties>> {
        compute_with(&Substitutions::new(1), &declared(source), None)
    }

    /// The value of `name` in `map`, serialized as its tokens' text with
    /// a space between each.
    fn value(map: &Option<Arc<CustomProperties>>, name: &str) -> Option<String> {
        let value = map.as_ref()?.get(name)?;
        Some(text(value))
    }

    fn text(tokens: &[Token]) -> String {
        tokens
            .iter()
            .filter(|t| !matches!(t, Token::Whitespace))
            .map(|t| match t {
                Token::Ident(s) => s.clone(),
                Token::Function(s) => format!("{s}("),
                Token::String { value, .. } => format!("{value:?}"),
                Token::Number { value, .. } => value.to_string(),
                Token::Dimension { value, unit } => format!("{value}{unit}"),
                Token::Percentage { value } => format!("{value}%"),
                Token::Comma => ",".into(),
                Token::CloseParen => ")".into(),
                Token::OpenSquare => "[".into(),
                Token::CloseSquare => "]".into(),
                Token::OpenCurly => "{".into(),
                Token::CloseCurly => "}".into(),
                Token::Delim(c) => c.to_string(),
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn substitute(value: &str, custom: &Option<Arc<CustomProperties>>) -> Substituted {
        Substitutions::new(1).substitute(&css::tokenize(value), None, custom.as_ref())
    }

    #[test]
    fn a_custom_value_is_almost_any_tokens() {
        // Variables 1 §2.1, and the web-platform-tests' @supports cases.
        for good in [
            "",
            "a",
            "if(x > 5) this.width = 10",
            "{ [ a ] }",
            "{;}",
            "{!}",
            "[;]",
            "f(;)",
            "<!-- -->",
            "var(--a)",
            "var(--a,)",
            "var(--a, )",
            "var(--a, b, c)",
            "var(--a, {b, c})",
            "var(--a, var(--b, var(--c, d)))",
            "var(var(--a))",
            "var(prop)",
            "var(20px)",
            "var(--a ())",
            "var({--a})",
            "var( { --a } )",
            "var({--a, --b})",
            "var({--a}, 1px)",
            "var(--a",
            "VAR(--a)",
            "env(safe-area-inset-top)",
            "env(test 0 1 2, green)",
            "env(-test)",
            "env(--test)",
            "env( test )",
            "env(test, {})",
            "env(test, env(other, blue))",
        ] {
            assert!(check(&css::tokenize(good), true), "{good:?} is valid");
        }
        for bad in [
            ";",
            "a ! b",
            "a)",
            "a ]",
            "(]",
            "\"bad\nstring",
            "var()",
            "var( )",
            "var(, 1px)",
            "var({})",
            "var({}, 1px)",
            "var(--a {--b})",
            "var({--a} --b)",
            "var(--a {--b}, 1px)",
            "var(--a, !)",
            "var(--a, !important)",
            "var(--a, ;)",
            "var(--a, {;})",
            "var(--a, {b} c)",
            "var(--a, b {c})",
            "var(--a, ])",
            "env(test ())",
            "env(test() )",
            "env(test (),)",
            "env(10px)",
            "env(env(test))",
            "env(test 0.1, green)",
            "env(test -1, green)",
            "env(test1 test2, green)",
            "env(initial)",
            "env({test})",
            "env(test, {)",
            "env()",
            "env(, 1px)",
        ] {
            assert!(!check(&css::tokenize(bad), true), "{bad:?} is invalid");
        }
        // Outside a custom property's value, only the functions are
        // checked: the rest is the property's to parse once substituted.
        assert!(check(&css::tokenize("a ! ) var(--a)"), false));
        assert!(!check(&css::tokenize("a var(--a, !)"), false));
    }

    #[test]
    fn custom_properties_compute_by_reference() {
        let map = compute("--a: 1px; --b: var(--a) 2px; --c: var(--b) var(--b)");
        assert_eq!(value(&map, "--a").as_deref(), Some("1px"));
        assert_eq!(value(&map, "--b").as_deref(), Some("1px 2px"));
        assert_eq!(value(&map, "--c").as_deref(), Some("1px 2px 1px 2px"));
        // In any order of declaration.
        let map = compute("--c: var(--b); --b: var(--a); --a: x");
        assert_eq!(value(&map, "--c").as_deref(), Some("x"));
        // Case-sensitive names; the last valid declaration wins.
        let map = compute("--A: 1; --a: 2; --a: 3; --a: !");
        assert_eq!(value(&map, "--A").as_deref(), Some("1"));
        assert_eq!(value(&map, "--a").as_deref(), Some("3"));
        // An empty value is a value; `initial` is the guaranteed-invalid
        // value; a missing reference without a fallback makes the
        // property guaranteed-invalid, and with one takes it.
        let map = compute("--e: ; --i: initial; --m: var(--nope); --f: var(--nope, y)");
        assert_eq!(value(&map, "--e").as_deref(), Some(""));
        assert_eq!(value(&map, "--i"), None);
        assert_eq!(value(&map, "--m"), None);
        assert_eq!(value(&map, "--f").as_deref(), Some("y"));
        // Substitution is of tokens: a number and an identifier.
        let map = compute("--gap: 20; --len: var(--gap)px");
        assert_eq!(
            map.as_ref().unwrap().get("--len").map(|v| v.to_vec()),
            Some(vec![
                Token::Number {
                    value: 20.0,
                    is_integer: true
                },
                Token::Ident("px".into())
            ])
        );
    }

    #[test]
    fn cycles_make_every_name_in_them_invalid() {
        // The web-platform-tests' variable-cycles.html, case by case.
        let cases: &[(&str, &[&str], &[&str])] = &[
            ("--a:var(--a)", &["--a"], &[]),
            ("--a:var(--b); --b:var(--a)", &["--a", "--b"], &[]),
            (
                "--a:var(--b, cycle); --b:var(--c, cycle); --c:var(--a, cycle)",
                &["--a", "--b", "--c"],
                &[],
            ),
            (
                "--x:var(--y, valid); --y:var(--a, valid); --a:var(--b, cycle); \
                 --b:var(--c, cycle); --c:var(--a, cycle)",
                &["--a", "--b", "--c"],
                &["--x", "--y"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle); --b:var(--c, cycle); \
                 --c:var(--a, cycle) var(--y); --y:valid",
                &["--a", "--b", "--c"],
                &["--x", "--y"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle); --b:var(--c, cycle) var(--y); \
                 --c:var(--a, cycle); --y:valid",
                &["--a", "--b", "--c"],
                &["--x", "--y"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle); --b:var(--c, cycle); \
                 --c:var(--a, cycle) var(--y); --y:var(--z); --z:valid",
                &["--a", "--b", "--c"],
                &["--x", "--y", "--z"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle); \
                 --b:var(--c, cycle) var(--a, cycle); --c:var(--d, cycle); --d:var(--b, cycle)",
                &["--a", "--b", "--c", "--d"],
                &["--x"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle); --b:var(--c, cycle); \
                 --c:var(--d, cycle) var(--a, cycle); --d:var(--b, cycle) var(--y); --y:valid",
                &["--a", "--b", "--c", "--d"],
                &["--x", "--y"],
            ),
            (
                "--x:var(--a, valid); --a:var(--b, cycle) var(--y, valid) var(--c, cycle); \
                 --b:var(--a, cycle); --c:var(--d, cycle); --d:var(--a, cycle); --y:valid",
                &["--a", "--b", "--c", "--d"],
                &["--x", "--y"],
            ),
            (
                "--x:var(--a, valid); --a:var(--y, var(--b, cycle)); \
                 --b:var(--y, var(--c, cycle)); --c:var(--y, var(--a, cycle)); --y:valid",
                &[],
                &["--a", "--b", "--c", "--x", "--y"],
            ),
            // Through a name substituted (Variables 2).
            ("--self: var(var(--self))", &["--self"], &[]),
            (
                "--name: --cyclic; --cyclic: var(var(--name))",
                &["--cyclic"],
                &["--name"],
            ),
        ];
        for (source, invalid, valid) in cases {
            // Each declared first and last, which changes which name the
            // walk meets first.
            for order in [false, true] {
                let mut list = declared(&format!("{source}; --sanity: valid"));
                if order {
                    list.reverse();
                }
                let map = compute_with(&Substitutions::new(1), &list, None);
                for name in *invalid {
                    assert_eq!(value(&map, name), None, "{source}: {name}");
                }
                for name in *valid {
                    assert!(value(&map, name).is_some(), "{source}: {name}");
                }
                assert_eq!(value(&map, "--sanity").as_deref(), Some("valid"));
            }
        }
    }

    #[test]
    fn a_name_may_come_from_a_substitution() {
        // The web-platform-tests' variable-reference-name-substitution.
        let parent = compute("--other: 10px");
        let cases = [
            ("--myvar: --other; --w: var(var(--myvar))", Some("10px")),
            (
                "--myvar: --other; --indirect: --myvar; --w: var(var(var(--indirect)))",
                Some("10px"),
            ),
            (
                "--myvar: not-a-name; --w: var(var(--myvar), 20px)",
                Some("20px"),
            ),
            ("--w: var(var(--unset), 30px)", Some("30px")),
            ("--myvar: --other; --w: var( var(--myvar) )", Some("10px")),
            ("--myvar: other; --w: var(var(--myvar))", None),
            ("--empty: ; --w: var(var(--empty), 40px)", Some("40px")),
            (
                "--two: --other --other; --w: var(var(--two), 50px)",
                Some("50px"),
            ),
            (
                "--dimension: 10px; --w: var(var(--dimension), 60px)",
                Some("60px"),
            ),
            (
                "--string: \"--other\"; --w: var(var(--string), 70px)",
                Some("70px"),
            ),
            ("--dashes: --; --w: var(var(--dashes), 80px)", Some("80px")),
            ("--w: var({--other})", Some("10px")),
            ("--myvar: --other; --w: var({var(--myvar)})", Some("10px")),
            ("--w: var( { --other } )", Some("10px")),
            ("--w: var({--unset}, 90px)", Some("90px")),
            ("--w: var(--unset, {a, b})", Some("a , b")),
        ];
        for (source, expected) in cases {
            let list = declared(source);
            let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
            assert_eq!(value(&map, "--w").as_deref(), expected, "{source}");
        }
    }

    #[test]
    fn inherited_and_keyword_values() {
        let parent = compute("--a: 1; --b: 2; --c: 3");
        let list = declared("--a: initial; --b: inherit; --c: unset; --d: var(--b) var(--c)");
        let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
        assert_eq!(value(&map, "--a"), None);
        assert_eq!(value(&map, "--b").as_deref(), Some("2"));
        assert_eq!(value(&map, "--c").as_deref(), Some("3"));
        assert_eq!(value(&map, "--d").as_deref(), Some("2 3"));
        // A lone keyword once substituted is that keyword (the
        // web-platform-tests' variable-css-wide-keywords-after-substitution).
        let list = declared(
            "--empty: ; --a: var(--empty) initial; --b: var(--empty) inherit; \
             --c: var(--empty) UNSET; --d: var(--empty) revert; --e: var(--a) var(--b)",
        );
        let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
        assert_eq!(value(&map, "--a"), None);
        assert_eq!(value(&map, "--b").as_deref(), Some("2"));
        assert_eq!(value(&map, "--c").as_deref(), Some("3"));
        assert_eq!(value(&map, "--d"), None, "the parent has none");
        assert_eq!(value(&map, "--e"), None, "--a is guaranteed-invalid");
        let list = declared("--a: var(--nope, inherit); --f: var(--a) x");
        let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
        assert_eq!(value(&map, "--a").as_deref(), Some("1"));
        assert_eq!(value(&map, "--f").as_deref(), Some("1 x"));
        // `revert`, which the cascade does not know (G7), is passed over:
        // the declaration before it stands.
        let list = declared("--a: 4; --a: revert");
        let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
        assert_eq!(value(&map, "--a").as_deref(), Some("4"));
        // Nothing declared, or nothing valid: the parent's map itself.
        let list = declared("--a: !; color: red");
        let map = compute_with(&Substitutions::new(1), &list, parent.as_ref());
        assert!(Arc::ptr_eq(map.as_ref().unwrap(), parent.as_ref().unwrap()));
        // Every property guaranteed-invalid: no map.
        assert_eq!(compute("--a: initial; --b: var(--b)"), None);
    }

    #[test]
    fn the_same_declarations_share_the_parents_map() {
        // As a rule on `*` gives every element the same declarations.
        let substitutions = Substitutions::new(1);
        let root = declared("--a: 1; --b: var(--a) var(--a)");
        let parent = compute_with(&substitutions, &root, None);
        let child = compute_with(&substitutions, &root, parent.as_ref());
        assert!(Arc::ptr_eq(
            parent.as_ref().unwrap(),
            child.as_ref().unwrap()
        ));
        // Other declarations of the same text are computed, and equal.
        let other = declared("--a: 1; --b: var(--a) var(--a)");
        let child = compute_with(&substitutions, &other, parent.as_ref());
        assert!(!Arc::ptr_eq(
            parent.as_ref().unwrap(),
            child.as_ref().unwrap()
        ));
        assert_eq!(parent, child);
        // As are the same declarations in other stylesheets.
        let child = compute_with(&Substitutions::new(2), &root, parent.as_ref());
        assert!(!Arc::ptr_eq(
            parent.as_ref().unwrap(),
            child.as_ref().unwrap()
        ));
        // An inline style's declarations have no identity: computed.
        let declarations: Vec<Declaration<'_>> = root
            .iter()
            .map(|d| Declaration {
                name: &d.name,
                value: &d.value,
                identity: None,
            })
            .collect();
        let child = substitutions.compute(&declarations, parent.as_ref());
        assert!(!Arc::ptr_eq(
            parent.as_ref().unwrap(),
            child.as_ref().unwrap()
        ));
        assert_eq!(parent, child);
    }

    #[test]
    fn a_property_value_substitutes_or_is_invalid() {
        let map = compute("--a: 1px; --color: red; --empty: ; --kw: initial");
        let value = |source: &str| match substitute(source, &map) {
            Substituted::Value(tokens) => Ok(text(&tokens)),
            other => Err(other),
        };
        assert_eq!(value("var(--a)"), Ok("1px".into()));
        assert_eq!(value("var(--a) var(--a)"), Ok("1px 1px".into()));
        assert_eq!(value("calc(var(--a) * 2)"), Ok("calc( 1px * 2 )".into()));
        assert_eq!(value("var(--nope, 2px)"), Ok("2px".into()));
        assert_eq!(value("var(--nope,   3px   )"), Ok("3px".into()));
        assert_eq!(value("var(--nope,)"), Ok("".into()));
        assert_eq!(value("var(--nope, var(--color))"), Ok("red".into()));
        assert_eq!(value("var(--nope, initial)"), Ok("initial".into()));
        assert_eq!(value("var(--empty) x"), Ok("x".into()));
        assert_eq!(value("var(--nope)"), Err(Substituted::Invalid));
        assert_eq!(value("var(--kw)"), Err(Substituted::Invalid));
        assert_eq!(value("var(--a) var(--nope)"), Err(Substituted::Invalid));
        assert_eq!(value("var(prop, 4px)"), Ok("4px".into()));
        assert_eq!(value("var(--a"), Ok("1px".into()));
        assert_eq!(value("var()"), Err(Substituted::Ignored));
        assert_eq!(value("var(--a, !)"), Err(Substituted::Ignored));
        // Environment variables: case-sensitive names, an index count to
        // match, a fallback otherwise.
        assert_eq!(value("env(safe-area-inset-left)"), Ok("0px".into()));
        assert_eq!(
            value("env(safe-area-max-inset-bottom, 5px)"),
            Ok("0px".into())
        );
        assert_eq!(value("ENV(preferred-text-scale)"), Ok("1".into()));
        assert_eq!(value("env(SAFE-AREA-INSET-LEFT, 5px)"), Ok("5px".into()));
        assert_eq!(
            value("env(safe-area-inset-left 0, 13px)"),
            Ok("13px".into())
        );
        assert_eq!(
            value("env(viewport-segment-width 0 0, 6px)"),
            Ok("6px".into())
        );
        assert_eq!(value("env(unknown, env(other, blue))"), Ok("blue".into()));
        assert_eq!(value("env(unknown, var(--color))"), Ok("red".into()));
        assert_eq!(value("env(unknown)"), Err(Substituted::Invalid));
        assert_eq!(value("env(unknown, {})"), Ok("".into()));
        assert_eq!(value("env(env(x))"), Err(Substituted::Ignored));
    }

    #[test]
    fn a_long_chain_and_deep_fallbacks_take_no_recursion() {
        // Ten thousand properties each naming the next, declared last
        // first, and a fallback nested ten thousand deep, on a small
        // stack.
        std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(|| {
                let n = 10_000;
                let mut source = String::from("--p0: end;");
                for i in (1..=n).rev() {
                    source.push_str(&format!("--p{i}: var(--p{});", i - 1));
                }
                let map = compute(&source);
                assert_eq!(value(&map, &format!("--p{n}")).as_deref(), Some("end"));
                let deep = format!("{}deep{}", "var(--nope, ".repeat(n), ")".repeat(n));
                assert_eq!(
                    substitute(&deep, &None),
                    Substituted::Value(Arc::from([Token::Ident("deep".into())]))
                );
                // A cycle ten thousand long.
                let mut source = String::new();
                for i in 0..n {
                    source.push_str(&format!("--q{i}: var(--q{});", (i + 1) % n));
                }
                assert_eq!(compute(&source), None);
            })
            .expect("a thread")
            .join()
            .expect("no overflow");
    }

    #[test]
    fn doubling_properties_end_at_the_bound() {
        // The billion laughs (Values 5): thirty-one levels each twice the
        // last, from a string some thirty bytes long.
        let mut source = String::from("--v0: \"Something really really really long\";");
        for i in 0..31 {
            source.push_str(&format!("--v{}: var(--v{i}) var(--v{i});", i + 1));
        }
        let substitutions = Substitutions::new(1);
        let map = compute_with(&substitutions, &declared(&source), None);
        let map = map.expect("the short ones have values");
        // Each level's weight doubles: from some level on, every one is
        // past the bound and guaranteed-invalid.
        let weights: Vec<Option<usize>> = (0..=31)
            .map(|i| {
                map.get(&format!("--v{i}"))
                    .map(|v| v.iter().map(weight).sum())
            })
            .collect();
        let first_invalid = weights.iter().position(Option::is_none).expect("one fails");
        assert!((10..20).contains(&first_invalid), "{weights:?}");
        assert!(weights[first_invalid..].iter().all(Option::is_none));
        assert!(
            weights[..first_invalid]
                .iter()
                .flatten()
                .all(|w| *w <= MAX_SUBSTITUTION)
        );
        // What it spent is about twice the last value kept, not 2^31.
        let spent = MAX_CASCADE_SUBSTITUTION - substitutions.remaining.get();
        assert!(spent < 4 * MAX_SUBSTITUTION, "{spent}");
    }

    #[test]
    fn a_cascade_spends_no_more_than_its_bound() {
        // A value near the bound, substituted until the cascade's weight
        // is spent; then nothing substitutes, however small.
        let words = MAX_SUBSTITUTION / (2 * std::mem::size_of::<Token>() + 1) * 9 / 10;
        let map = compute(&format!("--big: {}", "a ".repeat(words)));
        let big = map.as_ref().unwrap().get("--big").unwrap();
        let one: usize = big.iter().map(weight).sum();
        assert!(
            one <= MAX_SUBSTITUTION && one > MAX_SUBSTITUTION / 2,
            "{one}"
        );
        let substitutions = Substitutions::new(1);
        let value = css::tokenize("var(--big)");
        let mut substituted = 0;
        while let Substituted::Value(_) = substitutions.substitute(&value, None, map.as_ref()) {
            substituted += 1;
            assert!(
                substituted <= MAX_CASCADE_SUBSTITUTION / one,
                "the bound holds"
            );
        }
        assert_eq!(substituted, MAX_CASCADE_SUBSTITUTION / one);
        let small = compute("--s: 1");
        assert_eq!(
            substitutions.substitute(&css::tokenize("var(--s)"), None, small.as_ref()),
            Substituted::Invalid
        );
    }

    #[test]
    fn a_substitution_is_kept_for_the_same_map() {
        let substitutions = Substitutions::new(1);
        let map = compute("--a: 1px");
        let value = css::tokenize("var(--a)");
        let identity = Some(std::ptr::from_ref(&value) as usize);
        let first = substitutions.substitute(&value, identity, map.as_ref());
        let Substituted::Value(first) = first else {
            panic!("substitutes")
        };
        let Substituted::Value(again) = substitutions.substitute(&value, identity, map.as_ref())
        else {
            panic!("substitutes")
        };
        assert!(Arc::ptr_eq(&first, &again));
        // Another map, another substitution.
        let other = compute("--a: 2px");
        assert_eq!(
            substitutions.substitute(&value, identity, other.as_ref()),
            Substituted::Value(Arc::from(tokens("2px")))
        );
    }

    #[test]
    fn the_map_copies_on_write_and_compares_by_value() {
        let mut map = CustomProperties::default();
        for i in 0..5000 {
            map.insert(&format!("--n{i}"), Arc::from(tokens(&i.to_string())));
        }
        assert_eq!(map.len(), 5000);
        let mut copy = map.clone();
        copy.insert("--n7", Arc::from(tokens("x")));
        // One branch and one leaf copied; every other shared.
        let shared = map
            .branches
            .iter()
            .zip(&copy.branches)
            .filter(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            })
            .count();
        assert_eq!(shared, map.branches.iter().flatten().count() - 1);
        assert_eq!(text(map.get("--n7").unwrap()), "7");
        assert_eq!(text(copy.get("--n7").unwrap()), "x");
        assert_ne!(map, copy);
        copy.insert("--n7", Arc::from(tokens("7")));
        assert_eq!(map, copy);
        copy.remove("--n7");
        copy.remove("--absent");
        assert_eq!(copy.len(), 4999);
        assert_eq!(copy.get("--n7"), None);
        assert_ne!(map, copy);
        for i in 0..5000 {
            copy.remove(&format!("--n{i}"));
        }
        assert!(copy.is_empty());
        assert!(copy.branches.iter().all(Option::is_none));
        assert_eq!(copy, CustomProperties::default());
        assert_eq!(map.iter().count(), 5000);
    }
}
