//! Focus (M26.6, `docs/design/events.md`, HTML section 6.6): which
//! elements can take the focus, and in what order Tab moves it, for the
//! engine and its scripts' host alike. Shadow trees (gap batch H, D68)
//! are focus navigation scopes of their own, as their slots are, and a
//! host that delegates focus gives it to the first area in its tree.

use std::collections::HashMap;

use dom::{Document, NodeId, forms};
use style::Styles;
use style::computed::Display;

/// The `tabindex` an element's attribute gives it, if it parses (HTML's
/// rules for parsing integers).
pub(crate) fn tab_index(document: &Document, element: NodeId) -> Option<i32> {
    document
        .attr(element, "tabindex")
        .and_then(dom::parse_integer)
        .and_then(|n| i32::try_from(n).ok())
}

/// Whether the element can take the focus as it is, with no `tabindex`
/// (HTML section 6.6.3): a link, a control that is not disabled, an
/// `input` that is not hidden.
pub(crate) fn natively_focusable(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("a" | "area") => document.attr(element, "href").is_some(),
        Some("input") => {
            !document
                .attr(element, "type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
                && !forms::actually_disabled(document, element)
        }
        Some("button" | "select" | "textarea") => !forms::actually_disabled(document, element),
        // A frame's element, the focus passing into its document (M33.2).
        Some("iframe") => true,
        // A media element with controls, whose keys play, pause and skip
        // (M40), as the reference's.
        Some("video" | "audio") => document.attr(element, "controls").is_some(),
        // The summary of its `details`, which it opens and closes (gap
        // batch H).
        Some("summary") => document.summary_details(element).is_some(),
        _ => false,
    }
}

/// Whether `element` is in the content of a `details` element that is
/// not open, its summary aside, which is not rendered (HTML section
/// 15.5.1).
fn in_closed_details(document: &Document, element: NodeId) -> bool {
    let mut at = element;
    while let Some(parent) = document.parent(at) {
        if document.tag(parent) == Some("details")
            && document.attr(parent, "open").is_none()
            && document.summary_details(at) != Some(parent)
        {
            return true;
        }
        at = parent;
    }
    false
}

/// Whether the element is a focusable area (HTML section 6.6.2): one
/// that can take the focus as it is, or has a `tabindex`, and is being
/// rendered, by `styles`, which say it is when they are not given; a
/// disabled control does not take it whatever its `tabindex`, nor does
/// an inert element (gap batch H). An open `dialog` takes it too, as the
/// reference browser's does, though neither Tab nor a click gives it
/// ([`open_dialog_alone`]).
pub(crate) fn focusable(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    if !document.is_element(element) || !rendered(document, styles, element) {
        return false;
    }
    if styles
        .and_then(|styles| styles.get(element))
        .is_some_and(|style| style.inert)
    {
        return false;
    }
    if forms::can_be_disabled(document, element) && forms::actually_disabled(document, element) {
        return false;
    }
    natively_focusable(document, element)
        || tab_index(document, element).is_some()
        || is_open_dialog(document, element)
}

/// Whether the element is a `dialog` with an `open` attribute.
fn is_open_dialog(document: &Document, element: NodeId) -> bool {
    document.tag(element) == Some("dialog") && document.attr(element, "open").is_some()
}

/// Whether the element is focusable only as an open dialog, with no
/// `tabindex`: what the dialog focusing steps and `focus()` focus, and
/// Tab and a click pass over.
fn open_dialog_alone(document: &Document, element: NodeId) -> bool {
    is_open_dialog(document, element) && tab_index(document, element).is_none()
}

/// Whether the element is a focusable area that takes the focus itself:
/// [`focusable`], and not a shadow host that delegates focus, whose
/// focus goes to its focus delegate (D68).
pub(crate) fn focus_area(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    !delegates_focus(document, element) && focusable(document, styles, element)
}

/// Whether `element` hosts a shadow root that delegates focus.
pub(crate) fn delegates_focus(document: &Document, element: NodeId) -> bool {
    document
        .shadow_root(element)
        .and_then(|root| document.shadow_init(root))
        .is_some_and(|init| init.delegates_focus)
}

/// Whether the element is being rendered: in the document's flat tree
/// (D68), it and every ancestor there with a style other than
/// `display: none`.
fn rendered(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    if !document.in_flat_tree(element) || in_closed_details(document, element) {
        return false;
    }
    let Some(styles) = styles else {
        return true;
    };
    // An element of `display: contents` has no box, but takes the focus
    // as the reference browser's does; a slot so, as by default (D69),
    // does not, its focus scope standing for it.
    if document.is_slot(element)
        && styles
            .get(element)
            .is_some_and(|style| style.display == Display::Contents)
    {
        return false;
    }
    std::iter::once(element)
        .chain(document.flat_ancestors(element))
        .filter(|&node| document.is_element(node))
        .all(|node| {
            styles
                .get(node)
                .is_some_and(|style| style.display != Display::None)
        })
}

/// HTML's "focus delegate" of `target` (section 6.6.3), for a shadow host
/// that delegates focus (D68) and for a dialog (section 4.11.4, gap batch
/// H); none for another element, or a host that does not delegate. Where
/// it looks is the host's shadow tree, or the dialog's own subtree: the
/// first element there with `autofocus` that is a focusable area or
/// delegates to one, failing which the first that is or does, in tree
/// order, as the reference browser finds them (a dialog's focusable
/// areas whatever their `tabindex`).
pub(crate) fn focus_delegate(
    document: &Document,
    styles: Option<&Styles>,
    target: NodeId,
) -> Option<NodeId> {
    let root = match document.shadow_root(target) {
        Some(root) if delegates_focus(document, target) => root,
        Some(_) => return None,
        None if document.tag(target) == Some("dialog") => target,
        None => return None,
    };
    let elements = elements_under(document, root);
    autofocus_among(document, styles, &elements).or_else(|| {
        elements
            .iter()
            .find_map(|&element| candidate(document, styles, element))
    })
}

/// HTML's "autofocus delegate" of `target`: the first element in its
/// subtree (its shadow tree, for a host) with `autofocus` that is a
/// focusable area or delegates focus to one: what a popover shown
/// focuses (section 6.12, gap batch H).
pub(crate) fn autofocus_delegate(
    document: &Document,
    styles: Option<&Styles>,
    target: NodeId,
) -> Option<NodeId> {
    let root = document.shadow_root(target).unwrap_or(target);
    autofocus_among(document, styles, &elements_under(document, root))
}

/// The elements under `root`, `root` aside, in tree order.
fn elements_under(document: &Document, root: NodeId) -> Vec<NodeId> {
    document
        .subtree(root)
        .skip(1)
        .filter(|&node| document.is_element(node))
        .collect()
}

/// The first of `elements` with `autofocus` that is a [`candidate`].
fn autofocus_among(
    document: &Document,
    styles: Option<&Styles>,
    elements: &[NodeId],
) -> Option<NodeId> {
    elements
        .iter()
        .filter(|&&element| document.attr(element, "autofocus").is_some())
        .find_map(|&element| candidate(document, styles, element))
}

/// `element` when it is a focusable area that takes the focus itself,
/// else its own focus delegate: a nested host's is searched for in its
/// own tree, which the DOM's depth cap bounds.
fn candidate(document: &Document, styles: Option<&Styles>, element: NodeId) -> Option<NodeId> {
    if focus_area(document, styles, element) && !open_dialog_alone(document, element) {
        return Some(element);
    }
    if document.shadow_root(element).is_some() {
        return focus_delegate(document, styles, element);
    }
    None
}

/// The focus navigation scope owner an element belongs to (HTML section
/// 6.6.3, "associated focus navigation owner"): a slotted element's
/// slot, and a slot's fallback content's, as the reference browser
/// scopes it; a shadow tree's host; the document; `None` for a host's
/// child no slot has, which is in no scope.
fn scope_owner(document: &Document, element: NodeId) -> Option<NodeId> {
    let mut at = element;
    loop {
        let parent = document.parent(at)?;
        if document.shadow_root(parent).is_some() {
            return document.assigned_slot(at);
        }
        if document.is_slot(parent) && document.is_shadow_root(document.root_of(parent)) {
            return Some(parent);
        }
        if let Some(host) = document.shadow_host(parent) {
            return Some(host);
        }
        if document.is_document(parent) || document.document_element() == Some(parent) {
            return Some(document.root());
        }
        at = parent;
    }
}

/// The sequential focus navigation order (HTML section 6.6.3): the
/// flattened tabindex-ordered focus navigation scope of the document.
/// Each scope's focusable areas and the scope owners in it (hosts and
/// slots), but those whose `tabindex` is negative, those with a positive
/// one first, by it and then tree order, then the rest in tree order;
/// each owner followed by its own scope's, or replaced by it when it is
/// no focusable area itself.
pub(crate) fn sequential_order(document: &Document, styles: Option<&Styles>) -> Vec<NodeId> {
    let scopes = scopes(document, styles);
    flattened(document, styles, &scopes, document.root())
}

/// The area Tab moves the focus to from `current`, an element focused
/// that is not in the [`sequential_order`] (its `tabindex` negative, or
/// in a scope whose owner's is, D68), as the reference browser finds it:
/// the next area of `current`'s own scope, or of the scope around that,
/// and so on out to the document's, an owner standing for the areas of
/// its scope; the previous, `backward`. `None` when there is none, or
/// `current` is in no scope.
pub(crate) fn navigate_from(
    document: &Document,
    styles: Option<&Styles>,
    current: NodeId,
    backward: bool,
) -> Option<NodeId> {
    let scopes = scopes(document, styles);
    let root = document.root();
    let mut from = current;
    // Each turn moves out a scope, which the DOM's depth cap bounds.
    while from != root {
        let owner = scope_owner(document, from)?;
        let in_tree_order = scopes.get(&owner).map_or(&[][..], Vec::as_slice);
        let items = ordered(document, in_tree_order);
        // After `from` in its scope's order when it is there, else after
        // it in tree order, whatever the items' `tabindex`.
        let candidates: Vec<NodeId> = match items.iter().position(|&item| item == from) {
            Some(at) if backward => items[..at].iter().rev().copied().collect(),
            Some(at) => items[at + 1..].to_vec(),
            None if backward => in_tree_order
                .iter()
                .copied()
                .filter(|&item| document.shadow_including_precedes(item, from))
                .rev()
                .collect(),
            None => in_tree_order
                .iter()
                .copied()
                .filter(|&item| document.shadow_including_precedes(from, item))
                .collect(),
        };
        for item in candidates {
            let inner = match scopes.contains_key(&item) {
                true => flattened(document, styles, &scopes, item),
                false => Vec::new(),
            };
            let area = focus_area(document, styles, item).then_some(item);
            let found = match backward {
                true => inner.last().copied().or(area),
                false => area.or_else(|| inner.first().copied()),
            };
            if found.is_some() {
                return found;
            }
        }
        from = owner;
    }
    None
}

/// Each focus navigation scope's items, by its owner (the document, a
/// host or a slot): its focusable areas and the owners of the scopes in
/// it, but those whose `tabindex` is negative, in tree order.
fn scopes(document: &Document, styles: Option<&Styles>) -> HashMap<NodeId, Vec<NodeId>> {
    let root = document.root();
    let nodes = match document.has_shadow_roots() {
        true => document.shadow_including_subtree(root),
        false => document.subtree(root).collect(),
    };
    let mut scopes: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for node in nodes {
        if !document.is_element(node) {
            continue;
        }
        let owner_kind = document.shadow_root(node).is_some()
            || (document.is_slot(node) && document.is_shadow_root(document.root_of(node)));
        // An open dialog is focusable, but not by Tab (gap batch H).
        if !owner_kind && (!focusable(document, styles, node) || open_dialog_alone(document, node))
        {
            continue;
        }
        if tab_index(document, node).is_some_and(|n| n < 0) {
            continue;
        }
        if let Some(owner) = scope_owner(document, node) {
            scopes.entry(owner).or_default().push(node);
        }
    }
    scopes
}

/// The flattened order of `owner`'s scope: its items in tabindex order,
/// each owner among them followed by its own scope's, flattened so.
fn flattened(
    document: &Document,
    styles: Option<&Styles>,
    scopes: &HashMap<NodeId, Vec<NodeId>>,
    owner: NodeId,
) -> Vec<NodeId> {
    let items_of = |owner: NodeId| -> std::vec::IntoIter<NodeId> {
        ordered(document, scopes.get(&owner).map_or(&[], Vec::as_slice)).into_iter()
    };
    let mut out = Vec::new();
    // Each entry: a scope's items still to write, in order.
    let mut stack: Vec<std::vec::IntoIter<NodeId>> = vec![items_of(owner)];
    while let Some(items) = stack.last_mut() {
        let Some(item) = items.next() else {
            stack.pop();
            continue;
        };
        if focus_area(document, styles, item) {
            out.push(item);
        }
        if scopes.contains_key(&item) && stack.len() <= dom::MAX_DEPTH {
            stack.push(items_of(item));
        }
    }
    out
}

/// `items`, a scope's, in tabindex order: those with a positive
/// `tabindex` first, by it, then the rest, each group in tree order.
fn ordered(document: &Document, items: &[NodeId]) -> Vec<NodeId> {
    let (mut positive, rest): (Vec<NodeId>, Vec<NodeId>) = items
        .iter()
        .partition(|&&node| tab_index(document, node).is_some_and(|n| n > 0));
    // A stable sort keeps tree order among equals.
    positive.sort_by_key(|&node| tab_index(document, node));
    positive.extend(rest);
    positive
}

/// The element a click focuses (HTML section 6.6.4's "focusable area"
/// of the event's target): the nearest inclusive ancestor in the flat
/// tree that is a focusable area, or a host that delegates focus, for
/// its focus delegate.
pub(crate) fn click_focus_target(
    document: &Document,
    styles: Option<&Styles>,
    node: NodeId,
) -> Option<NodeId> {
    let found = std::iter::once(node)
        .chain(document.flat_ancestors(node))
        .find(|&node| {
            delegates_focus(document, node)
                || (focusable(document, styles, node) && !open_dialog_alone(document, node))
        })?;
    if !delegates_focus(document, found) {
        return Some(found);
    }
    // The focus already in the host's tree stays where it is.
    match document.focused() {
        Some(focused) if document.is_shadow_including_inclusive_ancestor(found, focused) => {
            Some(focused)
        }
        _ => focus_delegate(document, styles, found),
    }
}
