//! Selector matching: right-to-left, selectors-4 style.
//!
//! The rightmost compound must match the element; each combinator then
//! walks the DOM (ancestor chain or preceding siblings) to find a
//! context that matches the remaining compounds. Dynamic pseudo-classes
//! (`:hover` etc.) never match until input state exists (M26).

use super::flat_dom::FlatDom;
use super::{AttrOp, Combinator, PseudoClass, Selector, SelectorList, SimpleSelector};

/// Whether any selector in the list matches the element.
pub fn matches_selector_list(flat: &FlatDom, idx: usize, list: &SelectorList) -> bool {
    list.0.iter().any(|s| matches_selector(flat, idx, s))
}

/// Whether one complex selector matches the element.
pub fn matches_selector(flat: &FlatDom, idx: usize, selector: &Selector) -> bool {
    let Some(last) = selector.compounds.last() else {
        return false;
    };
    if !matches_compound(flat, idx, last) {
        return false;
    }
    // `compounds[compounds.len() - 1]` (the last one) just matched at
    // `idx`; walk what's left, right to left.
    matches_rest(flat, idx, selector.compounds.len() - 1, selector)
}

/// Whether the compounds before index `matched_idx` are also satisfied,
/// given that `selector.compounds[matched_idx]` has just matched at `idx`.
/// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`, so the
/// combinator to use when stepping left from `matched_idx` is
/// `combinators[matched_idx - 1]`.
fn matches_rest(flat: &FlatDom, idx: usize, matched_idx: usize, selector: &Selector) -> bool {
    if matched_idx == 0 {
        return true; // compound 0 (the first) just matched; nothing left.
    }
    let next = &selector.compounds[matched_idx - 1];
    match selector.combinators[matched_idx - 1] {
        Combinator::Child => match flat.parent_element(idx) {
            Some(p) => {
                matches_compound(flat, p, next) && matches_rest(flat, p, matched_idx - 1, selector)
            }
            None => false,
        },
        Combinator::Descendant => {
            let mut ancestor = flat.parent_element(idx);
            while let Some(a) = ancestor {
                if matches_compound(flat, a, next)
                    && matches_rest(flat, a, matched_idx - 1, selector)
                {
                    return true;
                }
                ancestor = flat.parent_element(a);
            }
            false
        }
        Combinator::NextSibling => match flat.prev_element_sibling(idx) {
            Some(s) => {
                matches_compound(flat, s, next) && matches_rest(flat, s, matched_idx - 1, selector)
            }
            None => false,
        },
        Combinator::LaterSibling => {
            let mut prev = flat.prev_element_sibling(idx);
            while let Some(s) = prev {
                if matches_compound(flat, s, next)
                    && matches_rest(flat, s, matched_idx - 1, selector)
                {
                    return true;
                }
                prev = flat.prev_element_sibling(s);
            }
            false
        }
    }
}

/// Whether every simple selector in the compound matches.
fn matches_compound(flat: &FlatDom, idx: usize, compound: &super::CompoundSelector) -> bool {
    compound
        .simple
        .iter()
        .all(|s| matches_simple(flat, idx, s))
}

/// Whether one simple selector matches the element.
fn matches_simple(flat: &FlatDom, idx: usize, simple: &SimpleSelector) -> bool {
    let node = &flat.nodes[idx];
    match simple {
        SimpleSelector::Type(t) => node.tag == *t,
        SimpleSelector::Universal => true,
        SimpleSelector::Id(v) => flat.attr(idx, "id") == Some(v.as_str()),
        SimpleSelector::Class(c) => flat
            .attr(idx, "class")
            .is_some_and(|cls| cls.split_whitespace().any(|w| w == c)),
        SimpleSelector::Attribute {
            name,
            op,
            value,
            case_insensitive,
        } => matches_attribute(flat, idx, name, *op, value, *case_insensitive),
        SimpleSelector::PseudoClass(pc) => matches_pseudo_class(flat, idx, pc),
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
        AttrOp::IncludeSpace => {
            !value.is_empty() && actual.split_whitespace().any(|w| w == value)
        }
        // `|=` is the value itself or a `value-` prefix.
        AttrOp::Dash => actual == value || actual.starts_with(&format!("{value}-")),
        AttrOp::Prefix => actual.starts_with(&value),
        AttrOp::Suffix => actual.ends_with(&value),
        AttrOp::Substring => actual.contains(&value),
    }
}

/// Pseudo-class semantics.
fn matches_pseudo_class(flat: &FlatDom, idx: usize, pc: &PseudoClass) -> bool {
    let node = &flat.nodes[idx];
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
            // position *among only the siblings that match S* — not its
            // plain sibling position. ("2n+1 of .x" means "every other
            // .x", not ".x at an odd position among all siblings".)
            Some(list) => {
                matches_selector_list(flat, idx, list)
                    && nth_matches(*a, *b, sibling_rank_among(flat, idx, list))
            }
        },
        PseudoClass::NthLastChild { a, b } => {
            nth_matches(*a, *b, node.sibling_count - 1 - node.element_index)
        }
        PseudoClass::NthOfType { a, b } => nth_matches(*a, *b, node.type_index),
        PseudoClass::NthLastOfType { a, b } => {
            nth_matches(*a, *b, node.sibling_type_count - 1 - node.type_index)
        }
        PseudoClass::Not(list) => !matches_selector_list(flat, idx, list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => matches_selector_list(flat, idx, list),
        // Dynamic pseudo-classes never match until input state exists (M26).
        PseudoClass::Hover
        | PseudoClass::Focus
        | PseudoClass::Active
        | PseudoClass::Visited
        | PseudoClass::Link => false,
    }
}

/// The 0-based rank of `idx` among its element siblings that also match
/// `list` (selectors-4's `of S` filter: positions are renumbered within
/// the filtered set, not the full sibling list). Callers only call this
/// once `idx` itself is known to match `list`, so it is always found.
fn sibling_rank_among(flat: &FlatDom, idx: usize, list: &SelectorList) -> u32 {
    let parent = flat.nodes[idx].parent.unwrap_or(0);
    flat.nodes[parent]
        .child_elements
        .iter()
        .filter(|&&sibling| matches_selector_list(flat, sibling, list))
        .position(|&sibling| sibling == idx)
        .expect("idx must be among its own parent's children") as u32
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
        let doc = p(vec![e("div").push(e("section").push(e("p"))).push(e("p")).into()]);
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
        let doc = p(vec![e("a").into(), e("b").into(), e("c").into(), e("c").into()]);
        assert_eq!(matched_tags(&doc, "a ~ c"), vec!["c", "c"]);
        assert_eq!(matched_tags(&doc, "c ~ a"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, "c ~ c"), vec!["c"]); // the second c
    }

    #[test]
    fn deep_right_to_left_chain() {
        let doc = p(vec![e("div")
            .attr("class", "a")
            .push(e("section").push(e("div").attr("class", "b").push(e("span"))))
            .into()]);
        assert_eq!(matched_tags(&doc, ".a .b span"), vec!["span"]);
        assert_eq!(matched_tags(&doc, ".a > .b"), Vec::<String>::new());
    }

    #[test]
    fn sibling_then_ancestor_combination() {
        let doc = p(vec![e("ul")
            .push(e("li").attr("class", "first"))
            .push(e("li"))
            .into()]);
        assert_eq!(matched_tags(&doc, "ul .first + li"), vec!["li"]);
    }

    // ---- attributes ---------------------------------------------------------

    #[test]
    fn attribute_exists() {
        let doc = p(vec![e("input").attr("disabled", "").into(), e("input").into()]);
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
        assert_eq!(matched_tags(&doc, "[href^='http://']"), Vec::<String>::new());
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
            e("div").into(),                                  // no children: empty
            e("div").text("   ").into(),                      // ws text: NOT empty
            e("div").push(e("p")).into(),                     // element child: NOT empty
        ]);
        assert_eq!(matched_tags(&doc, "div:empty"), vec!["div"]);
    }

    #[test]
    fn root_is_the_document_element() {
        let doc = p(vec![e("html")
            .push(e("head"))
            .push(e("body").push(e("p")))
            .into()]);
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
        let doc = p(vec![e("div")
            .push(e("a"))
            .push(e("b"))
            .push(e("a"))
            .push(e("b"))
            .into()]);
        // The second <a/> is a:nth-of-type(2) even though it's the 3rd child.
        assert_eq!(matched_tags(&doc, "a:nth-of-type(2)").len(), 1);
        assert_eq!(matched_tags(&doc, "a:nth-child(2)").len(), 0);
        assert_eq!(matched_tags(&doc, "b:nth-last-of-type(2n+1)").len(), 1);
    }

    #[test]
    fn nth_child_of_filter() {
        // <ul><li class=x/><li/><li class=x/><li/><li class=x/></ul>
        let doc = p(vec![e("ul")
            .push(e("li").attr("class", "x"))
            .push(e("li"))
            .push(e("li").attr("class", "x"))
            .push(e("li"))
            .push(e("li").attr("class", "x"))
            .into()]);
        // .x items are at of-list positions 0,1,2.
        assert_eq!(
            matched_tags(&doc, ":nth-child(odd of .x)").len(),
            2
        );
        assert_eq!(
            matched_tags(&doc, ":nth-child(2 of .x)").len(),
            1
        );
    }

    // ---- logical combinators ---------------------------------------------------

    #[test]
    fn not_negates() {
        let doc = p(vec![
            e("p").attr("class", "a").into(),
            e("p").into(),
        ]);
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
        let doc = p(vec![
            e("a").into(),
            e("b").into(),
            e("c").into(),
        ]);
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
        assert_eq!(matched_tags(&doc, "li:nth-child(-2n)"), Vec::<String>::new());
    }

    #[test]
    fn nth_child_of_filter_renumbers_not_reindexes() {
        // <ul><li/><li class=x/><li/><li class=x/><li class=x/></ul>
        // .x items sit at original positions 2, 4, 5 but of-list ranks
        // 0, 1, 2 — `nth-child(2 of .x)` must pick the of-list's 2nd (the
        // li at original position 4), not the original position 2.
        let doc = p(vec![e("ul")
            .push(e("li"))
            .push(e("li").attr("class", "x").attr("data-mark", "first"))
            .push(e("li"))
            .push(e("li").attr("class", "x").attr("data-mark", "second"))
            .push(e("li").attr("class", "x").attr("data-mark", "third"))
            .into()]);
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
        let doc = p(vec![e("ul")
            .push(e("li").attr("class", "x"))
            .push(e("li").attr("class", "x"))
            .into()]);
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
        let doc = p(vec![e("ul")
            .push(e("li").attr("class", "skip"))
            .push(e("li"))
            .push(e("li"))
            .into()]);
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
        let doc = p(vec![e("a")
            .push(e("b").push(e("c").push(e("d").push(e("e")))))
            .into()]);
        assert_eq!(matched_tags(&doc, "a e"), vec!["e"]);
        assert_eq!(matched_tags(&doc, "a b c d e"), vec!["e"]);
        assert_eq!(matched_tags(&doc, "a > e"), Vec::<String>::new());
    }

    #[test]
    fn child_combinator_chain_with_attribute_and_next_sibling() {
        let doc = p(vec![e("form")
            .push(e("input").attr("required", ""))
            .push(e("button"))
            .into()]);
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
        let doc = p(vec![e("a").into(), e("b").into(), e("c").into(), e("d").into()]);
        assert_eq!(matched_tags(&doc, "a ~ d"), vec!["d"]);
    }

    #[test]
    fn is_and_where_do_not_match_when_no_argument_matches() {
        let doc = p(vec![e("p").into()]);
        assert_eq!(matched_tags(&doc, ":is(div, span)"), Vec::<String>::new());
        assert_eq!(matched_tags(&doc, ":where(div, span)"), Vec::<String>::new());
    }

    #[test]
    fn nth_last_of_type_combined_with_type_and_class() {
        // <div><a class=x/><b/><a/><b/></div>: two <a>s, class filter
        // plus nth-last-of-type must both hold.
        let doc = p(vec![e("div")
            .push(e("a").attr("class", "x"))
            .push(e("b"))
            .push(e("a"))
            .push(e("b"))
            .into()]);
        assert_eq!(matched_tags(&doc, "a.x:nth-last-of-type(2)"), vec!["a"]);
        assert_eq!(matched_tags(&doc, "a.x:nth-last-of-type(1)"), Vec::<String>::new());
    }
}
