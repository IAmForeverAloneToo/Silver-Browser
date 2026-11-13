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

use crate::forms::ControlState;
use crate::{Element, Namespace, Node, Text, local_part};

mod attributes;
mod custom;
mod details;
mod documents;
mod focus_navigation;
mod live;
mod references;
mod shadow;
mod top_layer;
pub use attributes::{attribute_local_name, implied_attribute_namespace};
pub use custom::{
    CustomElementState, Definition, FormValue, MAX_CUSTOM_STATES, MAX_DEFINITIONS, Reaction,
    WINDOW_REGISTRY, callbacks,
};
pub use details::MAX_DETAILS_TOGGLES;
pub use documents::{CompatMode, DocumentFacts, DocumentInterface};
pub use focus_navigation::{
    FOCUSGROUP_TOKENS, FocusGroup, FocusGroupAttribute, FocusGroupBehavior, StartingPoint,
    parse_focusgroup,
};
pub use live::{Boundary, IteratorState, LiveRange, MAX_ITERATORS, MAX_RANGES};
pub use references::{ELEMENT_REFERENCES, MAX_EXPLICIT_ELEMENTS};
#[cfg(feature = "flat-steps")]
pub use shadow::take_flat_steps;
pub use shadow::{
    FlatChildren, FlatDepths, ShadowRootInit, ShadowRootMode, SlotAssignmentMode,
    is_valid_custom_element_name,
};
pub use top_layer::{Dialog, MAX_TOP_LAYER, Popover, PopoverList, PopoverMode, Watcher};

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
    /// A document node: the page's, the arena's root, or another
    /// document's beside it (D67), parentless either way.
    Document,
    /// An element: its tag name (lowercase for an HTML element, a
    /// foreign one's qualified name in its own case), its attributes, in
    /// order, and its namespace (M32).
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        ns: Namespace,
    },
    /// Character data; XML's CDATA sections are text too.
    Text(String),
    /// A comment (D58).
    Comment(String),
    /// A document type node, the document's child before its element.
    DocumentType {
        name: Box<str>,
        public_id: Box<str>,
        system_id: Box<str>,
    },
    /// A processing instruction, which XML documents have.
    ProcessingInstruction { target: Box<str>, data: String },
    /// A document fragment: never a child, its children moved where it
    /// is inserted; a template's contents are one ([`Document::template_content`]).
    DocumentFragment,
    /// A shadow root (gap batch H, D68): a document fragment its host
    /// links to, parentless, with what it was made with.
    ShadowRoot(ShadowRootInit),
}

impl NodeData {
    /// The DOM's `nodeType` for the node.
    pub fn node_type(&self) -> u16 {
        match self {
            NodeData::Element { .. } => 1,
            NodeData::Text(_) => 3,
            NodeData::ProcessingInstruction { .. } => 7,
            NodeData::Comment(_) => 8,
            NodeData::Document => 9,
            NodeData::DocumentType { .. } => 10,
            NodeData::DocumentFragment | NodeData::ShadowRoot(_) => 11,
        }
    }

    /// The data of a text, comment or processing instruction node.
    pub fn character_data(&self) -> Option<&str> {
        match self {
            NodeData::Text(data)
            | NodeData::Comment(data)
            | NodeData::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        }
    }

    fn character_data_mut(&mut self) -> Option<&mut String> {
        match self {
            NodeData::Text(data)
            | NodeData::Comment(data)
            | NodeData::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        }
    }
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
    /// The state flags the user's actions set on `element` changed
    /// (M26); `old` is what they were.
    StateChanged { element: NodeId, old: ElementState },
    /// A form control's state changed (M26.7): its value, checkedness,
    /// selectedness, indeterminate flag or selection.
    ControlChanged { element: NodeId },
    /// `child` was moved under `parent` (`moveBefore`, D59), after the
    /// `Removed` and `Inserted` records the move also made: what it holds
    /// (a frame, animations, a playing medium) is kept.
    Moved { parent: NodeId, child: NodeId },
    /// The flat tree's children of `node` changed (D68): a shadow root
    /// was attached to it, or the nodes assigned to it, a slot, are
    /// others. What its subtree in the flat tree computed is to be
    /// computed again.
    FlatTree { node: NodeId },
    /// More changed than the journal keeps ([`MAX_JOURNAL`]): treat the
    /// whole document as changed.
    Everything,
}

/// The most changes the observed queue keeps ([`Document::observe`]);
/// past it the oldest go, as a mutation observer's records past its
/// bound do.
pub const MAX_OBSERVED: usize = 1 << 16;

/// A change as the DOM's mutation observers are told of it (gap batch
/// A, D42): what "queue a mutation record" needs, one node at a time,
/// kept only while [`Document::observe`] is on, with the watched nodes
/// that were then the target's inclusive ancestors, nearest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    pub change: Mutated,
    pub watched: Vec<NodeId>,
}

/// What a [`Mutation`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutated {
    /// `added` was inserted under `parent`, or `removed` taken from it,
    /// between `prev` and `next`.
    ChildList {
        parent: NodeId,
        added: Option<NodeId>,
        removed: Option<NodeId>,
        prev: Option<NodeId>,
        next: Option<NodeId>,
    },
    /// The attribute of local name `name` and namespace `namespace` of
    /// `element` was set or removed; `old` is its value before, `None`
    /// when it had none.
    Attribute {
        element: NodeId,
        name: String,
        namespace: Option<Box<str>>,
        old: Option<String>,
    },
    /// A text node's data changed from `old`.
    Text { node: NodeId, old: String },
}

impl Mutated {
    /// The node the change is to: the parent, the element or the text.
    pub fn target(&self) -> NodeId {
        match self {
            Mutated::ChildList { parent, .. } => *parent,
            Mutated::Attribute { element, .. } => *element,
            Mutated::Text { node, .. } => *node,
        }
    }
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
    /// An offset past a node's length.
    IndexSize,
    /// [`Document::free`] was given an attached node, the root, or a
    /// template's contents.
    Attached,
    /// The operation is not supported for the node: a shadow root
    /// attached to an element that may not host one, or hosts one.
    NotSupported,
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
            MutationError::IndexSize => "the offset is past the node's length",
            MutationError::Attached => "the node is attached or is the document",
            MutationError::NotSupported => "the operation is not supported for the node",
        })
    }
}

impl std::error::Error for MutationError {}

/// The flags the user's actions set on an element (M26,
/// `docs/design/events.md`), and the top layer's (gap batch H): what the
/// dynamic pseudo-classes match. A set of [`ElementState`]'s constants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ElementState(pub u16);

impl ElementState {
    pub const NONE: ElementState = ElementState(0);
    /// Under the pointer, or an ancestor of what is: `:hover`.
    pub const HOVER: ElementState = ElementState(1);
    /// Being activated, a primary button held down on it or one of its
    /// descendants: `:active`.
    pub const ACTIVE: ElementState = ElementState(2);
    /// Focused: `:focus`.
    pub const FOCUS: ElementState = ElementState(4);
    /// Focused, the focus to be shown: `:focus-visible`.
    pub const FOCUS_VISIBLE: ElementState = ElementState(8);
    /// Focused, or an ancestor of what is: `:focus-within`.
    pub const FOCUS_WITHIN: ElementState = ElementState(16);
    /// The element the document's URL's fragment indicates: `:target`.
    pub const TARGET: ElementState = ElementState(32);
    /// A dialog shown by `showModal()` (HTML section 4.11.4): `:modal`.
    pub const MODAL: ElementState = ElementState(64);
    /// A popover in the showing state (HTML section 6.12): `:popover-open`.
    pub const POPOVER_OPEN: ElementState = ElementState(128);
    /// In the document's top layer (CSS Position 4 section 4): no
    /// selector reads it; the cascade does.
    pub const TOP_LAYER: ElementState = ElementState(256);
    /// An interest invoker whose target has its interest (HTML's
    /// `interestfor`): `:interest-source`.
    pub const INTEREST_SOURCE: ElementState = ElementState(512);
    /// The target of such an invoker: `:interest-target`.
    pub const INTEREST_TARGET: ElementState = ElementState(1024);
    /// A control whose picker is shown, a drop-down's list: `:open`,
    /// which a `dialog` and a `details` match by their `open` attribute.
    pub const OPEN: ElementState = ElementState(2048);
    /// Relevant to the user (CSS Containment 2 section 4.3, gap batch
    /// H): near the viewport, or holding the focus or the selection, as
    /// the engine finds at each rendering update. An element with
    /// `content-visibility: auto` skips its contents while it is not.
    pub const RELEVANT: ElementState = ElementState(4096);
    /// A change of whether the element is `:defined` (D70) is journaled
    /// as a change of its state flags from flags with this one set,
    /// which no element's flags have: what that pseudo-class's restyle
    /// reads.
    pub const DEFINED_CHANGED: ElementState = ElementState(8192);
    /// The same for a change of its custom states (`:state()`, D70).
    pub const STATES_CHANGED: ElementState = ElementState(16384);
    /// The two flags no element has, which journal those changes.
    pub const CUSTOM_CHANGES: ElementState = ElementState(8192 | 16384);

    /// Whether every flag of `flags` is set.
    pub fn contains(self, flags: ElementState) -> bool {
        self.0 & flags.0 == flags.0
    }

    /// These flags with `flags` set, or cleared when not `on`.
    pub fn with(self, flags: ElementState, on: bool) -> ElementState {
        if on {
            ElementState(self.0 | flags.0)
        } else {
            ElementState(self.0 & !flags.0)
        }
    }
}

/// A live node: what it is and where it is. Links are slot indices of
/// live nodes, so their generations are the slots' current ones.
#[derive(Debug, Clone)]
struct Record {
    data: NodeData,
    /// An element's state flags (M26).
    state: ElementState,
    /// A form control's state, once it has one (M26.7).
    control: Option<Box<ControlState>>,
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
    /// How many times a journal with something in it was drained.
    drains: u64,
    /// How many changes have been made, ever.
    version: u64,
    /// The bytes of data the live nodes hold ([`MAX_DATA_BYTES`]).
    data_bytes: usize,
    /// [`MAX_NODES`], lowered by tests.
    max_nodes: usize,
    /// [`MAX_DATA_BYTES`], lowered by tests.
    max_data: usize,
    /// The focused element (M26.6): it has the focus flags.
    focused: Option<NodeId>,
    /// The element that and whose ancestors have `focus-within`: the
    /// focused one, or while the focus moves, the ancestor it shares
    /// with where it goes (M26.11).
    within: Option<NodeId>,
    /// What mutation observers are to be told of, while some observer
    /// is registered (gap batch A); boxed, as most documents have none.
    observing: Option<Box<Observing>>,
    /// The window's named properties (gap batch A, D44): the ids of the
    /// elements in the tree, and the names of its HTML `embed`, `form`,
    /// `img` and `object` elements, each with how many elements have it;
    /// and the names that came or went since they were last taken;
    /// boxed, to keep the document small.
    named: Box<Named>,
    /// What fewer documents use, boxed: template contents (D58), live
    /// ranges and node iterators (D59), and whether it is an XML document.
    rare: Box<live::Live>,
}

/// Each HTML `template`'s contents (D58), a fragment made with it, and
/// each such fragment's template.
#[derive(Debug, Clone, Default)]
struct Templates {
    contents: std::collections::HashMap<NodeId, NodeId>,
    hosts: std::collections::HashMap<NodeId, NodeId>,
}

#[derive(Debug, Clone, Default)]
struct Named {
    counts: std::collections::HashMap<String, u32>,
    changes: Vec<(String, bool)>,
    /// More came or went than [`MAX_NAMED_CHANGES`]: they were dropped.
    overflow: bool,
}

/// The most named property changes kept between two takes; past it the
/// taker is told to read the names afresh.
pub const MAX_NAMED_CHANGES: usize = 1 << 16;

/// A document's changes kept for mutation observers, and the nodes they
/// are registered on, each with whether one watches its subtree.
#[derive(Debug, Clone, Default)]
struct Observing {
    changes: std::collections::VecDeque<Mutation>,
    watched: std::collections::HashMap<NodeId, bool>,
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
            drains: 0,
            version: 0,
            data_bytes: 0,
            max_nodes: MAX_NODES,
            max_data: MAX_DATA_BYTES,
            focused: None,
            within: None,
            observing: None,
            named: Box::default(),
            rare: Box::default(),
        };
        document.root = document.allocate(NodeData::Document);
        document
    }

    /// Whether it is an XML document rather than an HTML one.
    #[inline]
    pub fn is_xml_document(&self) -> bool {
        self.rare.xml
    }

    /// Makes it an XML document, or an HTML one, as its parser decides.
    pub fn set_xml_document(&mut self, xml: bool) {
        self.rare.xml = xml;
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
                Node::Element(Element { tag, attrs, ns, .. }) => NodeData::Element {
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                    ns: ns.clone(),
                },
                Node::Text(Text { data }) => NodeData::Text(data.clone()),
            };
            let id = document.allocate(data);
            document.link_last(parent, id.index);
            pending.extend(node.children().iter().rev().map(|c| (c, id.index)));
        }
        let root = document.root;
        document.index_names(root, true);
        document.named.changes.clear();
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
                NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot(_) => {
                    Node::Document(children)
                }
                // The owned tree has no such nodes: they are left out.
                NodeData::Comment(_)
                | NodeData::DocumentType { .. }
                | NodeData::ProcessingInstruction { .. } => continue,
                NodeData::Element { tag, attrs, ns } => Node::Element(Element {
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                    children,
                    ns: ns.clone(),
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

    /// The bytes the document holds (M35.5): its slots, the data of its
    /// live nodes, and its journal; from the sizes it knows, not the
    /// allocator's.
    pub fn heap_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Slot>()
            + self.free.capacity() * std::mem::size_of::<u32>()
            + self.journal.capacity() * std::mem::size_of::<Change>()
            + self.data_bytes
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

    /// An HTML element's tag name; `None` for an element of another
    /// namespace (M32), so that asking whether a node is an HTML `a`, an
    /// `img` or a `title` never takes an SVG one for it.
    #[inline]
    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match self.data(id)? {
            NodeData::Element {
                tag,
                ns: Namespace::Html,
                ..
            } => Some(tag),
            _ => None,
        }
    }

    /// Any element's namespace and qualified name, as stored.
    #[inline]
    pub fn element_name(&self, id: NodeId) -> Option<(&Namespace, &str)> {
        match self.data(id)? {
            NodeData::Element { tag, ns, .. } => Some((ns, tag)),
            _ => None,
        }
    }

    /// Any element's local name: its qualified name without its prefix,
    /// the whole of it when it has none (a colon in the name of an
    /// element `createElement` or the HTML parser made is part of it).
    #[inline]
    pub fn local_name(&self, id: NodeId) -> Option<&str> {
        self.element_name(id)
            .map(|(_, tag)| match tag.contains(':') {
                true if !self.rare.prefixed.contains(&id) => tag,
                _ => local_part(tag),
            })
    }

    /// An element's namespace prefix, if its name has one.
    pub fn element_prefix(&self, id: NodeId) -> Option<&str> {
        if !self.rare.prefixed.contains(&id) {
            return None;
        }
        self.element_name(id)?
            .1
            .split_once(':')
            .map(|(prefix, _)| prefix)
    }

    /// The DOM's `nodeType` of `id`: [`NodeData::node_type`]'s, 4 for a
    /// CDATA section; `None` for a stale id.
    pub fn node_type(&self, id: NodeId) -> Option<u16> {
        let kind = self.data(id)?.node_type();
        Some(match kind == 3 && self.rare.cdata.contains(&id) {
            true => 4,
            false => kind,
        })
    }

    /// Whether `id` is a CDATA section: a text node of its own kind.
    pub fn is_cdata(&self, id: NodeId) -> bool {
        self.rare.cdata.contains(&id)
    }

    /// A new, detached CDATA section (`createCDATASection`, an XML
    /// parser's `<![CDATA[`).
    pub fn create_cdata(&mut self, data: &str) -> Result<NodeId, MutationError> {
        let made = self.create_text(data)?;
        self.rare.cdata.insert(made);
        Ok(made)
    }

    /// Gives an element made with no prefix the prefix `prefix`: DOM's
    /// "create an element" sets a custom element's after its constructor
    /// ran (step 5.1.4.11, D70), `createElementNS()` having asked for one.
    pub fn set_element_prefix(
        &mut self,
        element: NodeId,
        prefix: &str,
    ) -> Result<(), MutationError> {
        let local = self
            .local_name(element)
            .ok_or(MutationError::WrongKind)?
            .to_owned();
        if self.rare.prefixed.contains(&element) || prefix.is_empty() {
            return Err(MutationError::NotSupported);
        }
        self.check_data(prefix.len() + 1)?;
        let record = self.record_mut(element).ok_or(MutationError::NoSuchNode)?;
        if let NodeData::Element { tag, .. } = &mut record.data {
            *tag = format!("{prefix}:{local}");
        }
        self.data_bytes += prefix.len() + 1;
        self.rare.prefixed.insert(element);
        Ok(())
    }

    /// Makes `form` the form owner of the control `element` the parser
    /// just inserted with its form element pointer set (HTML section
    /// 13.2.6.1, "create an element for a token" step 12): an owner it
    /// keeps, though no ancestor of it, until its ancestors change.
    pub fn set_parser_form_owner(&mut self, element: NodeId, form: NodeId) {
        if self.contains(element) && self.contains(form) {
            self.rare.parser_forms.insert(element, form);
        }
    }

    /// The form owner the parser gave `element`, while it has it.
    pub fn parser_form_owner(&self, element: NodeId) -> Option<NodeId> {
        self.rare.parser_forms.get(&element).copied()
    }

    /// Notes that `element`'s qualified name has a prefix: one with a
    /// colon `createElementNS` or the XML parser made.
    pub fn mark_prefixed(&mut self, element: NodeId) {
        if self
            .element_name(element)
            .is_some_and(|(_, tag)| tag.contains(':'))
        {
            self.rare.prefixed.insert(element);
        }
    }

    /// An element's namespace.
    #[inline]
    pub fn namespace(&self, id: NodeId) -> Option<&Namespace> {
        self.element_name(id).map(|(ns, _)| ns)
    }

    /// Whether `id` is an element of the HTML namespace.
    #[inline]
    pub fn is_html(&self, id: NodeId) -> bool {
        self.namespace(id) == Some(&Namespace::Html)
    }

    /// An SVG element's local name; `None` for any other node.
    #[inline]
    pub fn svg_tag(&self, id: NodeId) -> Option<&str> {
        match self.element_name(id)? {
            (Namespace::Svg, tag) => Some(local_part(tag)),
            _ => None,
        }
    }

    /// An element's state flags (M26); none for another node or a stale
    /// id.
    #[inline]
    pub fn state(&self, id: NodeId) -> ElementState {
        self.record(id)
            .map_or(ElementState::NONE, |record| record.state)
    }

    /// The focused element, if any (M26.6).
    pub fn focused(&self) -> Option<NodeId> {
        self.focused.filter(|&element| self.contains(element))
    }

    /// Focuses `element`, or nothing (M26.6): the `focus` flag, and
    /// `focus-visible` when `visible`, move to it and to the hosts of the
    /// shadow trees it is in (D68), and `focus-within` to it and its
    /// ancestors in the flat tree, journaled as state changes. A node
    /// that is not a live element takes no focus.
    pub fn set_focus(&mut self, element: Option<NodeId>, visible: bool) {
        let focus = ElementState::FOCUS.with(ElementState::FOCUS_VISIBLE, true);
        if let Some(old) = self.focused.take().filter(|&old| self.contains(old)) {
            let _ = self.set_state(old, self.state(old).with(focus, false));
        }
        self.unfocus_hosts();
        self.within = None;
        for node in std::mem::take(&mut self.rare.shadows.focus_within) {
            if self.contains(node) {
                let state = self.state(node).with(ElementState::FOCUS_WITHIN, false);
                let _ = self.set_state(node, state);
            }
        }
        let Some(element) = element.filter(|&element| self.is_element(element)) else {
            return;
        };
        let flags = ElementState::FOCUS.with(ElementState::FOCUS_VISIBLE, visible);
        let _ = self.set_state(element, self.state(element).with(flags, true));
        let (hosts, within) = self.focus_chains(element);
        for &host in &hosts {
            let _ = self.set_state(host, self.state(host).with(ElementState::FOCUS, true));
        }
        for &node in &within {
            let state = self.state(node).with(ElementState::FOCUS_WITHIN, true);
            let _ = self.set_state(node, state);
        }
        self.rare.shadows.focused_hosts = hosts;
        self.rare.shadows.focus_within = within;
        self.focused = Some(element);
        self.within = Some(element);
        self.focus_navigation_focused(element);
    }

    /// The hosts the focus in their shadow trees gave the focus flag
    /// lose it.
    fn unfocus_hosts(&mut self) {
        crate::hook!("dom::document::unfocus_hosts");
        for host in std::mem::take(&mut self.rare.shadows.focused_hosts) {
            if self.contains(host) {
                let _ = self.set_state(host, self.state(host).with(ElementState::FOCUS, false));
            }
        }
    }

    /// The focused element loses the focus on its way to `next` (HTML's
    /// focus update steps, M26.11): its focus flags go, and
    /// `focus-within` from it up to the nearest ancestor it shares with
    /// `next`, which keep it, as the reference browser keeps them while
    /// `blur` and `focusout` are dispatched; [`Document::set_focus`] then
    /// gives `next` the focus, or takes the rest.
    pub fn blur_toward(&mut self, next: NodeId) {
        let Some(old) = self.focused.take().filter(|&old| self.contains(old)) else {
            return;
        };
        let focus = ElementState::FOCUS.with(ElementState::FOCUS_VISIBLE, true);
        let _ = self.set_state(old, self.state(old).with(focus, false));
        self.unfocus_hosts();
        if self.within.take().is_none() {
            return;
        }
        let (_, ahead) = self.focus_chains(next);
        let chain = std::mem::take(&mut self.rare.shadows.focus_within);
        for (at, &node) in chain.iter().enumerate() {
            if ahead.contains(&node) {
                self.within = Some(node);
                self.rare.shadows.focus_within = chain[at..].to_vec();
                break;
            }
            if self.contains(node) {
                let state = self.state(node).with(ElementState::FOCUS_WITHIN, false);
                let _ = self.set_state(node, state);
            }
        }
    }

    /// Sets an element's state flags to `state` (M26), journaled when
    /// they change. A node that is not an element has none.
    pub fn set_state(&mut self, element: NodeId, state: ElementState) -> Result<(), MutationError> {
        let record = self.record_mut(element).ok_or(MutationError::NoSuchNode)?;
        if !matches!(record.data, NodeData::Element { .. }) {
            return Err(MutationError::WrongKind);
        }
        let old = record.state;
        if old == state {
            return Ok(());
        }
        record.state = state;
        self.record_change(Change::StateChanged { element, old });
        Ok(())
    }

    /// A form control's state, if it has had a change (M26.7); the
    /// `forms` functions read it with the attributes.
    pub fn control(&self, element: NodeId) -> Option<&ControlState> {
        self.record(element)?.control.as_deref()
    }

    /// Sets an element's control state (M26.7), journaled when it
    /// changes; its value counts against the data limit.
    pub fn set_control(
        &mut self,
        element: NodeId,
        state: ControlState,
    ) -> Result<(), MutationError> {
        let record = self.record(element).ok_or(MutationError::NoSuchNode)?;
        if !matches!(record.data, NodeData::Element { .. }) {
            return Err(MutationError::WrongKind);
        }
        if record.control.as_deref() == Some(&state) {
            return Ok(());
        }
        let old = record.control.as_ref().map_or(0, |c| c.bytes());
        let new = state.bytes();
        self.check_data(new.saturating_sub(old))?;
        self.data_bytes = self.data_bytes + new - old;
        if let Some(record) = self.record_mut(element) {
            record.control = Some(Box::new(state));
        }
        self.record_change(Change::ControlChanged { element });
        Ok(())
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

    /// The data of a text, comment or processing instruction node.
    #[inline]
    pub fn character_data(&self, id: NodeId) -> Option<&str> {
        self.data(id)?.character_data()
    }

    /// A template's contents, the fragment made with it (D58).
    pub fn template_content(&self, template: NodeId) -> Option<NodeId> {
        self.rare.templates.contents.get(&template).copied()
    }

    /// The template whose contents `fragment` is.
    pub fn template_host(&self, fragment: NodeId) -> Option<NodeId> {
        self.rare.templates.hosts.get(&fragment).copied()
    }

    /// The subtree at `id` in tree order, then the contents of each
    /// template in it and the shadow tree of each host in it (D68),
    /// theirs too.
    pub fn subtree_with_contents(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut tops = vec![id];
        while let Some(top) = tops.pop() {
            for node in self.subtree(top) {
                out.push(node);
                if let Some(content) = self.template_content(node) {
                    tops.push(content);
                }
                if let Some(root) = self.shadow_root(node) {
                    tops.push(root);
                }
            }
        }
        out
    }

    /// The document's doctype: its first `DocumentType` child.
    pub fn doctype(&self) -> Option<NodeId> {
        self.children(self.root)
            .find(|&n| matches!(self.data(n), Some(NodeData::DocumentType { .. })))
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

    /// The text of the subtree at `id`, in tree order: its text nodes'
    /// data, or a comment's or processing instruction's own.
    pub fn text_content(&self, id: NodeId) -> String {
        if let Some(data) = self.character_data(id) {
            return data.to_owned();
        }
        self.subtree(id).filter_map(|n| self.text(n)).collect()
    }

    // ---- Changing -----------------------------------------------------------

    /// A new, detached element; the tag name is lowercased, as an HTML
    /// document's `createElement` does.
    pub fn create_element(&mut self, tag: &str) -> Result<NodeId, MutationError> {
        crate::hook!("dom::document::create_element");
        self.check_room()?;
        self.check_data(tag.len())?;
        let tag = if self.rare.xml {
            tag.to_owned()
        } else {
            tag.to_ascii_lowercase()
        };
        Ok(self.allocate(NodeData::Element {
            tag,
            attrs: Vec::new(),
            ns: Namespace::Html,
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
        let element = self.allocate(NodeData::Element {
            tag: tag.to_ascii_lowercase(),
            attrs,
            ns: Namespace::Html,
        });
        self.details_made(element);
        Ok(element)
    }

    /// A new, detached element of the namespace `ns`, named `qualified`
    /// and given `attrs` as they are: an HTML element's name lowercased,
    /// another's kept in its case, as the parsers and `createElementNS`
    /// make them (M32).
    pub fn create_element_ns(
        &mut self,
        ns: Namespace,
        qualified: &str,
        attrs: Vec<(String, String)>,
    ) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(
            qualified.len() + attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>(),
        )?;
        let tag = if ns == Namespace::Html && !self.rare.xml {
            qualified.to_ascii_lowercase()
        } else {
            qualified.to_owned()
        };
        let element = self.allocate(NodeData::Element { tag, attrs, ns });
        self.details_made(element);
        Ok(element)
    }

    /// A new, detached text node.
    pub fn create_text(&mut self, data: &str) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(data.len())?;
        Ok(self.allocate(NodeData::Text(data.to_string())))
    }

    /// A new, detached comment.
    pub fn create_comment(&mut self, data: &str) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(data.len())?;
        Ok(self.allocate(NodeData::Comment(data.to_owned())))
    }

    /// A new, detached processing instruction.
    pub fn create_processing_instruction(
        &mut self,
        target: &str,
        data: &str,
    ) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(target.len() + data.len())?;
        Ok(self.allocate(NodeData::ProcessingInstruction {
            target: target.into(),
            data: data.to_owned(),
        }))
    }

    /// A new, detached document type node.
    pub fn create_doctype(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> Result<NodeId, MutationError> {
        self.check_room()?;
        self.check_data(name.len() + public_id.len() + system_id.len())?;
        Ok(self.allocate(NodeData::DocumentType {
            name: name.into(),
            public_id: public_id.into(),
            system_id: system_id.into(),
        }))
    }

    /// A new, empty document fragment.
    pub fn create_fragment(&mut self) -> Result<NodeId, MutationError> {
        self.check_room()?;
        Ok(self.allocate(NodeData::DocumentFragment))
    }

    /// Inserts `child` as `parent`'s last child, taking it from where it
    /// was first, as the DOM's `appendChild` does.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), MutationError> {
        crate::hook!("dom::document::append_child");
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
        crate::hook!("dom::document::insert_before");
        if !self.contains(parent) || !self.contains(child) {
            return Err(MutationError::NoSuchNode);
        }
        if let Some(r) = reference
            && self.parent(r) != Some(parent)
        {
            return Err(if self.contains(r) {
                MutationError::NotAChild
            } else {
                MutationError::NoSuchNode
            });
        }
        self.check_insert(parent, child, reference, None)?;
        let depth = self.element_depth(parent) + self.element_height(child);
        if depth > MAX_DEPTH {
            return Err(MutationError::TooDeep);
        }
        if matches!(
            self.data(child),
            Some(NodeData::DocumentFragment | NodeData::ShadowRoot(_))
        ) {
            // A fragment's children go in its place, in order.
            let moved: Vec<NodeId> = self.children(child).collect();
            for node in moved {
                let old_owner = self.owner_document(node);
                let crossed = self.crosses_contents(node, parent);
                self.detach(node);
                let new_owner = self.owner_document(parent);
                let adopted = self.registries_before_adoption(node, old_owner, new_owner, crossed);
                match reference {
                    Some(r) => self.link_before(parent.index, node.index, r.index),
                    None => self.link_last(parent.index, node.index),
                }
                self.owned_by_tree(node);
                let owner = self.owner_document(node);
                self.registries_adopted(adopted, owner);
                self.live_inserted(parent, node);
                self.record_change(Change::Inserted {
                    parent,
                    child: node,
                });
                self.shadow_inserted(parent, node);
                self.observe_insertion(parent, node);
                if self.connected(parent) && self.has_names(node) {
                    self.index_names(node, true);
                }
                if self.in_page(parent) {
                    self.autofocus_inserted(node);
                }
                self.custom_inserted(node, Some(old_owner));
            }
            return Ok(());
        }
        let reference = match reference {
            Some(r) if r == child => self.next_sibling(child),
            other => other,
        };
        let old_owner = self.owner_document(child);
        let crossed = self.crosses_contents(child, parent);
        self.detach(child);
        let new_owner = self.owner_document(parent);
        let adopted = self.registries_before_adoption(child, old_owner, new_owner, crossed);
        match reference {
            Some(r) => self.link_before(parent.index, child.index, r.index),
            None => self.link_last(parent.index, child.index),
        }
        // Inserted, it is its parent's document's (DOM's "adopt").
        self.owned_by_tree(child);
        let owner = self.owner_document(child);
        self.registries_adopted(adopted, owner);
        self.live_inserted(parent, child);
        self.record_change(Change::Inserted { parent, child });
        self.shadow_inserted(parent, child);
        self.observe_insertion(parent, child);
        if self.connected(parent) && self.has_names(child) {
            self.index_names(child, true);
        }
        if self.in_page(parent) {
            self.autofocus_inserted(child);
        }
        self.custom_inserted(child, Some(old_owner));
        Ok(())
    }

    /// Moves `node` under `parent` before `reference` without its removal
    /// and insertion steps (DOM's `moveBefore`, D59): in one tree, an
    /// element or character data, by the insertion rules; the journal
    /// says `Moved` after its `Removed` and `Inserted`, and the focus
    /// stays.
    pub fn move_before(
        &mut self,
        parent: NodeId,
        node: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), MutationError> {
        if !self.contains(parent) || !self.contains(node) {
            return Err(MutationError::NoSuchNode);
        }
        let movable = matches!(
            self.data(node),
            Some(NodeData::Element { .. } | NodeData::Text(_) | NodeData::Comment(_))
                | Some(NodeData::ProcessingInstruction { .. })
        );
        if !movable || self.root_of(parent) != self.root_of(node) {
            return Err(MutationError::HierarchyRequest);
        }
        if let Some(r) = reference
            && self.parent(r) != Some(parent)
        {
            return Err(MutationError::NotAChild);
        }
        self.check_insert(
            parent,
            node,
            reference,
            self.parent(node).filter(|&p| p == parent).map(|_| node),
        )?;
        let reference = match reference {
            Some(r) if r == node => self.next_sibling(node),
            other => other,
        };
        self.rare.moving = true;
        self.detach(node);
        self.rare.moving = false;
        match reference {
            Some(r) => self.link_before(parent.index, node.index, r.index),
            None => self.link_last(parent.index, node.index),
        }
        self.owned_by_tree(node);
        self.live_inserted(parent, node);
        self.record_change(Change::Inserted {
            parent,
            child: node,
        });
        self.shadow_inserted(parent, node);
        self.observe_insertion(parent, node);
        if self.connected(parent) && self.has_names(node) {
            self.index_names(node, true);
        }
        self.record_change(Change::Moved {
            parent,
            child: node,
        });
        self.custom_moved(node);
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
        let allowed = self.may_insert(parent_data, &child_record.data)
            && !matches!(
                child_record.data,
                NodeData::DocumentFragment | NodeData::ShadowRoot(_)
            );
        if !allowed || child == self.root {
            return Err(MutationError::HierarchyRequest);
        }
        self.link_last(parent.index, child.index);
        self.owned_by_tree(child);
        self.live_inserted(parent, child);
        self.record_change(Change::Inserted { parent, child });
        self.shadow_inserted(parent, child);
        self.observe_insertion(parent, child);
        if self.connected(parent) && self.has_names(child) {
            self.index_names(child, true);
        }
        if self.in_page(parent) {
            self.autofocus_inserted(child);
        }
        self.custom_inserted(child, None);
        Ok(())
    }

    /// Removes `child` from `parent`; it stays valid and detached.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), MutationError> {
        crate::hook!("dom::document::remove_child");
        if !self.contains(parent) || !self.contains(child) {
            return Err(MutationError::NoSuchNode);
        }
        if self.parent(child) != Some(parent) {
            return Err(MutationError::NotAChild);
        }
        self.detach(child);
        Ok(())
    }

    /// Sets attribute `name` (lowercased on an HTML element, as an HTML
    /// document's `setAttribute` does) of `element` to `value`, adding
    /// it last when the element has none; setting the value it already
    /// has changes nothing and records nothing.
    pub fn set_attribute(
        &mut self,
        element: NodeId,
        name: &str,
        value: &str,
    ) -> Result<(), MutationError> {
        crate::hook!("dom::document::set_attribute");
        let name = self.attribute_name(element, name);
        let renames = self.names_by(element, &name) && self.connected(element);
        if renames {
            self.index_names(element, false);
        }
        let set = self.set_attribute_named(element, name, value);
        if renames {
            self.index_names(element, true);
        }
        set
    }

    /// [`Document::set_attribute`] of a name as the element keeps it: the
    /// first attribute of that name changed, else one of no namespace
    /// made.
    fn set_attribute_named(
        &mut self,
        element: NodeId,
        name: String,
        value: &str,
    ) -> Result<(), MutationError> {
        let at = self
            .attrs_mut(element)?
            .iter()
            .position(|(k, _)| *k == name);
        self.change_attribute(element, at, name, value, None)
    }

    /// Sets the value of `element`'s attribute at `at` to `value`, or adds
    /// one named `name` in `namespace` last for `None` (DOM's "change an
    /// attribute" and "append an attribute").
    fn change_attribute(
        &mut self,
        element: NodeId,
        at: Option<usize>,
        name: String,
        value: &str,
        namespace: Option<&str>,
    ) -> Result<(), MutationError> {
        let had = match at.and_then(|at| self.attrs(element)?.get(at)) {
            // The same value changes nothing but is still a change to
            // mutation observers, as the DOM's "change an attribute" is.
            Some((_, v)) if v == value => {
                // A script's element for it goes all the same (gap batch
                // H).
                self.references_changed(&Change::AttributeChanged {
                    element,
                    name: name.clone(),
                    old: None,
                });
                if self.observing.is_some() {
                    let old = Some(value.to_owned());
                    let (local, namespace) = self.observed_name(element, at, &name);
                    self.observed(|| Mutated::Attribute {
                        element,
                        name: local,
                        namespace,
                        old,
                    });
                }
                if self.is_custom(element) {
                    let namespace = at.and_then(|at| self.attribute_namespace(element, at));
                    let namespace = namespace.map(str::to_owned);
                    self.custom_attribute_changed(
                        element,
                        &name,
                        namespace.as_deref(),
                        Some(value),
                        Some(value),
                    );
                }
                return Ok(());
            }
            Some((_, v)) => Some(v.len()),
            None => None,
        };
        let (more, less) = match had {
            Some(old) => (value.len(), old),
            None => (name.len() + value.len() + namespace.map_or(0, str::len), 0),
        };
        self.check_data(more.saturating_sub(less))?;
        self.data_bytes = self.data_bytes + more - less;
        let attrs = self.attrs_mut(element)?;
        let (old, at) = match at.and_then(|at| attrs.get_mut(at).map(|slot| (at, slot))) {
            Some((at, (_, v))) => (Some(std::mem::replace(v, value.to_string())), at),
            None => {
                attrs.push((name.clone(), value.to_string()));
                // Counted above with the name and value.
                self.data_bytes -= namespace.map_or(0, str::len);
                self.push_attribute_namespace(element, namespace);
                (None, self.attrs(element).map_or(0, |a| a.len() - 1))
            }
        };
        let default = matches!(name.as_str(), "checked" | "selected").then(|| name.clone());
        let slotting = matches!(name.as_str(), "slot" | "name").then(|| name.clone());
        if self.observing.is_some() {
            let (local, namespace) = self.observed_name(element, Some(at), &name);
            self.observed(|| Mutated::Attribute {
                element,
                name: local,
                namespace,
                old: old.clone(),
            });
        }
        if self.is_custom(element) {
            let namespace = self.attribute_namespace(element, at).map(str::to_owned);
            self.custom_attribute_changed(
                element,
                &name,
                namespace.as_deref(),
                old.as_deref(),
                Some(value),
            );
        }
        self.record_change(Change::AttributeChanged { element, name, old });
        if let Some(name) = slotting {
            self.shadow_attribute_changed(element, &name);
        }
        if let Some(name) = default {
            crate::forms::default_changed(self, element, &name);
        }
        Ok(())
    }

    /// The local name and namespace a mutation observer is told of for
    /// `element`'s attribute at `at`, named `name`.
    fn observed_name(
        &self,
        element: NodeId,
        at: Option<usize>,
        name: &str,
    ) -> (String, Option<Box<str>>) {
        let namespace = at.and_then(|at| self.attribute_namespace(element, at));
        (
            attributes::attribute_local_name(namespace, name).to_owned(),
            namespace.map(Into::into),
        )
    }

    /// An attribute's name as `element` keeps it: lowercased on an HTML
    /// element of an HTML document, as given on another.
    fn attribute_name(&self, element: NodeId, name: &str) -> String {
        if self.is_html(element) && !self.is_xml_node(element) {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        }
    }

    /// Removes attribute `name` (lowercased on an HTML element) of
    /// `element`; removing one it does not have changes nothing.
    pub fn remove_attribute(&mut self, element: NodeId, name: &str) -> Result<(), MutationError> {
        let name = self.attribute_name(element, name);
        let renames = self.names_by(element, &name) && self.connected(element);
        if renames {
            self.index_names(element, false);
        }
        let removed = self.remove_attribute_named(element, name);
        if renames {
            self.index_names(element, true);
        }
        removed
    }

    /// [`Document::remove_attribute`] of a name as the element keeps it:
    /// the first attribute of that name.
    fn remove_attribute_named(
        &mut self,
        element: NodeId,
        name: String,
    ) -> Result<(), MutationError> {
        let attrs = self.attrs_mut(element)?;
        let Some(at) = attrs.iter().position(|(k, _)| *k == name) else {
            return Ok(());
        };
        self.remove_attribute_at(element, at)
    }

    /// Removes `element`'s attribute at `at` (DOM's "remove an
    /// attribute").
    fn remove_attribute_at(&mut self, element: NodeId, at: usize) -> Result<(), MutationError> {
        let (local, namespace) = match self.observing.is_some() {
            true => {
                let name = self
                    .attrs(element)
                    .and_then(|a| a.get(at))
                    .map(|(n, _)| n.clone());
                self.observed_name(element, Some(at), &name.unwrap_or_default())
            }
            false => (String::new(), None),
        };
        let attrs = self.attrs_mut(element)?;
        if at >= attrs.len() {
            return Ok(());
        }
        let custom_namespace = match self.is_custom(element) {
            true => Some(self.attribute_namespace(element, at).map(str::to_owned)),
            false => None,
        };
        let attrs = self.attrs_mut(element)?;
        let (name, old) = attrs.remove(at);
        self.data_bytes -= name.len() + old.len();
        self.remove_attribute_namespace(element, at);
        let default = matches!(name.as_str(), "checked" | "selected").then(|| name.clone());
        let slotting = matches!(name.as_str(), "slot" | "name").then(|| name.clone());
        self.observed(|| Mutated::Attribute {
            element,
            name: local,
            namespace,
            old: Some(old.clone()),
        });
        if let Some(namespace) = custom_namespace {
            self.custom_attribute_changed(element, &name, namespace.as_deref(), Some(&old), None);
        }
        self.record_change(Change::AttributeChanged {
            element,
            name,
            old: Some(old),
        });
        if let Some(name) = slotting {
            self.shadow_attribute_changed(element, &name);
        }
        if let Some(name) = default {
            crate::forms::default_changed(self, element, &name);
        }
        Ok(())
    }

    /// Replaces a text, comment or processing instruction node's data;
    /// the same data changes nothing but is still a change to mutation
    /// observers ("replace data").
    pub fn set_text(&mut self, node: NodeId, data: &str) -> Result<(), MutationError> {
        let old = self.node_length(node);
        self.store_data(node, data)?;
        self.live_replaced(node, old);
        Ok(())
    }

    /// Puts `data` in place of character data's, telling observers and
    /// the journal, the live ranges left to the caller.
    fn store_data(&mut self, node: NodeId, data: &str) -> Result<(), MutationError> {
        let record = self.record(node).ok_or(MutationError::NoSuchNode)?;
        let Some(text) = record.data.character_data() else {
            return Err(MutationError::WrongKind);
        };
        if text == data {
            let old = data.to_owned();
            self.observed(|| Mutated::Text { node, old });
            return Ok(());
        }
        let old = text.len();
        self.check_data(data.len().saturating_sub(old))?;
        self.data_bytes = self.data_bytes + data.len() - old;
        let mut old = None;
        if let Some(text) = self
            .record_mut(node)
            .and_then(|r| r.data.character_data_mut())
        {
            old = Some(std::mem::replace(text, data.to_owned()));
        }
        if let Some(old) = old {
            self.observed(|| Mutated::Text { node, old });
        }
        self.record_change(Change::TextChanged { node });
        Ok(())
    }

    /// Appends `data` to a text node's, as the DOM's `appendData` does:
    /// what a parser does with text that arrives in pieces (M24.8),
    /// without copying what the node has.
    pub fn append_data(&mut self, node: NodeId, data: &str) -> Result<(), MutationError> {
        let record = self.record(node).ok_or(MutationError::NoSuchNode)?;
        if record.data.character_data().is_none() {
            return Err(MutationError::WrongKind);
        }
        if data.is_empty() {
            return Ok(());
        }
        self.check_data(data.len())?;
        self.data_bytes += data.len();
        let observing = self.observing.is_some();
        let mut old = None;
        if let Some(text) = self
            .record_mut(node)
            .and_then(|r| r.data.character_data_mut())
        {
            old = observing.then(|| text.clone());
            text.push_str(data);
        }
        if let Some(old) = old {
            self.observed(|| Mutated::Text { node, old });
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
        // A template's contents go with the template, and a document
        // stays while a node outside it is its, or it holds a document's
        // templates' contents.
        if id == self.root
            || self.parent(id).is_some()
            || self.template_host(id).is_some()
            || self.shadow_host(id).is_some()
            || (self.is_document(id) && self.document_in_use(id))
        {
            return Err(MutationError::Attached);
        }
        // The subtree, and the contents of each template in it, theirs too.
        let doomed = self.subtree_with_contents(id);
        for node in &doomed {
            if let Some(content) = self.rare.templates.contents.remove(node) {
                self.rare.templates.hosts.remove(&content);
            }
        }
        self.forget_documents(&doomed);
        self.forget_shadows(&doomed);
        self.forget_custom(&doomed);
        if !self.rare.parser_forms.is_empty() {
            for node in &doomed {
                self.rare.parser_forms.remove(node);
            }
            self.rare
                .parser_forms
                .retain(|_, form| !doomed.contains(form));
        }
        self.data_bytes -= self.forget_attribute_namespaces(&doomed);
        if !self.rare.prefixed.is_empty() || !self.rare.cdata.is_empty() {
            for node in &doomed {
                self.rare.prefixed.remove(node);
                self.rare.cdata.remove(node);
            }
        }
        for index in doomed.into_iter().map(|n| n.index) {
            let slot = &mut self.slots[index as usize];
            if let Some(record) = slot.record.take() {
                self.data_bytes -=
                    data_size(&record.data) + record.control.map_or(0, |c| c.bytes());
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

    /// Whether `node` is in the tree: the root is among its inclusive
    /// ancestors.
    fn connected(&self, node: NodeId) -> bool {
        node == self.root || self.ancestors(node).last() == Some(self.root)
    }

    /// Whether attribute `name` of `element` names it as a window's
    /// named property: its `id`, or the `name` of an HTML `embed`,
    /// `form`, `img` or `object`.
    fn names_by(&self, element: NodeId, name: &str) -> bool {
        name == "id"
            || (name == "name"
                && self.is_html(element)
                && matches!(
                    self.local_name(element),
                    Some("embed" | "form" | "img" | "object")
                ))
    }

    /// Whether an element of the subtree at `top` has a name the window
    /// is to have: what is looked at before the walk up to the root,
    /// which the parser's deepest pages would make costly.
    fn has_names(&self, top: NodeId) -> bool {
        self.subtree(top).any(|n| {
            ["id", "name"].iter().any(|attribute| {
                self.names_by(n, attribute)
                    && self.attr(n, attribute).is_some_and(|v| !v.is_empty())
            })
        })
    }

    /// Counts the names of the elements of the subtree at `top` in
    /// (`add`) or out, keeping each name that comes or goes.
    fn index_names(&mut self, top: NodeId, add: bool) {
        let elements: Vec<NodeId> = self.subtree(top).filter(|&n| self.is_element(n)).collect();
        for element in elements {
            for attribute in ["id", "name"] {
                if !self.names_by(element, attribute) {
                    continue;
                }
                let Some(value) = self.attr(element, attribute).filter(|v| !v.is_empty()) else {
                    continue;
                };
                let value = value.to_owned();
                let count = self.named.counts.entry(value.clone()).or_insert(0);
                let changed = match add {
                    true => {
                        *count += 1;
                        *count == 1
                    }
                    false => {
                        *count = count.saturating_sub(1);
                        *count == 0
                    }
                };
                if !add && *count == 0 {
                    self.named.counts.remove(&value);
                }
                if changed {
                    if self.named.changes.len() >= MAX_NAMED_CHANGES {
                        self.named.changes.clear();
                        self.named.overflow = true;
                    }
                    if !self.named.overflow {
                        self.named.changes.push((value, add));
                    }
                }
            }
        }
    }

    /// The names that came (`true`) or went since this was last asked,
    /// in order; `None` when more did than were kept, the names then to
    /// be read afresh ([`Document::named`]).
    pub fn take_named_changes(&mut self) -> Option<Vec<(String, bool)>> {
        if std::mem::take(&mut self.named.overflow) {
            self.named.changes.clear();
            return None;
        }
        Some(std::mem::take(&mut self.named.changes))
    }

    /// The window's named properties' names as the tree now is, those
    /// taken since forgotten.
    pub fn named(&mut self) -> Vec<String> {
        self.named.changes.clear();
        self.named.overflow = false;
        let mut names: Vec<String> = self.named.counts.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Keeps the changes mutation observers are told of from now on,
    /// or stops keeping them and drops those kept (gap batch A).
    pub fn observe(&mut self, on: bool) {
        match on {
            true => {
                self.observing.get_or_insert_with(Default::default);
            }
            false => self.observing = None,
        }
    }

    /// The changes kept for mutation observers since this was last
    /// asked, oldest first.
    pub fn take_observed(&mut self) -> Vec<Mutation> {
        self.observing
            .as_mut()
            .map(|observing| observing.changes.drain(..).collect())
            .unwrap_or_default()
    }

    /// Whether changes are kept for mutation observers and not taken.
    pub fn has_observed(&self) -> bool {
        self.observing
            .as_ref()
            .is_some_and(|observing| !observing.changes.is_empty())
    }

    /// The nodes observers are registered on from now on, each with
    /// whether one watches its subtree.
    pub fn set_watched(&mut self, watched: std::collections::HashMap<NodeId, bool>) {
        if let Some(observing) = &mut self.observing {
            observing.watched = watched;
        }
    }

    /// Keeps the change `made` makes, while observers are told of
    /// changes, with the watched nodes among its target's inclusive
    /// ancestors; the oldest goes past [`MAX_OBSERVED`].
    fn observed(&mut self, made: impl FnOnce() -> Mutated) {
        crate::hook!("dom::document::observed");
        let Some(observing) = &self.observing else {
            return;
        };
        let change = made();
        let target = change.target();
        let watched = std::iter::once(target)
            .chain(self.ancestors(target))
            .filter(|node| observing.watched.contains_key(node))
            .collect();
        if let Some(observing) = &mut self.observing {
            if observing.changes.len() >= MAX_OBSERVED {
                observing.changes.pop_front();
            }
            observing.changes.push_back(Mutation { change, watched });
        }
    }

    /// Keeps `child`'s insertion under `parent`, with its siblings now.
    fn observe_insertion(&mut self, parent: NodeId, child: NodeId) {
        if self.observing.is_some() {
            let (prev, next) = (self.prev_sibling(child), self.next_sibling(child));
            self.observed(|| Mutated::ChildList {
                parent,
                added: Some(child),
                removed: None,
                prev,
                next,
            });
        }
    }

    /// The changes since the journal was last drained, oldest first.
    pub fn take_changes(&mut self) -> Vec<Change> {
        self.drains += u64::from(!self.journal.is_empty());
        std::mem::take(&mut self.journal)
    }

    /// How many times a journal with something in it was drained: a
    /// reader that keeps every drain's changes counts them too, and
    /// knows by comparing that none it has not seen was taken.
    pub fn drains(&self) -> u64 {
        self.drains
    }

    /// The changes since the journal was last drained, oldest first,
    /// left in it: what a script's `getComputedStyle` restyles by during
    /// the run that made them, before the engine drains them (M25).
    pub fn changes(&self) -> &[Change] {
        &self.journal
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
            // Another document is parentless, and known for one.
            if record.data == NodeData::Document
                && i != self.root.index
                && (record.parent.is_some()
                    || !self.rare.documents.facts.contains_key(&NodeId {
                        index: i,
                        generation: slot.generation,
                    }))
            {
                return Err(format!("document node {i} is not a known parentless one"));
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
            .map(|r| data_size(&r.data) + r.control.as_ref().map_or(0, |c| c.bytes()))
            .sum::<usize>()
            + self
                .rare
                .attr_ns
                .values()
                .map(|kept| attributes::namespaces_size(kept))
                .sum::<usize>()
            + self.shadow_data_bytes()
            + self.custom_data_bytes();
        if data != self.data_bytes {
            return Err(format!(
                "{data} bytes of data counted as {}",
                self.data_bytes
            ));
        }
        for (&template, &content) in &self.rare.templates.contents {
            if !self.is_element(template)
                || self.data(content) != Some(&NodeData::DocumentFragment)
                || self.parent(content).is_some()
                || self.template_host(content) != Some(template)
            {
                return Err(format!("template {template}'s contents are wrong"));
            }
        }
        if self.rare.templates.hosts.len() != self.rare.templates.contents.len() {
            return Err("a template's contents are not mirrored".into());
        }
        self.validate_shadows()?;
        self.validate_custom()?;
        let documents = &self.rare.documents;
        if documents.facts.values().filter(|facts| facts.xml).count() != documents.xml {
            return Err("the XML documents are miscounted".into());
        }
        for &document in documents.facts.keys() {
            if !self.is_document(document) || document == self.root {
                return Err(format!("{document} is no other document"));
            }
        }
        for (&top, &owner) in &documents.owners {
            if !self.contains(top)
                || self.parent(top).is_some()
                || self.is_document(top)
                || !documents.facts.contains_key(&owner)
            {
                return Err(format!("{top}'s owner {owner} is wrong"));
            }
        }
        for (&of, &inert) in &documents.inert {
            let is_inert = documents.facts.get(&inert).is_some_and(|facts| facts.inert);
            if !self.is_document(of) || !is_inert || documents.inert.get(&inert) != Some(&inert) {
                return Err(format!("{of}'s inert template document is wrong"));
            }
        }
        for (&element, kept) in &self.rare.attr_ns {
            if self.attrs(element).map(<[_]>::len) != Some(kept.len()) {
                return Err(format!(
                    "{element}'s attributes' namespaces are not one each"
                ));
            }
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

    /// A new node holding `data`; an HTML `template` is made with its
    /// contents, an empty fragment (D58).
    fn allocate(&mut self, data: NodeData) -> NodeId {
        let template = matches!(
            &data,
            NodeData::Element { tag, ns: Namespace::Html, .. } if tag == "template"
        );
        let id = self.allocate_one(data);
        if template {
            let content = self.allocate_one(NodeData::DocumentFragment);
            self.rare.templates.contents.insert(id, content);
            self.rare.templates.hosts.insert(content, id);
        }
        id
    }

    fn allocate_one(&mut self, data: NodeData) -> NodeId {
        self.data_bytes += data_size(&data);
        let record = Record {
            data,
            state: ElementState::NONE,
            control: None,
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
    /// The DOM's hierarchy checks of inserting `node` under `parent`
    /// before `reference` ("ensure pre-insertion validity"), or in place
    /// of its child `replacing` ("replace a child"): a node into a
    /// container, never into itself or what holds it, and the document's
    /// one element and one doctype, the doctype first.
    pub fn check_insert(
        &self,
        parent: NodeId,
        node: NodeId,
        reference: Option<NodeId>,
        replacing: Option<NodeId>,
    ) -> Result<(), MutationError> {
        let parent_data = self.data(parent).ok_or(MutationError::NoSuchNode)?;
        let node_data = self.data(node).ok_or(MutationError::NoSuchNode)?;
        if !self.may_insert(parent_data, node_data)
            || self.is_host_including_ancestor(node, parent)
            || !self.fits_document(parent, node, reference, replacing)
        {
            return Err(MutationError::HierarchyRequest);
        }
        Ok(())
    }

    /// Whether the DOM lets a node like `child` be a child of one like
    /// `parent` ("ensure pre-insertion validity", steps 1, 4 and 5).
    fn may_insert(&self, parent: &NodeData, child: &NodeData) -> bool {
        let container = matches!(
            parent,
            NodeData::Document
                | NodeData::DocumentFragment
                | NodeData::ShadowRoot(_)
                | NodeData::Element { .. }
        );
        let document = matches!(parent, NodeData::Document);
        container
            && match child {
                NodeData::Document => false,
                NodeData::Text(_) => !document,
                NodeData::DocumentType { .. } => document,
                _ => true,
            }
    }

    /// Whether `ancestor` is a host-including inclusive ancestor of
    /// `node`: an ancestor, or one of a template whose contents hold it
    /// or of a host whose shadow tree does (D68).
    fn is_host_including_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == ancestor {
                return true;
            }
            at = self.fragment_parent(n);
        }
        false
    }

    /// A node's parent, or a template's contents' template, or a shadow
    /// root's host: the next node up through what holds a fragment.
    #[inline]
    fn fragment_parent(&self, node: NodeId) -> Option<NodeId> {
        self.parent(node)
            .or_else(|| self.template_host(node))
            .or_else(|| self.shadow_host(node))
    }

    /// The document's own rules (step 6 of "ensure pre-insertion
    /// validity", or of "replace" with `replacing`): at most one element
    /// and one doctype, the doctype first. `reference` is the child the
    /// node goes before.
    fn fits_document(
        &self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
        replacing: Option<NodeId>,
    ) -> bool {
        if !matches!(self.data(parent), Some(NodeData::Document)) {
            return true;
        }
        let is_doctype = |n: NodeId| matches!(self.data(n), Some(NodeData::DocumentType { .. }));
        // The children that stay: all but the one replaced.
        let stays = |n: NodeId| Some(n) != replacing;
        let has_element = || {
            self.children(parent)
                .any(|n| stays(n) && self.is_element(n))
        };
        let doctype_from = |r: Option<NodeId>| {
            std::iter::successors(r, |&n| self.next_sibling(n)).any(|n| stays(n) && is_doctype(n))
        };
        let element_before = |r: Option<NodeId>| match r {
            Some(r) => std::iter::successors(self.prev_sibling(r), |&n| self.prev_sibling(n))
                .any(|n| stays(n) && self.is_element(n)),
            None => has_element(),
        };
        let one_element =
            |count: usize| count == 0 || (count == 1 && !has_element() && !doctype_from(reference));
        match self.data(child) {
            Some(NodeData::DocumentFragment | NodeData::ShadowRoot(_)) => {
                let kids: Vec<NodeId> = self.children(child).collect();
                !kids.iter().any(|&n| self.is_text(n))
                    && one_element(kids.iter().filter(|&&n| self.is_element(n)).count())
            }
            Some(NodeData::Element { .. }) => one_element(1),
            Some(NodeData::DocumentType { .. }) => {
                !self.children(parent).any(|n| stays(n) && is_doctype(n))
                    && !element_before(reference)
            }
            _ => true,
        }
    }

    fn element_depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut at = Some(id);
        while let Some(node) = at {
            depth += usize::from(self.is_element(node));
            at = self.fragment_parent(node);
        }
        depth
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
            if let Some(content) = self.template_content(node) {
                stack.push((content, depth));
            }
            if let Some(root) = self.shadow_root(node) {
                stack.push((root, depth));
            }
        }
        deepest
    }

    /// Takes `child` out of its parent's children, recording the change;
    /// nothing for a detached node. The focus goes with an element taken
    /// out that has it or holds what has, at once (M26.6, HTML's focus
    /// fixup).
    fn detach(&mut self, child: NodeId) {
        if !self.rare.moving
            && let Some(focused) = self.focused().or(self.within.filter(|&w| self.contains(w)))
            && self.is_shadow_including_inclusive_ancestor(child, focused)
            && self.parent(child).is_some()
        {
            self.set_focus(None, false);
        }
        let Some(record) = self.record(child) else {
            return;
        };
        let Some(parent) = record.parent else {
            return;
        };
        if self.connected(child) && self.has_names(child) {
            self.index_names(child, false);
        }
        // Its controls' form owners are their ancestors' again (HTML's
        // "reset the form owner").
        if !self.rare.parser_forms.is_empty() {
            for node in self.subtree(child).collect::<Vec<_>>() {
                self.rare.parser_forms.remove(&node);
            }
        }
        // Disconnected, its custom elements hear it after (D70); a move
        // tells them it moved instead.
        let disconnects = !self.rare.moving && self.is_connected(child);
        // Taken out of another document's tree, it stays that document's.
        let mut owner = None;
        if let Some(parent) = self.link(Some(parent)) {
            if !self.rare.moving {
                let index = self.index_of(child);
                self.starting_point_leaving(parent, child, index);
            }
            self.live_removing(parent, child);
            owner = self.owner_leaving(parent);
        }
        if let Some(owner) = owner {
            self.rare.documents.owners.insert(child, owner);
        }
        let Some(record) = self.record(child) else {
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
        if !self.rare.moving {
            self.top_layer_removing(child);
        }
        let change = Change::Removed {
            parent: self.link(Some(parent)).expect("a parent is live"),
            child,
            prev: self.link(prev),
            next: self.link(next),
        };
        if let Change::Removed {
            parent, prev, next, ..
        } = change
            && let Some(observing) = &self.observing
        {
            // Taken out of a watched subtree, the node is watched until
            // its observers are next told (DOM's transient registrations).
            let subtree = std::iter::once(parent)
                .chain(self.ancestors(parent))
                .any(|node| observing.watched.get(&node) == Some(&true));
            if subtree && let Some(observing) = &mut self.observing {
                observing.watched.entry(child).or_insert(true);
            }
            self.observed(|| Mutated::ChildList {
                parent,
                added: None,
                removed: Some(child),
                prev,
                next,
            });
        }
        self.record_change(change);
        if let Some(parent) = self.link(Some(parent)) {
            self.shadow_removed(parent, child);
        }
        if disconnects {
            self.custom_removed(child);
        }
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
        self.references_changed(&change);
        self.details_changed(&change);
        self.dialogs_changed(&change);
        self.version += 1;
        self.custom_changed();
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
        NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot(_) => 0,

        NodeData::Element { tag, attrs, .. } => {
            tag.len() + attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
        }
        NodeData::Text(text) | NodeData::Comment(text) => text.len(),
        NodeData::DocumentType {
            name,
            public_id,
            system_id,
        } => name.len() + public_id.len() + system_id.len(),
        NodeData::ProcessingInstruction { target, data } => target.len() + data.len(),
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

    #[test]
    fn its_bytes_hold_its_nodes_data() {
        // M35.5.
        let mut document = Document::new();
        let before = document.heap_bytes();
        document.create_text(&"x".repeat(10_000)).unwrap();
        assert!(document.heap_bytes() >= before + 10_000);
    }

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
            Some(other) => format!("#{}", other.node_type()),
            None => "?".into(),
        })
        .collect()
    }

    #[test]
    fn the_document_takes_one_doctype_then_one_element_and_no_text() {
        let mut doc = Document::new();
        let root = doc.root();
        let html = doc.create_element("html").unwrap();
        let doctype = doc.create_doctype("html", "", "").unwrap();
        let text = doc.create_text("t").unwrap();
        let comment = doc.create_comment("c").unwrap();
        doc.append_child(root, html).unwrap();
        // A doctype after the element, or under one, is refused.
        assert_eq!(
            doc.append_child(root, doctype),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(
            doc.append_child(html, doctype),
            Err(MutationError::HierarchyRequest)
        );
        doc.insert_before(root, doctype, Some(html)).unwrap();
        let second = doc.create_doctype("x", "", "").unwrap();
        assert_eq!(
            doc.insert_before(root, second, Some(html)),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(
            doc.append_child(root, text),
            Err(MutationError::HierarchyRequest)
        );
        let body = doc.create_element("body").unwrap();
        assert_eq!(
            doc.append_child(root, body),
            Err(MutationError::HierarchyRequest)
        );
        // Comments go anywhere; nothing goes under one.
        doc.append_child(root, comment).unwrap();
        assert_eq!(
            doc.append_child(comment, text),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(doc.doctype(), Some(doctype));
        assert_eq!(doc.text_content(comment), "c");
        doc.validate().unwrap();
    }

    #[test]
    fn a_fragment_inserted_gives_its_children_in_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = doc.create_element("div").unwrap();
        doc.append_child(root, div).unwrap();
        let last = doc.create_element("p").unwrap();
        doc.append_child(div, last).unwrap();
        let fragment = doc.create_fragment().unwrap();
        let (a, b) = (
            doc.create_text("a").unwrap(),
            doc.create_comment("b").unwrap(),
        );
        doc.append_child(fragment, a).unwrap();
        doc.append_child(fragment, b).unwrap();
        doc.take_changes();
        doc.insert_before(div, fragment, Some(last)).unwrap();
        assert_eq!(doc.children(div).collect::<Vec<_>>(), [a, b, last]);
        assert_eq!(doc.first_child(fragment), None);
        assert!(doc.changes().contains(&Change::Inserted {
            parent: div,
            child: b
        }));
        // A fragment with two elements, or text, is refused by the document.
        let two = doc.create_fragment().unwrap();
        let (x, y) = (
            doc.create_element("x").unwrap(),
            doc.create_element("y").unwrap(),
        );
        doc.remove_child(root, div).unwrap();
        doc.append_child(two, x).unwrap();
        doc.append_child(two, y).unwrap();
        assert_eq!(
            doc.append_child(root, two),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(doc.first_child(two), Some(x), "nothing moved");
        doc.validate().unwrap();
    }

    #[test]
    fn a_template_owns_its_contents_and_frees_them_with_it() {
        let mut doc = Document::new();
        let root = doc.root();
        let template = doc.create_element("template").unwrap();
        let content = doc
            .template_content(template)
            .expect("contents made with it");
        assert_eq!(doc.template_host(content), Some(template));
        assert_eq!(doc.data(content), Some(&NodeData::DocumentFragment));
        // An SVG `template` is no HTML one: it has none.
        let svg = doc
            .create_element_ns(Namespace::Svg, "template", Vec::new())
            .unwrap();
        assert_eq!(doc.template_content(svg), None);
        doc.append_child(root, template).unwrap();
        let b = doc.create_element("b").unwrap();
        doc.append_child(content, b).unwrap();
        // The contents are no part of the tree, and go with the template
        // only: the template cannot go into its own contents.
        assert!(!doc.subtree(root).any(|n| n == b));
        assert_eq!(doc.free(content), Err(MutationError::Attached));
        assert_eq!(
            doc.append_child(b, template),
            Err(MutationError::HierarchyRequest)
        );
        doc.validate().unwrap();
        doc.remove_child(root, template).unwrap();
        doc.free(template).unwrap();
        assert!(!doc.contains(content) && !doc.contains(b));
        doc.validate().unwrap();
    }

    #[test]
    fn an_svg_element_keeps_its_namespace_and_case_and_is_no_html_element() {
        let mut document = Document::new();
        let root = document.root();
        let svg = document
            .create_element_ns(Namespace::Svg, "linearGradient", Vec::new())
            .unwrap();
        document.append_child(root, svg).unwrap();
        let a = document
            .create_element_ns(Namespace::Svg, "a", Vec::new())
            .unwrap();
        document.append_child(svg, a).unwrap();
        // Asking for an HTML `a` never finds SVG's.
        assert_eq!(document.tag(a), None);
        assert_eq!(document.svg_tag(a), Some("a"));
        assert_eq!(document.local_name(svg), Some("linearGradient"));
        assert_eq!(
            document.element_name(svg),
            Some((&Namespace::Svg, "linearGradient"))
        );
        assert!(!document.is_html(svg) && document.is_element(svg));
        // An SVG element's attributes keep their case; an HTML one's are
        // lowercased.
        document
            .set_attribute(svg, "gradientUnits", "userSpaceOnUse")
            .unwrap();
        assert_eq!(document.attr(svg, "gradientUnits"), Some("userSpaceOnUse"));
        document.remove_attribute(svg, "gradientunits").unwrap();
        assert_eq!(document.attr(svg, "gradientUnits"), Some("userSpaceOnUse"));
        let div = document
            .create_element_ns(Namespace::Html, "DIV", Vec::new())
            .unwrap();
        document.set_attribute(div, "ClassName", "x").unwrap();
        assert_eq!(document.tag(div), Some("div"));
        assert_eq!(document.attr(div, "classname"), Some("x"));
        // An XML document's HTML elements keep their names' case.
        let mut xml = Document::new();
        xml.set_xml_document(true);
        assert!(xml.is_xml_document() && !document.is_xml_document());
        let upper = xml
            .create_element_ns(Namespace::Html, "DIV", Vec::new())
            .unwrap();
        let made = xml.create_element("Span").unwrap();
        xml.set_attribute(upper, "ClassName", "x").unwrap();
        assert_eq!(xml.tag(upper), Some("DIV"));
        assert_eq!(xml.tag(made), Some("Span"));
        assert_eq!(xml.attr(upper, "ClassName"), Some("x"));
        assert_eq!(xml.attr(upper, "classname"), None);
        // A prefixed name's local part; a name with a colon made without
        // a prefix (`createElement`'s, the HTML parser's) is whole.
        let other = document
            .create_element_ns(Namespace::from_uri(Some("urn:x")), "x:item", Vec::new())
            .unwrap();
        assert_eq!(document.local_name(other), Some("x:item"));
        assert_eq!(document.element_prefix(other), None);
        document.mark_prefixed(other);
        assert_eq!(document.local_name(other), Some("item"));
        assert_eq!(document.element_prefix(other), Some("x"));
        assert_eq!(
            document.namespace(other).and_then(Namespace::uri),
            Some("urn:x")
        );
        // The owned tree keeps the namespace both ways.
        let copy = Document::from_node(&document.to_node());
        let first = copy.document_element().unwrap();
        assert_eq!(
            copy.element_name(first),
            Some((&Namespace::Svg, "linearGradient"))
        );
        assert_eq!(
            Namespace::from_uri(Some(crate::SVG_NAMESPACE)),
            Namespace::Svg
        );
        assert_eq!(Namespace::from_uri(Some("")), Namespace::Other(None));
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
    fn drains_count_the_journals_taken_with_something_in_them() {
        let mut document = Document::from_node(&sample());
        let body = element(&document, "body");
        assert_eq!(document.drains(), 0);
        document.take_changes();
        assert_eq!(document.drains(), 0, "an empty journal taken is no drain");
        let p = document.create_element("p").unwrap();
        document.append_child(body, p).unwrap();
        assert_eq!(document.drains(), 0, "a change is not a drain");
        document.take_changes();
        assert_eq!(document.drains(), 1);
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
    fn observed_changes_are_kept_only_while_observing() {
        let mut doc = Document::new();
        // The document takes one element: the changes are under it.
        let root = doc.create_element("html").unwrap();
        let top = doc.root();
        doc.append_child(top, root).unwrap();
        let a = doc.create_element("a").unwrap();
        doc.append_child(root, a).unwrap();
        assert!(doc.take_observed().is_empty(), "not observing");
        doc.observe(true);
        let b = doc.create_element("b").unwrap();
        let c = doc.create_text("x").unwrap();
        doc.append_child(root, b).unwrap();
        doc.append_new(b, c).unwrap();
        doc.set_attribute(a, "ID", "1").unwrap();
        doc.set_attribute(a, "id", "2").unwrap();
        // The same value again is a change to observers.
        doc.set_attribute(a, "id", "2").unwrap();
        doc.remove_attribute(a, "id").unwrap();
        doc.set_text(c, "y").unwrap();
        doc.set_text(c, "y").unwrap();
        doc.append_data(c, "z").unwrap();
        doc.insert_before(root, b, Some(a)).unwrap();
        let attribute = |old: Option<&str>| Mutated::Attribute {
            element: a,
            name: "id".to_owned(),
            namespace: None,
            old: old.map(str::to_owned),
        };
        let child = |added, removed, prev, next| Mutated::ChildList {
            parent: root,
            added,
            removed,
            prev,
            next,
        };
        let changes = |doc: &mut Document| {
            doc.take_observed()
                .into_iter()
                .map(|m| m.change)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            changes(&mut doc),
            [
                child(Some(b), None, Some(a), None),
                Mutated::ChildList {
                    parent: b,
                    added: Some(c),
                    removed: None,
                    prev: None,
                    next: None
                },
                attribute(None),
                attribute(Some("1")),
                attribute(Some("2")),
                attribute(Some("2")),
                Mutated::Text {
                    node: c,
                    old: "x".to_owned()
                },
                Mutated::Text {
                    node: c,
                    old: "y".to_owned()
                },
                Mutated::Text {
                    node: c,
                    old: "y".to_owned()
                },
                // A move is a removal and an insertion.
                child(None, Some(b), Some(a), None),
                child(Some(b), None, None, Some(a)),
            ]
        );
        assert!(doc.take_observed().is_empty());
        doc.observe(false);
        doc.remove_child(root, b).unwrap();
        doc.observe(true);
        assert!(
            doc.take_observed().is_empty(),
            "what changed unobserved is not kept"
        );
        // The watched among a change's inclusive ancestors, nearest
        // first; a node taken out of a subtree watched as a subtree is
        // watched itself, and of one watched without, not.
        doc.append_child(root, a).unwrap();
        doc.take_observed();
        doc.set_watched([(root, true)].into_iter().collect());
        doc.set_attribute(a, "x", "1").unwrap();
        doc.remove_child(root, a).unwrap();
        doc.set_attribute(a, "x", "2").unwrap();
        doc.set_watched([(root, false)].into_iter().collect());
        doc.append_child(root, a).unwrap();
        doc.remove_child(root, a).unwrap();
        doc.set_attribute(a, "x", "3").unwrap();
        let watched: Vec<Vec<NodeId>> =
            doc.take_observed().into_iter().map(|m| m.watched).collect();
        assert_eq!(
            watched,
            [
                vec![root],
                vec![root],
                vec![a],
                vec![root],
                vec![root],
                vec![]
            ]
        );
    }

    #[test]
    fn the_windows_names_come_and_go_with_the_elements_in_the_tree() {
        let page = Node::document(vec![
            Element::new("div")
                .attr("id", "a")
                .push(Element::new("img").attr("name", "pic"))
                .push(Element::new("span").attr("name", "not"))
                .into(),
        ]);
        let mut doc = Document::from_node(&page);
        assert_eq!(doc.take_named_changes(), Some(vec![]));
        assert_eq!(doc.named(), ["a", "pic"]);
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        // A detached element's id is no name until it is in the tree.
        let b = doc.create_element("p").unwrap();
        doc.set_attribute(b, "id", "b").unwrap();
        doc.append_child(div, b).unwrap();
        doc.set_attribute(b, "id", "c").unwrap();
        // Another element of the same id: no change; one of two gone, none.
        let a2 = doc.create_element("p").unwrap();
        doc.set_attribute(a2, "id", "a").unwrap();
        doc.append_child(div, a2).unwrap();
        doc.remove_child(div, a2).unwrap();
        // A subtree taken out takes its names.
        doc.remove_child(root, div).unwrap();
        let changes = |pairs: &[(&str, bool)]| {
            Some(
                pairs
                    .iter()
                    .map(|&(n, a)| (n.to_owned(), a))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            doc.take_named_changes(),
            changes(&[
                ("b", true),
                ("b", false),
                ("c", true),
                ("a", false),
                ("pic", false),
                ("c", false)
            ])
        );
        assert!(doc.named().is_empty());
    }

    #[test]
    fn the_observed_queue_keeps_its_newest_past_its_bound() {
        let mut doc = Document::new();
        let a = doc.create_element("a").unwrap();
        doc.append_child(doc.root(), a).unwrap();
        doc.observe(true);
        for i in 0..=MAX_OBSERVED {
            doc.set_attribute(a, "n", &i.to_string()).unwrap();
        }
        let kept = doc.take_observed();
        assert_eq!(kept.len(), MAX_OBSERVED);
        assert_eq!(
            kept[0].change,
            Mutated::Attribute {
                element: a,
                name: "n".to_owned(),
                namespace: None,
                old: Some("0".to_owned())
            }
        );
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
