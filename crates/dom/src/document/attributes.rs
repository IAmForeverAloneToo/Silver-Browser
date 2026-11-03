//! Attributes' namespaces (gap batch H, D67): an attribute is kept by
//! its qualified name, which its prefix is part of, and its namespace is
//! the one that name implies ([`implied_attribute_namespace`]) unless it
//! was given another: by `setAttributeNS`, by the XML parser, which
//! binds prefixes to any namespace, or by `setAttribute`, which makes an
//! attribute of no namespace whatever its name. The elements whose
//! attributes are not all in the namespaces their names imply keep each
//! attribute's, in order, in a map of the arena's rare state, so that the
//! attribute lists everything else reads stay as they are.
//!
//! Two attributes of an element may then share a qualified name in
//! different namespaces, as the DOM lets them: a lookup by name finds
//! the first (DOM section 4.9's "get an attribute by name").

use super::{Document, MutationError, NodeData, NodeId};
use crate::{Namespace, XLINK_NAMESPACE, XML_NAMESPACE, XMLNS_NAMESPACE, local_part};

/// The namespace of an attribute named `qualified` of an element of
/// `element`'s namespace when nothing says otherwise: on an element that
/// is not HTML's, XLink's, XML's or XMLNS's for the prefixes the HTML
/// parser's "adjust foreign attributes" gives them (section 13.2.6.5),
/// and XMLNS's for `xmlns`; none otherwise.
pub fn implied_attribute_namespace(element: &Namespace, qualified: &str) -> Option<&'static str> {
    if *element == Namespace::Html {
        return None;
    }
    if qualified == "xmlns" {
        return Some(XMLNS_NAMESPACE);
    }
    match qualified.split_once(':').map(|(prefix, _)| prefix) {
        Some("xlink") => Some(XLINK_NAMESPACE),
        Some("xml") => Some(XML_NAMESPACE),
        Some("xmlns") => Some(XMLNS_NAMESPACE),
        _ => None,
    }
}

/// An attribute's local name: its qualified name without its prefix when
/// it has a namespace, the whole name when it has none (an attribute of
/// no namespace has no prefix).
pub fn attribute_local_name<'a>(namespace: Option<&str>, qualified: &'a str) -> &'a str {
    match namespace {
        Some(_) => local_part(qualified),
        None => qualified,
    }
}

/// The bytes a list of namespaces holds, for [`super::MAX_DATA_BYTES`].
pub(super) fn namespaces_size(namespaces: &[Option<Box<str>>]) -> usize {
    namespaces.iter().flatten().map(|uri| uri.len()).sum()
}

impl Document {
    /// The namespace of `element`'s attribute at `index`; `None` for no
    /// namespace, or no such attribute.
    pub fn attribute_namespace(&self, element: NodeId, index: usize) -> Option<&str> {
        if let Some(namespaces) = self.rare.attr_ns.get(&element) {
            return namespaces.get(index)?.as_deref();
        }
        match self.data(element)? {
            NodeData::Element { attrs, ns, .. } => {
                implied_attribute_namespace(ns, &attrs.get(index)?.0)
            }
            _ => None,
        }
    }

    /// The index of `element`'s attribute of namespace `namespace` and
    /// local name `local` (DOM's "get an attribute by namespace and local
    /// name").
    pub fn find_attribute_ns(
        &self,
        element: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Option<usize> {
        let attrs = self.attrs(element)?;
        (0..attrs.len()).find(|&i| {
            let ns = self.attribute_namespace(element, i);
            ns == namespace && attribute_local_name(ns, &attrs[i].0) == local
        })
    }

    /// The value of `element`'s attribute of namespace `namespace` and
    /// local name `local` (`getAttributeNS`).
    pub fn attr_ns(&self, element: NodeId, namespace: Option<&str>, local: &str) -> Option<&str> {
        let at = self.find_attribute_ns(element, namespace, local)?;
        Some(self.attrs(element)?[at].1.as_str())
    }

    /// Sets the namespaces of a new element's attributes, one an
    /// attribute in order, as a parser resolved them; kept only when one
    /// is not what its name implies.
    pub fn set_attribute_namespaces(
        &mut self,
        element: NodeId,
        namespaces: Vec<Option<String>>,
    ) -> Result<(), MutationError> {
        let Some(NodeData::Element { attrs, ns, .. }) = self.data(element) else {
            return Err(MutationError::WrongKind);
        };
        if namespaces.len() != attrs.len() {
            return Err(MutationError::WrongKind);
        }
        let implied = attrs
            .iter()
            .zip(&namespaces)
            .all(|((name, _), given)| implied_attribute_namespace(ns, name) == given.as_deref());
        let old = self
            .rare
            .attr_ns
            .get(&element)
            .map_or(0, |kept| namespaces_size(kept));
        if implied {
            self.rare.attr_ns.remove(&element);
            self.data_bytes -= old;
            return Ok(());
        }
        let kept: Vec<Option<Box<str>>> = namespaces
            .into_iter()
            .map(|ns| ns.map(String::into_boxed_str))
            .collect();
        let new = namespaces_size(&kept);
        self.check_data(new.saturating_sub(old))?;
        self.data_bytes = self.data_bytes + new - old;
        self.rare.attr_ns.insert(element, kept);
        Ok(())
    }

    /// The namespaces of `element`'s attributes, one an attribute in
    /// order: what a copy of it is given.
    pub fn attribute_namespaces(&self, element: NodeId) -> Vec<Option<String>> {
        let count = self.attrs(element).map_or(0, <[_]>::len);
        (0..count)
            .map(|i| self.attribute_namespace(element, i).map(str::to_owned))
            .collect()
    }

    /// `setAttributeNS` (DOM section 4.9's "set an attribute value" with
    /// a namespace): the value of `element`'s attribute of `namespace`
    /// and `qualified`'s local name set, its prefix kept; else a new one
    /// named `qualified`, last. The caller has validated the name against
    /// the namespace.
    pub fn set_attribute_ns(
        &mut self,
        element: NodeId,
        namespace: Option<&str>,
        qualified: &str,
        value: &str,
    ) -> Result<(), MutationError> {
        let local = attribute_local_name(namespace, qualified);
        let at = self.find_attribute_ns(element, namespace, local);
        let name = match at {
            Some(at) => self.attrs(element).ok_or(MutationError::WrongKind)?[at]
                .0
                .clone(),
            None => qualified.to_owned(),
        };
        let renames = self.names_by(element, &name) && self.connected(element);
        if renames {
            self.index_names(element, false);
        }
        let set = self.change_attribute(element, at, name, value, namespace);
        if renames {
            self.index_names(element, true);
        }
        set
    }

    /// `removeAttributeNS`: `element`'s attribute of `namespace` and
    /// `local` removed; removing one it does not have changes nothing.
    pub fn remove_attribute_ns(
        &mut self,
        element: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Result<(), MutationError> {
        if !self.is_element(element) {
            return Err(MutationError::WrongKind);
        }
        let Some(at) = self.find_attribute_ns(element, namespace, local) else {
            return Ok(());
        };
        let name = self.attrs(element).ok_or(MutationError::WrongKind)?[at]
            .0
            .clone();
        let renames = self.names_by(element, &name) && self.connected(element);
        if renames {
            self.index_names(element, false);
        }
        let removed = self.remove_attribute_at(element, at);
        if renames {
            self.index_names(element, true);
        }
        removed
    }

    /// Keeps the namespace of the attribute just added last to `element`
    /// when it is not what its name implies, or when the element keeps
    /// its attributes' namespaces already.
    pub(super) fn push_attribute_namespace(&mut self, element: NodeId, namespace: Option<&str>) {
        let Some(NodeData::Element { attrs, ns, .. }) = self.data(element) else {
            return;
        };
        let Some((name, _)) = attrs.last() else {
            return;
        };
        let count = attrs.len();
        let implied = implied_attribute_namespace(ns, name) == namespace;
        if implied && !self.rare.attr_ns.contains_key(&element) {
            return;
        }
        if !self.rare.attr_ns.contains_key(&element) {
            // The attributes before it, in the namespaces they imply.
            let before: Vec<Option<Box<str>>> = (0..count - 1)
                .map(|i| self.attribute_namespace(element, i).map(Into::into))
                .collect();
            self.data_bytes += namespaces_size(&before);
            self.rare.attr_ns.insert(element, before);
        }
        self.data_bytes += namespace.map_or(0, str::len);
        if let Some(kept) = self.rare.attr_ns.get_mut(&element) {
            kept.push(namespace.map(Into::into));
        }
    }

    /// Forgets the namespace of `element`'s attribute at `index`, which is
    /// being removed.
    pub(super) fn remove_attribute_namespace(&mut self, element: NodeId, index: usize) {
        let Some(kept) = self.rare.attr_ns.get_mut(&element) else {
            return;
        };
        if index < kept.len() {
            let gone = kept.remove(index);
            self.data_bytes -= gone.map_or(0, |uri| uri.len());
        }
        if kept.is_empty() {
            self.rare.attr_ns.remove(&element);
        }
    }

    /// Forgets the namespaces of the attributes of the nodes `doomed`,
    /// which are being freed; their bytes are the caller's to count.
    pub(super) fn forget_attribute_namespaces(&mut self, doomed: &[NodeId]) -> usize {
        if self.rare.attr_ns.is_empty() {
            return 0;
        }
        doomed
            .iter()
            .filter_map(|node| self.rare.attr_ns.remove(node))
            .map(|kept| namespaces_size(&kept))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Change, Mutated, SVG_NAMESPACE};

    #[test]
    fn a_name_implies_a_namespace_only_on_an_element_that_is_not_htmls() {
        assert_eq!(
            implied_attribute_namespace(&Namespace::Svg, "xlink:href"),
            Some(XLINK_NAMESPACE)
        );
        assert_eq!(
            implied_attribute_namespace(&Namespace::Svg, "xmlns"),
            Some(XMLNS_NAMESPACE)
        );
        assert_eq!(
            implied_attribute_namespace(&Namespace::Svg, "xml:lang"),
            Some(XML_NAMESPACE)
        );
        assert_eq!(
            implied_attribute_namespace(&Namespace::Svg, "foo:bar"),
            None
        );
        assert_eq!(
            implied_attribute_namespace(&Namespace::Html, "xml:lang"),
            None
        );
        assert_eq!(
            attribute_local_name(Some(XML_NAMESPACE), "xml:lang"),
            "lang"
        );
        assert_eq!(attribute_local_name(None, "xml:lang"), "xml:lang");
    }

    #[test]
    fn an_attribute_keeps_the_namespace_it_was_given_beside_one_of_the_same_name() {
        let mut d = Document::new();
        let root = d.root();
        let p = d.create_element("p").unwrap();
        d.append_child(root, p).unwrap();
        d.set_attribute(p, "a", "plain").unwrap();
        d.set_attribute_ns(p, Some("urn:x"), "x:a", "one").unwrap();
        d.set_attribute_ns(p, Some("urn:y"), "y:a", "two").unwrap();
        // The same namespace and local name sets the value, the prefix kept.
        d.set_attribute_ns(p, Some("urn:x"), "z:a", "three")
            .unwrap();
        // An attribute of a namespace with no prefix is named by its local
        // name, as one of no namespace is.
        d.set_attribute_ns(p, Some("urn:z"), "a", "four").unwrap();
        let names: Vec<&str> = d
            .attrs(p)
            .unwrap()
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(names, ["a", "x:a", "y:a", "a"]);
        assert_eq!(d.attr_ns(p, Some("urn:x"), "a"), Some("three"));
        assert_eq!(d.attr_ns(p, Some("urn:y"), "a"), Some("two"));
        assert_eq!(d.attr_ns(p, Some("urn:z"), "a"), Some("four"));
        assert_eq!(d.attr_ns(p, None, "a"), Some("plain"));
        // By name, the first.
        assert_eq!(d.attr(p, "a"), Some("plain"));
        assert_eq!(d.attribute_namespace(p, 3), Some("urn:z"));
        d.validate().unwrap();
        d.remove_attribute_ns(p, Some("urn:x"), "a").unwrap();
        assert_eq!(d.attr_ns(p, Some("urn:x"), "a"), None);
        assert_eq!(d.attribute_namespace(p, 1), Some("urn:y"));
        d.remove_attribute(p, "a").unwrap();
        assert_eq!(d.attr_ns(p, None, "a"), None);
        assert_eq!(d.attr_ns(p, Some("urn:z"), "a"), Some("four"));
        d.validate().unwrap();
        d.remove_child(root, p).unwrap();
        let before = d.data_bytes();
        d.free(p).unwrap();
        assert!(d.data_bytes() < before);
        d.validate().unwrap();
    }

    #[test]
    fn set_attribute_makes_an_attribute_of_no_namespace_whatever_its_name() {
        let mut d = Document::new();
        let svg = d
            .create_element_ns(
                Namespace::Svg,
                "svg",
                vec![("xlink:href".into(), "a".into())],
            )
            .unwrap();
        // The parser's is XLink's, as its name implies.
        assert_eq!(d.attr_ns(svg, Some(XLINK_NAMESPACE), "href"), Some("a"));
        let use_ = d
            .create_element_ns(Namespace::Svg, "use", Vec::new())
            .unwrap();
        d.set_attribute(use_, "xlink:href", "b").unwrap();
        assert_eq!(d.attr_ns(use_, Some(XLINK_NAMESPACE), "href"), None);
        assert_eq!(d.attr_ns(use_, None, "xlink:href"), Some("b"));
        d.set_attribute_ns(use_, Some(XLINK_NAMESPACE), "xlink:href", "c")
            .unwrap();
        assert_eq!(d.attr_ns(use_, Some(XLINK_NAMESPACE), "href"), Some("c"));
        assert_eq!(d.attrs(use_).unwrap().len(), 2);
        d.validate().unwrap();
        // A parser's namespaces are kept where a name does not imply them.
        let g = d
            .create_element_ns(
                Namespace::Other(Some(SVG_NAMESPACE.into())),
                "g",
                vec![("p:a".into(), "1".into()), ("b".into(), "2".into())],
            )
            .unwrap();
        d.set_attribute_namespaces(g, vec![Some("urn:p".into()), None])
            .unwrap();
        assert_eq!(d.attr_ns(g, Some("urn:p"), "a"), Some("1"));
        assert_eq!(
            d.set_attribute_namespaces(g, vec![None]),
            Err(MutationError::WrongKind)
        );
        d.validate().unwrap();
    }

    #[test]
    fn an_observer_is_told_an_attributes_local_name_and_namespace() {
        let mut d = Document::new();
        let root = d.root();
        let p = d.create_element("p").unwrap();
        d.append_child(root, p).unwrap();
        d.observe(true);
        d.set_watched([(p, false)].into_iter().collect());
        d.take_changes();
        d.set_attribute_ns(p, Some("urn:x"), "x:a", "1").unwrap();
        d.remove_attribute_ns(p, Some("urn:x"), "a").unwrap();
        let observed: Vec<Mutated> = d.take_observed().into_iter().map(|m| m.change).collect();
        assert_eq!(
            observed,
            [
                Mutated::Attribute {
                    element: p,
                    name: "a".into(),
                    namespace: Some("urn:x".into()),
                    old: None,
                },
                Mutated::Attribute {
                    element: p,
                    name: "a".into(),
                    namespace: Some("urn:x".into()),
                    old: Some("1".into()),
                },
            ]
        );
        // The journal names it as the style sheets' selectors do, whole.
        assert!(d.take_changes().contains(&Change::AttributeChanged {
            element: p,
            name: "x:a".into(),
            old: None,
        }));
    }
}
