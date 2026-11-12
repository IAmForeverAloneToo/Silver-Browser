//! Element references by ID (HTML section 2.6.1, "reflecting content
//! attributes in IDL attributes" of element type, gap batch H): what an
//! element's `commandfor`, `popovertarget`, `interestfor`, label `for`
//! and ARIA ID references name, the elements a script set in their
//! place (`commandForElement`, `ariaLabelledByElements`, ...), and a
//! shadow root's `referenceTarget`, which a reference to its host
//! follows into its tree (the reference target proposal Chrome ships).

use super::{Change, Document, NodeId};

/// The attributes whose elements a script may set, which an explicit
/// element of stands in for while the attribute is not set again.
pub const ELEMENT_REFERENCES: [&str; 11] = [
    "popovertarget",
    "commandfor",
    "interestfor",
    "aria-actions",
    "aria-activedescendant",
    "aria-controls",
    "aria-describedby",
    "aria-details",
    "aria-errormessage",
    "aria-flowto",
    "aria-labelledby",
];

/// The most elements a script may set for one attribute.
pub const MAX_EXPLICIT_ELEMENTS: usize = 1 << 12;

impl Document {
    /// The first element in tree order under `root` whose ID is `id`.
    pub fn element_by_id_in(&self, root: NodeId, id: &str) -> Option<NodeId> {
        if id.is_empty() {
            return None;
        }
        self.subtree(root)
            .skip(1)
            .find(|&node| self.is_element(node) && self.attr(node, "id") == Some(id))
    }

    /// `candidate` with the reference targets of the shadow roots it
    /// hosts followed: while it is a host whose shadow root has a
    /// `referenceTarget`, the element of that ID in the root's tree;
    /// `None` when one names none. A tree's depth is bounded, and so is
    /// the walk.
    pub fn follow_reference_target(&self, candidate: NodeId) -> Option<NodeId> {
        let mut at = candidate;
        for _ in 0..=super::MAX_DEPTH {
            let Some(root) = self.shadow_root(at) else {
                return Some(at);
            };
            let Some(target) = self.reference_target(root) else {
                return Some(at);
            };
            at = self.element_by_id_in(root, target)?;
        }
        None
    }

    /// The elements a script set for `attribute` of `element`, if it set
    /// some since the attribute was last set or taken away.
    pub fn explicit_elements(&self, element: NodeId, attribute: &str) -> Option<&[NodeId]> {
        self.rare
            .references
            .get(&(element, Box::from(attribute)))
            .map(Vec::as_slice)
    }

    /// Sets the elements standing for `attribute` of `element`, or takes
    /// them away (`None`).
    pub fn set_explicit_elements(
        &mut self,
        element: NodeId,
        attribute: &str,
        elements: Option<Vec<NodeId>>,
    ) {
        let key = (element, Box::from(attribute));
        match elements {
            Some(mut elements) => {
                elements.truncate(MAX_EXPLICIT_ELEMENTS);
                self.rare.references.insert(key, elements);
            }
            None => {
                self.rare.references.remove(&key);
            }
        }
    }

    /// The elements `attribute` of `element` names, unresolved (HTML's
    /// "attr-associated elements"): those a script set that are in a tree
    /// of `element`'s shadow-including ancestors, else those of the
    /// attribute's IDs, each once, in `element`'s tree; for an attribute
    /// of one reference, the first alone.
    pub fn associated_elements(&self, element: NodeId, attribute: &str) -> Vec<NodeId> {
        if let Some(given) = self.explicit_elements(element, attribute) {
            return given
                .iter()
                .copied()
                .filter(|&e| {
                    self.contains(e)
                        && self.is_shadow_including_inclusive_ancestor(self.root_of(e), element)
                })
                .collect();
        }
        let Some(value) = self.attr(element, attribute) else {
            return Vec::new();
        };
        let root = self.root_of(element);
        let mut out = Vec::new();
        let ids: Vec<&str> =
            if attribute.starts_with("aria-") && !matches!(attribute, "aria-activedescendant") {
                value.split_ascii_whitespace().collect()
            } else {
                vec![value]
            };
        for id in ids {
            if let Some(found) = self.element_by_id_in(root, id)
                && !out.contains(&found)
            {
                out.push(found);
            }
        }
        out
    }

    /// The element `attribute` of `element` refers to, its reference
    /// targets followed: what a command, a popover target or an interest
    /// invoker acts on.
    pub fn referenced_element(&self, element: NodeId, attribute: &str) -> Option<NodeId> {
        let first = self
            .associated_elements(element, attribute)
            .into_iter()
            .next()?;
        self.follow_reference_target(first)
    }

    /// An attribute set or taken away: the elements a script set for it
    /// go.
    pub(super) fn references_changed(&mut self, change: &Change) {
        crate::hook!("dom::references::references_changed");
        if self.rare.references.is_empty() {
            return;
        }
        if let Change::AttributeChanged { element, name, .. } = change
            && ELEMENT_REFERENCES.contains(&name.as_str())
        {
            self.rare
                .references
                .remove(&(*element, Box::from(name.as_str())));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, Element, Node, ShadowRootInit};

    fn page() -> Document {
        Document::from_node(&Node::document(vec![
            Element::new("html")
                .push(
                    Element::new("body")
                        .push(
                            Element::new("button")
                                .attr("id", "b")
                                .attr("commandfor", "h"),
                        )
                        .push(Element::new("div").attr("id", "h"))
                        .push(Element::new("span").attr("id", "s")),
                )
                .into(),
        ]))
    }

    fn id(d: &Document, id: &str) -> crate::NodeId {
        d.element_by_id_in(d.root(), id).unwrap()
    }

    #[test]
    fn a_reference_follows_reference_targets_into_shadow_trees() {
        let mut d = page();
        let (b, h) = (id(&d, "b"), id(&d, "h"));
        assert_eq!(d.referenced_element(b, "commandfor"), Some(h));
        let root = d.attach_shadow(h, ShadowRootInit::open(), false).unwrap();
        let inner = d.create_element("div").unwrap();
        d.set_attribute(inner, "id", "t").unwrap();
        d.append_child(root, inner).unwrap();
        // No reference target: the host itself.
        assert_eq!(d.referenced_element(b, "commandfor"), Some(h));
        d.set_reference_target(root, Some("t")).unwrap();
        assert_eq!(d.referenced_element(b, "commandfor"), Some(inner));
        assert_eq!(d.associated_elements(b, "commandfor"), [h]);
        // One that names nothing refers to nothing.
        d.set_reference_target(root, Some("gone")).unwrap();
        assert_eq!(d.referenced_element(b, "commandfor"), None);
    }

    #[test]
    fn a_scripts_elements_stand_until_the_attribute_is_set() {
        let mut d = page();
        let (b, s, h) = (id(&d, "b"), id(&d, "s"), id(&d, "h"));
        d.set_explicit_elements(b, "commandfor", Some(vec![s]));
        assert_eq!(d.referenced_element(b, "commandfor"), Some(s));
        d.set_attribute(b, "commandfor", "h").unwrap();
        assert_eq!(d.referenced_element(b, "commandfor"), Some(h));
        // An element of a tree inside, not around, does not stand.
        let root = d.attach_shadow(h, ShadowRootInit::open(), false).unwrap();
        let inner = d.create_element("div").unwrap();
        d.append_child(root, inner).unwrap();
        d.set_explicit_elements(b, "commandfor", Some(vec![inner]));
        assert_eq!(d.referenced_element(b, "commandfor"), None);
        // ARIA's lists take each ID once.
        d.set_attribute(b, "aria-labelledby", "s h s nope").unwrap();
        assert_eq!(d.associated_elements(b, "aria-labelledby"), [s, h]);
    }
}
