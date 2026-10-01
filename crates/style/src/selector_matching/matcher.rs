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
//! one document benefits from: the `:nth-child(… of S)` ranks per
//! parent, which the cascade would otherwise recount for every element
//! of a long list.

use std::collections::{HashMap, HashSet};

use super::flat_dom::FlatDom;
use super::{AttrOp, Combinator, PseudoClass, Selector, SelectorList, SimpleSelector};

/// Whether any selector in the list matches the element. A one-off;
/// a pass over many elements wants a [`Matcher`].
pub fn matches_selector_list(flat: &FlatDom, idx: usize, list: &SelectorList) -> bool {
    Matcher::new(flat).matches_list(idx, list)
}

/// Whether one complex selector matches the element. A one-off; a pass
/// over many elements wants a [`Matcher`].
pub fn matches_selector(flat: &FlatDom, idx: usize, selector: &Selector) -> bool {
    Matcher::new(flat).matches(idx, selector)
}

/// Matches selectors against one document. The selectors it is given
/// must outlive it (`'a`): the `of S` cache below is keyed by where a
/// list lives.
pub struct Matcher<'a> {
    flat: &'a FlatDom,
    /// `:nth-child(An+B of S)`: for each (parent, S) seen, the rank of
    /// every element child among the siblings that match S, or `None`
    /// for a child that does not, computed once per parent.
    nth_ranks: HashMap<(usize, usize), Vec<Option<u32>>>,
}

/// One partial match on the explicit stack: `selector.compounds[k]`
/// matched at `node`, and `next` is the candidate to try next for
/// compound `k - 1` (`None` once they are exhausted).
struct Frame {
    k: usize,
    node: usize,
    next: Option<usize>,
}

impl<'a> Matcher<'a> {
    /// A matcher over `flat`, with empty caches.
    pub fn new(flat: &'a FlatDom) -> Self {
        Matcher {
            flat,
            nth_ranks: HashMap::new(),
        }
    }

    /// Whether any selector in the list matches the element.
    pub fn matches_list(&mut self, idx: usize, list: &'a SelectorList) -> bool {
        list.0.iter().any(|s| self.matches(idx, s))
    }

    /// Whether one complex selector matches the element.
    pub fn matches(&mut self, idx: usize, selector: &'a Selector) -> bool {
        let compounds = &selector.compounds;
        let Some(last) = compounds.last() else {
            return false;
        };
        if !self.matches_compound(idx, last) {
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
        let mut failed: Option<HashSet<(usize, usize)>> = None;
        let mut stack = vec![Frame {
            k: n - 1,
            node: idx,
            next: self.first_candidate(selector.combinators[n - 2], idx),
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
                    Combinator::Descendant => self.flat.parent_element(cand),
                    Combinator::LaterSibling => self.flat.prev_element_sibling(cand),
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
    fn first_candidate(&self, comb: Combinator, node: usize) -> Option<usize> {
        match comb {
            Combinator::Child | Combinator::Descendant => self.flat.parent_element(node),
            Combinator::NextSibling | Combinator::LaterSibling => {
                self.flat.prev_element_sibling(node)
            }
        }
    }

    /// Whether every simple selector in the compound matches.
    fn matches_compound(&mut self, idx: usize, compound: &'a super::CompoundSelector) -> bool {
        compound.simple.iter().all(|s| self.matches_simple(idx, s))
    }

    /// Whether one simple selector matches the element.
    fn matches_simple(&mut self, idx: usize, simple: &'a SimpleSelector) -> bool {
        let node = &self.flat.nodes[idx];
        match simple {
            SimpleSelector::Type(t) => node.tag == *t,
            SimpleSelector::Universal => true,
            SimpleSelector::Id(v) => self.flat.attr(idx, "id") == Some(v.as_str()),
            SimpleSelector::Class(c) => self
                .flat
                .attr(idx, "class")
                .is_some_and(|cls| cls.split_whitespace().any(|w| w == c)),
            SimpleSelector::Attribute {
                name,
                op,
                value,
                case_insensitive,
            } => matches_attribute(self.flat, idx, name, *op, value, *case_insensitive),
            SimpleSelector::PseudoClass(pc) => self.matches_pseudo_class(idx, pc),
        }
    }

    /// Pseudo-class semantics.
    fn matches_pseudo_class(&mut self, idx: usize, pc: &'a PseudoClass) -> bool {
        let node = &self.flat.nodes[idx];
        match pc {
            PseudoClass::FirstChild => node.element_index == 0,
            PseudoClass::LastChild => node.element_index + 1 == node.sibling_count,
            PseudoClass::OnlyChild => node.sibling_count == 1,
            PseudoClass::FirstOfType => node.type_index == 0,
            PseudoClass::LastOfType => node.type_index + 1 == node.sibling_type_count,
            PseudoClass::OnlyOfType => node.sibling_type_count == 1,
            // selectors-4: any text child (even whitespace) makes it non-empty.
            PseudoClass::Empty => node.child_elements.is_empty() && !node.has_text_child,
            PseudoClass::Root => node.parent == Some(0),
            PseudoClass::NthChild { a, b, of } => match of {
                None => nth_matches(*a, *b, node.element_index),
                // selectors-4 §4: with an `of S` filter, the element must
                // itself match S, and the An+B formula applies to its
                // position *among only the siblings that match S* — not
                // its plain sibling position. ("2n+1 of .x" means "every
                // other .x", not ".x at an odd position among all
                // siblings".)
                Some(list) => self
                    .rank_among(idx, list)
                    .is_some_and(|rank| nth_matches(*a, *b, rank)),
            },
            PseudoClass::NthLastChild { a, b } => {
                nth_matches(*a, *b, node.sibling_count - 1 - node.element_index)
            }
            PseudoClass::NthOfType { a, b } => nth_matches(*a, *b, node.type_index),
            PseudoClass::NthLastOfType { a, b } => {
                nth_matches(*a, *b, node.sibling_type_count - 1 - node.type_index)
            }
            PseudoClass::Not(list) => !self.matches_list(idx, list),
            PseudoClass::Is(list) | PseudoClass::Where(list) => self.matches_list(idx, list),
            // Dynamic pseudo-classes never match until input state exists (M26).
            PseudoClass::Hover
            | PseudoClass::Focus
            | PseudoClass::Active
            | PseudoClass::Visited
            | PseudoClass::Link => false,
        }
    }

    /// The 0-based rank of `idx` among its element siblings that match
    /// `list` (selectors-4's `of S` filter: positions are renumbered
    /// within the filtered set, not the full sibling list), or `None`
    /// when `idx` itself does not match. The ranks of a parent's whole
    /// child list are computed in one pass the first time any child
    /// asks, so a list of n items costs n matches, not n squared.
    fn rank_among(&mut self, idx: usize, list: &'a SelectorList) -> Option<u32> {
        let parent = self.flat.nodes[idx].parent.unwrap_or(0);
        let key = (parent, list as *const SelectorList as usize);
        if !self.nth_ranks.contains_key(&key) {
            let children = self.flat.nodes[parent].child_elements.clone();
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
        let position = self.flat.nodes[idx].element_index as usize;
        self.nth_ranks[&key].get(position).copied().flatten()
    }
}

/// Attribute selector semantics (selectors-4 §6.1).
fn matches_attribute(
    flat: &FlatDom,
    idx: usize,
    name: &str,
    op: AttrOp,
    value: &str,
    case_insensitive: bool,
) -> bool {
    let Some(actual) = flat.attr(idx, name) else {
        return false;
    };
    let (actual, value) = if case_insensitive {
        (actual.to_lowercase(), value.to_lowercase())
    } else {
        (actual.to_string(), value.to_string())
    };
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => actual == value,
        // `~=` matches whitespace-separated words; an empty value never matches.
        AttrOp::IncludeSpace => !value.is_empty() && actual.split_whitespace().any(|w| w == value),
        // `|=` is the value itself or a `value-` prefix.
        AttrOp::Dash => actual == value || actual.starts_with(&format!("{value}-")),
        // selectors-4 §6.2: an empty value "does not represent anything"
        // for the three substring operators.
        AttrOp::Prefix => !value.is_empty() && actual.starts_with(&value),
        AttrOp::Suffix => !value.is_empty() && actual.ends_with(&value),
        AttrOp::Substring => !value.is_empty() && actual.contains(&value),
    }
}

/// Whether the CSS `An+B` formula matches: the element's 1-indexed sibling
/// *position* equals `a*n + b` for some integer `n >= 0` (selectors-4 §4).
/// `zero_based_index` is `FlatDom`'s internal 0-based counter, so it's
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
    use dom::{Element, Node};

    /// The tags of all elements the selector matches, in document order.
    fn matched_tags(document: &Node, selector: &str) -> Vec<String> {
        let flat = FlatDom::build(document);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        let mut out = Vec::new();
        for idx in flat.element_indices() {
            if matches_selector_list(&flat, idx, &list) {
                out.push(flat.nodes[idx].tag.clone());
            }
        }
        out
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
        for sel in [":hover", ":focus", ":active", ":visited", ":link"] {
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
        let flat = FlatDom::build(&doc);
        let list = parse_selector_list(&css::tokenize(":nth-child(2 of .x)")).unwrap();
        let marks: Vec<&str> = flat
            .element_indices()
            .filter(|&i| matches_selector_list(&flat, i, &list))
            .filter_map(|i| flat.attr(i, "data-mark"))
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
        let doc = p(vec![e("p").attr("lang", "EN-us").into()]);
        assert_eq!(matched_tags(&doc, "[lang|=en]"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "[lang|=en i]"), vec!["p"]);
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
                let flat = FlatDom::build(&doc);
                let last = flat.nodes.len() - 1;
                let selector = vec!["div"; 20_000].join(" > ") + " > a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&flat, last, &list));
                let selector = vec!["div"; 20_001].join(" > ") + " > a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(!matches_selector_list(&flat, last, &list));
                let selector = vec!["div"; 20_000].join(" ") + " a";
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&flat, last, &list));

                let siblings: Vec<Node> = (0..300).map(|_| e("a").into()).collect();
                let doc = p(vec![e("div").push_children(siblings).into()]);
                let flat = FlatDom::build(&doc);
                let last = flat.nodes.len() - 1;
                let selector = vec!["a"; 300].join(" ~ ");
                let list = parse_selector_list(&css::tokenize(&selector)).expect("valid");
                assert!(matches_selector_list(&flat, last, &list));
                assert!(!matches_selector_list(&flat, last - 1, &list));
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
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
        let flat = FlatDom::build(&doc);
        let list = parse_selector_list(&css::tokenize("li:nth-child(odd of .x)")).expect("valid");
        let mut matcher = Matcher::new(&flat);
        let count = flat
            .element_indices()
            .filter(|&i| matcher.matches_list(i, &list))
            .count();
        // 6667 `.x` items, every other one.
        assert_eq!(count, 3334);
        let nested =
            parse_selector_list(&css::tokenize("li:nth-child(odd of :nth-child(odd of .x))"))
                .expect("valid");
        let mut matcher = Matcher::new(&flat);
        let count = flat
            .element_indices()
            .filter(|&i| matcher.matches_list(i, &nested))
            .count();
        assert_eq!(count, 1667);
    }
}
