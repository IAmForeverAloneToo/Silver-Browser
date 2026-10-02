//! Style invalidation (M13.1): which elements a set of changes to a
//! document can restyle, read from the changes its journal records
//! and from what the selectors in force depend on.
//!
//! The rules (docs/design/incremental.md):
//!
//! - An attribute changed on an element E. `style` and `dir` restyle E,
//!   since the cascade reads them directly. A `class` change restyles E
//!   when a class added or removed is one some selector names, an `id`
//!   change when the old or new id is named, any other attribute when
//!   its name is (a `class` or `id` named as an attribute too). Such an
//!   E is restyled with its subtree, since descendant and child
//!   combinators make descendants depend on it, and with its following
//!   siblings' subtrees when sibling combinators are in use, or all its
//!   siblings' when `of S` is. Any other attribute changes no style.
//! - A child inserted under or removed from a parent P. An inserted
//!   element is restyled with its subtree. P is restyled when `:empty`
//!   is in use. P's element children after the change point are
//!   restyled with their subtrees when sibling combinators or
//!   pseudo-classes that count from the start are in use, those before
//!   it when ones counting from the end are, and all of them when ones
//!   looking both ways are. Text counts for `:empty` and nothing else.
//! - A text node's data changed: no selector reads it.
//!
//! The changes are read after they were all made, so each rule looks
//! at the document as it now stands: an element no longer in the tree
//! is not restyled, an attribute is compared between the value it had
//! before the first of its changes and the value it has now, and a
//! change point whose neighbours have since moved is taken to be
//! anywhere among the parent's children. Restyling more than needed is
//! always correct; this is where precision is traded for simplicity.

use std::collections::{HashMap, HashSet};

use dom::{Change, Document, NodeId};

use crate::selector_matching::{Combinator, PseudoClass, SelectorList, SimpleSelector};

/// What the selectors of a set of stylesheets depend on, beyond the
/// tags and the tree's shape that every restyle accounts for: the
/// class names, ids and attribute names any selector mentions, inside
/// `:not()`, `:is()`, `:where()` and `of S` included, and which kinds
/// of selector make an element's style depend on its siblings or on
/// whether it has children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectorFeatures {
    classes: HashSet<String>,
    ids: HashSet<String>,
    attributes: HashSet<String>,
    /// `+` or `~`.
    sibling_combinators: bool,
    /// `:first-child`, `:first-of-type`, `:nth-child` or `:nth-of-type`:
    /// a position counted from the start.
    counts_from_start: bool,
    /// `:last-child`, `:last-of-type`, `:nth-last-child` or
    /// `:nth-last-of-type`: a position counted from the end.
    counts_from_end: bool,
    /// `:only-child`, `:only-of-type`, or `of S`: every sibling counts.
    every_sibling: bool,
    /// `:empty`.
    empty: bool,
}

impl SelectorFeatures {
    /// Adds what `list`'s selectors depend on.
    pub(crate) fn add(&mut self, list: &SelectorList) {
        for selector in &list.0 {
            for combinator in &selector.combinators {
                if matches!(
                    combinator,
                    Combinator::NextSibling | Combinator::LaterSibling
                ) {
                    self.sibling_combinators = true;
                }
            }
            for compound in &selector.compounds {
                for simple in &compound.simple {
                    self.add_simple(simple);
                }
            }
        }
    }

    fn add_simple(&mut self, simple: &SimpleSelector) {
        match simple {
            SimpleSelector::Type(_) | SimpleSelector::Universal => {}
            SimpleSelector::Id(id) => {
                self.ids.insert(id.clone());
            }
            SimpleSelector::Class(class) => {
                self.classes.insert(class.clone());
            }
            SimpleSelector::Attribute { name, .. } => {
                self.attributes.insert(name.clone());
            }
            SimpleSelector::PseudoClass(pc) => match pc {
                PseudoClass::FirstChild
                | PseudoClass::FirstOfType
                | PseudoClass::NthOfType { .. } => {
                    self.counts_from_start = true;
                }
                PseudoClass::NthChild { of: None, .. } => self.counts_from_start = true,
                PseudoClass::NthChild { of: Some(list), .. } => {
                    self.every_sibling = true;
                    self.add(list);
                }
                PseudoClass::LastChild
                | PseudoClass::LastOfType
                | PseudoClass::NthLastChild { .. }
                | PseudoClass::NthLastOfType { .. } => self.counts_from_end = true,
                PseudoClass::OnlyChild | PseudoClass::OnlyOfType => self.every_sibling = true,
                PseudoClass::Empty => self.empty = true,
                PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => {
                    self.add(list);
                }
                // `:root` depends on the parent, which an insertion
                // restyles whole; the dynamic ones never match yet.
                PseudoClass::Root
                | PseudoClass::Hover
                | PseudoClass::Focus
                | PseudoClass::Active
                | PseudoClass::Visited
                | PseudoClass::Link => {}
            },
        }
    }

    /// Whether changing attribute `name` from `old` to `new` can change
    /// which selectors match its element.
    fn mentions(&self, name: &str, old: Option<&str>, new: Option<&str>) -> bool {
        if self.attributes.contains(name) {
            return true;
        }
        match name {
            "class" => {
                let old: HashSet<&str> = old.unwrap_or("").split_ascii_whitespace().collect();
                let new: HashSet<&str> = new.unwrap_or("").split_ascii_whitespace().collect();
                old.symmetric_difference(&new)
                    .any(|class| self.classes.contains(*class))
            }
            "id" => [old, new]
                .into_iter()
                .flatten()
                .any(|id| self.ids.contains(id)),
            _ => false,
        }
    }
}

/// How much of the tree under an element a restyle must reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reach {
    /// The element; its children too if its style changes, since they
    /// inherit from it, and so on down.
    Element,
    /// The element and everything under it, changed or not.
    Subtree,
}

/// The elements a set of changes requires restyled.
#[derive(Debug, Default)]
pub(crate) struct Invalidation {
    /// Every change counts: cascade the document again.
    pub(crate) everything: bool,
    /// The elements to restyle, each with how far.
    pub(crate) dirty: HashMap<NodeId, Reach>,
    /// Nodes removed from the tree and not put back, whose subtrees'
    /// styles are to be forgotten.
    pub(crate) removed: Vec<NodeId>,
}

impl Invalidation {
    fn mark(&mut self, document: &Document, node: NodeId, reach: Reach) {
        if document.is_element(node) && attached(document, node) {
            let entry = self.dirty.entry(node).or_insert(reach);
            *entry = (*entry).max(reach);
        }
    }
}

/// Whether `node` is in `document`'s tree.
pub(crate) fn attached(document: &Document, node: NodeId) -> bool {
    node == document.root() || document.ancestors(node).last() == Some(document.root())
}

/// The elements `changes`, made to `document` since its styles were
/// computed, require restyled, given what the selectors in force
/// depend on.
pub(crate) fn invalidate(
    document: &Document,
    features: &SelectorFeatures,
    changes: &[Change],
) -> Invalidation {
    let mut out = Invalidation::default();
    // Each attribute's value before the first of its changes.
    let mut attributes: HashMap<(NodeId, &str), Option<&str>> = HashMap::new();
    for change in changes {
        match change {
            Change::Everything => {
                out.everything = true;
                return out;
            }
            Change::Inserted { parent, child } => {
                let element = !document.is_text(*child);
                if document.parent(*child) == Some(*parent) {
                    if element {
                        out.mark(document, *child, Reach::Subtree);
                        let before = document.prev_sibling(*child);
                        let after = document.next_sibling(*child);
                        siblings(
                            document,
                            features,
                            *parent,
                            Some(before),
                            Some(after),
                            &mut out,
                        );
                    }
                } else if element {
                    // Moved on since: where it stood is not known.
                    siblings(document, features, *parent, None, None, &mut out);
                }
                if features.empty {
                    out.mark(document, *parent, Reach::Element);
                }
            }
            Change::Removed {
                parent,
                child,
                prev,
                next,
            } => {
                // A freed node's kind is no longer known: an element,
                // to be safe.
                if !document.is_text(*child) {
                    // A neighbour still beside the change point marks it;
                    // one moved since leaves it anywhere.
                    let anchor = |n: &Option<NodeId>| match n {
                        None => Some(None),
                        Some(n) if document.parent(*n) == Some(*parent) => Some(Some(*n)),
                        Some(_) => None,
                    };
                    siblings(
                        document,
                        features,
                        *parent,
                        anchor(prev),
                        anchor(next),
                        &mut out,
                    );
                }
                if features.empty {
                    out.mark(document, *parent, Reach::Element);
                }
                if !attached(document, *child) {
                    out.removed.push(*child);
                }
            }
            Change::AttributeChanged { element, name, old } => {
                attributes
                    .entry((*element, name.as_str()))
                    .or_insert(old.as_deref());
            }
            Change::TextChanged { .. } => {}
        }
    }
    for ((element, name), old) in attributes {
        let new = document.attr(element, name);
        if new == old {
            continue;
        }
        if features.mentions(name, old, new) {
            out.mark(document, element, Reach::Subtree);
            if features.every_sibling {
                if let Some(parent) = document.parent(element) {
                    for sibling in document.element_children(parent) {
                        out.mark(document, sibling, Reach::Subtree);
                    }
                }
            } else if features.sibling_combinators {
                let mut at = document.next_element_sibling(element);
                while let Some(sibling) = at {
                    out.mark(document, sibling, Reach::Subtree);
                    at = document.next_element_sibling(sibling);
                }
            }
        } else if name == "style" || name == "dir" {
            out.mark(document, element, Reach::Element);
        }
    }
    out
}

/// Marks the element children of `parent` that a child inserted or
/// removed at a point among them can restyle, with their subtrees:
/// those after the point when sibling combinators or positions counted
/// from the start are in use, those before it when positions counted
/// from the end are, all of them when every sibling counts. `before`
/// and `after` are the point's neighbours, `Some(None)` at an end and
/// `None` when not known — every child on that side then.
fn siblings(
    document: &Document,
    features: &SelectorFeatures,
    parent: NodeId,
    before: Option<Option<NodeId>>,
    after: Option<Option<NodeId>>,
    out: &mut Invalidation,
) {
    let all = features.every_sibling
        || (before.is_none() && features.counts_from_end)
        || (after.is_none() && (features.sibling_combinators || features.counts_from_start));
    if all {
        let children: Vec<NodeId> = document.element_children(parent).collect();
        for child in children {
            out.mark(document, child, Reach::Subtree);
        }
        return;
    }
    if features.sibling_combinators || features.counts_from_start {
        let mut at = after.flatten();
        while let Some(node) = at {
            out.mark(document, node, Reach::Subtree);
            at = document.next_sibling(node);
        }
    }
    if features.counts_from_end {
        let mut at = before.flatten();
        while let Some(node) = at {
            out.mark(document, node, Reach::Subtree);
            at = document.prev_sibling(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CascadeContext, Restyle, Styles, Stylesheets};

    /// The document `html` parses to, `css` prepared as its one sheet,
    /// and its styles.
    fn setup(html: &str, css: &str) -> (Document, Stylesheets, Styles) {
        let node = html::parse_document(html).expect("never fails");
        let document = Document::from_node(&node);
        let sheets = Stylesheets::new(vec![css::parse_stylesheet(css).expect("valid")]);
        let styles = sheets.cascade(&document, &CascadeContext::default());
        (document, sheets, styles)
    }

    /// Applies `f` to the document, restyles by the changes it made, and
    /// checks the result against a full cascade: every element in the
    /// tree styled alike, nothing outside it styled. Returns what the
    /// restyle did.
    fn change(
        document: &mut Document,
        sheets: &Stylesheets,
        styles: &mut Styles,
        f: impl FnOnce(&mut Document),
    ) -> Restyle {
        f(document);
        let changes = document.take_changes();
        let context = CascadeContext::default();
        let restyle = sheets.restyle(document, &context, styles, &changes);
        let full = sheets.cascade(document, &context);
        for node in document.subtree(document.root()) {
            assert_eq!(styles.get(node), full.get(node), "{:?}", document.tag(node));
        }
        assert_eq!(styles.uses_viewport_units, full.uses_viewport_units);
        restyle
    }

    fn by_id(document: &Document, id: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("#{id}"))
    }

    #[test]
    fn the_features_are_gathered_from_every_selector_nested_ones_included() {
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                ".a #b [c] { x: y } :not(.d) :is(#e) :where([f]) { x: y }
                 :nth-child(2 of .g) { x: y } p ~ q { x: y } :empty { x: y }",
            )
            .unwrap(),
        ]);
        let f = sheets.features();
        for class in ["a", "d", "g"] {
            assert!(f.classes.contains(class), "{class}");
        }
        assert!(f.ids.contains("b") && f.ids.contains("e"));
        assert!(f.attributes.contains("c") && f.attributes.contains("f"));
        // The UA sheet's own: `input[type=hidden i]` and the like.
        assert!(f.attributes.contains("type"));
        assert!(f.sibling_combinators && f.every_sibling && f.empty);
        assert!(!f.counts_from_start && !f.counts_from_end);
        let plain = Stylesheets::new(vec![css::parse_stylesheet("p { x: y }").unwrap()]);
        assert!(!plain.features().sibling_combinators && !plain.features().empty);
    }

    #[test]
    fn an_attribute_no_selector_names_restyles_nothing() {
        let (mut d, sheets, mut styles) =
            setup("<div id=x class=a><p>one</p></div>", ".b { color: #f00 }");
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "c").unwrap();
            d.set_attribute(x, "title", "t").unwrap();
        });
        assert_eq!((r.full, r.computed), (false, 0));
        // Changed and changed back: nothing either.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "b").unwrap();
            d.set_attribute(x, "class", "c").unwrap();
        });
        assert_eq!(r.computed, 0);
    }

    #[test]
    fn a_named_class_restyles_the_element_and_its_subtree() {
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>one</p><p>two</p></div><div id=y></div>",
            ".b p { color: #f00 }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "b").unwrap();
        });
        assert_eq!(r.computed, 3, "the div and its two paragraphs");
        assert_eq!(r.changed.len(), 2, "the paragraphs turned red");
    }

    #[test]
    fn a_style_attribute_restyles_the_element_and_what_inherits_its_change() {
        let (mut d, sheets, mut styles) =
            setup("<div id=x><p>one</p><p>two</p></div>", "p { margin: 0 }");
        let x = by_id(&d, "x");
        // A property the children inherit reaches them.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "style", "color: #00f").unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (3, 3));
        // One they do not inherit leaves them as they were: restyled,
        // since `inherit` can take any property from a parent, found
        // unchanged, and gone no further.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "style", "color: #00f; padding: 4px")
                .unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (3, 1));
        // A style attribute set to what it was restyles nothing.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "style", "color: #00f").unwrap();
            d.set_attribute(x, "style", "color: #00f; padding: 4px")
                .unwrap();
        });
        assert_eq!(r.computed, 0);
    }

    #[test]
    fn an_insertion_restyles_the_new_subtree_and_the_siblings_selectors_reach() {
        // No sibling or structural selector: the new element alone.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>1</p><p>2</p><p>3</p></div>",
            "p { margin: 0 }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let p = d.create_element("p").unwrap();
            let first = d.first_child(x);
            d.insert_before(x, p, first).unwrap();
        });
        assert_eq!(r.computed, 1);
        // `p + p`: those after the new one too.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>1</p><p>2</p><p>3</p></div>",
            "p + p { color: #f00 }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let p = d.create_element("p").unwrap();
            let first = d.first_child(x);
            d.insert_before(x, p, first).unwrap();
        });
        assert_eq!(r.computed, 4, "the new p and the three after it");
        // `:last-child`: those before an append.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>1</p><p>2</p><p>3</p></div>",
            "p:last-child { color: #f00 }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let p = d.create_element("p").unwrap();
            d.append_child(x, p).unwrap();
        });
        assert_eq!(r.computed, 4, "the new p and the three before it");
        // `:empty`: the parent, whatever the child.
        let (mut d, sheets, mut styles) =
            setup("<div id=x></div>", ":empty { background-color: #0f0 }");
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let text = d.create_text("t").unwrap();
            d.append_child(x, text).unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (1, 1));
    }

    #[test]
    fn a_removal_forgets_the_subtree_and_restyles_what_it_stood_beside() {
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>1</p><p id=two><b>b</b></p><p>3</p></div>",
            "p + p { color: #f00 }",
        );
        let (x, two) = (by_id(&d, "x"), by_id(&d, "two"));
        let b = d.first_child(two).unwrap();
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_child(x, two).unwrap();
        });
        assert_eq!(r.computed, 1, "the p after it");
        assert_eq!((styles.get(two), styles.get(b)), (None, None));
        // Put back elsewhere: restyled there.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.append_child(x, two).unwrap();
        });
        assert_eq!(r.computed, 2, "the p and its b");
    }

    #[test]
    fn a_change_to_the_root_font_size_cascades_the_whole_document() {
        let (mut d, sheets, mut styles) =
            setup("<p id=x style=\"width: 2rem\">a</p>", "p { margin: 0 }");
        let html = d.document_element().unwrap();
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(html, "style", "font-size: 20px").unwrap();
        });
        assert!(r.full, "rem changed for every element");
    }

    #[test]
    fn a_new_viewport_unit_is_recorded() {
        let (mut d, sheets, mut styles) = setup("<p id=x>a</p>", "p { margin: 0 }");
        assert!(!styles.uses_viewport_units);
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "style", "width: 10vw").unwrap();
        });
        assert!(!r.full);
        assert!(styles.uses_viewport_units);
    }
}
