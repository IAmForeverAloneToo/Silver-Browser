//! The `details` element's state (HTML section 4.11.1, gap batch H): the
//! toggles its `open` attribute owes, which the bindings fire as `toggle`
//! events, each element's kept from its first change since the bindings
//! last took them, as the details toggle task tracker keeps its old
//! state; and the exclusivity of the open elements of one name, which
//! closes the others of its group when one opens and a newcomer that
//! finds one open.

use super::{Change, Document, NodeId};

/// The most toggles kept waiting for the bindings; past it the oldest
/// go.
pub const MAX_DETAILS_TOGGLES: usize = 1 << 12;

/// The toggles owed and whether a `details` with a name was ever made,
/// which an insertion has to look for.
#[derive(Debug, Default, Clone)]
pub(super) struct DetailsState {
    /// Each element whose `open` changed, with whether it was open before
    /// the first change since the bindings last took them, in order.
    toggles: Vec<(NodeId, bool)>,
    named: bool,
}

impl Document {
    /// Whether `node` is an HTML `details`.
    fn is_details(&self, node: NodeId) -> bool {
        self.tag(node) == Some("details") && self.is_html(node)
    }

    /// The `details` elements whose `open` changed since this was last
    /// asked, each with whether it was open before, in order: the
    /// `toggle` events the bindings owe them.
    pub fn take_details_toggles(&mut self) -> Vec<(NodeId, bool)> {
        std::mem::take(&mut self.rare.details.toggles)
    }

    /// Whether a `details` owes a `toggle`.
    pub fn has_details_toggles(&self) -> bool {
        !self.rare.details.toggles.is_empty()
    }

    /// `element`'s `open` changed from `was`: a toggle owed, kept from
    /// the first change while one waits.
    fn details_toggled(&mut self, element: NodeId, was: bool) {
        let toggles = &mut self.rare.details.toggles;
        if toggles.iter().any(|&(e, _)| e == element) {
            return;
        }
        if toggles.len() >= MAX_DETAILS_TOGGLES {
            toggles.remove(0);
        }
        toggles.push((element, was));
    }

    /// A `details` made with its attributes, as a parser or a clone makes
    /// one: its `open` is a change from closed, as appending the
    /// attribute is.
    pub(super) fn details_made(&mut self, element: NodeId) {
        if !self.is_details(element) {
            return;
        }
        if self.attr(element, "name").is_some() {
            self.rare.details.named = true;
        }
        if self.attr(element, "open").is_some() {
            self.details_toggled(element, false);
        }
    }

    /// The `details` `summary` is the summary for (HTML section 4.11.2):
    /// its parent, when it is that parent's first `summary` child.
    pub fn summary_details(&self, summary: NodeId) -> Option<NodeId> {
        if self.tag(summary) != Some("summary") || !self.is_html(summary) {
            return None;
        }
        let parent = self.parent(summary).filter(|&p| self.is_details(p))?;
        let first = self
            .children(parent)
            .find(|&c| self.tag(c) == Some("summary") && self.is_html(c))?;
        (first == summary).then_some(parent)
    }

    /// The name of `element`'s group: its non-empty `name`.
    fn details_name(&self, element: NodeId) -> Option<&str> {
        self.attr(element, "name").filter(|name| !name.is_empty())
    }

    /// The other `details` of `element`'s group in its tree, in tree
    /// order.
    fn details_group(&self, element: NodeId) -> Vec<NodeId> {
        let Some(name) = self.details_name(element) else {
            return Vec::new();
        };
        self.subtree(self.root_of(element))
            .filter(|&n| n != element && self.is_details(n) && self.details_name(n) == Some(name))
            .collect()
    }

    /// HTML's "ensure details exclusivity by closing other elements if
    /// needed": `element` opened, the others of its group close.
    fn close_other_details(&mut self, element: NodeId) {
        for other in self.details_group(element) {
            if self.attr(other, "open").is_some() {
                let _ = self.remove_attribute(other, "open");
            }
        }
    }

    /// HTML's "ensure details exclusivity by closing the given element if
    /// needed": `element`, open, closes when another of its group is.
    fn close_details_if_needed(&mut self, element: NodeId) {
        if self.attr(element, "open").is_none() {
            return;
        }
        let group = self.details_group(element);
        if group
            .iter()
            .any(|&other| self.attr(other, "open").is_some())
        {
            let _ = self.remove_attribute(element, "open");
        }
    }

    /// A change to the document: what it does to the `details` it
    /// touches.
    pub(super) fn details_changed(&mut self, change: &Change) {
        match change {
            Change::AttributeChanged { element, name, old } if self.is_details(*element) => {
                match name.as_str() {
                    "open" => {
                        let was = old.is_some();
                        let open = self.attr(*element, "open").is_some();
                        if was == open {
                            return;
                        }
                        self.details_toggled(*element, was);
                        if open {
                            self.close_other_details(*element);
                        }
                    }
                    "name" => {
                        self.rare.details.named = true;
                        self.close_details_if_needed(*element);
                    }
                    _ => {}
                }
            }
            // The insertion steps of each `details` inserted, in tree
            // order.
            Change::Inserted { child, .. } if self.rare.details.named => {
                let inserted: Vec<NodeId> = self
                    .subtree(*child)
                    .filter(|&n| self.is_details(n) && self.details_name(n).is_some())
                    .collect();
                for element in inserted {
                    self.close_details_if_needed(element);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, Element, Node};

    fn page() -> Document {
        Document::from_node(&Node::document(vec![
            Element::new("html")
                .push(
                    Element::new("body")
                        .push(Element::new("details").attr("id", "a").attr("name", "g"))
                        .push(Element::new("details").attr("id", "b").attr("name", "g"))
                        .push(Element::new("details").attr("id", "c")),
                )
                .into(),
        ]))
    }

    fn id(d: &Document, id: &str) -> crate::NodeId {
        d.element_by_id_in(d.root(), id).unwrap()
    }

    #[test]
    fn opening_one_of_a_name_closes_the_others_and_each_change_owes_a_toggle() {
        let mut d = page();
        let (a, b, c) = (id(&d, "a"), id(&d, "b"), id(&d, "c"));
        d.take_details_toggles();
        d.set_attribute(a, "open", "").unwrap();
        d.set_attribute(c, "open", "").unwrap();
        assert_eq!(d.take_details_toggles(), [(a, false), (c, false)]);
        d.set_attribute(b, "open", "").unwrap();
        assert!(d.attr(a, "open").is_none());
        // b opened, then a closed: each kept from its first change.
        assert_eq!(d.take_details_toggles(), [(b, false), (a, true)]);
        // Setting it again changes nothing.
        d.set_attribute(b, "open", "x").unwrap();
        assert!(!d.has_details_toggles());
        // Closed, opened and closed again before the bindings look: one
        // toggle, from open.
        d.remove_attribute(c, "open").unwrap();
        d.set_attribute(c, "open", "").unwrap();
        d.remove_attribute(c, "open").unwrap();
        assert_eq!(d.take_details_toggles(), [(c, true)]);
    }

    #[test]
    fn an_inserted_open_one_closes_when_its_group_has_one_open() {
        let mut d = page();
        let (a, body) = (id(&d, "a"), id(&d, "c"));
        let body = d.parent(body).unwrap();
        d.set_attribute(a, "open", "").unwrap();
        let made = d
            .create_element_with(
                "details",
                vec![("name".into(), "g".into()), ("open".into(), String::new())],
            )
            .unwrap();
        d.take_details_toggles();
        d.append_child(body, made).unwrap();
        assert!(d.attr(made, "open").is_none());
        assert!(d.attr(a, "open").is_some());
        assert_eq!(d.take_details_toggles(), [(made, true)]);
    }
}
