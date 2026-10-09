//! The host a document's scripts reach it through (M24.4, M25,
//! `docs/design/dom-scripting.md`): the document, lent for each run, and
//! what must last between runs: the nodes scripts hold objects for, and
//! the subtrees they took out of the tree, freed once nothing holds them.

use std::collections::HashSet;

use dom::{Document, MutationError, NodeData, NodeId};
use js::{Answer, Call, ConsoleLevel, DomError, Handle, ScriptingHost};

use super::{ConsoleEntry, cut, document_title};

/// The host a document's scripts reach it through.
#[derive(Default)]
pub(crate) struct DocumentHost {
    /// The document, lent for a run.
    pub(crate) document: Document,
    /// What scripts wrote to the console in the run.
    pub(crate) console: Vec<ConsoleEntry>,
    /// The nodes a script holds an object for: handles given out and not
    /// released (M25). None of them is freed.
    exposed: HashSet<NodeId>,
    /// The roots of the subtrees scripts made or took out of the tree,
    /// freed once out of it with nothing in them exposed.
    detached: Vec<NodeId>,
    /// The work done since the realm last asked.
    work: u64,
}

impl ScriptingHost for DocumentHost {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.console.push(ConsoleEntry {
            level,
            text: cut(message),
        });
    }

    fn title(&mut self) -> String {
        document_title(&self.document)
    }

    fn set_title(&mut self, title: String) {
        self.set_document_title(&title);
    }

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        self.work += 1;
        match call {
            Call::Document => Ok(self.give(Some(self.document.root()))),
            Call::Wrap(node) => {
                let id = self.node(node)?;
                self.exposed.insert(id);
                let name = match self.document.data(id) {
                    Some(NodeData::Element { tag, .. }) => tag.clone(),
                    Some(NodeData::Text(_)) => "#text".to_owned(),
                    _ => "#document".to_owned(),
                };
                Ok(Answer::String(name))
            }
            Call::Kind(node) => {
                let kind = match self.document.data(self.node(node)?) {
                    Some(NodeData::Element { .. }) => 1.0,
                    Some(NodeData::Text(_)) => 3.0,
                    _ => 9.0,
                };
                Ok(Answer::Number(kind))
            }
            Call::Tag(node) => {
                let tag = self.document.tag(self.node(node)?).unwrap_or_default();
                Ok(Answer::String(tag.to_owned()))
            }
            Call::Parent(node) => {
                let parent = self.document.parent(self.node(node)?);
                Ok(self.give(parent))
            }
            Call::FirstChild(node) => {
                let child = self.document.first_child(self.node(node)?);
                Ok(self.give(child))
            }
            Call::LastChild(node) => {
                let child = self.document.last_child(self.node(node)?);
                Ok(self.give(child))
            }
            Call::PreviousSibling(node) => {
                let sibling = self.document.prev_sibling(self.node(node)?);
                Ok(self.give(sibling))
            }
            Call::NextSibling(node) => {
                let sibling = self.document.next_sibling(self.node(node)?);
                Ok(self.give(sibling))
            }
            Call::Children(node) => {
                let node = self.node(node)?;
                let children: Vec<Handle> = self.document.children(node).map(handle).collect();
                self.work += children.len() as u64;
                Ok(Answer::Nodes(children))
            }
            Call::ElementChildren(node) => {
                let node = self.node(node)?;
                let mut children = Vec::new();
                for child in self.document.children(node) {
                    self.work += 1;
                    if self.document.is_element(child) {
                        children.push(handle(child));
                    }
                }
                Ok(Answer::Nodes(children))
            }
            Call::Version => Ok(Answer::Number(self.document.version() as f64)),
            Call::Connected(node) => {
                let node = self.node(node)?;
                let root = self.document.root();
                let top = self.document.ancestors(node).last().unwrap_or(node);
                Ok(Answer::Bool(top == root))
            }
            Call::Contains { node, other } => {
                let (node, other) = (self.node(node)?, self.node(other)?);
                Ok(Answer::Bool(
                    self.document.is_inclusive_ancestor(node, other),
                ))
            }
            Call::Data(node) => {
                let data = self.document.text(self.node(node)?).unwrap_or_default();
                self.work += data.len() as u64 / 64;
                Ok(Answer::String(data.to_owned()))
            }
            Call::SetData(node, data) => {
                let node = self.node(node)?;
                self.work += data.len() as u64 / 64;
                self.document.set_text(node, &data).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::TextContent(node) => {
                let node = self.node(node)?;
                if node == self.document.root() {
                    return Ok(Answer::MaybeString(None));
                }
                let mut text = String::new();
                for n in self.document.subtree(node) {
                    self.work += 1;
                    if let Some(data) = self.document.text(n) {
                        text.push_str(data);
                    }
                }
                self.work += text.len() as u64 / 64;
                Ok(Answer::MaybeString(Some(text)))
            }
            Call::SetTextContent(node, text) => {
                let node = self.node(node)?;
                self.work += text.len() as u64 / 64;
                self.set_text_content(node, &text)?;
                Ok(Answer::Nothing)
            }
            Call::Attribute(node, name) => {
                let value = self.document.attr(self.node(node)?, &name);
                Ok(Answer::MaybeString(value.map(str::to_owned)))
            }
            Call::SetAttribute(node, name, value) => {
                let node = self.node(node)?;
                if !valid_attribute_local_name(&name) {
                    return Err(DomError::new(
                        "InvalidCharacterError",
                        format!("'{name}' is not a valid attribute name."),
                    ));
                }
                self.work += (name.len() + value.len()) as u64 / 64;
                self.document
                    .set_attribute(node, &name, &value)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::RemoveAttribute(node, name) => {
                let node = self.node(node)?;
                // Only an element has attributes; another node has none
                // to remove.
                if self.document.is_element(node) {
                    self.document
                        .remove_attribute(node, &name)
                        .map_err(refused)?;
                }
                Ok(Answer::Nothing)
            }
            Call::AttributeNames(node) => {
                let attrs = self.document.attrs(self.node(node)?).unwrap_or_default();
                Ok(Answer::Strings(
                    attrs.iter().map(|(name, _)| name.clone()).collect(),
                ))
            }
            Call::CreateElement(name) => {
                if !valid_element_local_name(&name) {
                    return Err(DomError::new(
                        "InvalidCharacterError",
                        format!("The tag name provided ('{name}') is not a valid name."),
                    ));
                }
                let element = self.document.create_element(&name).map_err(refused)?;
                self.detached.push(element);
                Ok(self.give(Some(element)))
            }
            Call::CreateText(data) => {
                self.work += data.len() as u64 / 64;
                let text = self.document.create_text(&data).map_err(refused)?;
                self.detached.push(text);
                Ok(self.give(Some(text)))
            }
            Call::Insert {
                parent,
                node,
                before,
            } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                let before = before.map(|before| self.node(before)).transpose()?;
                self.insert(parent, node, before)?;
                Ok(Answer::Nothing)
            }
            Call::Remove { parent, node } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                if self.document.parent(node) != Some(parent) {
                    return Err(DomError::new(
                        "NotFoundError",
                        "The node to be removed is not a child of this node.",
                    ));
                }
                self.document.remove_child(parent, node).map_err(refused)?;
                self.detached.push(node);
                Ok(Answer::Nothing)
            }
            Call::Replace {
                parent,
                node,
                child,
            } => {
                let (parent, node, child) =
                    (self.node(parent)?, self.node(node)?, self.node(child)?);
                self.replace(parent, node, child)?;
                Ok(Answer::Nothing)
            }
            Call::Clone { node, deep } => {
                let node = self.node(node)?;
                let copy = self.clone_node(node, deep)?;
                Ok(self.give(Some(copy)))
            }
            Call::Released(node) => {
                if let Some(id) = dom::NodeId::from_bits(node.0) {
                    self.exposed.remove(&id);
                }
                Ok(Answer::Nothing)
            }
        }
    }

    fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }
}

impl DocumentHost {
    /// The node `handle` names, which a script holds and so is live.
    fn node(&self, handle: Handle) -> Result<NodeId, DomError> {
        NodeId::from_bits(handle.0)
            .filter(|&id| self.document.contains(id))
            .ok_or_else(|| DomError::new("NotFoundError", "The node is no longer there."))
    }

    /// `node`'s handle, given to a script. The script holds the node once
    /// it makes an object for it ([`Call::Wrap`]).
    fn give(&mut self, node: Option<NodeId>) -> Answer {
        Answer::Node(node.map(handle))
    }

    /// The DOM's "pre-insert": its checks, in its order, then `node`
    /// taken from where it was and put under `parent` before `before`.
    fn insert(
        &mut self,
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        let document = &self.document;
        let hierarchy = |reason: &str| Err(DomError::new("HierarchyRequestError", reason));
        if document.is_text(parent) {
            return hierarchy("This node type does not support this method.");
        }
        if document.is_inclusive_ancestor(node, parent) {
            return hierarchy("The new child element contains the parent.");
        }
        if let Some(before) = before
            && document.parent(before) != Some(parent)
        {
            return Err(DomError::new(
                "NotFoundError",
                "The node before which the new node is to be inserted is not a child of this \
                 node.",
            ));
        }
        if node == document.root() {
            return hierarchy(
                "Nodes of type '#document' may not be inserted inside nodes of type '#document'.",
            );
        }
        if parent == document.root() {
            if document.is_text(node) {
                return hierarchy(
                    "Nodes of type '#text' may not be inserted inside nodes of type '#document'.",
                );
            }
            if document.element_children(parent).next().is_some() {
                return hierarchy("Only one element on document allowed.");
            }
        }
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)
    }

    /// The DOM's "replace a child": its checks, in its order, then `child`
    /// taken out and `node` put in its place.
    fn replace(&mut self, parent: NodeId, node: NodeId, child: NodeId) -> Result<(), DomError> {
        let document = &self.document;
        let hierarchy = |reason: &str| Err(DomError::new("HierarchyRequestError", reason));
        if document.is_text(parent) {
            return hierarchy("This node type does not support this method.");
        }
        if document.is_inclusive_ancestor(node, parent) {
            return hierarchy("The new child element contains the parent.");
        }
        if document.parent(child) != Some(parent) {
            return Err(DomError::new(
                "NotFoundError",
                "The node to be replaced is not a child of this node.",
            ));
        }
        if parent == document.root() {
            if document.is_text(node) {
                return hierarchy(
                    "Nodes of type '#text' may not be inserted inside nodes of type '#document'.",
                );
            }
            if document.is_element(node)
                && document
                    .element_children(parent)
                    .any(|other| other != child)
            {
                return hierarchy("Only one element on document allowed.");
            }
        }
        // `dom` takes a reference that is `node` itself for the place after
        // it, as the DOM's step does by hand.
        let before = document.next_sibling(child);
        if document.first_child(node).is_some() {
            self.work += document.subtree(node).count() as u64;
        }
        self.document.remove_child(parent, child).map_err(refused)?;
        self.detached.push(child);
        self.document
            .insert_before(parent, node, before)
            .map_err(refused)
    }

    /// The DOM's `textContent` setter: an element's children replaced by
    /// one text node holding `text`, or by none for the empty string; a
    /// text node's data set; nothing for the document.
    fn set_text_content(&mut self, node: NodeId, text: &str) -> Result<(), DomError> {
        if self.document.is_text(node) {
            return self.document.set_text(node, text).map_err(refused);
        }
        if node == self.document.root() {
            return Ok(());
        }
        let made = if text.is_empty() {
            None
        } else {
            Some(self.document.create_text(text).map_err(refused)?)
        };
        while let Some(child) = self.document.first_child(node) {
            self.work += 1;
            if self.document.remove_child(node, child).is_err() {
                break;
            }
            self.detached.push(child);
        }
        if let Some(made) = made
            && let Err(error) = self.document.append_child(node, made)
        {
            let _ = self.document.free(made);
            return Err(refused(error));
        }
        Ok(())
    }

    /// A detached copy of `node` (DOM section 4.5, "clone a node"): its
    /// tag and attributes or its data, and its descendants' if `deep`.
    fn clone_node(&mut self, node: NodeId, deep: bool) -> Result<NodeId, DomError> {
        let copy = |document: &mut Document, from: NodeId| -> Result<NodeId, MutationError> {
            match document.data(from) {
                Some(NodeData::Element { tag, attrs }) => {
                    let (tag, attrs) = (tag.clone(), attrs.clone());
                    document.create_element_with(&tag, attrs)
                }
                Some(NodeData::Text(data)) => {
                    let data = data.clone();
                    document.create_text(&data)
                }
                _ => Err(MutationError::WrongKind),
            }
        };
        let top = copy(&mut self.document, node).map_err(refused)?;
        self.detached.push(top);
        if !deep {
            return Ok(top);
        }
        // Each entry: a node whose children are still to copy, and its copy.
        let mut pending = vec![(node, top)];
        while let Some((from, to)) = pending.pop() {
            let children: Vec<NodeId> = self.document.children(from).collect();
            for child in children {
                self.work += 1;
                let made = copy(&mut self.document, child).map_err(refused)?;
                if let Err(error) = self.document.append_new(to, made) {
                    let _ = self.document.free(made);
                    return Err(refused(error));
                }
                pending.push((child, made));
            }
        }
        Ok(top)
    }

    /// `document.title`'s setter (HTML section 3.1.7): the first `title`
    /// element's children replaced by `value`'s text, a `title` made last
    /// in the head if there is none; nothing without a document element,
    /// or without a head to put a `title` in. The children replaced are
    /// detached, and freed once no script holds them.
    pub(crate) fn set_document_title(&mut self, value: &str) {
        let document = &mut self.document;
        let Some(html) = document.document_element() else {
            return;
        };
        let existing = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("title"));
        let title = match existing {
            Some(title) => title,
            None => {
                let Some(head) = document
                    .element_children(html)
                    .find(|&child| document.tag(child) == Some("head"))
                else {
                    return;
                };
                let Ok(title) = document.create_element("title") else {
                    return;
                };
                if document.append_child(head, title).is_err() {
                    let _ = document.free(title);
                    return;
                }
                title
            }
        };
        // String replace all: the children go, the text comes in their place.
        let children: Vec<NodeId> = document.children(title).collect();
        for child in children {
            if document.remove_child(title, child).is_ok() {
                self.detached.push(child);
            }
        }
        if !value.is_empty()
            && let Ok(text) = document.create_text(value)
            && document.append_child(title, text).is_err()
        {
            let _ = document.free(text);
        }
    }

    /// Frees the subtrees scripts took out of the tree that are still
    /// out of it and hold no node a script has an object for; keeps the
    /// rest to look at again after the next run (M25).
    pub(crate) fn sweep(&mut self) {
        let document = &mut self.document;
        let exposed = &self.exposed;
        self.detached.retain(|&root| {
            if !document.contains(root)
                || root == document.root()
                || document.parent(root).is_some()
            {
                return false;
            }
            if document.subtree(root).any(|node| exposed.contains(&node)) {
                return true;
            }
            let _ = document.free(root);
            false
        });
    }

    /// The realm is gone, and with it every object a script held.
    pub(crate) fn forget_objects(&mut self) {
        self.exposed.clear();
    }
}

/// A node's handle.
fn handle(node: NodeId) -> Handle {
    Handle(node.to_bits())
}

/// The DOM's "valid attribute local name": not empty, and no ASCII
/// white space, NUL, `/`, `=` or `>`.
pub(crate) fn valid_attribute_local_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '=' | '>'))
}

/// A refusal of `dom`'s as the exception a script gets.
fn refused(error: MutationError) -> DomError {
    match error {
        MutationError::TooManyNodes => {
            DomError::new("RangeError", "The document holds too many nodes.")
        }
        MutationError::TooMuchData => {
            DomError::new("RangeError", "The document holds too much data.")
        }
        MutationError::TooDeep => DomError::new(
            "HierarchyRequestError",
            "The node would be nested too deeply.",
        ),
        MutationError::NotAChild => DomError::new(
            "NotFoundError",
            "The node before which the new node is to be inserted is not a child of this node.",
        ),
        MutationError::NoSuchNode => DomError::new("NotFoundError", "The node is no longer there."),
        MutationError::HierarchyRequest | MutationError::WrongKind | MutationError::Attached => {
            DomError::new(
                "HierarchyRequestError",
                "The operation would yield an incorrect node tree.",
            )
        }
    }
}

/// The DOM's "valid element local name": a name the HTML parser could
/// make (an ASCII letter first, then anything but ASCII white space,
/// NUL, `/` and `>`), or one of a few more beginning `:`, `_` or past
/// ASCII.
pub(crate) fn valid_element_local_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        return !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '>'));
    }
    if !(matches!(first, ':' | '_') || first >= '\u{80}') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_') || c >= '\u{80}')
}
