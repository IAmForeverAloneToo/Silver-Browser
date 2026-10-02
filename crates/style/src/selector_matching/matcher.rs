//! Selector matching: right-to-left, selectors-4 style.
//!
//! The rightmost compound must match the element; each combinator then
//! walks the DOM (ancestor chain or preceding siblings) to find a
//! context that matches the remaining compounds. Dynamic pseudo-classes
//! (`:hover` etc.) never match until input state exists (M26).
//!
//! The walk is iterative, with an explicit stack of partial matches, so
//! a selector of any length matches in constant stack; and it remembers
//! every `(compound, element)` pair it has found to lead nowhere, so a
//! chain of descendant combinators over a deep tree costs at most
//! compounds times ancestors rather than the exponential retrying a
//! naive backtrack does (`#x .a .a .a span` under forty nested `.a`
//! used to take minutes). A [`Matcher`] holds the caches a pass over
//! one document benefits from, valid while the document does not
//! change: each parent's children's positions, counted once for the
//! structural pseudo-classes, and the `:nth-child(… of S)` ranks per
//! parent, which the cascade would otherwise recount for every element
//! of a long list.
//!
//! Elements are named by their [`NodeId`] in a [`Document`] (M13); an
//! id that names no element matches nothing.
//!
//! A selector that ends in a pseudo-element represents that
//! pseudo-element, never an element: [`Matcher::matches`] says no to
//! it, and [`Matcher::matches_pseudo_element`] matches its compounds
//! against the originating element.
//!
//! Every element is an HTML element in an HTML document (the DOM has no
//! namespaces), so HTML's case rules always apply: an attribute
//! selector without a flag compares the values of the attributes HTML
//! lists ASCII case-insensitively, and every other value identically.
//! Values are compared in place, with nothing allocated.

use std::collections::{HashMap, HashSet};

use dom::{Document, NodeId};

use super::{
    AttrOp, CaseFlag, Combinator, PseudoClass, PseudoElement, Selector, SelectorList,
    SimpleSelector,
};

/// Whether any selector in the list matches the element. A one-off;
/// a pass over many elements wants a [`Matcher`].
pub fn matches_selector_list(document: &Document, element: NodeId, list: &SelectorList) -> bool {
    Matcher::new(document).matches_list(element, list)
}

/// Whether one complex selector matches the element. A one-off; a pass
/// over many elements wants a [`Matcher`].
pub fn matches_selector(document: &Document, element: NodeId, selector: &Selector) -> bool {
    Matcher::new(document).matches(element, selector)
}

/// Where an element stands among its parent's element children, for
/// the structural pseudo-classes: its 0-based index among them and
/// their number, and the same among those of its type.
#[derive(Debug, Clone, Copy)]
struct Position {
    index: u32,
    count: u32,
    type_index: u32,
    type_count: u32,
}

/// Matches selectors against one document, which must not change while
/// the matcher lives (its caches assume so). The selectors it is given
/// must outlive it (`'a`): the `of S` cache below is keyed by where a
/// list lives.
pub struct Matcher<'a> {
    document: &'a Document,
    /// Every element's [`Position`] by slot index, filled a parent's
    /// children at a time the first time one of them is asked about;
    /// empty until a structural pseudo-class is met.
    positions: Vec<Option<Position>>,
    /// `:nth-child(An+B of S)`: for each (parent, S) seen, the rank of
    /// every element child among the siblings that match S, or `None`
    /// for a child that does not, computed once per parent.
    nth_ranks: HashMap<(NodeId, usize), Vec<Option<u32>>>,
}

/// One partial match on the explicit stack: `selector.compounds[k]`
/// matched at `node`, and `next` is the candidate to try next for
/// compound `k - 1` (`None` once they are exhausted).
struct Frame {
    k: usize,
    node: NodeId,
    next: Option<NodeId>,
}

impl<'a> Matcher<'a> {
    /// A matcher over `document`, with empty caches.
    pub fn new(document: &'a Document) -> Self {
        Matcher {
            document,
            positions: Vec::new(),
            nth_ranks: HashMap::new(),
        }
    }

    /// Whether any selector in the list matches the element; nothing
    /// matches a node that is not one.
    pub fn matches_list(&mut self, element: NodeId, list: &'a SelectorList) -> bool {
        self.document.is_element(element) && self.matches_element_list(element, list)
    }

    /// Whether one complex selector matches the element; nothing matches
    /// a node that is not one, and a selector ending in a pseudo-element
    /// matches no element.
    pub fn matches(&mut self, element: NodeId, selector: &'a Selector) -> bool {
        self.document.is_element(element) && self.matches_element(element, selector)
    }

    /// Whether `selector` represents the `pseudo` pseudo-element of
    /// `element`: it ends in that pseudo-element, the pseudo-classes
    /// after it hold, and its compounds match `element` as the
    /// originating element. A `::-webkit-` one never matches.
    pub fn matches_pseudo_element(
        &mut self,
        element: NodeId,
        selector: &'a Selector,
        pseudo: PseudoElement,
    ) -> bool {
        let Some(ending) = &selector.pseudo_element else {
            return false;
        };
        ending.element == pseudo
            && pseudo != PseudoElement::WebkitPrefixed
            && ending.states.iter().all(state_matches)
            && self.document.is_element(element)
            && self.matches_compounds(element, selector)
    }

    /// [`Matcher::matches_list`] for a node the caller knows is an
    /// element, as the cascade does for every one it styles: the check
    /// is not repeated for each of the thousands of selectors it tries.
    pub(crate) fn matches_element_list(&mut self, element: NodeId, list: &'a SelectorList) -> bool {
        list.0.iter().any(|s| self.matches_element(element, s))
    }

    /// [`Matcher::matches`] for a node the caller knows is an element.
    pub(crate) fn matches_element(&mut self, element: NodeId, selector: &'a Selector) -> bool {
        selector.pseudo_element.is_none() && self.matches_compounds(element, selector)
    }

    /// Whether the selector's compounds match the element, its
    /// pseudo-element aside.
    fn matches_compounds(&mut self, element: NodeId, selector: &'a Selector) -> bool {
        let compounds = &selector.compounds;
        let Some(last) = compounds.last() else {
            return false;
        };
        if !self.matches_compound(element, last) {
            return false;
        }
        let n = compounds.len();
        if n == 1 {
            return true;
        }
        // `(k, node)` pairs known to lead nowhere: compound `k` either
        // does not match at `node`, or does and the compounds before it
        // cannot be satisfied from there. Allocated only once something
        // fails, which the common one-candidate selectors never do.
        let mut failed: Option<HashSet<(usize, NodeId)>> = None;
        let mut stack = vec![Frame {
            k: n - 1,
            node: element,
            next: self.first_candidate(selector.combinators[n - 2], element),
        }];
        loop {
            let (target, cand) = {
                let Some(frame) = stack.last_mut() else {
                    return false;
                };
                if frame.k == 0 {
                    return true;
                }
                let Some(cand) = frame.next else {
                    let frame = stack.pop().expect("the stack is not empty");
                    failed
                        .get_or_insert_with(HashSet::new)
                        .insert((frame.k, frame.node));
                    continue;
                };
                let comb = selector.combinators[frame.k - 1];
                frame.next = match comb {
                    Combinator::Child | Combinator::NextSibling => None,
                    Combinator::Descendant => self.document.parent_element(cand),
                    Combinator::LaterSibling => self.document.prev_element_sibling(cand),
                };
                (frame.k - 1, cand)
            };
            if failed.as_ref().is_some_and(|f| f.contains(&(target, cand))) {
                continue;
            }
            if self.matches_compound(cand, &compounds[target]) {
                let next = if target == 0 {
                    None
                } else {
                    self.first_candidate(selector.combinators[target - 1], cand)
                };
                stack.push(Frame {
                    k: target,
                    node: cand,
                    next,
                });
            } else {
                failed
                    .get_or_insert_with(HashSet::new)
                    .insert((target, cand));
            }
        }
    }

    /// The first element to try for the compound left of `node` across
    /// `comb`: the parent for the ancestor combinators, the previous
    /// element sibling for the sibling ones.
    fn first_candidate(&self, comb: Combinator, node: NodeId) -> Option<NodeId> {
        match comb {
            Combinator::Child | Combinator::Descendant => self.document.parent_element(node),
            Combinator::NextSibling | Combinator::LaterSibling => {
                self.document.prev_element_sibling(node)
            }
        }
    }

    /// Whether every simple selector in the compound matches.
    fn matches_compound(&mut self, element: NodeId, compound: &'a super::CompoundSelector) -> bool {
        compound
            .simple
            .iter()
            .all(|s| self.matches_simple(element, s))
    }

    /// Whether one simple selector matches the element.
    fn matches_simple(&mut self, element: NodeId, simple: &'a SimpleSelector) -> bool {
        let document = self.document;
        match simple {
            SimpleSelector::Type(t) => document.tag(element) == Some(t.as_str()),
            SimpleSelector::Universal => true,
            SimpleSelector::Id(v) => document.attr(element, "id") == Some(v.as_str()),
            // HTML splits `class` on ASCII white space only.
            SimpleSelector::Class(c) => document
                .attr(element, "class")
                .is_some_and(|cls| cls.split_ascii_whitespace().any(|w| w == c)),
            SimpleSelector::Attribute {
                name,
                op,
                value,
                case,
            } => matches_attribute(document, element, name, *op, value, *case),
            SimpleSelector::PseudoClass(pc) => self.matches_pseudo_class(element, pc),
        }
    }

    /// The element's [`Position`] among its parent's element children,
    /// the parent's children all counted the first time one is asked.
    fn position(&mut self, element: NodeId) -> Position {
        if let Some(Some(position)) = self.positions.get(element.index()) {
            return *position;
        }
        let document = self.document;
        if self.positions.len() < document.slot_count() {
            self.positions.resize(document.slot_count(), None);
        }
        let Some(parent) = document.parent(element) else {
            // A detached element stands alone.
            return Position {
                index: 0,
                count: 1,
                type_index: 0,
                type_count: 1,
            };
        };
        let children: Vec<NodeId> = document.element_children(parent).collect();
        let mut of_type: HashMap<&str, u32> = HashMap::new();
        let mut type_indices = Vec::with_capacity(children.len());
        for &child in &children {
            let seen = of_type
                .entry(document.tag(child).unwrap_or(""))
                .or_insert(0);
            type_indices.push(*seen);
            *seen += 1;
        }
        let count = children.len() as u32;
        for (i, (&child, type_index)) in children.iter().zip(type_indices).enumerate() {
            let type_count = of_type[document.tag(child).unwrap_or("")];
            self.positions[child.index()] = Some(Position {
                index: i as u32,
                count,
                type_index,
                type_count,
            });
        }
        self.positions[element.index()].expect("the element is among its parent's children")
    }

    /// Pseudo-class semantics.
    fn matches_pseudo_class(&mut self, element: NodeId, pc: &'a PseudoClass) -> bool {
        let document = self.document;
        match pc {
            PseudoClass::FirstChild => self.position(element).index == 0,
            PseudoClass::LastChild => {
                let p = self.position(element);
                p.index + 1 == p.count
            }
            PseudoClass::OnlyChild => self.position(element).count == 1,
            PseudoClass::FirstOfType => self.position(element).type_index == 0,
            PseudoClass::LastOfType => {
                let p = self.position(element);
                p.type_index + 1 == p.type_count
            }
            PseudoClass::OnlyOfType => self.position(element).type_count == 1,
            // selectors-4: any child, element or text (even white
            // space), makes it non-empty; the document holds no other
            // kind of child.
            PseudoClass::Empty => document.first_child(element).is_none(),
            PseudoClass::Root => document.parent(element) == Some(document.root()),
            PseudoClass::NthChild { a, b, of } => match of {
                None => nth_matches(*a, *b, self.position(element).index),
                // selectors-4 §4: with an `of S` filter, the element must
                // itself match S, and the An+B formula applies to its
                // position *among only the siblings that match S* — not
                // its plain sibling position. ("2n+1 of .x" means "every
                // other .x", not ".x at an odd position among all
                // siblings".)
                Some(list) => self
                    .rank_among(element, list)
                    .is_some_and(|rank| nth_matches(*a, *b, rank)),
            },
            PseudoClass::NthLastChild { a, b } => {
                let p = self.position(element);
                nth_matches(*a, *b, p.count - 1 - p.index)
            }
            PseudoClass::NthOfType { a, b } => {
                nth_matches(*a, *b, self.position(element).type_index)
            }
            PseudoClass::NthLastOfType { a, b } => {
                let p = self.position(element);
                nth_matches(*a, *b, p.type_count - 1 - p.type_index)
            }
            PseudoClass::Not(list) => !self.matches_element_list(element, list),
            PseudoClass::Is(list) | PseudoClass::Where(list) => {
                self.matches_element_list(element, list)
            }
            // Dynamic pseudo-classes never match until input state exists (M26).
            PseudoClass::Hover
            | PseudoClass::Focus
            | PseudoClass::FocusVisible
            | PseudoClass::FocusWithin
            | PseudoClass::Active
            | PseudoClass::Visited
            | PseudoClass::Link => false,
        }
    }

    /// The 0-based rank of `element` among its element siblings that match
    /// `list` (selectors-4's `of S` filter: positions are renumbered
    /// within the filtered set, not the full sibling list), or `None`
    /// when `element` itself does not match. The ranks of a parent's whole
    /// child list are computed in one pass the first time any child
    /// asks, so a list of n items costs n matches, not n squared.
    fn rank_among(&mut self, element: NodeId, list: &'a SelectorList) -> Option<u32> {
        let Some(parent) = self.document.parent(element) else {
            // A detached element is its own only sibling.
            return self.matches_list(element, list).then_some(0);
        };
        let key = (parent, list as *const SelectorList as usize);
        if !self.nth_ranks.contains_key(&key) {
            let children: Vec<NodeId> = self.document.element_children(parent).collect();
            let mut ranks = Vec::with_capacity(children.len());
            let mut rank = 0u32;
            for child in children {
                if self.matches_list(child, list) {
                    ranks.push(Some(rank));
                    rank += 1;
                } else {
                    ranks.push(None);
                }
            }
            self.nth_ranks.insert(key, ranks);
        }
        let position = self.position(element).index as usize;
        self.nth_ranks[&key].get(position).copied().flatten()
    }
}

/// Whether the pseudo-classes after a pseudo-element hold: no
/// pseudo-element is hovered, focused or active until M26.4, so each
/// user-action one is false, and `:is()`, `:where()` and `:not()` of
/// them follow from that.
fn state_matches(state: &PseudoClass) -> bool {
    let any = |list: &SelectorList| {
        list.0.iter().any(|selector| {
            selector.compounds.iter().all(|compound| {
                compound.simple.iter().all(|simple| match simple {
                    SimpleSelector::PseudoClass(state) => state_matches(state),
                    _ => false,
                })
            })
        })
    };
    match state {
        PseudoClass::Not(list) => !any(list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => any(list),
        _ => false,
    }
}

/// The attributes whose values HTML's attribute selectors compare ASCII
/// case-insensitively when no flag says otherwise (HTML §4.16.2,
/// "Case-sensitivity of selectors"), sorted for the binary search.
const HTML_CASE_INSENSITIVE_VALUES: [&str; 46] = [
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

/// Whether HTML compares the values of the attribute `name` (in ASCII
/// lowercase, as the parser leaves it) ASCII case-insensitively.
fn html_value_is_case_insensitive(name: &str) -> bool {
    HTML_CASE_INSENSITIVE_VALUES.binary_search(&name).is_ok()
}

/// Attribute selector semantics (selectors-4 §6.1, §6.3).
fn matches_attribute(
    document: &Document,
    element: NodeId,
    name: &str,
    op: AttrOp,
    value: &str,
    case: CaseFlag,
) -> bool {
    let Some(actual) = document.attr(element, name) else {
        return false;
    };
    let fold = match case {
        CaseFlag::I => true,
        CaseFlag::S => false,
        CaseFlag::Unset => html_value_is_case_insensitive(name),
    };
    let compare = Compare { fold };
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => compare.eq(actual, value),
        // `~=` matches words separated by ASCII white space, as HTML
        // splits them; an empty value never matches, nor one holding
        // white space, which no word holds.
        AttrOp::IncludeSpace => {
            !value.is_empty()
                && actual
                    .split_ascii_whitespace()
                    .any(|word| compare.eq(word, value))
        }
        // `|=` is the value itself or a `value-` prefix.
        AttrOp::Dash => {
            compare.eq(actual, value)
                || (actual.as_bytes().get(value.len()) == Some(&b'-')
                    && compare.starts_with(actual, value))
        }
        // selectors-4 §6.2: an empty value "does not represent anything"
        // for the three substring operators.
        AttrOp::Prefix => !value.is_empty() && compare.starts_with(actual, value),
        AttrOp::Suffix => !value.is_empty() && compare.ends_with(actual, value),
        AttrOp::Substring => !value.is_empty() && compare.contains(actual, value),
    }
}

/// Compares strings identically, or with ASCII letters folded when
/// `fold`, in place. Folding works byte by byte: it changes only ASCII
/// bytes, which UTF-8 never uses inside a multi-byte character, so a
/// match found on bytes is one on characters.
#[derive(Clone, Copy)]
struct Compare {
    fold: bool,
}

impl Compare {
    fn eq(self, a: &str, b: &str) -> bool {
        self.eq_bytes(a.as_bytes(), b.as_bytes())
    }

    fn starts_with(self, haystack: &str, needle: &str) -> bool {
        haystack
            .as_bytes()
            .get(..needle.len())
            .is_some_and(|head| self.eq_bytes(head, needle.as_bytes()))
    }

    fn ends_with(self, haystack: &str, needle: &str) -> bool {
        haystack
            .len()
            .checked_sub(needle.len())
            .is_some_and(|start| self.eq_bytes(&haystack.as_bytes()[start..], needle.as_bytes()))
    }

    fn contains(self, haystack: &str, needle: &str) -> bool {
        if !self.fold {
            return haystack.contains(needle);
        }
        if needle.is_empty() {
            return true;
        }
        haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
    }

    fn eq_bytes(self, a: &[u8], b: &[u8]) -> bool {
        if self.fold {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }
}

/// Whether the CSS `An+B` formula matches: the element's 1-indexed sibling
/// *position* equals `a*n + b` for some integer `n >= 0` (selectors-4 §4).
/// `zero_based_index` is [`Position`]'s 0-based counter, so it's
/// converted to a 1-indexed position before applying the formula.
fn nth_matches(a: i32, b: i32, zero_based_index: u32) -> bool {
    let position = zero_based_index as i64 + 1;
    let a = a as i64;
    let b = b as i64;
    if a == 0 {
        return position == b;
    }
    let diff = position - b;
    diff % a == 0 && diff / a >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_selector_list;
    use crate::selector_matching::PseudoElement;
    use dom::{Element, Node};

    /// The tags of all elements the selector matches, in document order.
    fn matched_tags(document: &Node, selector: &str) -> Vec<String> {
        let document = Document::from_node(document);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        let mut out = Vec::new();
        for id in elements(&document) {
            if matches_selector_list(&document, id, &list) {
                out.push(document.tag(id).unwrap().to_string());
            }
        }
        out
    }

    /// The document's elements in tree order.
    fn elements(document: &Document) -> Vec<NodeId> {
        document
            .subtree(document.root())
            .filter(|&id| document.is_element(id))
            .collect()
    }

    fn p(children: Vec<Node>) -> Node {
        Node::document(children)
    }

    fn e(tag: &str) -> Element {
        Element::new(tag)
    }

    // ---- simple selectors -------------------------------------------------

    #[test]
    fn type_matches_that_tag() {
        let doc = p(vec![e("div").push(e("p")).into()]);
        assert_eq!(matched_tags(&doc, "p"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "span"), Vec::<String>::new());
    }

    #[test]
    fn type_matching_is_case_insensitive() {
        let doc = p(vec![e("div").into()]);
        assert_eq!(matched_tags(&doc, "DIV"), vec!["div"]);
    }

    #[test]
    fn universal_matches_everything() {
        let doc = p(vec![e("div").push(e("p").push(e("b"))).into()]);
        assert_eq!(matched_tags(&doc, "*").len(), 3);
    }

    #[test]
    fn class_matches_whitespace_list() {
        let doc = p(vec![
            e("p").attr("class", "a b c").into(),
            e("p").attr("class", "ab").into(),
            e("p").attr("class", "xbx").into(),
            e("p").into(),
        ]);
        assert_eq!(matched_tags(&doc, ".b"), vec!["p"]);
    }

    #[test]
    fn id_matches() {
        let doc = p(vec![e("p").attr("id", "main").into(), e("p").into()]);
        assert_eq!(matched_tags(&doc, "#main"), vec!["p"]);
    }

    #[test]
    fn compound_requires_all_parts() {
        let doc = p(vec![
            e("div").attr("class", "a").into(),
            e("p").attr("class", "a").into(),
            e("div").attr("class", "b").into(),
        ]);
        assert_eq!(matched_tags(&doc, "div.a"), vec!["div"]);
    }

    // ---- combinators -------------------------------------------------------

    #[test]
    fn descendant_matches_any_depth() {
        let doc = p(vec![e("div").push(e("section").push(e("p"))).into()]);
        assert_eq!(matched_tags(&doc, "div p"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "section p"), vec!["p"]);
    }

    #[test]
    fn descendant_excludes_same_level() {
        let doc = p(vec![e("div").into(), e("p").into()]);
        assert_eq!(matched_tags(&doc, "div p"), Vec::<String>::new());
    }

    #[test]
    fn child_matches_only_direct() {
        let doc = p(vec![
            e("div").push(e("section").push(e("p"))).push(e("p")).into(),
        ]);
        assert_eq!(matched_tags(&doc, "div > p"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "div > section"), vec!["section"]);
        assert_eq!(matched_tags(&doc, "div > b"), Vec::<String>::new());
    }

    #[test]
    fn next_sibling_skips_text_nodes() {
        // a, b, c as siblings (not nested — `+` needs actual siblings),
        // with text between them: it must not break the combinator.
        let doc = p(vec![
            e("a").into(),
            Node::text(" "),
            e("b").into(),
            Node::text(" "),
            e("c").into(),
        ]);
        assert_eq!(matched_tags(&doc, "a + b"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "b + c"), vec!["c"]);
        assert_eq!(matched_tags(&doc, "a + c"), Vec::<String>::new());
    }

    #[test]
    fn later_sibling_matches_any_preceding() {
        // a, b, c, c as siblings (not nested — `~` needs actual siblings).
        let doc = p(vec![
            e("a").into(),
            e("b").into(),
            e("c").into(),
            e("c").into(),
        ]);
        assert_eq!(matched_tags(&doc, "a ~ c"), vec!["c", "c"]);
        assert_eq!(matched_tags(&doc, "c ~ a"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "c ~ c"), vec!["c"]); // the second c
    }

    #[test]
    fn deep_right_to_left_chain() {
        let doc = p(vec![
            e("div")
                .attr("class", "a")
                .push(e("section").push(e("div").attr("class", "b").push(e("span"))))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, ".a .b span"), vec!["span"]);
        assert_eq!(matched_tags(&doc, ".a > .b"), Vec::<String>::new());
    }

    #[test]
    fn sibling_then_ancestor_combination() {
        let doc = p(vec![
            e("ul")
                .push(e("li").attr("class", "first"))
                .push(e("li"))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, "ul .first + li"), vec!["li"]);
    }

    // ---- attributes ---------------------------------------------------------

    #[test]
    fn attribute_exists() {
        let doc = p(vec![
            e("input").attr("disabled", "").into(),
            e("input").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[disabled]"), vec!["input"]);
    }

    #[test]
    fn attribute_equals() {
        let doc = p(vec![
            e("a").attr("href", "https://x").into(),
            e("a").attr("href", "http://y").into(),
        ]);
        assert_eq!(matched_tags(&doc, r#"[href="https://x"]"#), vec!["a"]);
    }

    #[test]
    fn attribute_equals_is_case_sensitive_by_default() {
        let doc = p(vec![e("a").attr("href", "Case").into()]);
        assert_eq!(matched_tags(&doc, r#"[href="case"]"#), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, r#"[href="case" i]"#), vec!["a"]);
        assert_eq!(matched_tags(&doc, r#"[href="Case" s]"#), vec!["a"]);
    }

    #[test]
    fn attribute_include_space_matches_words() {
        let doc = p(vec![
            e("p").attr("class", "word another").into(),
            e("p").attr("class", "wordy").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[class~=word]"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "[class~=wordy]"), vec!["p"]);
    }

    #[test]
    fn attribute_dash() {
        let doc = p(vec![
            e("p").attr("lang", "en").into(),
            e("p").attr("lang", "en-US").into(),
            e("p").attr("lang", "enx").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[lang|=en]"), vec!["p", "p"]);
    }

    #[test]
    fn attribute_substring_operators() {
        let doc = p(vec![e("a").attr("href", "https://example.com/page").into()]);
        assert_eq!(matched_tags(&doc, "[href^='https://']"), vec!["a"]);
        assert_eq!(matched_tags(&doc, "[href$='.com/page']"), vec!["a"]);
        assert_eq!(matched_tags(&doc, "[href*='example']"), vec!["a"]);
        assert_eq!(
            matched_tags(&doc, "[href^='http://']"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn missing_attribute_never_matches() {
        let doc = p(vec![e("p").into()]);
        assert_eq!(matched_tags(&doc, "[a=b]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[a]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[a~='']"), Vec::<String>::new());
    }

    // ---- structural pseudo-classes -------------------------------------------

    #[test]
    fn child_position_pseudo_classes() {
        // <div><a/><b/><b/></div> and a lone <p/>
        let doc = p(vec![
            e("div").push(e("a")).push(e("b")).push(e("b")).into(),
            e("p").into(),
        ]);
        assert_eq!(matched_tags(&doc, ":first-child"), vec!["div", "a"]);
        // div is first (not last) among the top-level siblings, so `p` —
        // not `div` — is the one that's also a last-child.
        assert_eq!(matched_tags(&doc, ":last-child"), vec!["b", "p"]);
        // The lone p (top level, next to div): not only-child; div is first.
        assert_eq!(matched_tags(&doc, ":only-child"), Vec::<String>::new());
    }

    #[test]
    fn only_child_when_solitary() {
        let doc = p(vec![e("div").push(e("p")).into()]);
        assert_eq!(matched_tags(&doc, "p:only-child"), vec!["p"]);
    }

    #[test]
    fn of_type_pseudo_classes() {
        // <div><a/><a/><b/></div>
        let doc = p(vec![e("div").push(e("a")).push(e("a")).push(e("b")).into()]);
        assert_eq!(matched_tags(&doc, "b:first-of-type"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "a:last-of-type"), vec!["a"]); // the second a
        assert_eq!(matched_tags(&doc, "b:only-of-type"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "a:only-of-type"), Vec::<String>::new());
    }

    #[test]
    fn empty_element_rules() {
        let doc = p(vec![
            e("div").into(),              // no children: empty
            e("div").text("   ").into(),  // ws text: NOT empty
            e("div").push(e("p")).into(), // element child: NOT empty
        ]);
        assert_eq!(matched_tags(&doc, "div:empty"), vec!["div"]);
    }

    #[test]
    fn root_is_the_document_element() {
        let doc = p(vec![
            e("html")
                .push(e("head"))
                .push(e("body").push(e("p")))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, ":root"), vec!["html"]);
        assert_eq!(matched_tags(&doc, "head:root"), Vec::<String>::new());
    }

    // ---- nth -----------------------------------------------------------------

    #[test]
    fn nth_child_forms() {
        // Six <li/> in a <ul/>. Selectors are scoped to `li:` — a bare
        // `:nth-child(...)` would also test the `<ul>` itself, which is
        // (trivially) its own parent's first-and-only, hence odd-position,
        // child.
        let items: Vec<Node> = (0..6).map(|_| e("li").into()).collect();
        let doc = p(vec![e("ul").push_children(items).into()]);
        let count = |sel: &str| matched_tags(&doc, sel).len();
        assert_eq!(count("li:nth-child(odd)"), 3);
        assert_eq!(count("li:nth-child(even)"), 3);
        assert_eq!(count("li:nth-child(3)"), 1);
        assert_eq!(count("li:nth-child(2n+1)"), 3);
        assert_eq!(count("li:nth-child(-n+3)"), 3);
        // Positions 4, 5, 6 (1-indexed) of 6 — three, not two.
        assert_eq!(count("li:nth-child(n+4)"), 3);
        assert_eq!(count("li:nth-child(0n+2)"), 1);
    }

    #[test]
    fn nth_last_child_counts_from_the_end() {
        // Scoped to `li:` for the same reason as `nth_child_forms` above.
        let items: Vec<Node> = (0..5).map(|_| e("li").into()).collect();
        let doc = p(vec![e("ul").push_children(items).into()]);
        assert_eq!(matched_tags(&doc, "li:nth-last-child(1)").len(), 1);
        assert_eq!(matched_tags(&doc, "li:nth-last-child(-n+2)").len(), 2);
    }

    #[test]
    fn nth_of_type_ignores_other_tags() {
        // <div><a/><b/><a/><b/></div>: a-indices 0,1; b-indices 0,1.
        let doc = p(vec![
            e("div")
                .push(e("a"))
                .push(e("b"))
                .push(e("a"))
                .push(e("b"))
                .into(),
        ]);
        // The second <a/> is a:nth-of-type(2) even though it's the 3rd child.
        assert_eq!(matched_tags(&doc, "a:nth-of-type(2)").len(), 1);
        assert_eq!(matched_tags(&doc, "a:nth-child(2)").len(), 0);
        assert_eq!(matched_tags(&doc, "b:nth-last-of-type(2n+1)").len(), 1);
    }

    #[test]
    fn nth_child_of_filter() {
        // <ul><li class=x/><li/><li class=x/><li/><li class=x/></ul>
        let doc = p(vec![
            e("ul")
                .push(e("li").attr("class", "x"))
                .push(e("li"))
                .push(e("li").attr("class", "x"))
                .push(e("li"))
                .push(e("li").attr("class", "x"))
                .into(),
        ]);
        // .x items are at of-list positions 0,1,2.
        assert_eq!(matched_tags(&doc, ":nth-child(odd of .x)").len(), 2);
        assert_eq!(matched_tags(&doc, ":nth-child(2 of .x)").len(), 1);
    }

    // ---- logical combinators ---------------------------------------------------

    #[test]
    fn not_negates() {
        let doc = p(vec![e("p").attr("class", "a").into(), e("p").into()]);
        assert_eq!(matched_tags(&doc, "p:not(.a)"), vec!["p"]);
        assert_eq!(matched_tags(&doc, ":not(p)"), Vec::<String>::new());
    }

    #[test]
    fn not_with_complex_selector_inside() {
        // <div><p/></div><p/>
        let doc = p(vec![e("div").push(e("p")).into(), e("p").into()]);
        assert_eq!(matched_tags(&doc, "p:not(div p)"), vec!["p"]);
    }

    #[test]
    fn is_and_where_match_any_argument() {
        let doc = p(vec![e("a").into(), e("b").into(), e("c").into()]);
        assert_eq!(matched_tags(&doc, ":is(a, c)").len(), 2);
        assert_eq!(matched_tags(&doc, ":where(a, c)").len(), 2);
    }

    #[test]
    fn nth_inside_not() {
        let items: Vec<Node> = (0..4).map(|_| e("li").into()).collect();
        let doc = p(vec![e("ul").push_children(items).into()]);
        assert_eq!(matched_tags(&doc, "li:not(:nth-child(odd))").len(), 2);
    }

    #[test]
    fn is_accepts_complex_arguments() {
        let doc = p(vec![e("div").push(e("p")).into(), e("p").into()]);
        assert_eq!(matched_tags(&doc, ":is(div p)").len(), 1);
    }

    // ---- dynamic + lists --------------------------------------------------------

    #[test]
    fn dynamic_pseudo_classes_never_match_yet() {
        let doc = p(vec![e("a").attr("href", "x").into()]);
        for sel in [
            ":hover",
            ":focus",
            ":focus-visible",
            ":focus-within",
            ":active",
            ":visited",
            ":link",
        ] {
            assert_eq!(
                matched_tags(&doc, sel),
                Vec::<String>::new(),
                "{sel} must not match before M26"
            );
        }
    }

    #[test]
    fn selector_lists_match_any() {
        let doc = p(vec![e("div").push(e("p")).into(), e("span").into()]);
        assert_eq!(matched_tags(&doc, "p, span"), vec!["p", "span"]);
        assert_eq!(matched_tags(&doc, "b, i"), Vec::<String>::new());
    }

    // ---- additional regression coverage --------------------------------------
    //
    // The bugs the matcher shipped with (see docs/architecture.md M4 note)
    // were all masked by count-only assertions on evenly-split fixtures;
    // these check specific elements/positions, and exercise the `of S`
    // rewrite and the wrapping-container pitfall directly.

    #[test]
    fn nth_child_negative_a_only_matches_nothing_positive() {
        // a=-2,b=0: position = -2n for n>=0, i.e. {0,-2,-4,...} — never a
        // real (>=1) 1-indexed position, so nothing matches.
        let items: Vec<Node> = (0..4).map(|_| e("li").into()).collect();
        let doc = p(vec![e("ul").push_children(items).into()]);
        assert_eq!(
            matched_tags(&doc, "li:nth-child(-2n)"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn nth_child_of_filter_renumbers_not_reindexes() {
        // <ul><li/><li class=x/><li/><li class=x/><li class=x/></ul>
        // .x items sit at original positions 2, 4, 5 but of-list ranks
        // 0, 1, 2 — `nth-child(2 of .x)` must pick the of-list's 2nd (the
        // li at original position 4), not the original position 2.
        let doc = p(vec![
            e("ul")
                .push(e("li"))
                .push(e("li").attr("class", "x").attr("data-mark", "first"))
                .push(e("li"))
                .push(e("li").attr("class", "x").attr("data-mark", "second"))
                .push(e("li").attr("class", "x").attr("data-mark", "third"))
                .into(),
        ]);
        let document = Document::from_node(&doc);
        let list = parse_selector_list(&css::tokenize(":nth-child(2 of .x)")).unwrap();
        let marks: Vec<&str> = elements(&document)
            .into_iter()
            .filter(|&i| matches_selector_list(&document, i, &list))
            .filter_map(|i| document.attr(i, "data-mark"))
            .collect();
        assert_eq!(marks, vec!["second"]);
    }

    #[test]
    fn nth_child_of_filter_excludes_non_matching_wrapper() {
        // A wrapping element that doesn't match the `of` filter (here,
        // the <ul> itself isn't `.x`) must never contribute a spurious
        // match, regardless of its own sibling position.
        let doc = p(vec![
            e("ul")
                .push(e("li").attr("class", "x"))
                .push(e("li").attr("class", "x"))
                .into(),
        ]);
        assert_eq!(
            matched_tags(&doc, ":nth-child(odd of .x)"),
            vec!["li"] // just the first .x; not "ul" too
        );
    }

    #[test]
    fn bare_nth_child_on_sole_top_level_element_matches() {
        // A tag-agnostic `:nth-child(odd)` legitimately matches the sole
        // top-level element too (position 1 is odd) — this isn't a bug,
        // it's exactly why `nth_child_forms` above scopes to `li:`.
        let doc = p(vec![e("ul").push(e("li")).into()]);
        assert_eq!(matched_tags(&doc, ":nth-child(odd)"), vec!["ul", "li"]);
    }

    #[test]
    fn not_combined_with_structural_pseudo_class() {
        let doc = p(vec![
            e("ul")
                .push(e("li").attr("class", "skip"))
                .push(e("li"))
                .push(e("li"))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, "li:not(:first-child)"), vec!["li", "li"]);
        assert_eq!(matched_tags(&doc, "li:not(.skip)"), vec!["li", "li"]);
    }

    #[test]
    fn multiple_classes_all_required_on_matcher_side() {
        let doc = p(vec![
            e("p").attr("class", "a b c").into(),
            e("p").attr("class", "a b").into(),
        ]);
        assert_eq!(matched_tags(&doc, ".a.b.c"), vec!["p"]);
    }

    #[test]
    fn attribute_dash_case_insensitive_flag() {
        let doc = p(vec![e("p").attr("data-lang", "EN-us").into()]);
        assert_eq!(matched_tags(&doc, "[data-lang|=en]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[data-lang|=en i]"), vec!["p"]);
    }

    #[test]
    fn deeply_nested_descendant_chain_five_levels() {
        let doc = p(vec![
            e("a")
                .push(e("b").push(e("c").push(e("d").push(e("e")))))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, "a e"), vec!["e"]);
        assert_eq!(matched_tags(&doc, "a b c d e"), vec!["e"]);
        assert_eq!(matched_tags(&doc, "a > e"), Vec::<String>::new());
    }

    #[test]
    fn child_combinator_chain_with_attribute_and_next_sibling() {
        let doc = p(vec![
            e("form")
                .push(e("input").attr("required", ""))
                .push(e("button"))
                .into(),
        ]);
        assert_eq!(
            matched_tags(&doc, "form > input[required] + button"),
            vec!["button"]
        );
    }

    #[test]
    fn where_matches_like_is_despite_zero_specificity() {
        // Zero specificity (checked in parse.rs) must not mean "never
        // matches" — matching and specificity are independent.
        let doc = p(vec![e("p").attr("id", "x").into(), e("span").into()]);
        assert_eq!(matched_tags(&doc, ":where(#x, span)"), vec!["p", "span"]);
    }

    #[test]
    fn later_sibling_skips_multiple_non_matching_siblings() {
        let doc = p(vec![
            e("a").into(),
            e("b").into(),
            e("c").into(),
            e("d").into(),
        ]);
        assert_eq!(matched_tags(&doc, "a ~ d"), vec!["d"]);
    }

    #[test]
    fn is_and_where_do_not_match_when_no_argument_matches() {
        let doc = p(vec![e("p").into()]);
        assert_eq!(matched_tags(&doc, ":is(div, span)"), Vec::<String>::new());
        assert_eq!(
            matched_tags(&doc, ":where(div, span)"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn nth_last_of_type_combined_with_type_and_class() {
        // <div><a class=x/><b/><a/><b/></div>: two <a>s, class filter
        // plus nth-last-of-type must both hold.
        let doc = p(vec![
            e("div")
                .push(e("a").attr("class", "x"))
                .push(e("b"))
                .push(e("a"))
                .push(e("b"))
                .into(),
        ]);
        assert_eq!(matched_tags(&doc, "a.x:nth-last-of-type(2)"), vec!["a"]);
        assert_eq!(
            matched_tags(&doc, "a.x:nth-last-of-type(1)"),
            Vec::<String>::new()
        );
    }

    // ---- hostile input and the review's findings ---------------------------

    #[test]
    fn empty_substring_values_match_nothing() {
        // selectors-4 §6.2.
        let doc = p(vec![e("a").attr("href", "x").into(), e("a").into()]);
        assert_eq!(matched_tags(&doc, "[href^='']"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[href$='']"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[href*='']"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[href='']"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[href]"), vec!["a"]);
    }

    #[test]
    fn descendant_chains_do_not_backtrack_exponentially() {
        // Forty nested `.a` and a selector of twenty `.a` compounds that
        // can never match (the `#nope` head): a naive right-to-left walk
        // retries every ancestor for every compound.
        let mut inner: Node = e("span").into();
        for _ in 0..40 {
            inner = e("div").attr("class", "a").push(inner).into();
        }
        let doc = p(vec![inner]);
        let selector = format!("#nope {} span", ".a ".repeat(20));
        assert_eq!(matched_tags(&doc, &selector), Vec::<String>::new());
        let selector = format!("div {} span", ".a ".repeat(20));
        assert_eq!(matched_tags(&doc, &selector), vec!["span"]);
    }

    #[test]
    fn a_selector_with_very_many_compounds_matches_in_constant_stack() {
        // 20k `div > div > ... > a` against 20k nested divs, on a small
        // stack: the walk used to recurse once per compound. (A sibling
        // chain that long is quadratic by nature, so it is kept short.)
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut inner: Node = e("a").into();
                for _ in 0..20_000 {
                    inner = e("div").push(inner).into();
                }
                let doc = p(vec![inner]);
                let document = Document::from_node(&doc);
                let last = *elements(&document).last().unwrap();
                let selector = vec!["div"; 20_000].join(" > ") + " > a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&document, last, &list));
                let selector = vec!["div"; 20_001].join(" > ") + " > a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(!matches_selector_list(&document, last, &list));
                let selector = vec!["div"; 20_000].join(" ") + " a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&document, last, &list));

                let siblings: Vec<Node> = (0..300).map(|_| e("a").into()).collect();
                let doc = p(vec![e("div").push_children(siblings).into()]);
                let document = Document::from_node(&doc);
                let all = elements(&document);
                let (last, before) = (all[all.len() - 1], all[all.len() - 2]);
                let selector = vec!["a"; 300].join(" ~ ");
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&document, last, &list));
                assert!(!matches_selector_list(&document, before, &list));
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn a_text_node_matches_nothing_and_a_detached_element_stands_alone() {
        let mut document = Document::from_node(&p(vec![e("div").text("x").into()]));
        let div = document.document_element().unwrap();
        let text = document.first_child(div).unwrap();
        let any = parse_selector_list(&css::tokenize("*, :not(p), :empty")).unwrap();
        assert!(!matches_selector_list(&document, text, &any));
        let stale = document.create_element("p").unwrap();
        document.free(stale).unwrap();
        assert!(!matches_selector_list(&document, stale, &any), "a stale id");
        // A detached element stands alone: the first, last and only
        // child, and the only one of its type.
        let loose = document.create_element("li").unwrap();
        let alone = parse_selector_list(&css::tokenize(
            "li:first-child:last-child:only-child:only-of-type:nth-child(1):nth-last-child(1):nth-child(1 of li)",
        ))
        .unwrap();
        assert!(matches_selector_list(&document, loose, &alone));
        let root = parse_selector_list(&css::tokenize(":root")).unwrap();
        assert!(!matches_selector_list(&document, loose, &root));
        assert!(matches_selector_list(&document, div, &root));
    }

    #[test]
    fn nth_child_of_s_over_many_siblings_is_linear() {
        // 20k siblings with `:nth-child(odd of .x)` used to rescan the
        // whole sibling list for each one.
        let siblings: Vec<Node> = (0..20_000)
            .map(|i| {
                e("li")
                    .attr("class", if i % 3 == 0 { "x" } else { "y" })
                    .into()
            })
            .collect();
        let doc = p(vec![e("ul").push_children(siblings).into()]);
        let document = Document::from_node(&doc);
        let all = elements(&document);
        let list = parse_selector_list(&css::tokenize("li:nth-child(odd of .x)")).expect("valid");
        let mut matcher = Matcher::new(&document);
        let count = all
            .iter()
            .filter(|&&i| matcher.matches_list(i, &list))
            .count();
        // 6667 `.x` items, every other one.
        assert_eq!(count, 3334);
        let nested =
            parse_selector_list(&css::tokenize("li:nth-child(odd of :nth-child(odd of .x))"))
                .expect("valid");
        let mut matcher = Matcher::new(&document);
        let count = all
            .iter()
            .filter(|&&i| matcher.matches_list(i, &nested))
            .count();
        assert_eq!(count, 1667);
    }
    // ---- M16: pseudo-elements, HTML's case rule, in-place comparison -------

    #[test]
    fn a_selector_ending_in_a_pseudo_element_matches_no_element() {
        let doc = p(vec![e("p").into(), e("div").into()]);
        for sel in [
            "p::before",
            "::after",
            "p:before",
            "::-webkit-scrollbar",
            "*::marker",
        ] {
            assert_eq!(matched_tags(&doc, sel), Vec::<String>::new(), "{sel}");
        }
        // Its list's other selectors still match.
        assert_eq!(matched_tags(&doc, "p, div::before"), vec!["p"]);
    }

    #[test]
    fn a_pseudo_element_selector_matches_its_originating_element() {
        // <div class=a><p/></div><p/>
        let doc = p(vec![
            e("div").attr("class", "a").push(e("p")).into(),
            e("p").into(),
        ]);
        let document = Document::from_node(&doc);
        let all = elements(&document);
        let originating = |sel: &str, pseudo: PseudoElement| -> Vec<usize> {
            let list = parse_selector_list(&css::tokenize(sel)).expect("valid selector");
            let mut matcher = Matcher::new(&document);
            all.iter()
                .enumerate()
                .filter(|&(_, &id)| matcher.matches_pseudo_element(id, &list.0[0], pseudo))
                .map(|(i, _)| i)
                .collect()
        };
        // Elements in tree order: div, the inner p, the outer p.
        assert_eq!(originating(".a p::before", PseudoElement::Before), vec![1]);
        assert_eq!(originating(".a p:before", PseudoElement::Before), vec![1]);
        assert_eq!(
            originating(".a p::before", PseudoElement::After),
            Vec::<usize>::new()
        );
        assert_eq!(originating("p::after", PseudoElement::After), vec![1, 2]);
        assert_eq!(
            originating("::marker", PseudoElement::Marker),
            vec![0, 1, 2]
        );
        assert_eq!(
            originating("::first-line", PseudoElement::FirstLine),
            vec![0, 1, 2]
        );
        // A selector with no pseudo-element represents no pseudo-element.
        assert_eq!(originating("p", PseudoElement::Before), Vec::<usize>::new());
        // `::-webkit-` ones match nothing, even asked for by name.
        assert_eq!(
            originating("::-webkit-scrollbar", PseudoElement::WebkitPrefixed),
            Vec::<usize>::new()
        );
        // No pseudo-element is in a user-action state yet: `:hover` after
        // one never holds, and `:not(:hover)` always does.
        assert_eq!(
            originating("p::before:hover", PseudoElement::Before),
            Vec::<usize>::new()
        );
        assert_eq!(
            originating("p::before:is(:hover, :focus)", PseudoElement::Before),
            Vec::<usize>::new()
        );
        assert_eq!(
            originating("p::before:is(.a)", PseudoElement::Before),
            Vec::<usize>::new()
        );
        assert_eq!(
            originating("p::before:not(:hover)", PseudoElement::Before),
            vec![1, 2]
        );
        assert_eq!(
            originating(
                "p::before:not(:is(:hover)):where(:not(:active))",
                PseudoElement::Before
            ),
            vec![1, 2]
        );
        assert_eq!(
            originating("p::before:not(:not(:hover))", PseudoElement::Before),
            Vec::<usize>::new()
        );
        // A text node originates nothing.
        let list = parse_selector_list(&css::tokenize("::before")).unwrap();
        let text = Document::from_node(&p(vec![Node::text("x")]));
        let node = text.first_child(text.root()).unwrap();
        assert!(!Matcher::new(&text).matches_pseudo_element(
            node,
            &list.0[0],
            PseudoElement::Before
        ));
    }

    #[test]
    fn html_lists_the_attributes_whose_values_ignore_ascii_case() {
        // HTML §4.16.2: forty-six names, sorted for the binary search.
        assert!(HTML_CASE_INSENSITIVE_VALUES.windows(2).all(|w| w[0] < w[1]));
        for name in HTML_CASE_INSENSITIVE_VALUES {
            assert!(html_value_is_case_insensitive(name), "{name}");
            assert_eq!(name, name.to_ascii_lowercase());
        }
        for name in [
            "href",
            "class",
            "id",
            "title",
            "data-type",
            "value",
            "name",
            "src",
            "",
        ] {
            assert!(!html_value_is_case_insensitive(name), "{name}");
        }
    }

    #[test]
    fn html_attribute_values_compare_without_regard_to_ascii_case() {
        let doc = p(vec![
            e("input").attr("type", "CheckBox").into(),
            e("a")
                .attr("rel", "NoFollow noopener")
                .attr("hreflang", "EN-gb")
                .into(),
            e("p").attr("data-type", "CheckBox").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[type=checkbox]"), vec!["input"]);
        assert_eq!(matched_tags(&doc, "[type=CHECKBOX]"), vec!["input"]);
        assert_eq!(
            matched_tags(&doc, "[type^=che][type$=BOX][type*=kB]"),
            vec!["input"]
        );
        // `s` makes it identical, `i` makes any value insensitive.
        assert_eq!(
            matched_tags(&doc, "[type=checkbox s]"),
            Vec::<String>::new()
        );
        assert_eq!(matched_tags(&doc, "[type=CheckBox s]"), vec!["input"]);
        assert_eq!(
            matched_tags(&doc, "[data-type=checkbox]"),
            Vec::<String>::new()
        );
        assert_eq!(matched_tags(&doc, "[data-type=checkbox i]"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "[rel~=nofollow]"), vec!["a"]);
        assert_eq!(matched_tags(&doc, "[hreflang|=en]"), vec!["a"]);
        assert_eq!(matched_tags(&doc, "[hreflang|=en s]"), Vec::<String>::new());
        // The rule is the value's; the name was lowercased when parsed.
        assert_eq!(matched_tags(&doc, "[TYPE=checkbox]"), vec!["input"]);
    }

    #[test]
    fn case_insensitive_comparison_folds_ascii_letters_only() {
        // `to_lowercase` used to fold the Kelvin sign into `k` and `É`
        // into `é`; HTML and selectors-4 say ASCII case-insensitive.
        let doc = p(vec![
            e("p").attr("data-x", "\u{212A}").into(),
            e("b").attr("data-x", "ÉTÉ").into(),
            e("i").attr("data-x", "SS").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[data-x=k i]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[data-x='\u{212A}' i]"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "[data-x='été' i]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[data-x='ÉtÉ' i]"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "[data-x='ß' i]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[data-x*='T' i]"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "[data-x*='é' i]"), Vec::<String>::new());
    }

    #[test]
    fn in_place_comparisons_agree_with_the_string_operators() {
        // Every operator, folded and not, against values with multi-byte
        // characters, a needle longer than the value, and the edges.
        let values = [
            "", "a", "A", "é", "aé", "éA-b", "en", "EN-gb", "ENx", "x-y z",
        ];
        let needles = [
            "", "a", "é", "A", "en", "éa", "-", "éA-b", "en-gb", "x-y z", "z",
        ];
        for value in values {
            for needle in needles {
                for fold in [false, true] {
                    let compare = Compare { fold };
                    let (v, n) = if fold {
                        (value.to_ascii_lowercase(), needle.to_ascii_lowercase())
                    } else {
                        (value.to_string(), needle.to_string())
                    };
                    let case = format!("{value:?} {needle:?} fold={fold}");
                    assert_eq!(compare.eq(value, needle), v == n, "eq {case}");
                    assert_eq!(
                        compare.starts_with(value, needle),
                        v.starts_with(&n),
                        "^ {case}"
                    );
                    assert_eq!(
                        compare.ends_with(value, needle),
                        v.ends_with(&n),
                        "$ {case}"
                    );
                    assert_eq!(compare.contains(value, needle), v.contains(&n), "* {case}");
                }
            }
        }
    }

    #[test]
    fn dash_and_word_operators_compare_in_place() {
        let doc = p(vec![
            e("a").attr("data-x", "EN").into(),
            e("b").attr("data-x", "EN-gb").into(),
            e("i").attr("data-x", "ENx").into(),
            e("s").attr("data-x", "e").into(),
            e("u").attr("data-x", "one\tTWO\u{c}three").into(),
        ]);
        assert_eq!(matched_tags(&doc, "[data-x|=en i]"), vec!["a", "b"]);
        assert_eq!(matched_tags(&doc, "[data-x|=EN]"), vec!["a", "b"]);
        assert_eq!(matched_tags(&doc, "[data-x|=en]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[data-x~=two i]"), vec!["u"]);
        assert_eq!(matched_tags(&doc, "[data-x~=three]"), vec!["u"]);
        assert_eq!(
            matched_tags(&doc, "[data-x~='one TWO']"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn classes_are_split_on_ascii_white_space_only() {
        // HTML splits `class` on ASCII white space; a no-break space is
        // part of a class name.
        let doc = p(vec![
            e("p").attr("class", "a\u{a0}b").into(),
            e("b").attr("class", "c\td\u{c}e\rf").into(),
        ]);
        assert_eq!(matched_tags(&doc, ".a"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, ".b"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, ".a\u{a0}b"), vec!["p"]);
        assert_eq!(matched_tags(&doc, "[class~=b]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, ".c.d.e.f"), vec!["b"]);
        assert_eq!(matched_tags(&doc, "[class~=e]"), vec!["b"]);
    }
}
