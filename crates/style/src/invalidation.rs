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
//! an `attr()` reads in any declaration (M21.5). An `attr()` whose name
//! is substituted can read any attribute: a sheet with one makes every
//! attribute count. An inline style with an `attr()` in it makes any
//! change to its element's attributes restyle the element.
//!
//! The changes are read after they were all made, so each rule looks
//! at the document as it now stands: an element no longer in the tree
//! is not restyled, an attribute is compared between the value it had
//! before the first of its changes and the value it has now, and a
//! change point whose neighbours have since moved is taken to be
//! anywhere among the parent's children. Restyling more than needed is
//! always correct; this is where precision is traded for simplicity.

use std::collections::{HashMap, HashSet};

use dom::{Change, Document, ElementState, NodeId};

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
    /// `:checked`, `:indeterminate` or `:placeholder-shown`: a select's
    /// options and radio button groups count, and the controls' state
    /// (M26.7).
    checked: bool,
    /// An `attr()` whose name is substituted (M21.5): every attribute
    /// counts.
    any_attribute: bool,
    /// The element state flags the state pseudo-classes read (M26.4), by
    /// where they stand: in a selector's subject compound, which a change
    /// restyles the element for; in a compound a child or descendant
    /// combinator follows, its subtree; one a sibling combinator follows,
    /// its later siblings.
    states_subject: u8,
    states_ancestor: u8,
    states_sibling: u8,
}

/// Where in a selector a simple selector stands, for the state
/// pseudo-classes' invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    Subject,
    Ancestor,
    Sibling,
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
            let last = selector.compounds.len().saturating_sub(1);
            for (i, compound) in selector.compounds.iter().enumerate() {
                let at = if i == last {
                    At::Subject
                } else if matches!(
                    selector.combinators.get(i),
                    Some(Combinator::NextSibling | Combinator::LaterSibling)
                ) {
                    At::Sibling
                } else {
                    At::Ancestor
                };
                for simple in &compound.simple {
                    self.add_simple(simple, at);
                }
            }
            // The pseudo-classes after a pseudo-element read the
            // originating element's state (M26.4).
            if let Some(ending) = &selector.pseudo_element {
                for state in &ending.states {
                    self.add_pseudo_class(state, At::Subject);
                }
            }
        }
    }

    /// Adds what `list`'s selectors depend on, each standing at `at` in
    /// the selector around them: the state pseudo-classes in a nested
    /// list count where it stands.
    fn add_nested(&mut self, list: &SelectorList, at: At) {
        for selector in &list.0 {
            for compound in &selector.compounds {
                for simple in &compound.simple {
                    self.add_simple(simple, at);
                }
            }
            for combinator in &selector.combinators {
                if matches!(
                    combinator,
                    Combinator::NextSibling | Combinator::LaterSibling
                ) {
                    self.sibling_combinators = true;
                }
            }
        }
    }

    fn add_simple(&mut self, simple: &SimpleSelector, at: At) {
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
            SimpleSelector::PseudoClass(pc) => self.add_pseudo_class(pc, at),
        }
    }

    fn add_pseudo_class(&mut self, pc: &PseudoClass, at: At) {
        match pc {
            PseudoClass::FirstChild | PseudoClass::FirstOfType | PseudoClass::NthOfType { .. } => {
                self.counts_from_start = true;
            }
            PseudoClass::NthChild { of: None, .. } => self.counts_from_start = true,
            PseudoClass::NthChild { of: Some(list), .. } => {
                self.every_sibling = true;
                self.add_nested(list, At::Sibling);
            }
            PseudoClass::LastChild
            | PseudoClass::LastOfType
            | PseudoClass::NthLastChild { .. }
            | PseudoClass::NthLastOfType { .. } => self.counts_from_end = true,
            PseudoClass::OnlyChild | PseudoClass::OnlyOfType => self.every_sibling = true,
            PseudoClass::Empty => self.empty = true,
            PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => {
                self.add_nested(list, at);
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
            PseudoClass::Indeterminate => {
                self.checked = true;
                self.read(&["checked", "type", "name", "form", "value"]);
            }
            PseudoClass::PlaceholderShown => {
                self.checked = true;
                self.read(&["placeholder", "value", "type"]);
            }
            PseudoClass::Required | PseudoClass::Optional => self.read(&["required", "type"]),
            PseudoClass::Hover => self.read_state(ElementState::HOVER, at),
            PseudoClass::Focus => self.read_state(ElementState::FOCUS, at),
            PseudoClass::FocusVisible => self.read_state(ElementState::FOCUS_VISIBLE, at),
            PseudoClass::FocusWithin => self.read_state(ElementState::FOCUS_WITHIN, at),
            PseudoClass::Active => self.read_state(ElementState::ACTIVE, at),
            PseudoClass::Target => self.read_state(ElementState::TARGET, at),
            // `:root` (and `:scope`, which is `:root` in a style rule)
            // depends on the parent, which an insertion restyles whole;
            // `:visited` never matches.
            PseudoClass::Root | PseudoClass::Scope | PseudoClass::Visited => {}
        }
    }

    /// Counts the state flag `flag` as read at `at`.
    fn read_state(&mut self, flag: ElementState, at: At) {
        match at {
            At::Subject => self.states_subject |= flag.0,
            At::Ancestor => self.states_ancestor |= flag.0,
            At::Sibling => self.states_sibling |= flag.0,
        }
    }

    /// Adds what `declarations`, a style rule's, read: the attributes
    /// the `attr()`s in them name, of any property (M21.5), on the
    /// element, or the one a pseudo-element comes from.
    pub(crate) fn add_declarations(&mut self, declarations: &[css::Declaration]) {
        for declaration in declarations {
            match attributes_read(&declaration.value) {
                Some(names) => self.attributes.extend(names),
                None => self.any_attribute = true,
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
        if self.any_attribute || self.attributes.contains(name) {
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

/// The names of the attributes the `attr()`s in `tokens` read, ASCII
/// lowercased, `ns|name` and `|name` counted as `name`; `None` when one
/// reads a name substituted into it, which may be any.
fn attributes_read(tokens: &[css::Token]) -> Option<Vec<String>> {
    use css::Token;
    let mut names = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let Token::Function(function) = token else {
            continue;
        };
        if !function.eq_ignore_ascii_case("attr") {
            continue;
        }
        let mut rest = tokens[at + 1..]
            .iter()
            .filter(|t| !matches!(t, Token::Whitespace | Token::OpenCurly));
        let name = match (rest.next(), rest.next(), rest.next()) {
            (Some(Token::Delim('|')), Some(Token::Ident(name)), _) => name,
            (Some(Token::Ident(_)), Some(Token::Delim('|')), Some(Token::Ident(name))) => name,
            (Some(Token::Ident(name)), _, _) => name,
            // A name substituted, or none, which no attribute's change
            // can make valid.
            (Some(Token::Function(_)), _, _) => return None,
            _ => continue,
        };
        names.push(name.to_ascii_lowercase());
    }
    Some(names)
}

/// Whether `element`'s inline style has an `attr()` in it (M21.5): then
/// a change to any of its attributes can change its style.
fn inline_reads_attributes(document: &Document, element: NodeId) -> bool {
    document.attr(element, "style").is_some_and(|style| {
        css::tokenize(style)
            .iter()
            .any(|t| matches!(t, css::Token::Function(name) if name.eq_ignore_ascii_case("attr")))
    })
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
    // Each element's state before the first of its changes.
    let mut states: HashMap<NodeId, ElementState> = HashMap::new();
    // Whether every radio button is to be restyled (`:checked`).
    let mut radios = false;
    // The controls whose state changed (M26.7).
    let mut controls: Vec<NodeId> = Vec::new();
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
            Change::AttributeChanged { .. }
            | Change::TextChanged { .. }
            | Change::StateChanged { .. }
            | Change::ControlChanged { .. } => {}
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
            Change::StateChanged { element, old } => {
                states.entry(*element).or_insert(*old);
            }
            Change::ControlChanged { element } => controls.push(*element),
        }
    }
    // A control's state restyles what reads it (M26.7): the element, as
    // far as an attribute's change would, and an option's select.
    if features.checked {
        for element in controls {
            out.mark(document, element, Reach::Subtree);
            if document.tag(element) == Some("option")
                && let Some(select) = select_around(document, element)
            {
                out.mark(document, select, Reach::Subtree);
            }
            if let Some(parent) = document.parent(element)
                && (features.every_sibling || features.sibling_combinators)
            {
                for sibling in document.element_children(parent) {
                    out.mark(document, sibling, Reach::Subtree);
                }
            }
        }
    }
    // A state flag's change restyles as far as the selectors that read
    // it reach (M26.4): the element, its subtree, its later siblings.
    for (element, old) in states {
        let changed = old.0 ^ document.state(element).0;
        if changed & features.states_ancestor != 0 {
            out.mark(document, element, Reach::Subtree);
        } else if changed & features.states_subject != 0 {
            out.mark(document, element, Reach::Element);
        }
        if changed & features.states_sibling != 0 {
            if let Some(parent) = document.parent(element)
                && features.every_sibling
            {
                for sibling in document.element_children(parent) {
                    out.mark(document, sibling, Reach::Subtree);
                }
            } else {
                let mut at = document.next_element_sibling(element);
                while let Some(sibling) = at {
                    out.mark(document, sibling, Reach::Subtree);
                    at = document.next_element_sibling(sibling);
                }
            }
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
        } else if name == "style"
            || name == "dir"
            || tag.is_some_and(|tag| {
                crate::cascade::is_list_hint(tag, name)
                    || crate::cascade::is_img_hint(tag, name)
                    || crate::cascade::is_control_hint(tag, name)
            })
            || inline_reads_attributes(document, element)
        {
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
    use std::sync::Arc;

    use super::*;
    use crate::computed::Display;
    use crate::selector_matching::PseudoElement;
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
    fn a_change_of_a_transform_or_an_opacity_alone_is_composited() {
        // M22.4: the restyle sorts the changes it makes. One of the
        // composited properties alone, which leaves the box what it was
        // to layout, is listed with the effects it replaces and its new
        // ones, an element's or a pseudo-element's, and the element is
        // not listed as changed; any other is a change.
        let (mut document, sheets, mut styles) = setup(
            "<div id=d><p id=p>text</p></div><div id=e></div><section id=f></section>\
             <section id=g></section>",
            "div { rotate: 0deg; opacity: 0.9 } .turned { rotate: 30deg } .faded { opacity: 0.5 } \
             .wide { width: 10px } .flat { rotate: none } .origin { transform-origin: 0 0 } \
             div::after { content: 'z' } #e { opacity: 1 } #e.faded { opacity: 0.5 } \
             #f { transform-style: preserve-3d; will-change: opacity } #f.faded { opacity: 0.5 } \
             #g { will-change: transform } #g.turned { rotate: 30deg } \
             p::before { content: 'x'; translate: 0 0 } .turned p::before { translate: 0 5px }",
        );
        let [d, p, e, f, g] = ["d", "p", "e", "f", "g"].map(|id| by_id(&document, id));
        let effects = |styles: &Styles| styles.get(d).unwrap().effects.clone().unwrap();
        let old = effects(&styles);
        let restyle = change(&mut document, &sheets, &mut styles, |doc| {
            doc.set_attribute(d, "class", "turned").unwrap();
        });
        assert!(restyle.changed.is_empty(), "{restyle:?}");
        let sorted: Vec<_> = restyle
            .composited
            .iter()
            .map(|c| (c.node, c.pseudo, c.transformed))
            .collect();
        assert_eq!(
            sorted,
            vec![(d, None, true), (p, Some(PseudoElement::Before), true)]
        );
        assert!(Arc::ptr_eq(&restyle.composited[0].old, &old));
        assert!(Arc::ptr_eq(&restyle.composited[0].new, &effects(&styles)));
        // An opacity's change changes no transform.
        let restyle = change(&mut document, &sheets, &mut styles, |doc| {
            doc.set_attribute(d, "class", "turned faded").unwrap();
        });
        let sorted: Vec<_> = restyle
            .composited
            .iter()
            .map(|c| (c.node, c.transformed))
            .collect();
        assert_eq!((restyle.changed, sorted), (Vec::new(), vec![(d, false)]));
        // A width is read by layout; a transform's origin, outside the
        // composited properties, is not composited; with no transform
        // the box is no longer a containing block.
        for class in ["turned origin", "turned faded wide", "turned", "flat"] {
            let restyle = change(&mut document, &sheets, &mut styles, |doc| {
                doc.set_attribute(d, "class", class).unwrap();
            });
            assert_eq!(restyle.changed, vec![d], "{class}");
            assert!(restyle.composited.iter().all(|c| c.node != d), "{class}");
        }
        // An opacity from 1 makes a stacking context of a box no
        // transform applies to, an inline box's, which the cascade
        // cannot tell from another; it flattens a box's 3D content; and
        // a transform makes a containing block, whatever `will-change`
        // makes the box already.
        for (element, class) in [(e, "faded"), (f, "faded"), (g, "turned")] {
            let restyle = change(&mut document, &sheets, &mut styles, |doc| {
                doc.set_attribute(element, "class", class).unwrap();
            });
            assert_eq!(restyle.changed, vec![element], "{class}");
        }
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
            // Not `title`, which the user agent sheet's `abbr[title]`
            // names (M21.7).
            d.set_attribute(x, "data-t", "t").unwrap();
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
    fn a_state_restyles_as_far_as_the_selectors_that_read_it_reach() {
        let (mut d, sheets, mut styles) = setup(
            "<div id=a><p id=b>x</p><p id=c>y</p></div>\
             <div id=d><span id=e>z</span></div><p id=f>w</p>",
            "#b:hover { color: #f00 } :is(#d:hover) span { color: #00f } \
             #b:focus + p { color: #0f0 } p::before:focus-within { content: 'x' }              #c:target { color: #0ff }",
        );
        let set = |d: &mut Document, id: &str, state: ElementState| {
            let node = by_id(d, id);
            d.set_state(node, state).unwrap();
        };
        // In the subject: the element alone, and its children only when
        // its style changes, which #a's does not.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "a", ElementState::TARGET)
        });
        assert_eq!((r.computed, r.changed.len()), (1, 0));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "b", ElementState::HOVER)
        });
        assert_eq!((r.computed, r.changed.len()), (1, 1));
        // Before a descendant combinator: the subtree.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "d", ElementState::HOVER)
        });
        assert_eq!((r.computed, r.changed.len()), (2, 1));
        // Before a sibling combinator: the later siblings, the element
        // itself not, since no subject reads that flag.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "b", ElementState::HOVER.with(ElementState::FOCUS, true))
        });
        assert_eq!((r.computed, r.changed.len()), (1, 1));
        // After a pseudo-element: its element.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "f", ElementState::FOCUS_WITHIN)
        });
        assert_eq!(r.computed, 1);
        // A flag no selector reads restyles nothing.
        let (mut d, sheets, mut styles) = setup("<div id=a><p>x</p></div>", "p { color: #f00 }");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            set(d, "a", ElementState::HOVER.with(ElementState::TARGET, true))
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
    fn a_list_attribute_restyles_its_element() {
        // HTML's list hints (M21.6): `start` on an `ol` resets
        // `list-item`, and `type` on an `li` gives its style, which its
        // marker inherits.
        let (mut d, sheets, mut styles) = setup("<ol id=x><li id=y>a</li></ol><p id=z>b</p>", "");
        let (x, y, z) = (by_id(&d, "x"), by_id(&d, "y"), by_id(&d, "z"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "start", "4").unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (2, 1));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(y, "type", "I").unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (1, 1));
        // On another element it is no hint.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(z, "start", "4").unwrap();
        });
        assert_eq!(r.computed, 0);
    }

    #[test]
    fn an_img_dimension_attribute_restyles_its_element() {
        // HTML's dimension hints (M23.2): `width` and `height` on an
        // `img` are its `width`, `height` and `aspect-ratio`.
        let (mut d, sheets, mut styles) = setup("<img id=x><p id=z>b</p>", "");
        let (x, z) = (by_id(&d, "x"), by_id(&d, "z"));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "width", "40").unwrap();
        });
        assert_eq!((r.computed, r.changed.len()), (1, 1));
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "alt", "a").unwrap();
        });
        assert_eq!(r.computed, 0);
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(z, "height", "4").unwrap();
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
        // The UA sheet's `:link` reads `href`, and its form controls'
        // `:disabled` and a list box's `:checked` theirs (M26.7), without
        // the rest.
        let plain = Stylesheets::new(Vec::new());
        let f = plain.features();
        assert!(f.attributes.contains("href"));
        assert!(f.fieldsets && f.checked);
        assert!(!f.lang);
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
        // HTML's checkedness (M26.7): b's checking unchecked a, and b
        // leaving the group gives a nothing back.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(b, "name", "other").unwrap();
        });
        assert!(r.changed.is_empty(), "a stays unchecked");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(b, "name", "r").unwrap();
            d.remove_attribute(b, "checked").unwrap();
            // Added again: a has had it all along.
            d.remove_attribute(a, "checked").unwrap();
            d.set_attribute(a, "checked", "").unwrap();
        });
        assert_eq!(set(&r.changed), set(&[a, b]), "a checked again, b not");
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
    fn attr_names_count_as_read() {
        // Of any property since M21.5, in any form of the name.
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                "p::before { content: 'a' attr( Data-X ) attr(title) } \
                 p { color: attr(colour type(<color>)); width: attr({w} px, 0) } \
                 .a { content: attr(other); --c: attr(|plain) attr(svg|ns) }",
            )
            .unwrap(),
        ]);
        let f = sheets.features();
        for name in ["data-x", "title", "colour", "w", "other", "plain", "ns"] {
            assert!(f.attributes.contains(name), "{name}");
        }
        assert!(!f.any_attribute);
        // A substituted name may be any.
        let sheets = Stylesheets::new(vec![
            css::parse_stylesheet("p { --n: data-x; width: attr(var(--n) px) }").unwrap(),
        ]);
        assert!(sheets.features().any_attribute);
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

    #[test]
    fn a_custom_property_changed_restyles_what_inherits_it() {
        // M21.1: a custom property is inherited; a change to one reaches
        // the descendants whose styles it reaches, and restyles them as a
        // full cascade would style them.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x><section><p>one</p></section><p>two</p></div>",
            ".dark { --fg: #fff } p { color: var(--fg, #000) } \
             section { --pad: 1px } p { padding: var(--pad, 0) }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "class", "dark").unwrap();
        });
        assert_eq!(
            r.changed.len(),
            4,
            "the div, the section and the paragraphs"
        );
        let p = d
            .subtree(x)
            .find(|&n| d.tag(n) == Some("p"))
            .expect("a paragraph");
        assert_eq!(
            styles.get(p).unwrap().color,
            crate::computed::Color::rgb8(255, 255, 255)
        );
        // Through an inline style, and back.
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "style", "--fg: #0f0").unwrap();
        });
        assert_eq!(r.changed.len(), 4);
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.remove_attribute(x, "style").unwrap();
            d.remove_attribute(x, "class").unwrap();
        });
        assert_eq!(r.changed.len(), 4);
    }

    #[test]
    fn an_attribute_an_attr_reads_restyles_its_element() {
        // M21.5: in any property, a custom one's included, whose users
        // are restyled through it.
        let (mut d, sheets, mut styles) = setup(
            "<div id=x data-w=10><p>one</p></div>",
            "div { width: attr(data-w px); --c: attr(data-c type(<color>), red) } \
             p { color: var(--c) }",
        );
        let x = by_id(&d, "x");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "data-w", "20").unwrap();
        });
        assert_eq!(r.changed, [x]);
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(x, "data-c", "blue").unwrap();
        });
        assert_eq!(r.changed.len(), 2, "the div and its paragraph");
        // An inline style's `attr()`.
        let (mut d, sheets, mut styles) =
            setup("<p id=y style='width: attr(data-w px)' data-w=5>x</p>", "");
        let y = by_id(&d, "y");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(y, "data-w", "6").unwrap();
        });
        assert_eq!(r.changed, [y]);
        // A name substituted: any attribute.
        let (mut d, sheets, mut styles) = setup(
            "<p id=z data-n=data-w data-w=5>x</p>",
            "p { --n: attr(data-n type(<custom-ident>)); width: attr(var(--n) px) }",
        );
        let z = by_id(&d, "z");
        let r = change(&mut d, &sheets, &mut styles, |d| {
            d.set_attribute(z, "data-w", "7").unwrap();
        });
        assert_eq!(r.changed, [z]);
    }
}
