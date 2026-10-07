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
//! Three kinds of pseudo-class read more than an element and its
//! ancestors (M16), and add rules of their own when a sheet uses them:
//!
//! - `:lang()` reads the document's `<meta http-equiv>` pragma: a
//!   change to a `meta`'s `http-equiv` or `content`, or a `meta`
//!   inserted or removed, cascades the document whole.
//! - `:disabled` and `:enabled` read which `legend` is a fieldset's
//!   first: a child inserted under or removed from a `fieldset`
//!   restyles the fieldset's subtree.
//! - `:checked` reads a select's other options and a radio button's
//!   group: a child inserted or removed in a `select`, or a change to
//!   an `option`'s or `optgroup`'s `selected` or `disabled`, restyles
//!   the select's subtree; and any element inserted or removed, or a
//!   change to an `input`'s `checked`, `name`, `type` or `form`, or to
//!   any element's `id`, restyles every radio button in the document.
//!
//! The attributes they read on the element and its ancestors (`lang`,
//! `href`, `disabled`, `checked`, `selected`, `type`, `multiple`,
//! `size`, `required`) count as named by a selector, and so do the ones
//! a `content` declaration's `attr()` reads.
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
    /// `:lang()`: the document's pragma counts.
    lang: bool,
    /// `:disabled` or `:enabled`: a fieldset's first legend counts.
    fieldsets: bool,
    /// `:checked`: a select's options and radio button groups count.
    checked: bool,
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
            // The pseudo-classes after a pseudo-element are user-action
            // ones, which read nothing a change records (M26.4).
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
            SimpleSelector::PseudoClass(pc) => self.add_pseudo_class(pc),
        }
    }

    fn add_pseudo_class(&mut self, pc: &PseudoClass) {
        match pc {
            PseudoClass::FirstChild | PseudoClass::FirstOfType | PseudoClass::NthOfType { .. } => {
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
            PseudoClass::Link | PseudoClass::AnyLink => self.read(&["href"]),
            PseudoClass::Lang(_) => {
                self.lang = true;
                self.read(&["lang"]);
            }
            PseudoClass::Disabled | PseudoClass::Enabled => {
                self.fieldsets = true;
                self.read(&["disabled"]);
            }
            PseudoClass::Checked => {
                self.checked = true;
                self.read(&[
                    "checked", "selected", "type", "multiple", "size", "disabled",
                ]);
            }
            PseudoClass::Required | PseudoClass::Optional => self.read(&["required", "type"]),
            // `:root` depends on the parent, which an insertion
            // restyles whole; the dynamic ones never match yet.
            PseudoClass::Root
            | PseudoClass::Hover
            | PseudoClass::Focus
            | PseudoClass::FocusVisible
            | PseudoClass::FocusWithin
            | PseudoClass::Active
            | PseudoClass::Visited
            | PseudoClass::Target => {}
        }
    }

    /// Adds what `declarations`, a style rule's, read: the attributes
    /// `content`'s `attr()` names, on the element a `::before` or
    /// `::after` comes from.
    pub(crate) fn add_declarations(&mut self, declarations: &[css::Declaration]) {
        for declaration in declarations.iter().filter(|d| d.name == "content") {
            let mut tokens = declaration.value.iter();
            while let Some(token) = tokens.next() {
                if let css::Token::Function(name) = token
                    && name.eq_ignore_ascii_case("attr")
                    && let Some(css::Token::Ident(attribute)) = tokens
                        .by_ref()
                        .find(|token| !matches!(token, css::Token::Whitespace))
                {
                    self.attributes.insert(attribute.to_ascii_lowercase());
                }
            }
        }
    }

    /// Counts the attributes `names` as named by a selector.
    fn read(&mut self, names: &[&str]) {
        for name in names {
            self.attributes.insert((*name).to_owned());
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
    // Whether every radio button is to be restyled (`:checked`).
    let mut radios = false;
    for change in changes {
        match change {
            Change::Everything => {
                out.everything = true;
                return out;
            }
            Change::Inserted { parent, child } | Change::Removed { parent, child, .. } => {
                // A freed node's kind is no longer known: it could have
                // been a `meta`, or held one.
                let holds_meta = !document.contains(*child) || holds(document, *child, "meta");
                if features.lang && holds_meta {
                    out.everything = true;
                    return out;
                }
                around_child(document, features, *parent, &mut out);
                radios |= features.checked && !document.is_text(*child);
                // Which element is the root can change with the
                // document node's children, and the root's `display`
                // is blockified (M16): each of them, the root that was
                // and the root that is, is restyled.
                if *parent == document.root() {
                    for top in document.element_children(*parent) {
                        out.mark(document, top, Reach::Element);
                    }
                }
            }
            Change::AttributeChanged { .. } | Change::TextChanged { .. } => {}
        }
        match change {
            Change::Everything => {}
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
        let tag = document.tag(element);
        if features.lang && tag == Some("meta") && matches!(name, "http-equiv" | "content") {
            out.everything = true;
            return out;
        }
        if features.checked {
            if matches!(tag, Some("option" | "optgroup"))
                && matches!(name, "selected" | "disabled")
                && let Some(select) = select_around(document, element)
            {
                out.mark(document, select, Reach::Subtree);
            }
            radios |= name == "id"
                || (tag == Some("input") && matches!(name, "checked" | "name" | "type" | "form"));
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
    if radios {
        let buttons: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&node| {
                document.tag(node) == Some("input")
                    && document
                        .attr(node, "type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
            })
            .collect();
        for button in buttons {
            out.mark(document, button, Reach::Element);
        }
    }
    out
}

/// Whether `node` or a node under it is an element named `tag`.
fn holds(document: &Document, node: NodeId, tag: &str) -> bool {
    document
        .subtree(node)
        .any(|node| document.tag(node) == Some(tag))
}

/// The `select` that is `node` or its nearest ancestor that is one.
fn select_around(document: &Document, node: NodeId) -> Option<NodeId> {
    std::iter::once(node)
        .chain(document.ancestors(node))
        .find(|&node| document.tag(node) == Some("select"))
}

/// What a child inserted under or removed from `parent` restyles for
/// the pseudo-classes that read past an element (M16): the parent's
/// subtree when it is a `fieldset` and `:disabled` or `:enabled` is in
/// use, since its first legend may have changed; the subtree of the
/// `select` around it when `:checked` is, since its options may have.
fn around_child(
    document: &Document,
    features: &SelectorFeatures,
    parent: NodeId,
    out: &mut Invalidation,
) {
    if features.fieldsets && document.tag(parent) == Some("fieldset") {
        out.mark(document, parent, Reach::Subtree);
    }
    if features.checked
        && let Some(select) = select_around(document, parent)
    {
        out.mark(document, select, Reach::Subtree);
    }
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
    use crate::computed::Display;
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
            assert_eq!(
                styles.pseudo_styles(node),
                full.pseudo_styles(node),
                "{:?}'s pseudo-elements",
                document.tag(node)
            );
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
    fn an_element_becoming_the_root_is_restyled_as_the_root() {
        // Two top-level elements: the span is inline until the div
        // before it goes, and the root's `display` is blockified.
        let mut document = Document::from_node(&dom::Node::document(vec![
            dom::Element::new("div").into(),
            dom::Element::new("span").into(),
        ]));
        let sheets = Stylesheets::new(vec![css::parse_stylesheet("span { color: #f00 }").unwrap()]);
        let mut styles = sheets.cascade(&document, &CascadeContext::default());
        let root = document.root();
        let (div, span) = (
            document.first_child(root).unwrap(),
            document.last_child(root).unwrap(),
        );
        assert_eq!(styles.get(span).unwrap().display, Display::Inline);
        let restyle = change(&mut document, &sheets, &mut styles, |d| {
            d.remove_child(root, div).unwrap();
        });
        assert!(!restyle.full);
        assert_eq!(styles.get(span).unwrap().display, Display::Block);
        // And put back, the span is an element like any other again.
        change(&mut document, &sheets, &mut styles, |d| {
            d.insert_before(root, div, Some(span)).unwrap();
        });
        assert_eq!(styles.get(span).unwrap().display, Display::Inline);
    }

    #[test]
    fn the_features_are_gathered_from_every_selector_nested_ones_included() {
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                ".a #b [c] { x: y } :not(.d) :is(#e) :where([f]) { x: y }
                 :nth-child(2 of .g) { x: y } p ~ q { x: y } :empty { x: y }
                 .h::before:not(:hover), [i] .j:after { x: y }",
            )
            .unwrap(),
        ]);
        let f = sheets.features();
        // A pseudo-element's originating element is matched as any
        // element is, so its selector's features count.
        for class in ["a", "d", "g", "h", "j"] {
            assert!(f.classes.contains(class), "{class}");
        }
        assert!(f.ids.contains("b") && f.ids.contains("e"));
        assert!(["c", "f", "i"].iter().all(|a| f.attributes.contains(*a)));
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
    // ---- M16: the pseudo-classes that read past an element ---------------

    /// The nodes, in no order: elements one depth apart are restyled in
    /// no order.
    fn set(nodes: &[NodeId]) -> HashSet<NodeId> {
        nodes.iter().copied().collect()
    }

    #[test]
    fn the_new_pseudo_classes_count_what_they_read() {
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet(":lang(en) {} :any-link {} :enabled {} :checked {} :optional {}")
                .unwrap(),
        ]);
        let f = sheets.features();
        assert!(f.lang && f.fieldsets && f.checked);
        for name in [
            "lang", "href", "disabled", "checked", "selected", "type", "multiple", "size",
            "required",
        ] {
            assert!(f.attributes.contains(name), "{name}");
        }
        // The UA sheet's `:link` reads `href` without the rest.
        let plain = Stylesheets::new(Vec::new());
        let f = plain.features();
        assert!(f.attributes.contains("href"));
        assert!(!f.lang && !f.fieldsets && !f.checked);
    }

    #[test]
    fn a_lang_change_restyles_the_subtree_and_a_pragma_change_the_document() {
        let (mut d, sheets, mut styles) = setup(
            "<meta id=m http-equiv=content-language content=fr>\
             <div id=x><p>one</p></div><p id=y>two</p>",
            ":lang(de) { color: #f00 } :lang(fr) { margin: 0 }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "lang", "de").unwrap();
        });
        assert!(!r.full);
        assert_eq!(r.computed, 2, "the div and its paragraph");
        let m = by_id(&d, "m");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(m, "content", "de").unwrap();
        });
        assert!(r.full, "the pragma's content");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(m, "http-equiv", "refresh").unwrap();
        });
        assert!(r.full, "the pragma's name");
        let head = d.parent(m).unwrap();
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_child(head, m).unwrap()
        });
        assert!(r.full, "a meta removed");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.append_child(head, m).unwrap()
        });
        assert!(r.full, "a meta inserted");
        // Anything else inserted, a meta's other attributes and a meta
        // without `:lang()` in use cascade nothing whole.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let p = d.create_element("p").unwrap();
            d.append_child(x, p).unwrap();
            d.set_attribute(m, "name", "x").unwrap();
        });
        assert!(!r.full);
        let (mut d, sheets, mut styles) =
            setup("<meta id=m http-equiv=content-language content=fr>", "p {}");
        let m = by_id(&d, "m");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(m, "content", "de").unwrap();
        });
        assert!(!r.full);
    }

    #[test]
    fn a_freed_node_may_have_held_a_meta() {
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p id=y>one</p></div>",
            ":lang(de) { color: #f00 }",
        );
        let (x, y) = (by_id(&d, "x"), by_id(&d, "y"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_child(x, y).unwrap();
            d.free(y).unwrap();
        });
        assert!(r.full);
    }

    #[test]
    fn a_child_of_a_fieldset_inserted_or_removed_restyles_the_fieldset() {
        let (mut d, sheets, mut styles) = setup(
            "<fieldset id=f disabled><legend id=l><input id=a></legend><input id=b></fieldset>\
             <div id=d><input></div>",
            "input:disabled { color: #f00 }",
        );
        let (f, l, a) = (by_id(&d, "f"), by_id(&d, "l"), by_id(&d, "a"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let legend = d.create_element("legend").unwrap();
            d.insert_before(f, legend, Some(l)).unwrap();
        });
        assert!(!r.full);
        assert!(r.changed.contains(&a), "a is no longer in the first legend");
        let first = d.first_child(f).unwrap();
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_child(f, first).unwrap()
        });
        assert!(r.changed.contains(&a), "a is in the first legend again");
        // Under anything but a fieldset, nothing more than before.
        let div = by_id(&d, "d");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let legend = d.create_element("legend").unwrap();
            d.append_child(div, legend).unwrap();
        });
        assert_eq!(r.computed, 1, "the legend alone");
    }

    #[test]
    fn an_option_changed_or_moved_restyles_its_select() {
        let (mut d, sheets, mut styles) = setup(
            "<select id=s><optgroup id=g><option id=a>1</option></optgroup>\
             <option id=b>2</option></select><p>x</p>",
            "option:checked { color: #f00 }",
        );
        let (a, b, g) = (by_id(&d, "a"), by_id(&d, "b"), by_id(&d, "g"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(b, "selected", "").unwrap();
        });
        assert_eq!(set(&r.changed), set(&[a, b]), "a unselected, b selected");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_attribute(b, "selected").unwrap();
        });
        assert_eq!(set(&r.changed), set(&[a, b]), "a the first option again");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(g, "disabled", "").unwrap();
        });
        assert_eq!(set(&r.changed), set(&[a, b]), "b the first not disabled");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_attribute(g, "disabled").unwrap();
            d.set_attribute(a, "disabled", "").unwrap();
        });
        assert!(r.changed.is_empty(), "a's disabled, b stays the first");
        let s = by_id(&d, "s");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            let option = d.create_element("option").unwrap();
            d.insert_before(s, option, Some(g)).unwrap();
        });
        assert!(r.changed.contains(&b), "the new first option takes it");
        assert!(!r.full);
    }

    #[test]
    fn a_radio_buttons_group_is_restyled_with_it() {
        let (mut d, sheets, mut styles) = setup(
            "<form id=f><input id=a type=radio name=r checked><p><input id=b type=radio name=r></p>\
             </form><input id=c type=radio name=r checked form=g><p>x</p>",
            ":checked { color: #f00 }",
        );
        let (a, b, c, f) = (
            by_id(&d, "a"),
            by_id(&d, "b"),
            by_id(&d, "c"),
            by_id(&d, "f"),
        );
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(b, "checked", "").unwrap();
        });
        assert_eq!(set(&r.changed), set(&[a, b]), "b checked, a unchecked");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(b, "name", "other").unwrap();
        });
        assert_eq!(r.changed, [a], "a checked again");
        // `c` names form `g`, which is none: alone, and checked. The form
        // taking that ID takes `c` into `a`'s group, after it.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(f, "id", "g").unwrap();
        });
        assert_eq!(r.changed, [a]);
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(c, "type", "checkbox").unwrap();
        });
        assert!(r.changed.contains(&a));
        // An element inserted anywhere may change a group: made aside
        // first, so that its insertion is the one change.
        let mut input = None;
        change(&mut d, &sheets, &mut styles, |d| {
            let made = d.create_element("input").unwrap();
            for (name, value) in [
                ("type", "radio"),
                ("name", "r"),
                ("checked", ""),
                ("form", "g"),
            ] {
                d.set_attribute(made, name, value).unwrap();
            }
            input = Some(made);
        });
        let body = d.parent(c).unwrap();
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.append_child(body, input.unwrap()).unwrap();
        });
        assert!(r.changed.contains(&a), "a unchecked by the button after it");
        assert!(!r.full);
    }
    // ---- M16: pseudo-element styles ------------------------------------------

    #[test]
    fn content_attr_names_count_as_read() {
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                "p::before { content: 'a' attr( Data-X ) attr(title) } \
                 p { color: attr(nope) } .a { content: attr(other) }",
            )
            .unwrap(),
        ]);
        let f = sheets.features();
        for name in ["data-x", "title", "other"] {
            assert!(f.attributes.contains(name), "{name}");
        }
        assert!(!f.attributes.contains("nope"), "only `content` reads one");
    }

    #[test]
    fn a_change_to_a_pseudo_elements_style_is_one_to_its_element() {
        let (mut d, sheets, mut styles) = setup(
            "<p id=x>one</p><p>two</p>",
            ".b::before { content: '!' } p::after { content: attr(title) }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "b").unwrap();
        });
        assert_eq!(r.changed, [x], "its ::before, not its own style");
        assert!(
            styles
                .pseudo(x, crate::selector_matching::PseudoElement::Before)
                .is_some()
        );
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "title", "t").unwrap();
        });
        assert_eq!(r.changed, [x], "its ::after's content");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "lang", "fr").unwrap();
        });
        assert!(r.changed.is_empty(), "nothing reads `lang`");
    }
    #[test]
    fn selectors_inside_media_supports_and_layers_are_read_whatever_the_media() {
        // `@media print` does not hold on the default screen, but its
        // selectors count: a resize or a print can bring them in force.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><p>one</p></div>",
            "@media all { @layer a { [data-on] p { color: #f00 } } }\
             @supports (color: red) { .lit p { color: #0f0 } }\
             @media print { [data-print] p { color: #00f } }\
             @media all { p::before { content: attr(data-note) } }",
        );
        let x = by_id(&d, "x");
        let p = d.subtree(x).nth(1).expect("the p");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "data-on", "").unwrap();
        });
        assert_eq!(r.changed, [p]);
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "lit").unwrap();
        });
        assert_eq!(r.changed, [p]);
        assert!(sheets.features().attributes.contains("data-print"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(p, "data-note", "!").unwrap();
        });
        assert_eq!(r.changed, [p], "its ::before's content");
    }
}
