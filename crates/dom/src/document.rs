//! The live document (M13, docs/design/incremental.md): every node in
//! one arena of slots, named by a [`NodeId`] that stays the same while
//! the node lives, whatever is inserted or removed around it, so that
//! the stages after the DOM can say "this element changed" and keep
//! what they computed for the rest.
//!
//! A [`NodeId`] is a slot's index and the slot's generation. Freeing a
//! node bumps its slot's generation before the slot is used again, so
//! an id kept after its node is gone resolves to nothing: every
//! accessor answers `None` for it and every mutation an error, and no
//! id, stale or forged, makes this crate panic. A slot whose generation
//! is exhausted is never used again.
//!
//! Each node links to its parent, its first and last child and its
//! siblings, so every step through the tree is one lookup, and every
//! walk here is iterative: a tree of any depth is safe to traverse and
//! to drop.
//!
//! The mutations are the DOM standard's operations, checked as the
//! standard checks them before anything changes — no node inserted
//! into itself or a descendant, no child under a text node, no text
//! directly under the document — and against two limits of this
//! engine: [`MAX_DEPTH`] nested elements, the HTML parser's own cap,
//! which the recursive stages after the DOM rely on, and
//! [`MAX_NODES`] live nodes. A failed check leaves the document as it
//! was. A removed node stays valid and detached, and can be inserted
//! again, as the DOM's `removeChild` leaves it; [`Document::free`]
//! releases a detached subtree. Every change is recorded in the
//! document's journal ([`Change`], [`Document::take_changes`]) in the
//! DOM's own terms, for the stages after it to read.

use std::fmt;

use crate::{Element, Node, Text};

/// The deepest elements may nest: 512, the HTML parser's cap on its
/// stack of open elements (`html::MAX_DEPTH` is this constant), which
/// keeps the recursive walks over a finished tree within any thread's
/// stack. A text node may sit one level below the deepest element.
pub const MAX_DEPTH: usize = 512;

/// The most nodes a document may hold alive through its mutation API:
/// about four million, past any real page (the largest have a few
/// hundred thousand) and short of what would exhaust memory before
/// their attributes and text are counted. [`Document::from_node`] takes
/// a tree as it is; the parser bounds what it builds.
pub const MAX_NODES: usize = 1 << 22;

/// The most records the journal keeps between two drains. Past it the
/// records are replaced by one [`Change::Everything`]: a reader then
/// treats the whole document as changed, which is always correct, and
/// a document mutated many times with nobody reading the journal does
/// not grow it without bound.
pub const MAX_JOURNAL: usize = 1 << 16;

/// The most bytes of text, tag names and attribute names and values a
/// document may hold in its live nodes through its mutation API: 256
/// MiB, what a script's heap may hold (M25), so that a script cannot
/// make the document keep a string many times over that it holds
/// once. No page parsed from a body within net's limit comes near it.
pub const MAX_DATA_BYTES: usize = 256 << 20;

/// The generations a slot may have: a slot is retired once this many
/// nodes have had it, so that an id fits in the 53 bits a script's
/// number holds exactly ([`NodeId::to_bits`]). A slot reused two
/// million times is let go of, a few dozen bytes.
pub const MAX_GENERATION: u32 = 1 << 21;

/// A node's name in its [`Document`]: its slot and the slot's
/// generation, so that it names this node and no later one in the
/// same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    index: u32,
    generation: u32,
}

impl NodeId {
    /// The slot's index: dense, below [`Document::slot_count`], and
    /// shared by successive nodes in the slot, so a table indexed by it
    /// must check the generation too (or use the whole id).
    #[inline]
    pub fn index(self) -> usize {
        self.index as usize
    }

    /// The slot's generation when this node was created in it.
    #[inline]
    pub fn generation(self) -> u32 {
        self.generation
    }

    /// The id as one number below 2^53: the index in the low 32 bits,
    /// the generation above them (M25, a script's handle on a node).
    #[inline]
    pub fn to_bits(self) -> u64 {
        u64::from(self.index) | (u64::from(self.generation) << 32)
    }

    /// The id [`NodeId::to_bits`] made `bits` from; `None` for a number
    /// no id makes. Whether it names a live node is the document's to
    /// say.
    pub fn from_bits(bits: u64) -> Option<NodeId> {
        let generation = u32::try_from(bits >> 32).ok()?;
        if generation >= MAX_GENERATION {
            return None;
        }
        Some(NodeId {
            index: bits as u32,
            generation,
        })
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}.{}", self.index, self.generation)
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeData {
    /// The document node, the root; there is one per document.
    Document,
    /// An element: its lowercase tag name and its attributes, in order.
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
    },
    /// Character data.
    Text(String),
}

/// One change to the document, as the DOM sees it. Ids may be stale by
/// the time a record is read, if a later change freed the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// `child` was inserted under `parent`.
    Inserted { parent: NodeId, child: NodeId },
    /// `child` was removed from `parent`, where it stood between `prev`
    /// and `next` (each `None` at an end).
    Removed {
        parent: NodeId,
        child: NodeId,
        prev: Option<NodeId>,
        next: Option<NodeId>,
    },
    /// Attribute `name` of `element` was set or removed; `old` is its
    /// value before, `None` when it had none.
    AttributeChanged {
        element: NodeId,
        name: String,
        old: Option<String>,
    },
    /// A text node's data changed.
    TextChanged { node: NodeId },
    /// More changed than the journal keeps ([`MAX_JOURNAL`]): treat the
    /// whole document as changed.
    Everything,
}

/// Why a mutation was refused; the document is as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationError {
    /// An id names no live node of this document.
    NoSuchNode,
    /// The DOM forbids the insertion: a node into itself or one of its
    /// descendants, anything under a text node, the document node
    /// anywhere, or text directly under the document.
    HierarchyRequest,
    /// The reference node is not a child of the parent.
    NotAChild,
    /// The insertion would nest elements deeper than [`MAX_DEPTH`].
    TooDeep,
    /// The document already holds [`MAX_NODES`] nodes.
    TooManyNodes,
    /// The change would take the document's data past
    /// [`MAX_DATA_BYTES`].
    TooMuchData,
    /// The operation needs an element, or a text node, and got another
    /// kind of node.
    WrongKind,
    /// [`Document::free`] was given an attached node, or the root.
    Attached,
}

impl fmt::Display for MutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MutationError::NoSuchNode => "no such node",
            MutationError::HierarchyRequest => "the insertion is not allowed there",
            MutationError::NotAChild => "the reference node is not a child of the parent",
            MutationError::TooDeep => "the insertion would nest elements too deeply",
            MutationError::TooManyNodes => "the document holds too many nodes",
            MutationError::TooMuchData => "the document holds too much data",
            MutationError::WrongKind => "the node is of the wrong kind",
            MutationError::Attached => "the node is attached or is the document",
        })
    }
}

impl std::error::Error for MutationError {}

/// A live node: what it is and where it is. Links are slot indices of
/// live nodes, so their generations are the slots' current ones.
#[derive(Debug, Clone)]
struct Record {
    data: NodeData,
    parent: Option<u32>,
    first_child: Option<u32>,
    last_child: Option<u32>,
    prev_sibling: Option<u32>,
    next_sibling: Option<u32>,
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    record: Option<Record>,
}

/// A document: its nodes, its root, and the journal of its changes.
#[derive(Debug, Clone)]
pub struct Document {
    slots: Vec<Slot>,
    /// Freed slots ready for reuse, the most recently freed last.
    free: Vec<u32>,
    live: usize,
    root: NodeId,
    journal: Vec<Change>,
    /// How many changes have been made, ever.
    version: u64,
    /// The bytes of data the live nodes hold ([`MAX_DATA_BYTES`]).
    data_bytes: usize,
    /// [`MAX_NODES`], lowered by tests.
    max_nodes: usize,
    /// [`MAX_DATA_BYTES`], lowered by tests.
    max_data: usize,
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

impl Document {
    /// A document with nothing under its root.
    pub fn new() -> Self {
        let mut document = Document {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
            root: NodeId {
                index: 0,
                generation: 0,
            },
            journal: Vec::new(),
            version: 0,
            data_bytes: 0,
            max_nodes: MAX_NODES,
            max_data: MAX_DATA_BYTES,
        };
        document.root = document.allocate(NodeData::Document);
        document
    }

    /// The document `node` describes: a [`Node::Document`]'s children
    /// become the root's, anything else becomes the root's one child.
    /// The tree is taken as it is, its depth and size unchecked (the
    /// parser bounds what it builds; a tree built by hand is its
    /// builder's to bound), and the journal starts empty. Linear, and
    /// iterative at any depth.
    pub fn from_node(node: &Node) -> Self {
        let mut document = Document::new();
        let root = document.root.index;
        let top: Vec<&Node> = match node {
            Node::Document(children) => children.iter().collect(),
            other => vec![other],
        };
        // Each entry: a node still to add and the slot to add it under.
        let mut pending: Vec<(&Node, u32)> = top.into_iter().rev().map(|n| (n, root)).collect();
        while let Some((node, parent)) = pending.pop() {
            let data = match node {
                // A document inside a document is taken for its children.
                Node::Document(children) => {
                    pending.extend(children.iter().rev().map(|c| (c, parent)));
                    continue;
                }
                Node::Element(Element { tag, attrs, .. }) => NodeData::Element {
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                },
                Node::Text(Text { data }) => NodeData::Text(data.clone()),
            };
            let id = document.allocate(data);
            document.link_last(parent, id.index);
            pending.extend(node.children().iter().rev().map(|c| (c, id.index)));
        }
        document
    }

    /// The tree as an owned [`Node`], a [`Node::Document`] — the
    /// snapshot a test compares or a fresh pipeline starts from.
    /// Iterative at any depth.
    pub fn to_node(&self) -> Node {
        self.subtree_node(self.root)
            .unwrap_or_else(|| Node::document(Vec::new()))
    }

    /// The subtree at `id` as an owned [`Node`]; `None` for a stale id.
    pub fn subtree_node(&self, id: NodeId) -> Option<Node> {
        self.record(id)?;
        // Children are built before their parent: a postorder walk
        // keeps a stack of finished child lists, one per open node.
        let mut finished: Vec<Vec<Node>> = vec![Vec::new()];
        let mut stack: Vec<(u32, bool)> = vec![(id.index, false)];
        while let Some((index, expanded)) = stack.pop() {
            let record = self.slots[index as usize].record.as_ref()?;
            if !expanded {
                stack.push((index, true));
                finished.push(Vec::new());
                let mut child = record.last_child;
                while let Some(c) = child {
                    stack.push((c, false));
                    child = self.slots[c as usize].record.as_ref()?.prev_sibling;
                }
                continue;
            }
            let children = finished.pop()?;
            let node = match &record.data {
                NodeData::Document => Node::Document(children),
                NodeData::Element { tag, attrs } => Node::Element(Element {
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                    children,
                }),
                NodeData::Text(data) => Node::Text(Text { data: data.clone() }),
            };
            finished.last_mut()?.push(node);
        }
        finished.pop()?.pop()
    }

    // ---- Reading ----------------------------------------------------------

    /// The document node.
    #[inline]
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The first element under the document node, the root element
    /// (`html` in a parsed page).
    pub fn document_element(&self) -> Option<NodeId> {
        self.children(self.root).find(|&c| self.is_element(c))
    }

    /// How many nodes are alive, attached or not.
    pub fn len(&self) -> usize {
        self.live
    }

    /// Whether the document holds nothing but its root.
    pub fn is_empty(&self) -> bool {
        self.live <= 1
    }

    /// One past the highest slot index in use: the size a table indexed
    /// by [`NodeId::index`] needs.
    #[inline]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// The live node in slot `index`, if any.
    #[inline]
    pub fn id_at(&self, index: usize) -> Option<NodeId> {
        let slot = self.slots.get(index)?;
        slot.record.as_ref()?;
        Some(NodeId {
            index: index as u32,
            generation: slot.generation,
        })
    }

    /// Whether `id` names a live node of this document.
    #[inline]
    pub fn contains(&self, id: NodeId) -> bool {
        self.record(id).is_some()
    }

    /// What `id` is.
    #[inline]
    pub fn data(&self, id: NodeId) -> Option<&NodeData> {
        self.record(id).map(|r| &r.data)
    }

    /// Whether `id` is an element.
    #[inline]
    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.data(id), Some(NodeData::Element { .. }))
    }

    /// Whether `id` is a text node.
    #[inline]
    pub fn is_text(&self, id: NodeId) -> bool {
        matches!(self.data(id), Some(NodeData::Text(_)))
    }

    /// An element's tag name.
    #[inline]
    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match self.data(id)? {
            NodeData::Element { tag, .. } => Some(tag),
            _ => None,
        }
    }

    /// An element's attributes, in order.
    #[inline]
    pub fn attrs(&self, id: NodeId) -> Option<&[(String, String)]> {
        match self.data(id)? {
            NodeData::Element { attrs, .. } => Some(attrs),
            _ => None,
        }
    }

    /// An element's attribute `name` (matched exactly; names are stored
    /// lowercase by the parser and by [`Document::set_attribute`]).
    #[inline]
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.attrs(id)?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// A text node's data.
    #[inline]
    pub fn text(&self, id: NodeId) -> Option<&str> {
        match self.data(id)? {
            NodeData::Text(data) => Some(data),
            _ => None,
        }
    }

    /// The node's parent; `None` for the root and a detached node.
    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.link(self.record(id)?.parent)
    }

    /// The node's parent when that is an element. (One lookup per node
    /// visited: selector matching walks these links for every element.)
    #[inline]
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.record(id)?.parent?;
        self.element_at(parent)
    }

    #[inline]
    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.link(self.record(id)?.first_child)
    }

    #[inline]
    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.link(self.record(id)?.last_child)
    }

    #[inline]
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.link(self.record(id)?.next_sibling)
    }

    #[inline]
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.link(self.record(id)?.prev_sibling)
    }

    /// The nearest element before `id` among its siblings.
    #[inline]
    pub fn prev_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut at = self.record(id)?.prev_sibling;
        while let Some(index) = at {
            let slot = &self.slots[index as usize];
            let record = slot.record.as_ref()?;
            if matches!(record.data, NodeData::Element { .. }) {
                return Some(NodeId {
                    index,
                    generation: slot.generation,
                });
            }
            at = record.prev_sibling;
        }
        None
    }

    /// The nearest element after `id` among its siblings.
    #[inline]
    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut at = self.record(id)?.next_sibling;
        while let Some(index) = at {
            let slot = &self.slots[index as usize];
            let record = slot.record.as_ref()?;
            if matches!(record.data, NodeData::Element { .. }) {
                return Some(NodeId {
                    index,
                    generation: slot.generation,
                });
            }
            at = record.next_sibling;
        }
        None
    }

    /// The node's children, first to last; none for a stale id.
    #[inline]
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.first_child(id),
        }
    }

    /// The node's element children, first to last.
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id).filter(|&c| self.is_element(c))
    }

    /// The node's ancestors, its parent first and the root (for an
    /// attached node) last.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), |&p| self.parent(p))
    }

    /// The subtree at `id` in tree order (preorder), `id` first; empty
    /// for a stale id. Iterative.
    pub fn subtree(&self, id: NodeId) -> Subtree<'_> {
        Subtree {
            document: self,
            top: id,
            next: self.contains(id).then_some(id),
        }
    }

    /// Whether `ancestor` is `node` or one of its ancestors.
    pub fn is_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        self.contains(node) && (ancestor == node || self.ancestors(node).any(|a| a == ancestor))
    }

    /// The text of the subtree at `id`, in tree order.
    pub fn text_content(&self, id: NodeId) -> String {
        self.subtree(id).filter_map(|n| self.text(n)).collect()
    }

    // ---- Changing -----------------------------------------------------------

    /// A new, detached element; the tag name is lowercased, as an HTML
    /// document's `createElement` does.
    pub fn create_element(&mut self, tag: &str) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(tag.len())?;
        Ok(self.allocate(NodeData::Element {
            tag: tag.to_ascii_lowercase(),
            attrs: Vec::new(),
        }))
    }

    /// A new, detached element with `attrs` as they are given, which
    /// their maker has lowercased and freed of duplicates: what a parser
    /// makes of a start tag (M24.8), its attributes not checked one by
    /// one against the others, which a tag of many would make
    /// quadratic.
    pub fn create_element_with(
        &mut self,
        tag: &str,
        attrs: Vec<(String, String)>,
    ) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(tag.len() + attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>())?;
        Ok(self.allocate(NodeData::Element {
            tag: tag.to_ascii_lowercase(),
            attrs,
        }))
    }

    /// A new, detached text node.
    pub fn create_text(&mut self, data: &str) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(data.len())?;
        Ok(self.allocate(NodeData::Text(data.to_string())))
    }

    /// Inserts `child` as `parent`'s last child, taking it from where it
    /// was first, as the DOM's `appendChild` does.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), MutationError> {
        self.insert_before(parent, child, None)
    }

    /// Inserts `child` under `parent` before `reference`, or last when
    /// `reference` is `None`, taking it from where it was first, as the
    /// DOM's `insertBefore` does (a `reference` that is `child` itself
    /// means the place after it).
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), MutationError> {
        let parent_data = self.data(parent).ok_or(MutationError::NoSuchNode)?;
        let child_data = self.data(child).ok_or(MutationError::NoSuchNode)?;
        if let Some(r) = reference
            && self.parent(r) != Some(parent)
        {
            return Err(if self.contains(r) {
                MutationError::NotAChild
            } else {
                MutationError::NoSuchNode
            });
        }
        let allowed = match (parent_data, child_data) {
            (NodeData::Text(_), _) | (_, NodeData::Document) => false,
            (NodeData::Document, NodeData::Text(_)) => false,
            _ => !self.is_inclusive_ancestor(child, parent),
        };
        if !allowed {
            return Err(MutationError::HierarchyRequest);
        }
        let depth = self.element_depth(parent) + self.element_height(child);
        if depth > MAX_DEPTH {
            return Err(MutationError::TooDeep);
        }
        let reference = match reference {
            Some(r) if r == child => self.next_sibling(child),
            other => other,
        };
        self.detach(child);
        match reference {
            Some(r) => self.link_before(parent.index, child.index, r.index),
            None => self.link_last(parent.index, child.index),
        }
        self.record_change(Change::Inserted { parent, child });
        Ok(())
    }

    /// Appends `child`, a node made and never inserted, so detached and
    /// with nothing under it, as `parent`'s last child: what a parser
    /// does with each node it makes (M24.8). It is `append_child` without
    /// the walks up the tree, which a page nested at the parser's depth
    /// cap would make quadratic: the depth is not checked, the parser
    /// bounding its own as [`Document::from_node`]'s callers do, and a
    /// childless node cannot be its parent's ancestor.
    pub fn append_new(&mut self, parent: NodeId, child: NodeId) -> Result<(), MutationError> {
        let parent_data = self.data(parent).ok_or(MutationError::NoSuchNode)?;
        let child_record = self.record(child).ok_or(MutationError::NoSuchNode)?;
        if child_record.parent.is_some() || child_record.first_child.is_some() || child == parent {
            return Err(MutationError::HierarchyRequest);
        }
        let allowed = !matches!(
            (parent_data, &child_record.data),
            (NodeData::Text(_), _)
                | (_, NodeData::Document)
                | (NodeData::Document, NodeData::Text(_))
        );
        if !allowed || child == self.root {
            return Err(MutationError::HierarchyRequest);
        }
        self.link_last(parent.index, child.index);
        self.record_change(Change::Inserted { parent, child });
        Ok(())
    }

    /// Removes `child` from `parent`; it stays valid and detached.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), MutationError> {
        if !self.contains(parent) || !self.contains(child) {
            return Err(MutationError::NoSuchNode);
        }
        if self.parent(child) != Some(parent) {
            return Err(MutationError::NotAChild);
        }
        self.detach(child);
        Ok(())
    }

    /// Sets attribute `name` (lowercased) of `element` to `value`,
    /// adding it last when the element has none; setting the value it
    /// already has changes nothing and records nothing.
    pub fn set_attribute(
        &mut self,
        element: NodeId,
        name: &str,
        value: &str,
    ) -> Result<(), MutationError> {
        let name = name.to_ascii_lowercase();
        let had = match self.attrs_mut(element)?.iter().find(|(k, _)| *k == name) {
            Some((_, v)) if v == value => return Ok(()),
            Some((_, v)) => Some(v.len()),
            None => None,
        };
        let (more, less) = match had {
            Some(old) => (value.len(), old),
            None => (name.len() + value.len(), 0),
        };
        self.check_data(more.saturating_sub(less))?;
        self.data_bytes = self.data_bytes + more - less;
        let attrs = self.attrs_mut(element)?;
        let old = match attrs.iter_mut().find(|(k, _)| *k == name) {
            Some((_, v)) => Some(std::mem::replace(v, value.to_string())),
            None => {
                attrs.push((name.clone(), value.to_string()));
                None
            }
        };
        self.record_change(Change::AttributeChanged { element, name, old });
        Ok(())
    }

    /// Removes attribute `name` (lowercased) of `element`; removing one
    /// it does not have changes nothing.
    pub fn remove_attribute(&mut self, element: NodeId, name: &str) -> Result<(), MutationError> {
        let name = name.to_ascii_lowercase();
        let attrs = self.attrs_mut(element)?;
        let Some(at) = attrs.iter().position(|(k, _)| *k == name) else {
            return Ok(());
        };
        let (_, old) = attrs.remove(at);
        self.data_bytes -= name.len() + old.len();
        self.record_change(Change::AttributeChanged {
            element,
            name,
            old: Some(old),
        });
        Ok(())
    }

    /// Replaces a text node's data; the same data changes nothing.
    pub fn set_text(&mut self, node: NodeId, data: &str) -> Result<(), MutationError> {
        let record = self.record(node).ok_or(MutationError::NoSuchNode)?;
        let NodeData::Text(text) = &record.data else {
            return Err(MutationError::WrongKind);
        };
        if text == data {
            return Ok(());
        }
        let old = text.len();
        self.check_data(data.len().saturating_sub(old))?;
        self.data_bytes = self.data_bytes + data.len() - old;
        if let Some(NodeData::Text(text)) = self.record_mut(node).map(|r| &mut r.data) {
            data.clone_into(text);
        }
        self.record_change(Change::TextChanged { node });
        Ok(())
    }

    /// Appends `data` to a text node's, as the DOM's `appendData` does:
    /// what a parser does with text that arrives in pieces (M24.8),
    /// without copying what the node has.
    pub fn append_data(&mut self, node: NodeId, data: &str) -> Result<(), MutationError> {
        let record = self.record(node).ok_or(MutationError::NoSuchNode)?;
        if !matches!(record.data, NodeData::Text(_)) {
            return Err(MutationError::WrongKind);
        }
        if data.is_empty() {
            return Ok(());
        }
        self.check_data(data.len())?;
        self.data_bytes += data.len();
        if let Some(NodeData::Text(text)) = self.record_mut(node).map(|r| &mut r.data) {
            text.push_str(data);
        }
        self.record_change(Change::TextChanged { node });
        Ok(())
    }

    /// Frees the detached subtree at `id`: its ids stop resolving and
    /// its slots are reused. The root and attached nodes are refused.
    pub fn free(&mut self, id: NodeId) -> Result<(), MutationError> {
        if !self.contains(id) {
            return Err(MutationError::NoSuchNode);
        }
        if id == self.root || self.parent(id).is_some() {
            return Err(MutationError::Attached);
        }
        let doomed: Vec<u32> = self.subtree(id).map(|n| n.index).collect();
        for index in doomed {
            let slot = &mut self.slots[index as usize];
            if let Some(record) = slot.record.take() {
                self.data_bytes -= data_size(&record.data);
            }
            self.live -= 1;
            // A slot whose generations are spent is retired, never
            // reused, so no later node can answer to an old id.
            let next = slot.generation + 1;
            if next < MAX_GENERATION {
                slot.generation = next;
                self.free.push(index);
            }
        }
        Ok(())
    }

    /// The changes since the journal was last drained, oldest first.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }

    /// Whether anything changed since the journal was last drained.
    pub fn has_changes(&self) -> bool {
        !self.journal.is_empty()
    }

    /// How many changes have been made to the document, ever: a count
    /// that a reader keeps to know, by comparing, whether anything
    /// changed since it last looked (M25's live collections), whoever
    /// drains the journal meanwhile.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The bytes of text, tag names and attribute names and values the
    /// live nodes hold ([`MAX_DATA_BYTES`]).
    pub fn data_bytes(&self) -> usize {
        self.data_bytes
    }

    /// Checks the arena's invariants, for tests and fuzz targets:
    /// every link names a live node and is mirrored (a child's parent
    /// lists it, siblings point at each other, the first and last child
    /// are the ends), every node is in exactly one place, no node is its
    /// own ancestor, the root is the document node and parentless, and
    /// the count of live nodes and the free list agree with the slots.
    /// Linear in the slots, plus the depth for each node's ancestry.
    pub fn validate(&self) -> Result<(), String> {
        let live = |i: u32| {
            self.slots
                .get(i as usize)
                .and_then(|s| s.record.as_ref())
                .ok_or(format!("link to dead slot {i}"))
        };
        let root = self.record(self.root).ok_or("the root is dead")?;
        if root.data != NodeData::Document || root.parent.is_some() {
            return Err("the root is not a parentless document node".into());
        }
        let mut counted = 0usize;
        let mut placed = 0usize;
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(record) = &slot.record else { continue };
            counted += 1;
            let i = i as u32;
            if record.data == NodeData::Document && i != self.root.index {
                return Err(format!("a second document node in slot {i}"));
            }
            // The children list, walked forwards, mirrors itself.
            let mut prev = None;
            let mut child = record.first_child;
            let mut steps = 0usize;
            while let Some(c) = child {
                let r = live(c)?;
                if r.parent != Some(i) || r.prev_sibling != prev {
                    return Err(format!("slot {c} is linked wrongly under {i}"));
                }
                steps += 1;
                if steps > self.slots.len() {
                    return Err(format!("the children of {i} loop"));
                }
                prev = Some(c);
                child = r.next_sibling;
            }
            placed += steps;
            if record.last_child != prev {
                return Err(format!("slot {i}'s last child is wrong"));
            }
            // The ancestry ends at a parentless node.
            let mut at = record.parent;
            let mut depth = 0usize;
            while let Some(p) = at {
                depth += 1;
                if depth > self.slots.len() {
                    return Err(format!("slot {i} is its own ancestor"));
                }
                at = live(p)?.parent;
            }
        }
        let parentless = self
            .slots
            .iter()
            .filter_map(|s| s.record.as_ref())
            .filter(|r| r.parent.is_none())
            .count();
        if placed + parentless != counted || counted != self.live {
            return Err("a node is in more than one place, or the count is wrong".into());
        }
        let data: usize = self
            .slots
            .iter()
            .filter_map(|s| s.record.as_ref())
            .map(|r| data_size(&r.data))
            .sum();
        if data != self.data_bytes {
            return Err(format!(
                "{data} bytes of data counted as {}",
                self.data_bytes
            ));
        }
        if let Some(slot) = self.slots.iter().find(|s| s.generation >= MAX_GENERATION) {
            return Err(format!("a slot at generation {}", slot.generation));
        }
        let mut seen = std::collections::HashSet::new();
        for &f in &self.free {
            let slot = self
                .slots
                .get(f as usize)
                .ok_or("a free index past the slots")?;
            if slot.record.is_some() || !seen.insert(f) {
                return Err(format!("free slot {f} is live or listed twice"));
            }
        }
        Ok(())
    }

    // ---- Internals ----------------------------------------------------------

    #[inline]
    fn record(&self, id: NodeId) -> Option<&Record> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.record.as_ref()
    }

    fn record_mut(&mut self, id: NodeId) -> Option<&mut Record> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.record.as_mut()
    }

    fn attrs_mut(&mut self, element: NodeId) -> Result<&mut Vec<(String, String)>, MutationError> {
        let record = self.record_mut(element).ok_or(MutationError::NoSuchNode)?;
        match &mut record.data {
            NodeData::Element { attrs, .. } => Ok(attrs),
            _ => Err(MutationError::WrongKind),
        }
    }

    /// The id of the element in slot `index`, from a link; `None` when
    /// it is another kind of node.
    #[inline]
    fn element_at(&self, index: u32) -> Option<NodeId> {
        let slot = self.slots.get(index as usize)?;
        match slot.record.as_ref()?.data {
            NodeData::Element { .. } => Some(NodeId {
                index,
                generation: slot.generation,
            }),
            _ => None,
        }
    }

    /// The id of the live node in slot `index`, from a link.
    #[inline]
    fn link(&self, index: Option<u32>) -> Option<NodeId> {
        let index = index?;
        Some(NodeId {
            index,
            generation: self.slots.get(index as usize)?.generation,
        })
    }

    fn check_room(&self) -> Result<(), MutationError> {
        if self.live >= self.max_nodes {
            Err(MutationError::TooManyNodes)
        } else {
            Ok(())
        }
    }

    /// Whether `more` bytes of data fit.
    fn check_data(&self, more: usize) -> Result<(), MutationError> {
        if more > self.max_data.saturating_sub(self.data_bytes) {
            Err(MutationError::TooMuchData)
        } else {
            Ok(())
        }
    }

    fn allocate(&mut self, data: NodeData) -> NodeId {
        self.data_bytes += data_size(&data);
        let record = Record {
            data,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
        };
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.record = Some(record);
            return NodeId {
                index,
                generation: slot.generation,
            };
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot {
            generation: 0,
            record: Some(record),
        });
        NodeId {
            index,
            generation: 0,
        }
    }

    /// The number of elements from `id` up, `id` included when it is one.
    fn element_depth(&self, id: NodeId) -> usize {
        std::iter::once(id)
            .chain(self.ancestors(id))
            .filter(|&n| self.is_element(n))
            .count()
    }

    /// The most elements on one path down from `id`, `id` included when
    /// it is one. Iterative.
    fn element_height(&self, id: NodeId) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(id, usize::from(self.is_element(id)))];
        while let Some((node, depth)) = stack.pop() {
            deepest = deepest.max(depth);
            for child in self.children(node) {
                stack.push((child, depth + usize::from(self.is_element(child))));
            }
        }
        deepest
    }

    /// Takes `child` out of its parent's children, recording the change;
    /// nothing for a detached node.
    fn detach(&mut self, child: NodeId) {
        let Some(record) = self.record(child) else {
            return;
        };
        let Some(parent) = record.parent else {
            return;
        };
        let (prev, next) = (record.prev_sibling, record.next_sibling);
        match prev {
            Some(p) => self.slot_mut(p).next_sibling = next,
            None => self.slot_mut(parent).first_child = next,
        }
        match next {
            Some(n) => self.slot_mut(n).prev_sibling = prev,
            None => self.slot_mut(parent).last_child = prev,
        }
        let record = self.slot_mut(child.index);
        record.parent = None;
        record.prev_sibling = None;
        record.next_sibling = None;
        let change = Change::Removed {
            parent: self.link(Some(parent)).expect("a parent is live"),
            child,
            prev: self.link(prev),
            next: self.link(next),
        };
        self.record_change(change);
    }

    /// Links the detached `child` as `parent`'s last child.
    fn link_last(&mut self, parent: u32, child: u32) {
        let last = self.slot_mut(parent).last_child;
        match last {
            Some(l) => self.slot_mut(l).next_sibling = Some(child),
            None => self.slot_mut(parent).first_child = Some(child),
        }
        let record = self.slot_mut(child);
        record.parent = Some(parent);
        record.prev_sibling = last;
        record.next_sibling = None;
        self.slot_mut(parent).last_child = Some(child);
    }

    /// Links the detached `child` under `parent` before its child
    /// `reference`.
    fn link_before(&mut self, parent: u32, child: u32, reference: u32) {
        let prev = self.slot_mut(reference).prev_sibling;
        match prev {
            Some(p) => self.slot_mut(p).next_sibling = Some(child),
            None => self.slot_mut(parent).first_child = Some(child),
        }
        self.slot_mut(reference).prev_sibling = Some(child);
        let record = self.slot_mut(child);
        record.parent = Some(parent);
        record.prev_sibling = prev;
        record.next_sibling = Some(reference);
    }

    /// The live record in slot `index`, which the caller knows is live.
    fn slot_mut(&mut self, index: u32) -> &mut Record {
        self.slots[index as usize]
            .record
            .as_mut()
            .expect("links name live slots")
    }

    fn record_change(&mut self, change: Change) {
        self.version += 1;
        if self.journal.last() == Some(&Change::Everything) {
            return;
        }
        if self.journal.len() >= MAX_JOURNAL {
            self.journal.clear();
            self.journal.push(Change::Everything);
            return;
        }
        self.journal.push(change);
    }
}

/// The bytes of data a node holds, for [`MAX_DATA_BYTES`].
fn data_size(data: &NodeData) -> usize {
    match data {
        NodeData::Document => 0,
        NodeData::Element { tag, attrs } => {
            tag.len() + attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
        }
        NodeData::Text(text) => text.len(),
    }
}

/// [`Document::children`].
pub struct Children<'a> {
    document: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.document.next_sibling(current);
        Some(current)
    }
}

/// [`Document::subtree`].
pub struct Subtree<'a> {
    document: &'a Document,
    top: NodeId,
    next: Option<NodeId>,
}

impl Iterator for Subtree<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        let document = self.document;
        // Down to the first child, else across to the next sibling of
        // the nearest node at or above this one, stopping at the top.
        self.next = document.first_child(current).or_else(|| {
            let mut at = current;
            loop {
                if at == self.top {
                    return None;
                }
                if let Some(next) = document.next_sibling(at) {
                    return Some(next);
                }
                at = document.parent(at)?;
            }
        });
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Node {
        Node::document(vec![
            Element::new("html")
                .push(Element::new("head").push(Element::new("title").text("t")))
                .push(
                    Element::new("body")
                        .push(Element::new("p").attr("class", "a").text("one"))
                        .text(" ")
                        .push(Element::new("p").text("two")),
                )
                .into(),
        ])
    }

    fn tags(document: &Document, ids: impl Iterator<Item = NodeId>) -> Vec<String> {
        ids.map(|id| match document.data(id) {
            Some(NodeData::Element { tag, .. }) => tag.clone(),
            Some(NodeData::Text(t)) => format!("'{t}'"),
            Some(NodeData::Document) => "#document".into(),
            None => "?".into(),
        })
        .collect()
    }

    fn element(document: &Document, tag: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some(tag))
            .expect("the element")
    }

    #[test]
    fn a_tree_goes_in_and_comes_back_out_unchanged() {
        let node = sample();
        let document = Document::from_node(&node);
        assert_eq!(document.to_node(), node);
        assert_eq!(document.len(), 11, "the root counts");
        assert!(!document.has_changes(), "building is not a change");
        // A bare element becomes the document's one child.
        let bare = Document::from_node(&Element::new("p").text("x").into());
        assert_eq!(
            bare.to_node(),
            Node::document(vec![Element::new("p").text("x").into()])
        );
    }

    #[test]
    fn navigation_follows_the_tree_in_order() {
        let document = Document::from_node(&sample());
        let root = document.root();
        assert_eq!(
            tags(&document, document.subtree(root)),
            [
                "#document",
                "html",
                "head",
                "title",
                "'t'",
                "body",
                "p",
                "'one'",
                "' '",
                "p",
                "'two'"
            ]
        );
        let html = document.document_element().unwrap();
        assert_eq!(document.tag(html), Some("html"));
        let body = element(&document, "body");
        assert_eq!(tags(&document, document.children(body)), ["p", "' '", "p"]);
        assert_eq!(tags(&document, document.element_children(body)), ["p", "p"]);
        let first = document.first_child(body).unwrap();
        let last = document.last_child(body).unwrap();
        assert_eq!(document.next_element_sibling(first), Some(last));
        assert_eq!(document.prev_element_sibling(last), Some(first));
        assert_eq!(document.prev_sibling(first), None);
        assert_eq!(document.parent_element(first), Some(body));
        assert_eq!(
            document.parent_element(html),
            None,
            "the document is no element"
        );
        assert_eq!(
            tags(&document, document.ancestors(first)),
            ["body", "html", "#document"]
        );
        assert_eq!(document.attr(first, "class"), Some("a"));
        assert_eq!(document.text_content(body), "one two");
        assert_eq!(
            tags(&document, document.subtree(first)),
            ["p", "'one'"],
            "a subtree stops at its top"
        );
    }

    #[test]
    fn insertions_and_removals_move_nodes_and_are_journaled() {
        let mut document = Document::from_node(&sample());
        let body = element(&document, "body");
        let first = document.first_child(body).unwrap();
        let li = document.create_element("LI").unwrap();
        assert_eq!(document.tag(li), Some("li"), "lowercased");
        assert_eq!(document.parent(li), None, "created detached");
        document.insert_before(body, li, Some(first)).unwrap();
        assert_eq!(document.first_child(body), Some(li));
        let text = document.create_text("x").unwrap();
        document.append_child(li, text).unwrap();
        // Appending an attached node moves it.
        document.append_child(body, li).unwrap();
        assert_eq!(document.last_child(body), Some(li));
        assert_eq!(document.first_child(body), Some(first));
        // A reference that is the child itself means the place after it.
        document.insert_before(body, li, Some(li)).unwrap();
        assert_eq!(document.last_child(body), Some(li));
        document.remove_child(body, li).unwrap();
        assert_eq!(document.parent(li), None);
        assert!(document.contains(li), "removed, not freed");
        assert_eq!(document.text_content(li), "x", "its subtree goes with it");
        let last_p = document.last_child(body).unwrap();
        assert_eq!(
            document.take_changes(),
            [
                Change::Inserted {
                    parent: body,
                    child: li
                },
                Change::Inserted {
                    parent: li,
                    child: text
                },
                Change::Removed {
                    parent: body,
                    child: li,
                    prev: None,
                    next: Some(first)
                },
                Change::Inserted {
                    parent: body,
                    child: li
                },
                Change::Removed {
                    parent: body,
                    child: li,
                    prev: Some(last_p),
                    next: None
                },
                Change::Inserted {
                    parent: body,
                    child: li
                },
                Change::Removed {
                    parent: body,
                    child: li,
                    prev: Some(last_p),
                    next: None
                },
            ]
        );
        assert!(!document.has_changes(), "drained");
    }

    #[test]
    fn attributes_and_text_change_and_record_their_old_values() {
        let mut document = Document::from_node(&sample());
        let p = element(&document, "p");
        document.set_attribute(p, "CLASS", "b").unwrap();
        document.set_attribute(p, "class", "b").unwrap();
        document.set_attribute(p, "id", "x").unwrap();
        document.remove_attribute(p, "id").unwrap();
        document.remove_attribute(p, "title").unwrap();
        let text = document.first_child(p).unwrap();
        document.set_text(text, "uno").unwrap();
        document.set_text(text, "uno").unwrap();
        assert_eq!(document.attr(p, "class"), Some("b"));
        assert_eq!(document.attr(p, "id"), None);
        assert_eq!(document.text(text), Some("uno"));
        assert_eq!(
            document.take_changes(),
            [
                Change::AttributeChanged {
                    element: p,
                    name: "class".into(),
                    old: Some("a".into())
                },
                Change::AttributeChanged {
                    element: p,
                    name: "id".into(),
                    old: None
                },
                Change::AttributeChanged {
                    element: p,
                    name: "id".into(),
                    old: Some("x".into())
                },
                Change::TextChanged { node: text },
            ],
            "no record for a change to the same value or of nothing"
        );
        assert_eq!(document.set_text(p, "x"), Err(MutationError::WrongKind));
        assert_eq!(
            document.set_attribute(text, "a", "b"),
            Err(MutationError::WrongKind)
        );
    }

    #[test]
    fn the_doms_hierarchy_checks_refuse_and_leave_the_document_as_it_was() {
        let mut document = Document::from_node(&sample());
        let before = document.to_node();
        let root = document.root();
        let body = element(&document, "body");
        let html = document.document_element().unwrap();
        let p = element(&document, "p");
        let text = document.first_child(p).unwrap();
        let loose = document.create_text("loose").unwrap();
        let err = |r: Result<(), MutationError>| r.unwrap_err();
        assert_eq!(
            err(document.append_child(body, html)),
            MutationError::HierarchyRequest
        );
        assert_eq!(
            err(document.append_child(body, body)),
            MutationError::HierarchyRequest
        );
        assert_eq!(
            err(document.append_child(text, loose)),
            MutationError::HierarchyRequest
        );
        assert_eq!(
            err(document.append_child(body, root)),
            MutationError::HierarchyRequest
        );
        assert_eq!(
            err(document.append_child(root, loose)),
            MutationError::HierarchyRequest
        );
        assert_eq!(
            err(document.insert_before(body, loose, Some(text))),
            MutationError::NotAChild
        );
        assert_eq!(
            err(document.remove_child(body, text)),
            MutationError::NotAChild
        );
        assert_eq!(err(document.free(body)), MutationError::Attached);
        assert_eq!(err(document.free(root)), MutationError::Attached);
        assert_eq!(document.to_node(), before);
        assert!(!document.has_changes());
    }

    #[test]
    fn a_freed_node_is_gone_and_its_slot_reused_under_a_new_id() {
        let mut document = Document::from_node(&sample());
        let body = element(&document, "body");
        let p = element(&document, "p");
        let text = document.first_child(p).unwrap();
        document.remove_child(body, p).unwrap();
        let live = document.len();
        document.free(p).unwrap();
        assert_eq!(document.len(), live - 2, "the element and its text");
        for stale in [p, text] {
            assert!(!document.contains(stale));
            assert_eq!(document.data(stale), None);
            assert_eq!(document.parent(stale), None);
            assert_eq!(document.children(stale).count(), 0);
            assert_eq!(document.subtree(stale).count(), 0);
            assert_eq!(
                document.append_child(body, stale),
                Err(MutationError::NoSuchNode)
            );
            assert_eq!(document.free(stale), Err(MutationError::NoSuchNode));
        }
        let new = document.create_element("span").unwrap();
        assert!(
            new.index() == p.index() || new.index() == text.index(),
            "a slot reused"
        );
        assert_ne!(new, p);
        assert_ne!(new, text);
        assert_eq!(document.id_at(new.index()), Some(new));
        // A forged id is no node.
        let forged = NodeId {
            index: 9999,
            generation: 0,
        };
        assert_eq!(document.data(forged), None);
        assert_eq!(
            document.append_child(body, forged),
            Err(MutationError::NoSuchNode)
        );
    }

    #[test]
    fn elements_nest_no_deeper_than_the_parsers_cap() {
        let mut document = Document::new();
        let mut parent = document.root();
        for _ in 0..MAX_DEPTH {
            let el = document.create_element("b").unwrap();
            document.append_child(parent, el).unwrap();
            parent = el;
        }
        let one_more = document.create_element("b").unwrap();
        assert_eq!(
            document.append_child(parent, one_more),
            Err(MutationError::TooDeep)
        );
        // Text may sit below the deepest element.
        let text = document.create_text("x").unwrap();
        document.append_child(parent, text).unwrap();
        // A subtree two deep fits only two levels from the cap.
        let outer = document.create_element("i").unwrap();
        let inner = document.create_element("i").unwrap();
        document.append_child(outer, inner).unwrap();
        let near = document.ancestors(parent).next().unwrap();
        assert_eq!(
            document.append_child(near, outer),
            Err(MutationError::TooDeep)
        );
        let nearer = document.ancestors(parent).nth(1).unwrap();
        document.append_child(nearer, outer).unwrap();
    }

    #[test]
    fn the_data_count_follows_every_change_and_the_cap_refuses_past_it() {
        let mut document = Document::from_node(&sample());
        // html head title "t" body p class=a "one" " " p "two"
        let parsed = 4 + 4 + 5 + 1 + 4 + 1 + 6 + 3 + 1 + 1 + 3;
        assert_eq!(document.data_bytes(), parsed);
        let p = element(&document, "p");
        document.set_attribute(p, "class", "abc").unwrap();
        assert_eq!(document.data_bytes(), parsed + 2);
        document.set_attribute(p, "id", "x").unwrap();
        assert_eq!(document.data_bytes(), parsed + 5);
        document.remove_attribute(p, "class").unwrap();
        assert_eq!(document.data_bytes(), parsed - 3);
        let text = document.first_child(p).unwrap();
        document.set_text(text, "uno!").unwrap();
        document.append_data(text, "..").unwrap();
        assert_eq!(document.data_bytes(), parsed);
        let body = element(&document, "body");
        let span = document.create_element("span").unwrap();
        let more = document.create_text("more").unwrap();
        document.append_child(span, more).unwrap();
        document.append_child(body, span).unwrap();
        assert_eq!(document.data_bytes(), parsed + 8);
        document.remove_child(body, span).unwrap();
        document.free(span).unwrap();
        assert_eq!(document.data_bytes(), parsed);
        document.validate().unwrap();

        // Past the cap, each kind of change is refused and changes nothing.
        document.max_data = document.data_bytes() + 4;
        assert_eq!(
            document.create_text("12345"),
            Err(MutationError::TooMuchData)
        );
        assert_eq!(
            document.create_element("abcde"),
            Err(MutationError::TooMuchData)
        );
        assert_eq!(
            document.create_element_with("a", vec![("b".into(), "cde".into())]),
            Err(MutationError::TooMuchData)
        );
        assert_eq!(
            document.set_attribute(p, "title", "abc"),
            Err(MutationError::TooMuchData)
        );
        assert_eq!(
            document.set_text(text, "uno!..12345"),
            Err(MutationError::TooMuchData)
        );
        assert_eq!(
            document.append_data(text, "12345"),
            Err(MutationError::TooMuchData)
        );
        document.validate().unwrap();
        // What fits goes in, and what shrinks always does.
        document.append_data(text, "1234").unwrap();
        assert_eq!(document.create_text("x"), Err(MutationError::TooMuchData));
        document.set_text(text, "").unwrap();
        document.create_text("x").unwrap();
        document.validate().unwrap();
    }

    #[test]
    fn the_version_counts_every_change_whoever_drains_the_journal() {
        let mut document = Document::from_node(&sample());
        assert_eq!(document.version(), 0, "a tree taken whole is no change");
        let body = element(&document, "body");
        let p = document.create_element("p").unwrap();
        assert_eq!(document.version(), 0, "a detached node is no change");
        document.append_child(body, p).unwrap();
        document.set_attribute(p, "a", "b").unwrap();
        document.set_attribute(p, "a", "b").unwrap();
        assert_eq!(document.version(), 2, "the same value changes nothing");
        document.take_changes();
        document.remove_child(body, p).unwrap();
        assert_eq!(document.version(), 3);
        // Past the journal's bound the record is one, the count goes on.
        let text = document.create_text("").unwrap();
        for i in 0..MAX_JOURNAL + 10 {
            document.set_text(text, &i.to_string()).unwrap();
        }
        assert_eq!(document.take_changes(), vec![Change::Everything]);
        assert_eq!(document.version(), 3 + MAX_JOURNAL as u64 + 10);
    }

    #[test]
    fn an_id_is_one_number_below_two_to_the_fifty_third() {
        let mut document = Document::new();
        let a = document.create_element("a").unwrap();
        assert_eq!(NodeId::from_bits(a.to_bits()), Some(a));
        let last = NodeId {
            index: u32::MAX,
            generation: MAX_GENERATION - 1,
        };
        assert!(last.to_bits() < 1 << 53);
        assert_eq!(NodeId::from_bits(last.to_bits()), Some(last));
        assert_eq!(NodeId::from_bits(u64::from(MAX_GENERATION) << 32), None);
        assert_eq!(NodeId::from_bits(u64::MAX), None);
    }

    #[test]
    fn a_slot_is_retired_at_its_last_generation() {
        let mut document = Document::new();
        let a = document.create_element("a").unwrap();
        document.slots[a.index as usize].generation = MAX_GENERATION - 2;
        let a = NodeId {
            index: a.index,
            generation: MAX_GENERATION - 2,
        };
        document.free(a).unwrap();
        let b = document.create_element("b").unwrap();
        assert_eq!((b.index, b.generation), (a.index, MAX_GENERATION - 1));
        document.free(b).unwrap();
        let c = document.create_element("c").unwrap();
        assert_ne!(c.index, a.index, "the slot is not reused");
        assert!(!document.contains(a) && !document.contains(b));
        document.validate().unwrap();
    }

    #[test]
    fn the_node_cap_refuses_creation_past_it() {
        let mut document = Document::new();
        document.max_nodes = 3;
        document.create_element("a").unwrap();
        document.create_text("b").unwrap();
        assert_eq!(
            document.create_element("c"),
            Err(MutationError::TooManyNodes)
        );
        assert_eq!(document.create_text("c"), Err(MutationError::TooManyNodes));
    }

    #[test]
    fn a_journal_nobody_reads_collapses_into_everything() {
        let mut document = Document::new();
        let a = document.create_element("a").unwrap();
        document.append_child(document.root(), a).unwrap();
        for i in 0..MAX_JOURNAL + 10 {
            document.set_attribute(a, "n", &i.to_string()).unwrap();
        }
        assert_eq!(document.take_changes(), [Change::Everything]);
        document.set_attribute(a, "n", "x").unwrap();
        assert_eq!(document.take_changes().len(), 1, "fresh after a drain");
    }

    #[test]
    fn random_mutations_keep_the_arena_consistent_and_refusals_change_nothing() {
        // A seeded walk through every operation, stale and forged ids
        // among the arguments, the invariants checked after each step.
        let mut state: u64 = 0x5eed_1234_abcd_0001;
        let mut next = move |n: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as usize) % n.max(1)
        };
        let mut document = Document::from_node(&sample());
        let mut ids: Vec<NodeId> = document.subtree(document.root()).collect();
        let (mut done, mut refused) = (0, 0);
        for _ in 0..4000 {
            let pick = |next: &mut dyn FnMut(usize) -> usize, ids: &[NodeId]| {
                if next(20) == 0 {
                    NodeId {
                        index: next(64) as u32,
                        generation: next(3) as u32,
                    }
                } else {
                    ids[next(ids.len())]
                }
            };
            let a = pick(&mut next, &ids);
            let b = pick(&mut next, &ids);
            let c = pick(&mut next, &ids);
            let before = document.to_node();
            let result = match next(10) {
                0 => document.create_element("e").map(|id| ids.push(id)),
                1 => document.create_text("t").map(|id| ids.push(id)),
                2 | 3 => document.append_child(a, b),
                4 => document.insert_before(a, b, Some(c)),
                5 => document.remove_child(a, b),
                6 => document.set_attribute(a, "class", if next(2) == 0 { "x" } else { "y" }),
                7 => document.remove_attribute(a, "class"),
                8 => document.set_text(a, "u"),
                _ => document.free(a),
            };
            if let Err(e) = document.validate() {
                panic!("invariant broken: {e}");
            }
            match result {
                Ok(()) => done += 1,
                Err(_) => {
                    refused += 1;
                    assert_eq!(document.to_node(), before, "a refusal changed the tree");
                }
            }
        }
        assert!(
            done > 1000 && refused > 500,
            "both paths exercised: {done} {refused}"
        );
        let snapshot = document.to_node();
        assert_eq!(Document::from_node(&snapshot).to_node(), snapshot);
    }

    #[test]
    fn a_parser_makes_elements_with_their_attributes_appends_new_nodes_and_text() {
        let mut doc = Document::new();
        let root = doc.root();
        let html = doc
            .create_element_with("HTML", vec![("lang".into(), "en".into())])
            .unwrap();
        assert_eq!(doc.tag(html), Some("html"));
        assert_eq!(doc.attr(html, "lang"), Some("en"));
        doc.append_new(root, html).unwrap();
        let text = doc.create_text("a").unwrap();
        doc.append_new(html, text).unwrap();
        doc.append_data(text, "b").unwrap();
        assert_eq!(doc.text(text), Some("ab"));
        assert!(matches!(
            doc.take_changes().last(),
            Some(Change::TextChanged { node }) if *node == text
        ));
        // Only a node never inserted, under a parent that may hold it.
        assert_eq!(
            doc.append_new(root, html),
            Err(MutationError::HierarchyRequest)
        );
        let other = doc.create_element("p").unwrap();
        let inner = doc.create_element("b").unwrap();
        doc.append_child(other, inner).unwrap();
        assert_eq!(
            doc.append_new(html, other),
            Err(MutationError::HierarchyRequest)
        );
        let leaf = doc.create_element("i").unwrap();
        doc.append_new(html, leaf).unwrap();
        assert_eq!(
            doc.append_new(root, leaf),
            Err(MutationError::HierarchyRequest)
        );
        let loose = doc.create_text("t").unwrap();
        assert_eq!(
            doc.append_new(root, loose),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(
            doc.append_new(text, loose),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(
            doc.append_new(root, root),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(doc.append_data(html, "x"), Err(MutationError::WrongKind));
        doc.validate().unwrap();
    }

    #[test]
    fn a_very_deep_tree_converts_walks_and_drops_on_a_small_stack() {
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut el = Element::new("b").text("leaf");
                for _ in 0..100_000 {
                    el = Element::new("b").push(el);
                }
                let node = Node::document(vec![el.into()]);
                let document = Document::from_node(&node);
                assert_eq!(document.text_content(document.root()), "leaf");
                assert_eq!(document.subtree(document.root()).count(), 100_003);
                // Compared by content: the derived equality recurses.
                let back = document.to_node();
                assert_eq!(back.text_content(), "leaf");
                drop(back);
                drop(document);
                drop(node);
            })
            .expect("thread");
        handle.join().expect("no stack overflow");
    }
}
