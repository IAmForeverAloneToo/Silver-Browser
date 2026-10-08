//! Custom properties and the arbitrary substitution functions `var()`,
//! `env()` and `attr()` (M21.1, M21.3, M21.5; `docs/design/values.md`).
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
//! `attr(<attr-name> <attr-type>?, <declaration-value>?)` (Values 5 §8.7)
//! reads an attribute of the element, or of a pseudo-element's
//! originating element, named as an attribute selector names one: as a
//! string, untyped or `raw-string`; as one number, `number`, made a
//! dimension by a unit or a percentage by `%`; or parsed as CSS against
//! the `<syntax>` of a `type()`, its own substitution functions replaced
//! under the guard «attribute, name». One that fails takes its fallback;
//! without one, an untyped `attr()` is an empty string and a typed one the
//! guaranteed-invalid value. Its first argument is substituted before it
//! is read, and, when nothing is substituted into it, checked where it is
//! parsed, as the web-platform-tests and the reference browser have it
//! (`attr(x type(a |), 1px)` is invalid there, not a fallback); the
//! draft's `number` and units this engine does not know are valid there
//! and fail where the attribute is read (`attr`, `check`).
//!
//! An element's own custom properties are computed name by name, each
//! one another needs before it, with the names being computed, and the
//! attributes being read as types, as the guard of Appendix A: a context
//! met again while it is guarded makes every context from it to the
//! newest cyclic. A cyclic name's value is the guaranteed-invalid value, a
//! fallback in the cycle notwithstanding; a cyclic attribute's `attr()`
//! takes its fallback. The walk keeps its own stack (`resolve`), so a
//! chain of ten thousand properties each naming the next, or fallbacks
//! nested ten thousand deep, cost memory on the heap and no recursion.
//!
//! Substitution is bounded, as Values 5 §"Safely Handling Overly-Long
//! Substitution" asks, by what the tokens it makes weigh: each token its
//! size in memory and the bytes of the name, string or unit it carries.
//! No substitution — a function's result, or a value with its functions
//! replaced — may weigh more than [`MAX_SUBSTITUTION`](crate::custom::MAX_SUBSTITUTION); one that would is
//! the guaranteed-invalid value, so thirty custom properties each
//! doubling the last end there and not at a billion tokens. And the
//! substitutions of one cascade together may weigh at most
//! [`MAX_CASCADE_SUBSTITUTION`](crate::custom::MAX_CASCADE_SUBSTITUTION); past that every one is the
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

mod attr;
mod check;
mod resolve;

pub(crate) use attr::DataType;
pub(crate) use check::check;
use resolve::{Resolver, State};

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
    /// The names whose values are animation-tainted (Variables 1 §2,
    /// M22): an animation's, or one that substituted such a value,
    /// sorted; `None` when there is none, as nearly always.
    tainted: Option<Arc<[Arc<str>]>>,
    /// The names whose values read an attribute (Values 5's attr()
    /// taint, M23.3): one whose declaration has an `attr()`, or a `var()`
    /// of such a value, or the parent's inherited; sorted, `None` when
    /// there is none. An attribute's value never becomes a URL.
    attr_tainted: Option<Arc<[Arc<str>]>>,
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
            tainted: None,
            attr_tainted: None,
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

    /// Whether the value of `name` is animation-tainted (Variables 1 §2):
    /// an animation's, or one that substituted such a value. A `var()`
    /// of it in a property that is not animatable is the
    /// guaranteed-invalid value (§3).
    pub fn is_tainted(&self, name: &str) -> bool {
        self.tainted
            .as_deref()
            .is_some_and(|names| names.binary_search_by(|n| (**n).cmp(name)).is_ok())
    }

    /// Whether any value is animation-tainted.
    pub(crate) fn has_taint(&self) -> bool {
        self.tainted.is_some()
    }

    /// Marks the value of `name` animation-tainted, or not.
    fn set_tainted(&mut self, name: &str, tainted: bool) {
        mark(&mut self.tainted, name, tainted);
    }

    /// Whether the value of `name` read an attribute (M23.3), by its own
    /// `attr()` or another custom property's.
    pub(crate) fn is_attr_tainted(&self, name: &str) -> bool {
        self.attr_tainted
            .as_deref()
            .is_some_and(|names| names.binary_search_by(|n| (**n).cmp(name)).is_ok())
    }

    /// Marks the value of `name` as having read an attribute, or not.
    fn set_attr_tainted(&mut self, name: &str, tainted: bool) {
        mark(&mut self.attr_tainted, name, tainted);
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
            && self.tainted == other.tainted
            && self.attr_tainted == other.attr_tainted
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
    Attr,
}

/// The substitution function `token` opens, if it opens one: function
/// names are ASCII case-insensitive.
fn substitution_function(token: &Token) -> Option<Function> {
    match token {
        Token::Function(name) if name.eq_ignore_ascii_case("var") => Some(Function::Var),
        Token::Function(name) if name.eq_ignore_ascii_case("env") => Some(Function::Env),
        Token::Function(name) if name.eq_ignore_ascii_case("attr") => Some(Function::Attr),
        _ => None,
    }
}

/// Whether `tokens` hold a substitution function, at any depth.
pub(crate) fn has_substitution(tokens: &[Token]) -> bool {
    tokens.iter().any(|t| substitution_function(t).is_some())
}

/// Whether `tokens` hold an `attr()`, at any depth: what they give
/// depends on the element's attributes.
fn reads_attributes(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|t| substitution_function(t) == Some(Function::Attr))
}

/// Puts `name` in the sorted set `names`, or takes it out.
fn mark(names: &mut Option<Arc<[Arc<str>]>>, name: &str, on: bool) {
    let list = names.as_deref().unwrap_or_default();
    match (list.binary_search_by(|n| (**n).cmp(name)), on) {
        (Ok(index), false) => {
            let mut list = list.to_vec();
            list.remove(index);
            *names = (!list.is_empty()).then(|| list.into());
        }
        (Err(index), true) => {
            let mut list = list.to_vec();
            list.insert(index, Arc::from(name));
            *names = Some(list.into());
        }
        _ => {}
    }
}

/// The names of the custom properties a `var()` in `tokens` reads, its
/// fallback's among them.
fn var_names(tokens: &[Token]) -> impl Iterator<Item = &str> {
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, token)| match token {
            Token::Function(name) if name.eq_ignore_ascii_case("var") => tokens[i + 1..]
                .iter()
                .find(|t| !matches!(t, Token::Whitespace))
                .and_then(|t| match t {
                    Token::Ident(name) => Some(name.as_str()),
                    _ => None,
                }),
            _ => None,
        })
}

/// Whether `value`, a declaration's of a property other than a custom
/// one, would make an attribute's value a URL (Values 5's attr() taint,
/// M23.3): an `attr()`, or a `var()` of a custom property in `custom`
/// that read one, inside an `image-set()`, whose strings are URLs; or a
/// `var()` of such a property whose value holds a URL. An `attr()`'s
/// own value never parses to a URL: the resolver fails one that does.
pub(crate) fn url_from_attribute(value: &[Token], custom: Option<&Arc<CustomProperties>>) -> bool {
    let tainted = |name: &str| custom.is_some_and(|map| map.is_attr_tainted(name));
    if var_names(value).any(|name| {
        tainted(name)
            && custom
                .and_then(|map| map.get(name))
                .is_some_and(|v| carries_url(v))
    }) {
        return true;
    }
    // How deep the walk is in an `image-set()`, counting the blocks
    // inside it: anything an attribute gives there is a URL's.
    let mut depth = 0usize;
    for (i, token) in value.iter().enumerate() {
        match token {
            Token::Function(name) if depth > 0 => {
                let read = name.eq_ignore_ascii_case("attr")
                    || (name.eq_ignore_ascii_case("var")
                        && var_names(&value[i..]).next().is_some_and(tainted));
                if read {
                    return true;
                }
                depth += 1;
            }
            Token::Function(name)
                if name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set") =>
            {
                depth = 1
            }
            Token::OpenParen | Token::OpenSquare | Token::OpenCurly if depth > 0 => depth += 1,
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly if depth > 0 => depth -= 1,
            _ => {}
        }
    }
    false
}

/// Whether `tokens` hold a URL: a `url()` or a function whose strings
/// are URLs (M23.3).
pub(crate) fn carries_url(tokens: &[Token]) -> bool {
    tokens.iter().any(|token| match token {
        Token::Url { .. } => true,
        Token::Function(name) => ["url", "src", "image", "image-set", "-webkit-image-set"]
            .iter()
            .any(|f| name.eq_ignore_ascii_case(f)),
        _ => false,
    })
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
pub(crate) fn is_custom_ident(name: &str) -> bool {
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

/// How one element's custom property declaration stands in the cascade.
pub(crate) struct Declaration<'t> {
    /// The property's name.
    pub(crate) name: &'t str,
    /// Its value's tokens.
    pub(crate) value: &'t [Token],
    /// Its place in memory, when it is a stylesheet's, which every
    /// element it applies to shares; `None` for an inline style's.
    pub(crate) identity: Option<usize>,
    /// Whether it is an animation's, a keyframe's (M22): its value is
    /// animation-tainted.
    pub(crate) animated: bool,
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
    /// Values substituted, by the declaration's place in memory, the
    /// custom properties' and whether its property is animatable: the
    /// same declaration with the same custom properties substitutes to
    /// the same tokens, which the elements that share a map, nearly all
    /// of a page's, then substitute once. The map is held, so its place
    /// is not reused while the entry lives.
    values: RefCell<HashMap<(usize, usize, bool), Kept>>,
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
    /// precedent first, whose parent's are `parent`, and whose
    /// attributes, or its originating element's, are `attributes`.
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
    /// every element. Declarations that read the element's attributes,
    /// by `attr()`, are never so shared. A map is `None` when no custom
    /// property has a value.
    ///
    /// A value is animation-tainted (Variables 1 §2, M22) when its
    /// declaration is an animation's, when it substituted a tainted
    /// value, or when it is the parent's tainted value inherited.
    pub(crate) fn compute(
        &self,
        declared: &[Declaration<'_>],
        parent: Option<&Arc<CustomProperties>>,
        attributes: &[(String, String)],
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
            } else if is_revert(declaration.value) || !check(declaration.value) {
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
            .map(|(declaration, _)| {
                declaration
                    .identity
                    .filter(|_| !reads_attributes(declaration.value))
            })
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
        let (states, taints): (Vec<State<'_>>, Vec<bool>) = winners
            .iter()
            .map(|(declaration, kind)| match kind {
                Kind::Keyword(CssWide::Initial) => (State::Done(None), false),
                Kind::Keyword(CssWide::Inherit | CssWide::Unset) => (
                    State::Done(inherited.and_then(|m| m.get(declaration.name)).cloned()),
                    inherited.is_some_and(|m| m.is_tainted(declaration.name)),
                ),
                Kind::Literal => (
                    State::Done(Some(self.literal(declaration))),
                    declaration.animated,
                ),
                Kind::Substituted => (State::Pending(declaration.value), declaration.animated),
            })
            .unzip();
        let names = winners.iter().map(|(d, _)| d.name).collect();
        let mut resolver = Resolver::new(names, states, inherited, attributes, self);
        resolver.taints = taints;
        for index in 0..winners.len() {
            if matches!(resolver.states[index], State::Pending(_)) {
                resolver.compute(index);
            }
        }
        let mut map = match parent {
            Some(parent) => CustomProperties::clone(parent),
            None => CustomProperties::default(),
        };
        // Which values read an attribute (M23.3): those whose declaration
        // has an `attr()`, those inherited from one that did, and, to a
        // fixed point, those with a `var()` of one that did.
        let mut attr: Vec<bool> = winners
            .iter()
            .map(|(declaration, kind)| match kind {
                Kind::Keyword(CssWide::Inherit | CssWide::Unset) => {
                    inherited.is_some_and(|m| m.is_attr_tainted(declaration.name))
                }
                Kind::Substituted => reads_attributes(declaration.value),
                Kind::Keyword(CssWide::Initial) | Kind::Literal => false,
            })
            .collect();
        loop {
            let mut grew = false;
            for (index, (declaration, kind)) in winners.iter().enumerate() {
                if attr[index] || !matches!(kind, Kind::Substituted) {
                    continue;
                }
                let reads = var_names(declaration.value).any(|name| match places.get(name) {
                    Some(&other) => attr[other],
                    None => inherited.is_some_and(|m| m.is_attr_tainted(name)),
                });
                if reads {
                    attr[index] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        for ((((declaration, _), state), &tainted), &reads) in winners
            .iter()
            .zip(&resolver.states)
            .zip(&resolver.taints)
            .zip(&attr)
        {
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
            map.set_tainted(declaration.name, tainted && value.is_some());
            map.set_attr_tainted(declaration.name, reads && value.is_some());
        }
        map.declared = fingerprint;
        (!map.is_empty()).then(|| Arc::new(map))
    }

    /// `value`, a property's other than a custom one, with its
    /// substitution functions replaced against the element's custom
    /// properties `custom` and its attributes, or its originating
    /// element's, `attributes`; `identity` is the declaration's, when it
    /// is a stylesheet's. A value that reads no attribute is kept by its
    /// declaration and custom properties, for the next element that has
    /// the same. For a property that is not `animatable`, a `var()` of an
    /// animation-tainted custom property is the guaranteed-invalid value
    /// (Variables 1 §3).
    pub(crate) fn substitute(
        &self,
        value: &[Token],
        identity: Option<usize>,
        custom: Option<&Arc<CustomProperties>>,
        attributes: &[(String, String)],
        animatable: bool,
    ) -> Substituted {
        let key = identity
            .filter(|_| !reads_attributes(value))
            .map(|identity| {
                (
                    identity,
                    custom.map_or(0, |m| Arc::as_ptr(m) as usize),
                    animatable,
                )
            });
        if let Some(key) = key
            && let Some(kept) = self.values.borrow().get(&key)
        {
            return kept.substituted.clone();
        }
        let substituted = if !check(value) {
            Substituted::Ignored
        } else {
            let inherited = custom.map(|m| &**m);
            let mut resolver = Resolver::new(Vec::new(), Vec::new(), inherited, attributes, self);
            resolver.forbid_tainted = !animatable;
            match resolver.substitute(value) {
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
    css_wide_keyword(value).is_some() || (!is_revert(value) && check(value))
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

    /// A number for the stylesheets of one test cascade, never given
    /// before, as the engine numbers each set of stylesheets.
    /// [`Substitutions::new`] takes every declaration with an identity
    /// it meets to be its stylesheets', alive as long as they are; a
    /// test's declarations are freed when it moves on, and their places
    /// may then be another list's, so no two cascades may share a number.
    fn sheets() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
                animated: false,
            })
            .collect();
        substitutions.compute(&declarations, parent, &[])
    }

    fn compute(source: &str) -> Option<Arc<CustomProperties>> {
        compute_with(&Substitutions::new(sheets()), &declared(source), None)
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
        Substitutions::new(sheets()).substitute(
            &css::tokenize(value),
            None,
            custom.as_ref(),
            &[],
            true,
        )
    }

    #[test]
    fn an_animations_values_taint_what_takes_them() {
        // `--a` is an animation's, and the parent's `--p` is tainted:
        // what takes either, through fallbacks or other properties, is
        // tainted, as is `--p` inherited by keyword.
        let substitutions = Substitutions::new(sheets());
        let parent = compute_with(&substitutions, &declared("--p: 1"), None);
        let parent = parent.map(|map| {
            let mut map = CustomProperties::clone(&map);
            map.set_tainted("--p", true);
            Arc::new(map)
        });
        let own = declared(
            "--a: 2s; --b: var(--a); --c: var(--x, 1px) var(--zzz, var(--a)); \
             --d: var(--p); --e: var(--d); --f: var(--x, var(--a)); --p: inherit; \
             --g: var(--x, 1px)",
        );
        let declarations: Vec<Declaration<'_>> = own
            .iter()
            .map(|d| Declaration {
                name: &d.name,
                value: &d.value,
                identity: None,
                animated: d.name == "--a",
            })
            .collect();
        let map = substitutions.compute(&declarations, parent.as_ref(), &[]);
        let map = map.expect("values");
        let tainted: Vec<&str> = ["--a", "--b", "--c", "--d", "--e", "--f", "--g", "--p"]
            .into_iter()
            .filter(|name| map.is_tainted(name))
            .collect();
        // `--c`'s fallbacks are both taken: `var(--zzz, var(--a))` takes
        // `--a`.
        assert_eq!(tainted, ["--a", "--b", "--c", "--d", "--e", "--f", "--p"]);
        // Where a value is the guaranteed-invalid value, there is no taint.
        let none = declared("--a: initial; --b: var(--a)");
        let declarations: Vec<Declaration<'_>> = none
            .iter()
            .map(|d| Declaration {
                name: &d.name,
                value: &d.value,
                identity: None,
                animated: true,
            })
            .collect();
        let empty = substitutions.compute(&declarations, None, &[]);
        assert!(empty.is_none());
        // In a property that is not animatable, a `var()` of a tainted
        // value is the guaranteed-invalid value, its fallback taken.
        let map = Some(map);
        let in_property = |value: &str, animatable: bool| match Substitutions::new(sheets())
            .substitute(&css::tokenize(value), None, map.as_ref(), &[], animatable)
        {
            Substituted::Value(tokens) => Some(text(&tokens)),
            _ => None,
        };
        assert_eq!(in_property("var(--a)", true), Some("2s".into()));
        assert_eq!(in_property("var(--a)", false), None);
        assert_eq!(in_property("var(--a, 3s)", false), Some("3s".into()));
        assert_eq!(in_property("var(--b, 3s)", false), Some("3s".into()));
        assert_eq!(in_property("var(--x, var(--a))", false), None);
        // A map with no taint gives the same either way.
        assert_eq!(
            Substitutions::new(sheets()).substitute(
                &css::tokenize("var(--p)"),
                None,
                compute("--p: 1").as_ref(),
                &[],
                false
            ),
            Substituted::Value(Arc::from(css::tokenize("1").as_slice()))
        );
        // Two maps alike but for a taint differ.
        let plain = compute("--q: 1").expect("a value");
        let mut tainted = CustomProperties::clone(&plain);
        tainted.set_tainted("--q", true);
        assert_ne!(*plain, tainted);
        tainted.set_tainted("--q", false);
        assert_eq!(*plain, tainted);
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
            assert!(check(&css::tokenize(good)), "{good:?} is valid");
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
            assert!(!check(&css::tokenize(bad)), "{bad:?} is invalid");
        }
        // Any other property's value with a substitution function in it
        // is held to the same, as the reference browser holds it: the
        // rest is the property's to parse once substituted.
        assert!(check(&css::tokenize("a (!) [;] var(--a)")));
        for bad in [
            "a ! var(--a)",
            "a ) var(--a)",
            "var(--a) ]",
            "var(--a) !important",
        ] {
            assert!(!check(&css::tokenize(bad)), "{bad:?} is invalid");
        }
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
                let map = compute_with(&Substitutions::new(sheets()), &list, None);
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
            let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
            assert_eq!(value(&map, "--w").as_deref(), expected, "{source}");
        }
    }

    #[test]
    fn inherited_and_keyword_values() {
        let parent = compute("--a: 1; --b: 2; --c: 3");
        let list = declared("--a: initial; --b: inherit; --c: unset; --d: var(--b) var(--c)");
        let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
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
        let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
        assert_eq!(value(&map, "--a"), None);
        assert_eq!(value(&map, "--b").as_deref(), Some("2"));
        assert_eq!(value(&map, "--c").as_deref(), Some("3"));
        assert_eq!(value(&map, "--d"), None, "the parent has none");
        assert_eq!(value(&map, "--e"), None, "--a is guaranteed-invalid");
        let list = declared("--a: var(--nope, inherit); --f: var(--a) x");
        let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
        assert_eq!(value(&map, "--a").as_deref(), Some("1"));
        assert_eq!(value(&map, "--f").as_deref(), Some("1 x"));
        // `revert`, which the cascade does not know (G7), is passed over:
        // the declaration before it stands.
        let list = declared("--a: 4; --a: revert");
        let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
        assert_eq!(value(&map, "--a").as_deref(), Some("4"));
        // Nothing declared, or nothing valid: the parent's map itself.
        let list = declared("--a: !; color: red");
        let map = compute_with(&Substitutions::new(sheets()), &list, parent.as_ref());
        assert!(Arc::ptr_eq(map.as_ref().unwrap(), parent.as_ref().unwrap()));
        // Every property guaranteed-invalid: no map.
        assert_eq!(compute("--a: initial; --b: var(--b)"), None);
    }

    #[test]
    fn the_same_declarations_share_the_parents_map() {
        // As a rule on `*` gives every element the same declarations.
        let substitutions = Substitutions::new(sheets());
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
        let child = compute_with(&Substitutions::new(sheets()), &root, parent.as_ref());
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
                animated: false,
            })
            .collect();
        let child = substitutions.compute(&declarations, parent.as_ref(), &[]);
        assert!(!Arc::ptr_eq(
            parent.as_ref().unwrap(),
            child.as_ref().unwrap()
        ));
        assert_eq!(parent, child);
    }

    #[test]
    fn a_place_is_the_parents_declaration_only_under_its_stylesheets_number() {
        // A declaration is known by its place in memory and its
        // stylesheets' number: under the number the parent's map was
        // computed with, a place of the parent's is the parent's
        // declaration, whatever text is there now, and the map is shared.
        // That is why a place freed and given to another declaration must
        // never come back under the same number. Under another number it
        // is another declaration, and computed.
        fn at(list: &[Declared], place: usize) -> Vec<Declaration<'_>> {
            list.iter()
                .map(|d| Declaration {
                    name: &d.name,
                    value: &d.value,
                    identity: Some(place),
                    animated: false,
                })
                .collect()
        }
        let first = declared("--other: 10px");
        let second = declared("--w: var(var(--unset), 30px)");
        let substitutions = Substitutions::new(sheets());
        let parent = substitutions.compute(&at(&first, 64), None, &[]);
        let same = substitutions.compute(&at(&second, 64), parent.as_ref(), &[]);
        assert!(Arc::ptr_eq(
            same.as_ref().unwrap(),
            parent.as_ref().unwrap()
        ));
        let other = Substitutions::new(sheets()).compute(&at(&second, 64), parent.as_ref(), &[]);
        assert_eq!(value(&other, "--w").as_deref(), Some("30px"));
        assert_eq!(value(&other, "--other").as_deref(), Some("10px"));
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

    /// Attributes from `pairs`, as the cascade lends them.
    fn attributes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn attributes_substitute_by_their_types() {
        // The web-platform-tests' attr-all-types, for what this engine
        // reads.
        let attrs = attributes(&[
            ("title", "a b"),
            ("data-foo", "10"),
            ("data-bar", "3"),
            ("data-baz", "3px"),
            ("data-s", "\"hi\""),
            ("data-calc", "calc(1px + 3px)"),
            ("data-v", "var(--x, 3)"),
            ("data-n", "attr(data-bar, 11)"),
            (
                "data-m",
                "attr(data-bar type(<number>), 11) var(--length, 3px)",
            ),
            ("data-open", "attr("),
        ]);
        let map = compute("--color: red");
        let value = |source: &str| match Substitutions::new(sheets()).substitute(
            &css::tokenize(source),
            None,
            map.as_ref(),
            &attrs,
            true,
        ) {
            Substituted::Value(tokens) => Ok(text(&tokens)),
            other => Err(other),
        };
        let string = |s: &str| Ok(format!("{s:?}"));
        assert_eq!(value("attr(title)"), string("a b"));
        assert_eq!(value("attr(TITLE raw-string)"), string("a b"));
        assert_eq!(value("attr(data-open)"), string("attr("));
        assert_eq!(value("attr(nope)"), string(""));
        assert_eq!(value("attr(nope,)"), Ok("".into()));
        assert_eq!(value("attr(nope, serif)"), Ok("serif".into()));
        assert_eq!(value("attr(nope raw-string)"), string(""));
        assert_eq!(
            value("attr(nope type(<string>))"),
            Err(Substituted::Invalid)
        );
        assert_eq!(value("attr(svg|title)"), string(""));
        assert_eq!(value("attr(|title)"), string("a b"));
        assert_eq!(value("attr(data-foo px)"), Ok("10px".into()));
        assert_eq!(value("attr(data-foo %)"), Ok("10%".into()));
        assert_eq!(value("attr(data-foo number)"), Ok("10".into()));
        assert_eq!(
            value("calc(attr(data-foo px) + 1px)"),
            Ok("calc( 10px + 1px )".into())
        );
        assert_eq!(value("attr(data-baz px)"), Err(Substituted::Invalid));
        assert_eq!(value("attr(data-foo xx, 3px)"), Ok("3px".into()));
        assert_eq!(value("attr(nope px, 3px)"), Ok("3px".into()));
        assert_eq!(value("attr(data-baz type(<length>))"), Ok("3px".into()));
        assert_eq!(
            value("attr(data-calc type(<length>), 42px)"),
            Ok("calc( 1px + 3px )".into())
        );
        assert_eq!(
            value("attr(data-foo type(<length>), red)"),
            Ok("red".into())
        );
        assert_eq!(
            value("attr(data-foo type(<length> | <number>))"),
            Ok("10".into())
        );
        assert_eq!(value("attr(data-s type(<string>))"), string("hi"));
        assert_eq!(value("attr(data-open type(*), anim)"), Ok("anim".into()));
        assert_eq!(value("attr(data-open type(*))"), Err(Substituted::Invalid));
        // An attribute's value, typed, has its own functions substituted.
        assert_eq!(value("attr(data-v type(*))"), Ok("3".into()));
        assert_eq!(value("attr(data-n type(*))"), string("3"));
        assert_eq!(value("attr(data-m type(*))"), Ok("3 3px".into()));
        // The first argument substituted first.
        let map = compute("--name: data-foo px; --type: type(<number>)");
        let value = |source: &str| {
            Substitutions::new(sheets()).substitute(
                &css::tokenize(source),
                None,
                map.as_ref(),
                &attrs,
                true,
            )
        };
        assert_eq!(
            value("attr(var(--name))"),
            Substituted::Value(Arc::from(tokens("10px")))
        );
        assert_eq!(
            value("attr(data-foo var(--type))"),
            Substituted::Value(Arc::from(tokens("10")))
        );
        assert_eq!(value("attr(var(--nope))"), Substituted::Invalid);
        // Where it is parsed, its first argument is checked when nothing is
        // substituted into it: these are ignored, not their fallbacks.
        assert_eq!(
            value("attr(data-foo type(invalid |), 10px)"),
            Substituted::Ignored
        );
        assert_eq!(value("attr(data-foo &, 3px)"), Substituted::Ignored);
        assert_eq!(value("attr(\"x\")"), Substituted::Ignored);
    }

    #[test]
    fn attribute_cycles_fall_back_and_property_cycles_do_not() {
        // The web-platform-tests' attr-cycle.
        let compute_with_attributes = |source: &str, pairs: &[(&str, &str)]| {
            let list = declared(source);
            let attrs = attributes(pairs);
            let declarations: Vec<Declaration<'_>> = list
                .iter()
                .map(|d| Declaration {
                    name: &d.name,
                    value: &d.value,
                    identity: None,
                    animated: false,
                })
                .collect();
            Substitutions::new(sheets()).compute(&declarations, None, &attrs)
        };
        // A custom property, the element's attributes, and its value.
        type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], Option<&'a str>);
        let cases: &[Case<'_>] = &[
            (
                "--x: attr(data-foo type(<length>))",
                &[("data-foo", "attr(data-foo type(<length>))")],
                None,
            ),
            (
                "--y: attr(data-foo type(*))",
                &[("data-foo", "attr(data-foo type(*))")],
                None,
            ),
            (
                "--x: attr(data-foo type(<length>), 3px)",
                &[("data-foo", "var(--x)")],
                None,
            ),
            (
                "--x: attr(data-foo type(<length>), 3px)",
                &[("data-foo", "var(--x, 11px)")],
                None,
            ),
            (
                "--x: attr(data-foo type(<length>), 11px)",
                &[("data-foo", "attr(data-foo type(<length>), 3px)")],
                Some("11px"),
            ),
            (
                "--y: attr(data-foo type(*), 11px)",
                &[("data-foo", "attr(data-foo type(*))")],
                Some("11px"),
            ),
            (
                "--y: attr(data-foo type(*), 1px)",
                &[
                    ("data-foo", "attr(data-bar type(*), 2px)"),
                    ("data-bar", "attr(data-baz type(*), 3px)"),
                    ("data-baz", "attr(data-foo type(*), 4px)"),
                ],
                Some("1px"),
            ),
            (
                "--y: var(--x, 100); --x: attr(data-foo type(*))",
                &[("data-foo", "var(--y)")],
                None,
            ),
        ];
        for (source, pairs, expected) in cases {
            let map = compute_with_attributes(source, pairs);
            let name = if source.starts_with("--y") {
                "--y"
            } else {
                "--x"
            };
            assert_eq!(
                value(&map, name).as_deref(),
                *expected,
                "{source} {pairs:?}"
            );
        }
        // Not a cycle: another attribute's value, or a fallback.
        let map = compute_with_attributes(
            "--y: attr(data-foo type(*), attr(data-bar))",
            &[("data-foo", "attr(data-foo type(*))"), ("data-bar", "abc")],
        );
        assert_eq!(value(&map, "--y").as_deref(), Some("\"abc\""));
    }

    #[test]
    fn declarations_reading_attributes_are_never_shared() {
        let substitutions = Substitutions::new(sheets());
        let list = declared("--a: attr(data-a)");
        let compute = |pairs: &[(&str, &str)], parent: Option<&Arc<CustomProperties>>| {
            let declarations: Vec<Declaration<'_>> = list
                .iter()
                .map(|d| Declaration {
                    name: &d.name,
                    value: &d.value,
                    identity: Some(std::ptr::from_ref(d) as usize),
                    animated: false,
                })
                .collect();
            substitutions.compute(&declarations, parent, &attributes(pairs))
        };
        let parent = compute(&[("data-a", "1")], None);
        let child = compute(&[("data-a", "2")], parent.as_ref());
        assert_eq!(value(&child, "--a").as_deref(), Some("\"2\""));
        assert_eq!(value(&parent, "--a").as_deref(), Some("\"1\""));
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
        let substitutions = Substitutions::new(sheets());
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
        let substitutions = Substitutions::new(sheets());
        let value = css::tokenize("var(--big)");
        let mut substituted = 0;
        while let Substituted::Value(_) =
            substitutions.substitute(&value, None, map.as_ref(), &[], true)
        {
            substituted += 1;
            assert!(
                substituted <= MAX_CASCADE_SUBSTITUTION / one,
                "the bound holds"
            );
        }
        assert_eq!(substituted, MAX_CASCADE_SUBSTITUTION / one);
        let small = compute("--s: 1");
        assert_eq!(
            substitutions.substitute(&css::tokenize("var(--s)"), None, small.as_ref(), &[], true),
            Substituted::Invalid
        );
    }

    #[test]
    fn a_substitution_is_kept_for_the_same_map() {
        let substitutions = Substitutions::new(sheets());
        let map = compute("--a: 1px");
        let value = css::tokenize("var(--a)");
        let identity = Some(std::ptr::from_ref(&value) as usize);
        let first = substitutions.substitute(&value, identity, map.as_ref(), &[], true);
        let Substituted::Value(first) = first else {
            panic!("substitutes")
        };
        let Substituted::Value(again) =
            substitutions.substitute(&value, identity, map.as_ref(), &[], true)
        else {
            panic!("substitutes")
        };
        assert!(Arc::ptr_eq(&first, &again));
        // Another map, another substitution.
        let other = compute("--a: 2px");
        assert_eq!(
            substitutions.substitute(&value, identity, other.as_ref(), &[], true),
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
