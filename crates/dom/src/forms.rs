//! What HTML says of forms and their controls that more than one stage
//! reads (M26): whether a control is disabled, a `select`'s options, and
//! an element's form owner. Style's matcher reads them for `:disabled`,
//! `:checked` and the radio groups; the engine for `click()`, an event
//! handler's scope and, later, the controls' state.

use crate::{Document, NodeId};

/// The elements `:enabled` and `:disabled` apply to (HTML section
/// 4.16.3).
pub fn can_be_disabled(document: &Document, element: NodeId) -> bool {
    matches!(
        document.tag(element),
        Some("button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset")
    )
}

/// Whether the element is actually disabled (HTML section 4.16.3): a
/// `button`, `input`, `select`, `textarea` or `fieldset` with a
/// `disabled` attribute or in a `fieldset` that disables it (sections
/// 4.10.19.5 and 4.10.15); an `optgroup` with one; an option that is
/// disabled.
pub fn actually_disabled(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("button" | "input" | "select" | "textarea" | "fieldset") => {
            document.attr(element, "disabled").is_some() || disabled_by_fieldset(document, element)
        }
        Some("optgroup") => document.attr(element, "disabled").is_some(),
        Some("option") => option_disabled(document, element),
        _ => false,
    }
}

/// Whether a `fieldset` with a `disabled` attribute around the element
/// disables it: one it is not inside the first `legend` child of.
fn disabled_by_fieldset(document: &Document, element: NodeId) -> bool {
    let mut child = element;
    for ancestor in document.ancestors(element) {
        if document.tag(ancestor) == Some("fieldset")
            && document.attr(ancestor, "disabled").is_some()
            && !(document.tag(child) == Some("legend")
                && document
                    .element_children(ancestor)
                    .find(|&c| document.tag(c) == Some("legend"))
                    == Some(child))
        {
            return true;
        }
        child = ancestor;
    }
    false
}

/// Whether an option is disabled (HTML section 4.10.10): it has a
/// `disabled` attribute, or the nearest of its ancestors that is an
/// `optgroup`, `select`, `hr`, `datalist` or `option` is an `optgroup`
/// with one.
pub fn option_disabled(document: &Document, option: NodeId) -> bool {
    if document.attr(option, "disabled").is_some() {
        return true;
    }
    for ancestor in document.ancestors(option) {
        match document.tag(ancestor) {
            Some("select" | "hr" | "datalist" | "option") => return false,
            Some("optgroup") => return document.attr(ancestor, "disabled").is_some(),
            _ => {}
        }
    }
    false
}

/// The `select` whose list of options holds `option` (HTML section
/// 4.10.7): its nearest `select` ancestor, unless an `hr`, a `datalist`
/// or an `option` comes before it, or two `optgroup`s do.
pub fn owner_select(document: &Document, option: NodeId) -> Option<NodeId> {
    let mut optgroups = 0;
    for ancestor in document.ancestors(option) {
        match document.tag(ancestor) {
            Some("select") => return Some(ancestor),
            Some("hr" | "datalist" | "option") => return None,
            Some("optgroup") => {
                optgroups += 1;
                if optgroups == 2 {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// A `select`'s list of options (HTML section 4.10.7), in tree order:
/// its `option` descendants, not looking inside a `select`, `hr`,
/// `option` or `datalist`, nor an `optgroup` inside another.
pub fn list_of_options(document: &Document, select: NodeId) -> Vec<NodeId> {
    let mut options = Vec::new();
    let mut node = document.first_child(select);
    while let Some(at) = node {
        let tag = document.tag(at);
        if tag == Some("option") {
            options.push(at);
        }
        let closed = matches!(tag, Some("select" | "hr" | "option" | "datalist"))
            || (tag == Some("optgroup")
                && document
                    .ancestors(at)
                    .take_while(|&ancestor| ancestor != select)
                    .any(|ancestor| document.tag(ancestor) == Some("optgroup")));
        node = match document.first_child(at) {
            Some(child) if !closed => Some(child),
            _ => next_within(document, at, select),
        };
    }
    options
}

/// The node after `node` and its subtree in tree order, within `root`.
fn next_within(document: &Document, mut node: NodeId, root: NodeId) -> Option<NodeId> {
    while node != root {
        if let Some(next) = document.next_sibling(node) {
            return Some(next);
        }
        node = document.parent(node)?;
    }
    None
}

/// The option a `select` without `multiple` leaves selected, by the
/// selectedness setting algorithm (HTML section 4.10.7) over its
/// options' `selected` attributes: the last with one; with none, and a
/// display size of 1, the first that is not disabled. Its display size
/// is its `size`, a non-negative integer, else 1.
pub fn selected_option(document: &Document, select: NodeId) -> Option<NodeId> {
    let options = list_of_options(document, select);
    if let Some(&last) = options
        .iter()
        .rev()
        .find(|&&option| document.attr(option, "selected").is_some())
    {
        return Some(last);
    }
    let size = document
        .attr(select, "size")
        .and_then(crate::parse_non_negative_integer)
        .unwrap_or(1);
    if size != 1 {
        return None;
    }
    options
        .into_iter()
        .find(|&option| !option_disabled(document, option))
}

/// Whether the element is form-associated (HTML section 4.10.2): it can
/// have a form owner.
pub fn form_associated(document: &Document, element: NodeId) -> bool {
    matches!(
        document.tag(element),
        Some("button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea" | "img")
    )
}

/// The element's form owner (HTML section 4.10.18.3): for one with a
/// `form` attribute, in the document, the first element in tree order
/// with that ID if it is a `form`, else none; for any other, its nearest
/// `form` ancestor. Style's matcher works it out the same way with the
/// IDs it keeps.
pub fn form_owner(document: &Document, element: NodeId) -> Option<NodeId> {
    let root = document.root();
    let connected = element == root || document.ancestors(element).last() == Some(root);
    if let Some(id) = document.attr(element, "form")
        && connected
    {
        let owner = document
            .subtree(root)
            .find(|&node| !id.is_empty() && document.attr(node, "id") == Some(id))?;
        return (document.tag(owner) == Some("form")).then_some(owner);
    }
    document
        .ancestors(element)
        .find(|&ancestor| document.tag(ancestor) == Some("form"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, Node};

    fn e(tag: &str) -> Element {
        Element::new(tag)
    }

    /// The element with ID `id`.
    fn by_id(document: &Document, id: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&node| document.attr(node, "id") == Some(id))
            .expect("the element")
    }

    #[test]
    fn a_controls_form_owner_is_its_form_attributes_form_or_its_ancestor() {
        let page: Node = e("body")
            .push(
                e("form")
                    .attr("id", "outer")
                    .push(e("input").attr("id", "inside"))
                    .push(e("input").attr("id", "elsewhere").attr("form", "other"))
                    .push(e("input").attr("id", "nowhere").attr("form", "missing"))
                    .push(e("input").attr("id", "not-a-form").attr("form", "p")),
            )
            .push(e("form").attr("id", "other"))
            .push(e("p").attr("id", "p"))
            .push(e("input").attr("id", "loose"))
            .into();
        let document = Document::from_node(&page);
        let owner = |id| form_owner(&document, by_id(&document, id));
        assert_eq!(owner("inside"), Some(by_id(&document, "outer")));
        assert_eq!(owner("elsewhere"), Some(by_id(&document, "other")));
        assert_eq!(owner("nowhere"), None);
        assert_eq!(owner("not-a-form"), None);
        assert_eq!(owner("loose"), None);
    }

    #[test]
    fn a_control_is_disabled_by_its_attribute_or_its_fieldset_but_the_first_legend() {
        let page: Node = e("fieldset")
            .attr("disabled", "")
            .push(e("legend").push(e("button").attr("id", "in-legend")))
            .push(e("legend").push(e("button").attr("id", "in-second")))
            .push(e("input").attr("id", "inside"))
            .push(e("div").push(e("a").attr("id", "link")))
            .into();
        let document = Document::from_node(&page);
        let disabled = |id| actually_disabled(&document, by_id(&document, id));
        assert!(!disabled("in-legend"));
        assert!(disabled("in-second"));
        assert!(disabled("inside"));
        assert!(!disabled("link"));
    }
}
