//! Documents beside the page's (gap batch H, D67): another document of
//! the realm is a `Document` node allocated in the same arena, parentless,
//! with its own subtree, and the page's document, the arena's root, is
//! the one the engine renders and the one [`Document::root`] names.
//! What the host knows of the page's document (its URL, its type, its
//! mode) the arena keeps of each other one ([`DocumentFacts`]).
//!
//! A node's owner document (DOM section 4.4's node document) is its
//! tree's root when that is a document; for a node not in a document's
//! tree, the one its tree's top was made for or taken out of, kept in a
//! map for the tops whose owner is not the page's document; for a node
//! of a template's contents, the template's owner's inert template
//! document, which HTML section 4.12.3 makes with the first template it
//! is asked for. An insertion moves a node into its parent's document
//! and a removal from another document's tree records where it came
//! from; a node's id stays what it was, so a script's object for a node
//! of another document is made as any other.

use std::collections::HashMap;

use super::{Document, MutationError, NodeData, NodeId};

/// A document's mode (DOM section 4.5): what the HTML parser decides by
/// the doctype (section 13.2.6.4.1), no quirks for any other document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CompatMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

/// The interface a document's object is of: `Document` for one the
/// `Document` constructor made, `XMLDocument` for one of type XML a
/// parser or `createDocument` made, `HTMLDocument` for an HTML one, as
/// the reference browser has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentInterface {
    Document,
    HtmlDocument,
    XmlDocument,
}

/// What a document beside the page's is (DOM section 4.5's document's
/// type, content type, URL and mode), set when it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentFacts {
    /// Its type is XML rather than HTML: its HTML elements' names and
    /// attributes keep their case.
    pub xml: bool,
    /// The interface its object is of.
    pub interface: DocumentInterface,
    /// Its content type, e.g. `text/html` or `application/xml`.
    pub content_type: String,
    /// Its character encoding's name: UTF-8 but for a response's.
    pub charset: String,
    /// Its URL.
    pub url: String,
    /// Its mode.
    pub mode: CompatMode,
    /// It is an inert template document (HTML section 4.12.3), which
    /// holds templates' contents.
    pub inert: bool,
}

impl DocumentFacts {
    /// An HTML document's facts at `url`, of interface `HTMLDocument`, in
    /// no-quirks mode.
    pub fn html(url: &str) -> DocumentFacts {
        DocumentFacts {
            xml: false,
            interface: DocumentInterface::HtmlDocument,
            content_type: "text/html".to_owned(),
            charset: "UTF-8".to_owned(),
            url: url.to_owned(),
            mode: CompatMode::NoQuirks,
            inert: false,
        }
    }

    /// An XML document's facts at `url` of `content_type`, of interface
    /// `XMLDocument`.
    pub fn xml(content_type: &str, url: &str) -> DocumentFacts {
        DocumentFacts {
            xml: true,
            interface: DocumentInterface::XmlDocument,
            content_type: content_type.to_owned(),
            charset: "UTF-8".to_owned(),
            url: url.to_owned(),
            mode: CompatMode::NoQuirks,
            inert: false,
        }
    }
}

/// The documents beside the page's, the owners of the tops of trees not
/// the page's, and each document's inert template document.
#[derive(Debug, Clone, Default)]
pub(super) struct Documents {
    pub(super) facts: HashMap<NodeId, DocumentFacts>,
    /// How many of them are XML documents: while none is of another
    /// type than the page's, every node's document is of its type.
    pub(super) xml: usize,
    pub(super) owners: HashMap<NodeId, NodeId>,
    pub(super) inert: HashMap<NodeId, NodeId>,
}

impl Document {
    /// A new document beside the page's, empty, with `facts`.
    pub fn create_document(&mut self, facts: DocumentFacts) -> Result<NodeId, MutationError> {
        self.check_room()?;
        let id = self.allocate_one(NodeData::Document);
        self.keep_facts(id, facts);
        Ok(id)
    }

    /// Keeps the facts of the new document `id`.
    fn keep_facts(&mut self, id: NodeId, facts: DocumentFacts) {
        self.rare.documents.xml += usize::from(facts.xml);
        self.rare.documents.facts.insert(id, facts);
    }

    /// A new, detached element of namespace `ns` named `qualified` as it
    /// is given, `document`'s: what a script's `createElement` or
    /// `createElementNS` of another document makes, the caller having
    /// cased the name as that document's type says.
    pub fn create_element_for(
        &mut self,
        document: NodeId,
        ns: crate::Namespace,
        qualified: &str,
    ) -> Result<NodeId, MutationError> {
        if !self.is_document(document) {
            return Err(MutationError::NoSuchNode);
        }
        self.check_room()?;
        self.check_data(qualified.len())?;
        let element = self.allocate(NodeData::Element {
            tag: qualified.to_owned(),
            attrs: Vec::new(),
            ns,
        });
        self.set_owner(element, document)?;
        Ok(element)
    }

    /// A copy of `node` alone, `document`'s (DOM section 4.5's "clone a
    /// single node"): an element's name, attributes, their namespaces and
    /// its prefix, or a node's data; a document's copy is a new document
    /// of the same facts, which the caller gives for the page's.
    pub fn copy_node(
        &mut self,
        node: NodeId,
        document: NodeId,
        page_facts: impl FnOnce() -> DocumentFacts,
    ) -> Result<NodeId, MutationError> {
        let data = self.data(node).ok_or(MutationError::NoSuchNode)?.clone();
        // A shadow root is copied with its host alone, by its caller.
        if matches!(data, NodeData::ShadowRoot(_)) {
            return Err(MutationError::NotSupported);
        }
        if data == NodeData::Document {
            let facts = match self.document_facts(node) {
                Some(facts) => DocumentFacts {
                    inert: false,
                    ..facts.clone()
                },
                None => page_facts(),
            };
            return self.create_document(facts);
        }
        let namespaces = self.rare.attr_ns.get(&node).cloned();
        let extra = namespaces
            .as_deref()
            .map_or(0, super::attributes::namespaces_size);
        self.check_room()?;
        self.check_data(super::data_size(&data) + extra)?;
        let made = self.allocate(data);
        if let Some(namespaces) = namespaces {
            self.data_bytes += extra;
            self.rare.attr_ns.insert(made, namespaces);
        }
        if self.rare.prefixed.contains(&node) {
            self.rare.prefixed.insert(made);
        }
        if self.rare.cdata.contains(&node) {
            self.rare.cdata.insert(made);
        }
        self.set_owner(made, document)?;
        // A copy keeps its `is` value (DOM's "create an element", D70);
        // its registry, and the upgrade it may be owed, are its caller's
        // to give ([`Document::set_copied_registry`]).
        if let Some(is) = self.is_value(node).map(str::to_owned) {
            self.set_is_value(made, Some(&is))?;
        }
        Ok(made)
    }

    /// Whether `id` is a document node, the page's or another.
    pub fn is_document(&self, id: NodeId) -> bool {
        matches!(self.data(id), Some(NodeData::Document))
    }

    /// The documents beside the page's, in no order.
    pub fn other_documents(&self) -> Vec<NodeId> {
        self.rare.documents.facts.keys().copied().collect()
    }

    /// What another document is; `None` for the page's, whose facts are
    /// its host's, and for any other node.
    pub fn document_facts(&self, document: NodeId) -> Option<&DocumentFacts> {
        self.rare.documents.facts.get(&document)
    }

    /// Sets another document's mode, as its parser decided it.
    pub fn set_document_mode(&mut self, document: NodeId, mode: CompatMode) {
        if let Some(facts) = self.rare.documents.facts.get_mut(&document) {
            facts.mode = mode;
        }
    }

    /// Whether `document` is of type XML: the page's as its parser made
    /// it, another's as it was made.
    pub fn is_xml(&self, document: NodeId) -> bool {
        match self.rare.documents.facts.get(&document) {
            Some(facts) => facts.xml,
            None => self.rare.xml,
        }
    }

    /// Whether `node`'s owner document is of type XML.
    pub fn is_xml_node(&self, node: NodeId) -> bool {
        let documents = &self.rare.documents;
        let all_xml = documents.xml == documents.facts.len();
        if (self.rare.xml && all_xml) || (!self.rare.xml && documents.xml == 0) {
            return self.rare.xml;
        }
        self.is_xml(self.owner_document(node))
    }

    /// A document's element: its first element child.
    pub fn document_element_of(&self, document: NodeId) -> Option<NodeId> {
        self.children(document).find(|&c| self.is_element(c))
    }

    /// A document's doctype: its first `DocumentType` child.
    pub fn doctype_of(&self, document: NodeId) -> Option<NodeId> {
        self.children(document)
            .find(|&n| matches!(self.data(n), Some(NodeData::DocumentType { .. })))
    }

    /// The owner document of `node` (DOM's node document): see the
    /// module doc. A document is its own. The contents of a template
    /// whose document has no inert template document yet answer that
    /// document, which is of the same type; [`Document::owner_document_made`]
    /// makes it.
    pub fn owner_document(&self, node: NodeId) -> NodeId {
        let (owner, contents) = self.owner_and_contents(node);
        match contents {
            true => self
                .rare
                .documents
                .inert
                .get(&owner)
                .copied()
                .unwrap_or(owner),
            false => owner,
        }
    }

    /// [`Document::owner_document`], the inert template document made
    /// when the node is in a template's contents and it is not yet.
    pub fn owner_document_made(&mut self, node: NodeId) -> NodeId {
        let (owner, contents) = self.owner_and_contents(node);
        match contents {
            true => self.template_document(owner),
            false => owner,
        }
    }

    /// The document `node`'s tree belongs to, and whether that tree is a
    /// template's contents (then in the inert template document of that
    /// document, which inert template documents are their own).
    pub(super) fn owner_and_contents(&self, node: NodeId) -> (NodeId, bool) {
        let mut at = node;
        let mut contents = false;
        // A template's contents' template may be in a template's contents
        // too: each step goes up to the next host.
        for _ in 0..=self.slots.len() {
            let top = self.root_of(at);
            if self.is_document(top) {
                return (top, contents);
            }
            if let Some(host) = self.template_host(top) {
                contents = true;
                at = host;
                continue;
            }
            // A shadow tree is its host's document's (D68).
            if let Some(host) = self.shadow_host(top) {
                at = host;
                continue;
            }
            let owner = self
                .rare
                .documents
                .owners
                .get(&top)
                .copied()
                .filter(|&owner| self.contains(owner))
                .unwrap_or(self.root);
            return (owner, contents);
        }
        (self.root, contents)
    }

    /// HTML section 4.12.3's "appropriate template contents owner
    /// document" of `document`: its inert template document, made of its
    /// type the first time it is asked for; an inert template document's
    /// is itself.
    pub fn template_document(&mut self, document: NodeId) -> NodeId {
        if let Some(&inert) = self.rare.documents.inert.get(&document) {
            return inert;
        }
        if self
            .document_facts(document)
            .is_some_and(|facts| facts.inert)
        {
            return document;
        }
        let xml = self.is_xml(document);
        let mut facts = if xml {
            DocumentFacts::xml("application/xml", "about:blank")
        } else {
            DocumentFacts::html("about:blank")
        };
        facts.inert = true;
        let inert = self.allocate_one(NodeData::Document);
        self.keep_facts(inert, facts);
        self.rare.documents.inert.insert(document, inert);
        self.rare.documents.inert.insert(inert, inert);
        inert
    }

    /// Makes `document` the owner of `node`, a top of a tree: a node a
    /// script made for it (`createElement` of another document) or one
    /// the DOM's "adopt" took out of its tree.
    pub fn set_owner(&mut self, node: NodeId, document: NodeId) -> Result<(), MutationError> {
        if !self.contains(node) || !self.is_document(document) {
            return Err(MutationError::NoSuchNode);
        }
        if self.parent(node).is_some() || self.is_document(node) {
            return Err(MutationError::Attached);
        }
        if document == self.root {
            self.rare.documents.owners.remove(&node);
        } else {
            self.rare.documents.owners.insert(node, document);
        }
        Ok(())
    }

    /// The DOM's "adopt" (section 4.5) of `node` into `document`: taken
    /// out of its parent, then made the document's. A document cannot be
    /// adopted.
    pub fn adopt(&mut self, node: NodeId, document: NodeId) -> Result<(), MutationError> {
        if !self.contains(node) || !self.is_document(document) {
            return Err(MutationError::NoSuchNode);
        }
        if self.is_document(node) || self.is_shadow_root(node) {
            return Err(MutationError::HierarchyRequest);
        }
        let old = self.owner_document(node);
        if let Some(parent) = self.parent(node) {
            self.remove_child(parent, node)?;
        }
        let before = self.registries_before_adoption(node, old, document, false);
        self.set_owner(node, document)?;
        self.registries_adopted(before, document);
        let nodes = self.shadow_including_subtree(node);
        self.custom_adopted(&nodes, old);
        Ok(())
    }

    /// A node inserted into a tree is its parent's document's: it is no
    /// longer a top with an owner of its own.
    pub(super) fn owned_by_tree(&mut self, node: NodeId) {
        if !self.rare.documents.owners.is_empty() {
            self.rare.documents.owners.remove(&node);
        }
    }

    /// Before `child` is taken out of `parent`: the document it leaves,
    /// which it keeps as its owner, when that is not the page's.
    pub(super) fn owner_leaving(&mut self, parent: NodeId) -> Option<NodeId> {
        if self.rare.documents.facts.is_empty()
            && self.template_host(self.root_of(parent)).is_none()
        {
            // No document but the page's, and no template's contents:
            // the owner is the page's document.
            return None;
        }
        let top = self.root_of(parent);
        if top == self.root {
            return None;
        }
        Some(self.owner_document_made(parent)).filter(|&owner| owner != self.root)
    }

    /// Whether `document` is still some node's owner, or the inert
    /// template document of a live document: it is not freed while it is.
    pub fn document_in_use(&self, document: NodeId) -> bool {
        let documents = &self.rare.documents;
        documents.owners.values().any(|&owner| owner == document)
            || documents
                .inert
                .iter()
                .any(|(&of, &inert)| inert == document && of != document && self.contains(of))
    }

    /// Forgets what the documents' maps say of the nodes `doomed`, which
    /// are being freed.
    pub(super) fn forget_documents(&mut self, doomed: &[NodeId]) {
        let documents = &mut self.rare.documents;
        if documents.facts.is_empty() && documents.owners.is_empty() {
            return;
        }
        for node in doomed {
            if let Some(facts) = documents.facts.remove(node) {
                documents.xml -= usize::from(facts.xml);
            }
            documents.owners.remove(node);
            documents.inert.remove(node);
        }
    }

    /// A new document beside the page's, with `facts`, holding a copy of
    /// what `from` holds: its nodes of every kind, its templates'
    /// contents and its attributes' namespaces, as a parser built them
    /// into a document of its own (`DOMParser`, a response's document).
    /// Linear in `from`'s nodes; refused whole if they do not fit.
    pub fn graft(
        &mut self,
        from: &Document,
        facts: DocumentFacts,
    ) -> Result<NodeId, MutationError> {
        if from.len() + 1 > self.max_nodes.saturating_sub(self.live) {
            return Err(MutationError::TooManyNodes);
        }
        self.check_data(from.data_bytes())?;
        let document = self.create_document(facts)?;
        // Each entry: a node of `from` still to copy and the node of this
        // document to copy it under.
        let mut pending: Vec<(NodeId, NodeId)> = Vec::new();
        let tops: Vec<NodeId> = from.children(from.root()).collect();
        pending.extend(tops.into_iter().rev().map(|n| (n, document)));
        let mut roots = Vec::new();
        while let Some((node, parent)) = pending.pop() {
            let Some(data) = from.data(node) else {
                continue;
            };
            let made = self.allocate(data.clone());
            self.link_last(parent.index, made.index);
            if from.rare.prefixed.contains(&node) {
                self.rare.prefixed.insert(made);
            }
            if from.rare.cdata.contains(&node) {
                self.rare.cdata.insert(made);
            }
            if let Some(namespaces) = from.rare.attr_ns.get(&node) {
                self.data_bytes += super::attributes::namespaces_size(namespaces);
                self.rare.attr_ns.insert(made, namespaces.clone());
            }
            // A `details` the parser opened owes its `toggle` here, where
            // the bindings take it, as the reference browser fires it for
            // a document of no browsing context too.
            self.details_made(made);
            let children: Vec<NodeId> = from.children(node).collect();
            pending.extend(children.into_iter().rev().map(|c| (c, made)));
            if let (Some(from_contents), Some(contents)) =
                (from.template_content(node), self.template_content(made))
            {
                let children: Vec<NodeId> = from.children(from_contents).collect();
                pending.extend(children.into_iter().rev().map(|c| (c, contents)));
            }
            // A host's shadow tree, as its parser attached it (D68).
            if let Some(from_root) = from.shadow_root(node)
                && let Some(init) = from.shadow_init(from_root)
                && let Ok(root) =
                    self.attach_shadow(made, init, from.is_declarative_shadow(from_root))
            {
                let _ = self.set_reference_target(root, from.reference_target(from_root));
                let children: Vec<NodeId> = from.children(from_root).collect();
                pending.extend(children.into_iter().rev().map(|c| (c, root)));
                roots.push(root);
            }
        }
        // The copies' slots, linked without the insertion steps, are
        // assigned once the trees are whole.
        for root in roots {
            self.reassign(root);
        }
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Namespace;

    #[test]
    fn another_document_is_a_parentless_document_node_its_nodes_owned_by_it() {
        let mut d = Document::new();
        let page = d.root();
        let other = d
            .create_document(DocumentFacts::html("about:blank"))
            .unwrap();
        assert!(d.is_document(other));
        assert_eq!(d.parent(other), None);
        assert_eq!(d.owner_document(other), other);
        let html = d.create_element("html").unwrap();
        d.set_owner(html, other).unwrap();
        assert_eq!(d.owner_document(html), other, "made for it");
        d.append_child(other, html).unwrap();
        let body = d.create_element("body").unwrap();
        assert_eq!(d.owner_document(body), page);
        d.append_child(html, body).unwrap();
        // In its tree, the document's; the page's document is not its root.
        assert_eq!(d.owner_document(body), other);
        assert_eq!(d.document_element_of(other), Some(html));
        assert_eq!(d.document_element(), None);
        assert!(!d.subtree(page).any(|n| n == body));
        d.validate().unwrap();
        // Taken out of it, it keeps it as its owner.
        d.remove_child(html, body).unwrap();
        assert_eq!(d.owner_document(body), other);
        d.validate().unwrap();
        // Put into the page's, it is the page's.
        let main = d.create_element("html").unwrap();
        d.append_child(page, main).unwrap();
        d.append_child(main, body).unwrap();
        assert_eq!(d.owner_document(body), page);
        d.remove_child(main, body).unwrap();
        assert_eq!(
            d.owner_document(body),
            page,
            "out of the page's, the page's"
        );
        d.validate().unwrap();
    }

    #[test]
    fn adopting_takes_a_node_out_and_makes_it_the_documents() {
        let mut d = Document::new();
        let page = d.root();
        let other = d
            .create_document(DocumentFacts::xml("application/xml", "about:blank"))
            .unwrap();
        let html = d.create_element("html").unwrap();
        d.append_child(page, html).unwrap();
        let p = d.create_element("p").unwrap();
        d.append_child(html, p).unwrap();
        d.adopt(p, other).unwrap();
        assert_eq!(d.parent(p), None);
        assert_eq!(d.owner_document(p), other);
        assert!(d.is_xml_node(p) && !d.is_xml_node(html));
        assert_eq!(d.adopt(other, page), Err(MutationError::HierarchyRequest));
        // A document still some node's owner is not freed; once it is
        // not, it is.
        assert!(d.document_in_use(other));
        assert_eq!(d.free(other), Err(MutationError::Attached));
        d.adopt(p, page).unwrap();
        assert!(!d.document_in_use(other));
        d.free(other).unwrap();
        assert!(d.other_documents().is_empty());
        d.validate().unwrap();
    }

    #[test]
    fn a_templates_contents_are_owned_by_an_inert_document_of_its_documents_type() {
        let mut d = Document::new();
        let page = d.root();
        let template = d.create_element("template").unwrap();
        d.append_child(page, template).unwrap();
        let contents = d.template_content(template).unwrap();
        let b = d.create_element("b").unwrap();
        d.append_child(contents, b).unwrap();
        // Before it is made, the page's document answers for it.
        assert_eq!(d.owner_document(b), page);
        let inert = d.owner_document_made(b);
        assert_ne!(inert, page);
        assert_eq!(d.owner_document(contents), inert);
        assert_eq!(d.owner_document(b), inert);
        assert!(d.document_facts(inert).is_some_and(|f| f.inert && !f.xml));
        // An inert document's own is itself; one is made per document.
        assert_eq!(d.template_document(inert), inert);
        assert_eq!(d.template_document(page), inert);
        // Taken out of the contents, the node stays the inert document's.
        d.remove_child(contents, b).unwrap();
        assert_eq!(d.owner_document(b), inert);
        assert!(d.document_in_use(inert));
        d.validate().unwrap();
        // An XML document's is XML.
        let xml = d
            .create_document(DocumentFacts::xml("application/xml", "x"))
            .unwrap();
        let svg_template = d
            .create_element_ns(Namespace::Html, "template", Vec::new())
            .unwrap();
        d.append_child(xml, svg_template).unwrap();
        let inner = d.template_content(svg_template).unwrap();
        let made = d.owner_document_made(inner);
        assert!(d.is_xml(made) && made != inert);
        d.validate().unwrap();
    }

    #[test]
    fn a_copy_keeps_what_the_dom_keeps_beside_the_data_and_is_its_documents() {
        let mut d = Document::new();
        let xml = d
            .create_document(DocumentFacts::xml("application/xml", "x"))
            .unwrap();
        let element = d
            .create_element_for(xml, Namespace::Other(Some("urn:a".into())), "p:Item")
            .unwrap();
        d.mark_prefixed(element);
        d.set_attribute_ns(element, Some("urn:b"), "q:at", "1")
            .unwrap();
        let cdata = d.create_cdata("c").unwrap();
        d.append_child(element, cdata).unwrap();
        let copy = d.copy_node(element, xml, || unreachable!()).unwrap();
        assert_eq!(d.owner_document(copy), xml);
        assert_eq!(d.element_name(copy), d.element_name(element));
        assert_eq!(d.element_prefix(copy), Some("p"));
        assert_eq!(d.attr_ns(copy, Some("urn:b"), "at"), Some("1"));
        let text = d.copy_node(cdata, xml, || unreachable!()).unwrap();
        assert!(d.is_cdata(text));
        assert_eq!(d.node_type(text), Some(4));
        // A document's copy is a document of its facts; the page's, of
        // those the caller gives.
        let again = d.copy_node(xml, xml, || unreachable!()).unwrap();
        assert!(d.is_document(again) && d.is_xml(again) && again != xml);
        let page = d.root();
        let page_copy = d
            .copy_node(page, page, || DocumentFacts::html("about:blank"))
            .unwrap();
        assert!(!d.is_xml(page_copy));
        d.validate().unwrap();
        // The XML documents are counted: with none of another type than
        // the page's, a node's is the page's type without a walk.
        assert!(d.is_xml_node(element) && !d.is_xml_node(page));
        d.free(again).unwrap();
        d.free(page_copy).unwrap();
        d.validate().unwrap();
    }

    #[test]
    fn a_graft_copies_every_kind_of_node_and_the_templates_contents() {
        let mut from = Document::new();
        let root = from.root();
        let doctype = from.create_doctype("html", "", "").unwrap();
        from.append_child(root, doctype).unwrap();
        let html = from.create_element("html").unwrap();
        from.append_child(root, html).unwrap();
        let comment = from.create_comment("c").unwrap();
        from.append_child(html, comment).unwrap();
        let template = from.create_element("template").unwrap();
        from.append_child(html, template).unwrap();
        let inside = from.create_text("inside").unwrap();
        from.append_child(from.template_content(template).unwrap(), inside)
            .unwrap();
        let mut d = Document::new();
        let made = d.graft(&from, DocumentFacts::html("about:blank")).unwrap();
        let kinds: Vec<u16> = d
            .subtree(made)
            .map(|n| d.data(n).unwrap().node_type())
            .collect();
        assert_eq!(kinds, [9, 10, 1, 8, 1]);
        let copy = d
            .subtree(made)
            .find(|&n| d.tag(n) == Some("template"))
            .unwrap();
        let contents = d.template_content(copy).unwrap();
        assert_eq!(d.text_content(contents), "inside");
        assert_eq!(d.doctype_of(made), d.first_child(made));
        assert_eq!(d.data_bytes(), from.data_bytes());
        d.validate().unwrap();
        // Too big, it is refused whole.
        let mut small = Document::new();
        small.max_nodes = 3;
        assert_eq!(
            small.graft(&from, DocumentFacts::html("about:blank")),
            Err(MutationError::TooManyNodes)
        );
        assert!(small.other_documents().is_empty());
    }

    #[test]
    fn a_grafted_open_details_owes_its_toggle() {
        // As the parser made it open, in the document it is copied into:
        // the toggle the bindings fire (HTML section 4.11.1).
        let mut from = Document::new();
        let root = from.root();
        let html = from.create_element("html").unwrap();
        from.append_child(root, html).unwrap();
        for open in [true, false] {
            let attrs = match open {
                true => vec![("open".to_owned(), String::new())],
                false => Vec::new(),
            };
            let details = from.create_element_with("details", attrs).unwrap();
            from.append_child(html, details).unwrap();
        }
        let mut d = Document::new();
        d.take_details_toggles();
        let made = d.graft(&from, DocumentFacts::html("about:blank")).unwrap();
        let copies: Vec<NodeId> = d
            .subtree(made)
            .filter(|&n| d.tag(n) == Some("details"))
            .collect();
        assert_eq!(d.take_details_toggles(), [(copies[0], false)]);
    }
}
