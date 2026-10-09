//! A small tree of nodes that answers the bindings' calls, as the
//! engine's host does over a real document, for the tests of the
//! bindings themselves.

use std::collections::BTreeSet;

use crate::{Answer, Call, DomError, Handle};

#[derive(Debug, Clone)]
struct Node {
    /// 1 for an element, 3 for text, 9 for the document.
    kind: u8,
    tag: String,
    data: String,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// The tree: the document is node 0; a handle is a node's index.
#[derive(Debug, Clone)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
    /// Handles given out and not released.
    pub(crate) exposed: BTreeSet<u64>,
    /// Handles released, in order.
    pub(crate) released: Vec<u64>,
    /// The work each call reports.
    pub(crate) work_per_call: u64,
    work: u64,
}

impl Default for Tree {
    fn default() -> Tree {
        let mut tree = Tree {
            nodes: Vec::new(),
            exposed: BTreeSet::new(),
            released: Vec::new(),
            work_per_call: 0,
            work: 0,
        };
        tree.add(9, "", "");
        tree
    }
}

impl Tree {
    fn add(&mut self, kind: u8, tag: &str, data: &str) -> usize {
        self.nodes.push(Node {
            kind,
            tag: tag.to_owned(),
            data: data.to_owned(),
            parent: None,
            children: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// Appends a new element under `parent`; its index.
    pub(crate) fn element(&mut self, parent: usize, tag: &str) -> usize {
        let id = self.add(1, tag, "");
        self.nodes[id].parent = Some(parent);
        self.nodes[parent].children.push(id);
        id
    }

    /// Appends a new text node under `parent`; its index.
    pub(crate) fn text(&mut self, parent: usize, data: &str) -> usize {
        let id = self.add(3, "", data);
        self.nodes[id].parent = Some(parent);
        self.nodes[parent].children.push(id);
        id
    }

    pub(crate) fn tag(&self, node: usize) -> &str {
        &self.nodes[node].tag
    }

    pub(crate) fn children(&self, node: usize) -> &[usize] {
        &self.nodes[node].children
    }

    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    fn node(&self, handle: Handle) -> Result<usize, DomError> {
        let i = usize::try_from(handle.0).unwrap_or(usize::MAX);
        if i < self.nodes.len() {
            Ok(i)
        } else {
            Err(DomError::new("NotFoundError", "no such node"))
        }
    }

    fn sibling(&self, node: usize, step: isize) -> Option<usize> {
        let parent = self.nodes[node].parent?;
        let siblings = &self.nodes[parent].children;
        let at = siblings.iter().position(|&c| c == node)?;
        siblings.get(at.checked_add_signed(step)?).copied()
    }

    fn give(&mut self, node: Option<usize>) -> Answer {
        Answer::Node(node.map(|n| Handle(n as u64)))
    }

    fn text_content(&self, node: usize) -> String {
        let n = &self.nodes[node];
        if n.kind == 3 {
            return n.data.clone();
        }
        n.children.iter().map(|&c| self.text_content(c)).collect()
    }

    fn is_inclusive_ancestor(&self, ancestor: usize, mut node: usize) -> bool {
        loop {
            if node == ancestor {
                return true;
            }
            match self.nodes[node].parent {
                Some(parent) => node = parent,
                None => return false,
            }
        }
    }

    pub(crate) fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        self.work += self.work_per_call;
        Ok(match call {
            Call::Document => self.give(Some(0)),
            Call::Wrap(h) => {
                let node = self.node(h)?;
                self.exposed.insert(node as u64);
                let n = &self.nodes[node];
                let name = match n.kind {
                    1 => n.tag.clone(),
                    3 => "#text".to_owned(),
                    _ => "#document".to_owned(),
                };
                // The tests' tree holds no node's object: each is weak.
                Answer::Strings(vec![name, String::new()])
            }
            Call::Children(h) => {
                let node = self.node(h)?;
                let children = self.nodes[node].children.iter();
                Answer::Nodes(children.map(|&c| Handle(c as u64)).collect())
            }
            Call::Version => Answer::Number(0.0),
            Call::TakePins => Answer::Pins(Vec::new()),
            Call::Kind(h) => Answer::Number(f64::from(self.nodes[self.node(h)?].kind)),
            Call::Tag(h) => Answer::String(self.nodes[self.node(h)?].tag.clone()),
            Call::Parent(h) => {
                let parent = self.nodes[self.node(h)?].parent;
                self.give(parent)
            }
            Call::FirstChild(h) => {
                let first = self.nodes[self.node(h)?].children.first().copied();
                self.give(first)
            }
            Call::LastChild(h) => {
                let last = self.nodes[self.node(h)?].children.last().copied();
                self.give(last)
            }
            Call::PreviousSibling(h) => {
                let sibling = self.sibling(self.node(h)?, -1);
                self.give(sibling)
            }
            Call::NextSibling(h) => {
                let sibling = self.sibling(self.node(h)?, 1);
                self.give(sibling)
            }
            Call::Data(h) => Answer::String(self.nodes[self.node(h)?].data.clone()),
            Call::TextContent(h) => {
                let node = self.node(h)?;
                Answer::MaybeString((node != 0).then(|| self.text_content(node)))
            }
            Call::CreateElement(tag) => {
                if tag.is_empty() || tag.contains(' ') {
                    return Err(DomError::new("InvalidCharacterError", "bad name"));
                }
                let id = self.add(1, &tag.to_ascii_lowercase(), "");
                self.give(Some(id))
            }
            Call::CreateText(data) => {
                let id = self.add(3, "", &data);
                self.give(Some(id))
            }
            Call::Insert {
                parent,
                node,
                before,
            } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                if self.nodes[parent].kind == 3 || self.is_inclusive_ancestor(node, parent) {
                    return Err(DomError::new("HierarchyRequestError", "no"));
                }
                if let Some(old) = self.nodes[node].parent.take() {
                    self.nodes[old].children.retain(|&c| c != node);
                }
                let at = match before {
                    Some(before) => {
                        let before = self.node(before)?;
                        self.nodes[parent]
                            .children
                            .iter()
                            .position(|&c| c == before)
                            .ok_or_else(|| DomError::new("NotFoundError", "not a child"))?
                    }
                    None => self.nodes[parent].children.len(),
                };
                self.nodes[parent].children.insert(at, node);
                self.nodes[node].parent = Some(parent);
                Answer::Nothing
            }
            Call::Remove { parent, node } => {
                let (parent, node) = (self.node(parent)?, self.node(node)?);
                if self.nodes[node].parent != Some(parent) {
                    return Err(DomError::new("NotFoundError", "not a child"));
                }
                self.nodes[parent].children.retain(|&c| c != node);
                self.nodes[node].parent = None;
                Answer::Nothing
            }
            Call::Released(h) => {
                self.exposed.remove(&h.0);
                self.released.push(h.0);
                Answer::Nothing
            }
            _ => return Err(DomError::not_supported()),
        })
    }

    pub(crate) fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }
}
