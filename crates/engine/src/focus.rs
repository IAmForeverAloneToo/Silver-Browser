//! Focus (M26.6, `docs/design/events.md`, HTML section 6.6): which
//! elements can take the focus, and in what order Tab moves it, for the
//! engine and its scripts' host alike.

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
        _ => false,
    }
}

/// Whether the element is a focusable area (HTML section 6.6.2): one
/// that can take the focus as it is, or has a `tabindex`, and is being
/// rendered, by `styles`, which say it is when they are not given; a
/// disabled control does not take it whatever its `tabindex`.
pub(crate) fn focusable(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    if !document.is_element(element) || !rendered(document, styles, element) {
        return false;
    }
    if forms::can_be_disabled(document, element) && forms::actually_disabled(document, element) {
        return false;
    }
    natively_focusable(document, element) || tab_index(document, element).is_some()
}

/// Whether the element is being rendered: in the document, it and every
/// ancestor with a style other than `display: none`.
fn rendered(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    let root = document.root();
    let connected = document.ancestors(element).last() == Some(root);
    if !connected {
        return false;
    }
    let Some(styles) = styles else {
        return true;
    };
    std::iter::once(element)
        .chain(document.ancestors(element))
        .filter(|&node| document.is_element(node))
        .all(|node| {
            styles
                .get(node)
                .is_some_and(|style| style.display != Display::None)
        })
}

/// The sequential focus navigation order (HTML section 6.6.3): the
/// focusable areas whose `tabindex` is not negative, those with a
/// positive one first, by it and then tree order, then the rest in tree
/// order.
pub(crate) fn sequential_order(document: &Document, styles: Option<&Styles>) -> Vec<NodeId> {
    let mut positive: Vec<(i32, NodeId)> = Vec::new();
    let mut rest = Vec::new();
    for node in document.subtree(document.root()) {
        if !focusable(document, styles, node) {
            continue;
        }
        match tab_index(document, node) {
            Some(n) if n < 0 => {}
            Some(n) if n > 0 => positive.push((n, node)),
            _ => rest.push(node),
        }
    }
    // A stable sort keeps tree order among equals.
    positive.sort_by_key(|&(n, _)| n);
    positive
        .into_iter()
        .map(|(_, node)| node)
        .chain(rest)
        .collect()
}

/// The element a click focuses (HTML section 6.6.4's "focusable area"
/// of the event's target): the nearest focusable inclusive ancestor.
pub(crate) fn click_focus_target(
    document: &Document,
    styles: Option<&Styles>,
    node: NodeId,
) -> Option<NodeId> {
    std::iter::once(node)
        .chain(document.ancestors(node))
        .find(|&node| focusable(document, styles, node))
}
