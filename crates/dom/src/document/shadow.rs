//! Shadow trees (gap batch H, D68): a shadow root is a node of its own
//! kind ([`NodeData::ShadowRoot`]), allocated in the arena and parentless
//! as DOM section 4.8 has it, its host's link to it and its link to its
//! host kept beside the tree ([`Document::shadow_root`],
//! [`Document::shadow_host`]); its children are its tree.
//!
//! Slots are assigned as DOM section 4.2.2.4 says, when a tree changes:
//! a shadow root's slots, in tree order, are kept until a slot comes or
//! goes, and each slot's assigned nodes and each slottable's assigned
//! slot are kept, so that `assignedNodes()`, `assignedSlot` and the flat
//! tree read them. A slot whose assigned nodes change, or whose fallback
//! content changes while it has none, is signaled
//! ([`Document::take_slot_changes`]): the bindings fire `slotchange` at
//! it when they notify mutation observers. A change of a slot's assigned
//! nodes is journaled as [`Change::FlatTree`].
//!
//! The flat tree (CSS Scoping 1 section 2.2) is a view of the arena that
//! the cascade, layout, hit testing and rendering walk: a host's flat
//! children are its shadow root's children, a slot's are its assigned
//! nodes or, with none, its own children, and a host's own children are
//! in it only where they are slotted.

use std::collections::{HashMap, HashSet};

use super::{Change, Children, Document, MutationError, NodeData, NodeId};
use crate::Namespace;

/// A shadow root's mode (DOM section 4.8): whether its host's
/// `shadowRoot` and an event's `composedPath()` show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowRootMode {
    Open,
    Closed,
}

/// How a shadow root's slots are given their slottables: by their names,
/// or by the slots' `assign()` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotAssignmentMode {
    Named,
    Manual,
}

/// What a shadow root is made with (`ShadowRootInit`, and a declarative
/// shadow root's attributes), which it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShadowRootInit {
    pub mode: ShadowRootMode,
    pub delegates_focus: bool,
    pub slot_assignment: SlotAssignmentMode,
    pub clonable: bool,
    pub serializable: bool,
}

impl ShadowRootInit {
    /// An open root of named slots, nothing else asked for.
    pub fn open() -> ShadowRootInit {
        ShadowRootInit {
            mode: ShadowRootMode::Open,
            delegates_focus: false,
            slot_assignment: SlotAssignmentMode::Named,
            clonable: false,
            serializable: false,
        }
    }
}

/// The local names of the HTML elements that may host a shadow root
/// (DOM section 4.9, "valid shadow host name"), besides custom elements.
const SHADOW_HOSTS: [&str; 18] = [
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];

/// The names a custom element may not have (HTML section 4.13.2).
const RESERVED_NAMES: [&str; 8] = [
    "annotation-xml",
    "color-profile",
    "font-face",
    "font-face-src",
    "font-face-uri",
    "font-face-format",
    "font-face-name",
    "missing-glyph",
];

/// Whether `name` is a valid custom element name (HTML section 4.13.2,
/// as the reference browser has it): an ASCII lower alpha first, a
/// hyphen in it, no ASCII upper alpha, none of NULL, tab, line feed,
/// form feed, carriage return, space, `/` and `>` (a valid element local
/// name), and not one of the reserved names.
pub fn is_valid_custom_element_name(name: &str) -> bool {
    let allowed = |c: char| {
        !c.is_ascii_uppercase()
            && !matches!(c, '\0' | '\t' | '\n' | '\x0C' | '\r' | ' ' | '/' | '>')
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(allowed)
        && name.contains('-')
        && !RESERVED_NAMES.contains(&name)
}

/// The shadow roots, their hosts, and their slots' assignments.
#[derive(Debug, Clone, Default)]
pub(super) struct Shadows {
    /// Each host's shadow root.
    pub(super) roots: HashMap<NodeId, NodeId>,
    /// Each shadow root's host.
    pub(super) hosts: HashMap<NodeId, NodeId>,
    /// The shadow roots a parser made from a declarative template and
    /// no script has attached since (DOM's "declarative" flag).
    declarative: HashSet<NodeId>,
    /// Each shadow root's slots in tree order, while no slot came or
    /// went.
    slots: HashMap<NodeId, Vec<NodeId>>,
    /// Each slot's assigned nodes, for those that have some.
    assigned: HashMap<NodeId, Vec<NodeId>>,
    /// Each assigned slottable's slot.
    assigned_slot: HashMap<NodeId, NodeId>,
    /// Each slot's manually assigned nodes (`assign()`), for those that
    /// have some.
    manual: HashMap<NodeId, Vec<NodeId>>,
    /// Each manually assigned node's slot.
    manual_slot: HashMap<NodeId, NodeId>,
    /// The signal slots (DOM section 4.2.2.5), in order.
    signals: Vec<NodeId>,
    /// The hosts given the focus flag for the focused element in their
    /// shadow trees, which lose it with it.
    pub(super) focused_hosts: Vec<NodeId>,
    /// The elements given the focus-within flag, the focused one first,
    /// which lose it with the focus.
    pub(super) focus_within: Vec<NodeId>,
    /// Each shadow root's reference target (`referenceTarget`), for those
    /// that have one.
    reference_targets: HashMap<NodeId, String>,
}

/// [`Document::flat_children`]: a node's children, or a slot's
/// assigned nodes.
pub enum FlatChildren<'a> {
    Tree(Children<'a>),
    Assigned(std::slice::Iter<'a, NodeId>),
}

impl Iterator for FlatChildren<'_> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<NodeId> {
        match self {
            FlatChildren::Tree(children) => children.next(),
            FlatChildren::Assigned(nodes) => nodes.next().copied(),
        }
    }
}

impl Document {
    /// DOM's "attach a shadow root" (section 4.9) to `host` with `init`,
    /// declarative when a parser makes it: refused for a node that is no
    /// element, and as not supported for an element that may not host
    /// one, or that hosts one already but for a declarative one of the
    /// same mode, which is emptied, made no longer declarative and
    /// answered. The new root's host's flat children change.
    pub fn attach_shadow(
        &mut self,
        host: NodeId,
        init: ShadowRootInit,
        declarative: bool,
    ) -> Result<NodeId, MutationError> {
        if !self.is_element(host) {
            return Err(MutationError::WrongKind);
        }
        if !self.may_host_shadow(host) {
            return Err(MutationError::NotSupported);
        }
        if let Some(existing) = self.shadow_root(host) {
            let same_mode = self.shadow_init(existing).map(|i| i.mode) == Some(init.mode);
            if !self.rare.shadows.declarative.contains(&existing) || !same_mode {
                return Err(MutationError::NotSupported);
            }
            let children: Vec<NodeId> = self.children(existing).collect();
            for child in children {
                self.detach(child);
            }
            self.rare.shadows.declarative.remove(&existing);
            return Ok(existing);
        }
        self.check_room()?;
        let root = self.allocate_one(NodeData::ShadowRoot(init));
        self.rare.shadows.roots.insert(host, root);
        self.rare.shadows.hosts.insert(root, host);
        if declarative {
            self.rare.shadows.declarative.insert(root);
        }
        self.record_change(Change::FlatTree { node: host });
        Ok(root)
    }

    /// Whether `element` may host a shadow root (DOM section 4.9's
    /// "valid shadow host name"): an HTML element of one of the names
    /// listed, or of a valid custom element name.
    pub fn may_host_shadow(&self, element: NodeId) -> bool {
        match self.element_name(element) {
            Some((Namespace::Html, _)) => {
                let name = self.local_name(element).unwrap_or_default();
                SHADOW_HOSTS.contains(&name) || is_valid_custom_element_name(name)
            }
            _ => false,
        }
    }

    /// Whether any shadow root was ever attached and is still alive.
    #[inline]
    pub fn has_shadow_roots(&self) -> bool {
        !self.rare.shadows.hosts.is_empty()
    }

    /// The shadow root `host` hosts.
    #[inline]
    pub fn shadow_root(&self, host: NodeId) -> Option<NodeId> {
        if self.rare.shadows.roots.is_empty() {
            return None;
        }
        self.rare.shadows.roots.get(&host).copied()
    }

    /// The host of the shadow root `root`.
    #[inline]
    pub fn shadow_host(&self, root: NodeId) -> Option<NodeId> {
        if self.rare.shadows.hosts.is_empty() {
            return None;
        }
        self.rare.shadows.hosts.get(&root).copied()
    }

    /// Whether `node` is a shadow root.
    #[inline]
    pub fn is_shadow_root(&self, node: NodeId) -> bool {
        matches!(self.data(node), Some(NodeData::ShadowRoot(_)))
    }

    /// What the shadow root `root` was made with.
    pub fn shadow_init(&self, root: NodeId) -> Option<ShadowRootInit> {
        match self.data(root)? {
            NodeData::ShadowRoot(init) => Some(*init),
            _ => None,
        }
    }

    /// Whether the shadow root `root` is a declarative one no script has
    /// attached since.
    pub fn is_declarative_shadow(&self, root: NodeId) -> bool {
        self.rare.shadows.declarative.contains(&root)
    }

    /// A shadow root's reference target (`referenceTarget`), an ID in its
    /// tree, or none.
    pub fn reference_target(&self, root: NodeId) -> Option<&str> {
        self.rare
            .shadows
            .reference_targets
            .get(&root)
            .map(String::as_str)
    }

    /// Sets a shadow root's reference target, or takes it away; its bytes
    /// count against the data limit.
    pub fn set_reference_target(
        &mut self,
        root: NodeId,
        target: Option<&str>,
    ) -> Result<(), MutationError> {
        if !self.is_shadow_root(root) {
            return Err(MutationError::WrongKind);
        }
        let old = self.reference_target(root).map_or(0, str::len);
        let new = target.map_or(0, str::len);
        self.check_data(new.saturating_sub(old))?;
        self.data_bytes = self.data_bytes + new - old;
        match target {
            Some(target) => {
                let target = target.to_owned();
                self.rare.shadows.reference_targets.insert(root, target);
            }
            None => {
                self.rare.shadows.reference_targets.remove(&root);
            }
        }
        Ok(())
    }

    /// The node's shadow-including parent: its parent, or a shadow
    /// root's host.
    #[inline]
    pub fn host_or_parent(&self, node: NodeId) -> Option<NodeId> {
        self.parent(node).or_else(|| self.shadow_host(node))
    }

    /// The node's shadow-including ancestors (DOM section 4.8), its
    /// parent or host first.
    pub fn shadow_including_ancestors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.host_or_parent(node), |&n| self.host_or_parent(n))
    }

    /// The node's shadow-including root: its root, or that root's host's
    /// shadow-including root.
    pub fn shadow_including_root(&self, node: NodeId) -> NodeId {
        self.shadow_including_ancestors(node).last().unwrap_or(node)
    }

    /// DOM's `getRootNode()`: the node's root, or with `composed` its
    /// shadow-including root.
    pub fn root_node(&self, node: NodeId, composed: bool) -> NodeId {
        match composed {
            true => self.shadow_including_root(node),
            false => self.root_of(node),
        }
    }

    /// Whether `node` is connected (DOM section 4.2): its
    /// shadow-including root is a document, the page's or another.
    pub fn is_connected(&self, node: NodeId) -> bool {
        self.is_document(self.shadow_including_root(node))
    }

    /// Whether `node` is in the page's document, through hosts.
    pub fn in_page(&self, node: NodeId) -> bool {
        self.shadow_including_root(node) == self.root
    }

    /// Whether `ancestor` is a shadow-including inclusive ancestor of
    /// `node`.
    pub fn is_shadow_including_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        self.contains(node)
            && (ancestor == node || self.shadow_including_ancestors(node).any(|a| a == ancestor))
    }

    /// DOM's "retarget" `a` against `b` (section 4.8): `a`, or the host
    /// of the shadow tree it is in, outwards, until one whose root is a
    /// shadow-including inclusive ancestor of `b`.
    pub fn retarget(&self, a: NodeId, b: NodeId) -> NodeId {
        let mut at = a;
        loop {
            let root = self.root_of(at);
            match self.shadow_host(root) {
                Some(host) if !self.is_shadow_including_inclusive_ancestor(root, b) => at = host,
                _ => return at,
            }
        }
    }

    /// The subtree at `id` in shadow-including tree order: each host's
    /// shadow root and its tree right after the host, before its
    /// children. Iterative.
    pub fn shadow_including_subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            if !self.contains(node) {
                continue;
            }
            out.push(node);
            let children: Vec<NodeId> = self.children(node).collect();
            stack.extend(children.into_iter().rev());
            if let Some(root) = self.shadow_root(node) {
                stack.push(root);
            }
        }
        out
    }

    // ---- Slots ------------------------------------------------------------

    /// Whether `node` is a slot: an HTML `slot` element.
    #[inline]
    pub fn is_slot(&self, node: NodeId) -> bool {
        self.tag(node) == Some("slot")
    }

    /// A slot's name: its `name` attribute, or the empty string.
    pub fn slot_name(&self, slot: NodeId) -> &str {
        self.attr(slot, "name").unwrap_or_default()
    }

    /// Whether `node` is a slottable: an element or a text node.
    #[inline]
    pub fn is_slottable(&self, node: NodeId) -> bool {
        matches!(
            self.data(node),
            Some(NodeData::Element { .. } | NodeData::Text(_))
        )
    }

    /// A slottable's name: an element's `slot` attribute, or the empty
    /// string.
    pub fn slottable_name(&self, node: NodeId) -> &str {
        self.attr(node, "slot").unwrap_or_default()
    }

    /// A slot's assigned nodes, in tree order.
    pub fn assigned_nodes(&self, slot: NodeId) -> &[NodeId] {
        if self.rare.shadows.assigned.is_empty() {
            return &[];
        }
        self.rare
            .shadows
            .assigned
            .get(&slot)
            .map_or(&[], Vec::as_slice)
    }

    /// The slot `node` is assigned to.
    pub fn assigned_slot(&self, node: NodeId) -> Option<NodeId> {
        if self.rare.shadows.assigned_slot.is_empty() {
            return None;
        }
        self.rare.shadows.assigned_slot.get(&node).copied()
    }

    /// DOM's "find flattened slottables" of `slot`: its assigned nodes,
    /// or with none its slottable children, each that is a slot in a
    /// shadow tree replaced by its own, in order.
    pub fn flattened_assigned_nodes(&self, slot: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        // Each entry: a slot whose flattened slottables are still to
        // write, and how many of them are written.
        let mut stack: Vec<(Vec<NodeId>, usize)> = Vec::new();
        if !self.is_shadow_root(self.root_of(slot)) {
            return out;
        }
        stack.push((self.slottables_or_fallback(slot), 0));
        while let Some((nodes, at)) = stack.last_mut() {
            let Some(&node) = nodes.get(*at) else {
                stack.pop();
                continue;
            };
            *at += 1;
            if self.is_slot(node)
                && self.is_shadow_root(self.root_of(node))
                && stack.len() <= super::MAX_DEPTH
            {
                let inner = self.slottables_or_fallback(node);
                stack.push((inner, 0));
            } else {
                out.push(node);
            }
        }
        out
    }

    /// A slot's assigned nodes, or with none its slottable children.
    fn slottables_or_fallback(&self, slot: NodeId) -> Vec<NodeId> {
        let assigned = self.assigned_nodes(slot);
        match assigned.is_empty() {
            true => self
                .children(slot)
                .filter(|&c| self.is_slottable(c))
                .collect(),
            false => assigned.to_vec(),
        }
    }

    /// The nodes `assign()` gave `slot`, in the order given.
    pub fn manually_assigned(&self, slot: NodeId) -> &[NodeId] {
        self.rare
            .shadows
            .manual
            .get(&slot)
            .map_or(&[], Vec::as_slice)
    }

    /// The slot's `assign()` (HTML section 4.12.4): its manually assigned
    /// nodes become `nodes`, each once, the first place it is given
    /// winning, each taken from the slot that had it; then the slots of
    /// the trees concerned are assigned again.
    pub fn assign_slot(&mut self, slot: NodeId, nodes: &[NodeId]) -> Result<(), MutationError> {
        if !self.is_slot(slot) {
            return Err(MutationError::WrongKind);
        }
        if nodes.iter().any(|&n| !self.is_slottable(n)) {
            return Err(MutationError::WrongKind);
        }
        let signaled = self.rare.shadows.signals.len();
        let shadows = &mut self.rare.shadows;
        let before = shadows.manual.remove(&slot).unwrap_or_default();
        for old in &before {
            if shadows.manual_slot.get(old) == Some(&slot) {
                shadows.manual_slot.remove(old);
            }
        }
        let mut seen = HashSet::new();
        let mut set = Vec::new();
        let mut others = Vec::new();
        for &node in nodes {
            if !seen.insert(node) {
                continue;
            }
            if let Some(previous) = shadows.manual_slot.insert(node, slot)
                && previous != slot
            {
                if let Some(list) = shadows.manual.get_mut(&previous) {
                    list.retain(|&n| n != node);
                    if list.is_empty() {
                        shadows.manual.remove(&previous);
                    }
                }
                others.push(previous);
            }
            set.push(node);
        }
        // A slot whose manually assigned nodes change is signaled, as the
        // reference browser signals it, whether its assigned nodes change
        // or not; and so is each slot a node was taken from.
        let changed = set != before;
        if !set.is_empty() {
            shadows.manual.insert(slot, set);
        }
        if changed {
            self.signal_slot_change(slot);
        }
        for &other in &others {
            self.signal_slot_change(other);
        }
        let mut trees: Vec<NodeId> = std::iter::once(slot)
            .chain(others)
            .map(|s| self.root_of(s))
            .collect();
        trees.dedup();
        for tree in trees {
            self.reassign(tree);
        }
        // The slots this signaled, in tree order, as the reference
        // browser fires their `slotchange`s.
        let mut tail = self.rare.shadows.signals.split_off(signaled);
        tail.sort_by(|&a, &b| match self.shadow_including_precedes(a, b) {
            true => std::cmp::Ordering::Less,
            false if a == b => std::cmp::Ordering::Equal,
            false => std::cmp::Ordering::Greater,
        });
        self.rare.shadows.signals.extend(tail);
        Ok(())
    }

    /// Whether `a` comes before `b` in shadow-including tree order: a
    /// host's shadow root and its tree before the host's children.
    pub fn shadow_including_precedes(&self, a: NodeId, b: NodeId) -> bool {
        if a == b {
            return false;
        }
        let path = |n: NodeId| {
            let mut path: Vec<NodeId> = std::iter::once(n)
                .chain(self.shadow_including_ancestors(n))
                .collect();
            path.reverse();
            path
        };
        let (pa, pb) = (path(a), path(b));
        let shared = pa.iter().zip(&pb).take_while(|(x, y)| x == y).count();
        match (pa.get(shared), pb.get(shared)) {
            (None, _) => true,
            (_, None) => false,
            (Some(&x), Some(&y)) if self.is_shadow_root(x) => !self.is_shadow_root(y),
            (Some(_), Some(&y)) if self.is_shadow_root(y) => false,
            (Some(&x), Some(&y)) => {
                std::iter::successors(self.next_sibling(x), |&n| self.next_sibling(n))
                    .any(|n| n == y)
            }
        }
    }

    /// The slots signaled since this was last asked (DOM's signal
    /// slots), in order.
    pub fn take_slot_changes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.rare.shadows.signals)
    }

    /// Whether a slot is signaled.
    pub fn has_slot_changes(&self) -> bool {
        !self.rare.shadows.signals.is_empty()
    }

    /// DOM's "signal a slot change": `slot` among the signal slots.
    fn signal_slot_change(&mut self, slot: NodeId) {
        if !self.rare.shadows.signals.contains(&slot) {
            self.rare.shadows.signals.push(slot);
        }
    }

    /// The slots of the shadow tree at `root`, in tree order, kept until
    /// one comes or goes.
    fn slots_of(&mut self, root: NodeId) -> Vec<NodeId> {
        if let Some(slots) = self.rare.shadows.slots.get(&root) {
            return slots.clone();
        }
        let slots: Vec<NodeId> = self.subtree(root).filter(|&n| self.is_slot(n)).collect();
        self.rare.shadows.slots.insert(root, slots.clone());
        slots
    }

    /// DOM's "assign slottables for a tree" of the tree at `root`, when
    /// it is a shadow tree: each of its slots given the host's children
    /// whose slot it is, by name, or the manually assigned ones of them.
    pub(super) fn reassign(&mut self, root: NodeId) {
        let Some(host) = self.shadow_host(root) else {
            return;
        };
        let slots = self.slots_of(root);
        let manual = self
            .shadow_init(root)
            .is_some_and(|init| init.slot_assignment == SlotAssignmentMode::Manual);
        let mut lists: Vec<Vec<NodeId>> = vec![Vec::new(); slots.len()];
        if manual {
            for (list, &slot) in lists.iter_mut().zip(&slots) {
                *list = self
                    .manually_assigned(slot)
                    .iter()
                    .copied()
                    .filter(|&n| self.parent(n) == Some(host))
                    .collect();
            }
        } else if !slots.is_empty() {
            let mut first: HashMap<&str, usize> = HashMap::new();
            for (i, &slot) in slots.iter().enumerate() {
                first.entry(self.slot_name(slot)).or_insert(i);
            }
            for child in self.children(host) {
                if self.is_slottable(child)
                    && let Some(&i) = first.get(self.slottable_name(child))
                {
                    lists[i].push(child);
                }
            }
        }
        for (slot, list) in slots.into_iter().zip(lists) {
            self.set_assigned(slot, list);
        }
    }

    /// Gives `slot` the assigned nodes `list`, signaling it and
    /// journaling its flat children when they differ from what it had.
    fn set_assigned(&mut self, slot: NodeId, list: Vec<NodeId>) {
        let shadows = &mut self.rare.shadows;
        let old = shadows.assigned.get(&slot).map_or(&[][..], Vec::as_slice);
        if old == list.as_slice() {
            return;
        }
        for node in old {
            if shadows.assigned_slot.get(node) == Some(&slot) {
                shadows.assigned_slot.remove(node);
            }
        }
        for &node in &list {
            shadows.assigned_slot.insert(node, slot);
        }
        match list.is_empty() {
            true => shadows.assigned.remove(&slot),
            false => shadows.assigned.insert(slot, list),
        };
        self.signal_slot_change(slot);
        self.record_change(Change::FlatTree { node: slot });
    }

    /// After `node` was inserted under `parent` (DOM's insertion steps
    /// 7.4 to 7.6): a host's new child is given its slot, a slot in a
    /// shadow tree with no assigned nodes is signaled for its fallback
    /// content, and a shadow tree a slot came into is assigned again.
    pub(super) fn shadow_inserted(&mut self, parent: NodeId, node: NodeId) {
        if self.rare.shadows.hosts.is_empty() {
            return;
        }
        if let Some(root) = self.shadow_root(parent)
            && self.is_slottable(node)
        {
            self.slot_inserted(root, node);
        }
        let tree = self.root_of(parent);
        if !self.is_shadow_root(tree) {
            return;
        }
        if self.is_slot(parent) && self.assigned_nodes(parent).is_empty() {
            self.signal_slot_change(parent);
        }
        if self.subtree(node).any(|n| self.is_slot(n)) {
            self.rare.shadows.slots.remove(&tree);
            self.reassign(tree);
        }
    }

    /// The slottable `node`, just inserted under the host of `root`, given
    /// its slot: when it is the host's last child in a tree of named
    /// slots, as a parser's are, its slot's last; else the tree is
    /// assigned again.
    fn slot_inserted(&mut self, root: NodeId, node: NodeId) {
        let named = self
            .shadow_init(root)
            .is_some_and(|init| init.slot_assignment == SlotAssignmentMode::Named);
        if !named || self.next_sibling(node).is_some() {
            self.reassign(root);
            return;
        }
        let slots = self.slots_of(root);
        let name = self.slottable_name(node);
        let Some(slot) = slots.into_iter().find(|&s| self.slot_name(s) == name) else {
            return;
        };
        let shadows = &mut self.rare.shadows;
        shadows.assigned.entry(slot).or_default().push(node);
        shadows.assigned_slot.insert(node, slot);
        self.signal_slot_change(slot);
        self.record_change(Change::FlatTree { node: slot });
    }

    /// After `child` was taken out of `parent` (DOM's removing steps 7 to
    /// 9): its slot assigned again without it, a slot in a shadow tree
    /// with no assigned nodes signaled for its fallback content, and when
    /// slots left a shadow tree, they lose their assigned nodes and the
    /// tree is assigned again.
    pub(super) fn shadow_removed(&mut self, parent: NodeId, child: NodeId) {
        if self.rare.shadows.hosts.is_empty() {
            return;
        }
        if let Some(slot) = self.assigned_slot(child) {
            let list: Vec<NodeId> = self
                .assigned_nodes(slot)
                .iter()
                .copied()
                .filter(|&n| n != child)
                .collect();
            self.set_assigned(slot, list);
        }
        let tree = self.root_of(parent);
        if !self.is_shadow_root(tree) {
            return;
        }
        if self.is_slot(parent) && self.assigned_nodes(parent).is_empty() {
            self.signal_slot_change(parent);
        }
        let gone: Vec<NodeId> = self.subtree(child).filter(|&n| self.is_slot(n)).collect();
        if gone.is_empty() {
            return;
        }
        for slot in gone {
            self.set_assigned(slot, Vec::new());
        }
        self.rare.shadows.slots.remove(&tree);
        self.reassign(tree);
    }

    /// After `element`'s attribute `name` changed (DOM's attribute change
    /// steps for slots and slottables): a slot's `name`, or a host's
    /// child's `slot`, assigns its tree again.
    pub(super) fn shadow_attribute_changed(&mut self, element: NodeId, name: &str) {
        if self.rare.shadows.hosts.is_empty() {
            return;
        }
        match name {
            "slot" => {
                if let Some(root) = self.parent(element).and_then(|p| self.shadow_root(p)) {
                    self.reassign(root);
                }
            }
            "name" if self.is_slot(element) => {
                let tree = self.root_of(element);
                if self.is_shadow_root(tree) {
                    self.rare.shadows.slots.remove(&tree);
                    self.reassign(tree);
                }
            }
            _ => {}
        }
    }

    // ---- The flat tree ------------------------------------------------------

    /// The node's children in the flat tree: a host's shadow root's
    /// children, a slot's assigned nodes when it has some, else its own
    /// children.
    #[inline]
    pub fn flat_children(&self, node: NodeId) -> FlatChildren<'_> {
        if let Some(root) = self.shadow_root(node) {
            return FlatChildren::Tree(self.children(root));
        }
        let assigned = self.assigned_nodes(node);
        match assigned.is_empty() {
            true => FlatChildren::Tree(self.children(node)),
            false => FlatChildren::Assigned(assigned.iter()),
        }
    }

    /// The node's parent in the flat tree: its assigned slot, a shadow
    /// tree's top's host, or its parent; `None` for a host's child no
    /// slot has and a slot's fallback content while the slot has
    /// assigned nodes, which are not in it, and for a shadow root.
    #[inline]
    pub fn flat_parent(&self, node: NodeId) -> Option<NodeId> {
        if self.rare.shadows.hosts.is_empty() {
            return self.parent(node);
        }
        if let Some(slot) = self.assigned_slot(node) {
            return Some(slot);
        }
        let parent = self.parent(node)?;
        if let Some(host) = self.shadow_host(parent) {
            return Some(host);
        }
        if self.shadow_root(parent).is_some() || !self.assigned_nodes(parent).is_empty() {
            return None;
        }
        Some(parent)
    }

    /// The node's parent in the flat tree when that is an element.
    #[inline]
    pub fn flat_parent_element(&self, node: NodeId) -> Option<NodeId> {
        self.flat_parent(node).filter(|&p| self.is_element(p))
    }

    /// The node's ancestors in the flat tree, its parent first.
    pub fn flat_ancestors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.flat_parent(node), |&n| self.flat_parent(n))
    }

    /// The subtree at `node` in the flat tree, in its order, `node`
    /// first. Iterative.
    pub fn flat_subtree(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            if !self.contains(at) {
                continue;
            }
            out.push(at);
            let children: Vec<NodeId> = self.flat_children(at).collect();
            stack.extend(children.into_iter().rev());
        }
        out
    }

    /// Whether `node` is in the flat tree of the page: its flat
    /// ancestors end at the page's document.
    pub fn in_flat_tree(&self, node: NodeId) -> bool {
        node == self.root || self.flat_ancestors(node).last() == Some(self.root)
    }

    // ---- Bookkeeping -------------------------------------------------------

    /// The elements whose focus flag the focus of `element` sets: it, and
    /// each host of a shadow tree that holds it (HTML's "focused area"
    /// of a shadow host); and those its focus-within flag reaches: it and
    /// its ancestors in the flat tree, through a node not in it by its
    /// shadow-including parent.
    pub(super) fn focus_chains(&self, element: NodeId) -> (Vec<NodeId>, Vec<NodeId>) {
        let mut hosts = Vec::new();
        let mut tree = self.root_of(element);
        while let Some(host) = self.shadow_host(tree) {
            hosts.push(host);
            tree = self.root_of(host);
        }
        let within: Vec<NodeId> = std::iter::successors(Some(element), |&n| {
            self.flat_parent(n).or_else(|| self.host_or_parent(n))
        })
        .filter(|&n| self.is_element(n))
        .collect();
        (hosts, within)
    }

    /// Forgets what the shadow roots' maps say of the nodes `doomed`,
    /// which are being freed.
    pub(super) fn forget_shadows(&mut self, doomed: &[NodeId]) {
        let shadows = &mut self.rare.shadows;
        if shadows.hosts.is_empty() && shadows.manual.is_empty() {
            return;
        }
        let gone: HashSet<NodeId> = doomed.iter().copied().collect();
        let mut freed = 0;
        for node in doomed {
            if let Some(target) = shadows.reference_targets.remove(node) {
                freed += target.len();
            }
            shadows.roots.remove(node);
            shadows.hosts.remove(node);
            shadows.declarative.remove(node);
            shadows.slots.remove(node);
            shadows.assigned.remove(node);
            shadows.assigned_slot.remove(node);
            shadows.manual.remove(node);
            shadows.manual_slot.remove(node);
        }
        shadows.manual.retain(|_, list| {
            list.retain(|n| !gone.contains(n));
            !list.is_empty()
        });
        shadows.signals.retain(|n| !gone.contains(n));
        shadows.focused_hosts.retain(|n| !gone.contains(n));
        shadows.focus_within.retain(|n| !gone.contains(n));
        self.data_bytes -= freed;
    }

    /// The bytes the shadow roots' reference targets hold, for
    /// [`Document::validate`].
    pub(super) fn shadow_data_bytes(&self) -> usize {
        self.rare
            .shadows
            .reference_targets
            .values()
            .map(String::len)
            .sum()
    }

    /// Checks the shadow roots' maps, for [`Document::validate`]: each
    /// host an element whose root is a parentless shadow root linked
    /// back, and each assignment mirrored and of a host's child.
    pub(super) fn validate_shadows(&self) -> Result<(), String> {
        let shadows = &self.rare.shadows;
        if shadows.roots.len() != shadows.hosts.len() {
            return Err("a shadow root's host is not mirrored".into());
        }
        for (&host, &root) in &shadows.roots {
            if !self.is_element(host)
                || !self.is_shadow_root(root)
                || self.parent(root).is_some()
                || shadows.hosts.get(&root) != Some(&host)
            {
                return Err(format!("{host}'s shadow root {root} is wrong"));
            }
        }
        for (&slot, nodes) in &shadows.assigned {
            let host = self.shadow_host(self.root_of(slot));
            for node in nodes {
                if shadows.assigned_slot.get(node) != Some(&slot)
                    || host.is_none()
                    || self.parent(*node) != host
                {
                    return Err(format!("{node}'s assignment to {slot} is wrong"));
                }
            }
        }
        if shadows.assigned_slot.len() != shadows.assigned.values().map(Vec::len).sum::<usize>() {
            return Err("an assigned slot is not mirrored".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page with a `div` host in its body, its shadow root holding
    /// `markup`'s elements as given: each a tag and a `name` or `slot`.
    fn page() -> (Document, NodeId) {
        let mut d = Document::new();
        let root = d.root();
        let html = d.create_element("html").unwrap();
        d.append_child(root, html).unwrap();
        let host = d.create_element("div").unwrap();
        d.append_child(html, host).unwrap();
        (d, host)
    }

    fn element(d: &mut Document, parent: NodeId, tag: &str, attr: Option<(&str, &str)>) -> NodeId {
        let e = d.create_element(tag).unwrap();
        if let Some((name, value)) = attr {
            d.set_attribute(e, name, value).unwrap();
        }
        d.append_child(parent, e).unwrap();
        e
    }

    #[test]
    fn a_shadow_root_is_a_parentless_node_linked_to_its_host() {
        let (mut d, host) = page();
        d.take_changes();
        let root = d
            .attach_shadow(host, ShadowRootInit::open(), false)
            .unwrap();
        assert_eq!(d.take_changes(), [Change::FlatTree { node: host }]);
        assert_eq!(d.parent(root), None);
        assert_eq!(d.shadow_root(host), Some(root));
        assert_eq!(d.shadow_host(root), Some(host));
        assert_eq!(d.node_type(root), Some(11));
        // A second root is refused, and so is a host of another name.
        assert_eq!(
            d.attach_shadow(host, ShadowRootInit::open(), false),
            Err(MutationError::NotSupported)
        );
        let img = d.create_element("img").unwrap();
        assert_eq!(
            d.attach_shadow(img, ShadowRootInit::open(), false),
            Err(MutationError::NotSupported)
        );
        let custom = d.create_element("my-widget").unwrap();
        assert!(
            d.attach_shadow(custom, ShadowRootInit::open(), false)
                .is_ok()
        );
        let svg = d
            .create_element_ns(Namespace::Svg, "div", Vec::new())
            .unwrap();
        assert!(!d.may_host_shadow(svg));
        // Its tree is connected through its host, owned by its document,
        // and no part of the document's tree.
        let p = element(&mut d, root, "p", None);
        assert!(d.is_connected(p) && d.in_page(p));
        assert_eq!(d.root_node(p, false), root);
        assert_eq!(d.root_node(p, true), d.root());
        assert_eq!(d.owner_document(p), d.root());
        assert!(!d.subtree(d.root()).any(|n| n == p));
        assert!(d.shadow_including_subtree(d.root()).contains(&p));
        assert!(d.is_shadow_including_inclusive_ancestor(host, p));
        assert!(!d.is_inclusive_ancestor(host, p));
        // The host cannot go into its own shadow tree, nor the root be
        // adopted or freed while it is the host's.
        assert_eq!(
            d.append_child(p, host),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(
            d.adopt(root, d.root()),
            Err(MutationError::HierarchyRequest)
        );
        assert_eq!(d.free(root), Err(MutationError::Attached));
        d.validate().unwrap();
        // Freed with its host.
        let html = d.parent(host).unwrap();
        d.remove_child(html, host).unwrap();
        assert!(!d.is_connected(p));
        d.free(host).unwrap();
        assert!(!d.contains(root) && !d.contains(p));
        assert!(!d.has_shadow_roots() || d.shadow_root(custom).is_some());
        d.validate().unwrap();
    }

    #[test]
    fn a_declarative_root_is_emptied_and_kept_by_an_attach_of_its_mode() {
        let (mut d, host) = page();
        let root = d.attach_shadow(host, ShadowRootInit::open(), true).unwrap();
        assert!(d.is_declarative_shadow(root));
        let p = element(&mut d, root, "p", None);
        let mut closed = ShadowRootInit::open();
        closed.mode = ShadowRootMode::Closed;
        assert_eq!(
            d.attach_shadow(host, closed, false),
            Err(MutationError::NotSupported)
        );
        assert_eq!(
            d.attach_shadow(host, ShadowRootInit::open(), false),
            Ok(root)
        );
        assert_eq!(d.first_child(root), None);
        assert_eq!(d.parent(p), None);
        assert!(!d.is_declarative_shadow(root));
        assert_eq!(
            d.attach_shadow(host, ShadowRootInit::open(), false),
            Err(MutationError::NotSupported)
        );
    }

    #[test]
    fn named_slots_take_the_hosts_children_by_name_in_tree_order() {
        let (mut d, host) = page();
        let a = element(&mut d, host, "span", Some(("slot", "a")));
        let text = d.create_text("t").unwrap();
        d.append_child(host, text).unwrap();
        let other = element(&mut d, host, "i", Some(("slot", "none")));
        let root = d
            .attach_shadow(host, ShadowRootInit::open(), false)
            .unwrap();
        // No slot yet: nothing is assigned, and nothing is in the flat
        // tree but the shadow tree.
        assert_eq!(d.assigned_slot(a), None);
        let wrapper = element(&mut d, root, "div", None);
        d.take_slot_changes();
        let default = element(&mut d, wrapper, "slot", None);
        let named = element(&mut d, root, "slot", Some(("name", "a")));
        assert_eq!(d.assigned_nodes(default), [text]);
        assert_eq!(d.assigned_nodes(named), [a]);
        assert_eq!(d.assigned_slot(other), None);
        assert_eq!(d.take_slot_changes(), [default, named]);
        // The flat tree: the host's shadow tree, the slots' assigned
        // nodes in their places.
        assert_eq!(d.flat_children(host).collect::<Vec<_>>(), [wrapper, named]);
        assert_eq!(d.flat_children(default).collect::<Vec<_>>(), [text]);
        assert_eq!(d.flat_parent(a), Some(named));
        assert_eq!(d.flat_parent(wrapper), Some(host));
        assert_eq!(d.flat_parent(other), None);
        assert!(d.in_flat_tree(text) && !d.in_flat_tree(other));
        // A second slot of the same name takes nothing; the first going
        // gives it the nodes.
        let second = element(&mut d, root, "slot", Some(("name", "a")));
        assert!(d.assigned_nodes(second).is_empty());
        d.take_changes();
        d.remove_child(root, named).unwrap();
        assert_eq!(d.assigned_nodes(second), [a]);
        assert!(d.assigned_nodes(named).is_empty());
        assert!(d.changes().contains(&Change::FlatTree { node: second }));
        assert_eq!(d.take_slot_changes(), [named, second]);
        // A child's `slot` and a slot's `name` assign again.
        d.set_attribute(other, "slot", "a").unwrap();
        assert_eq!(d.assigned_nodes(second), [a, other]);
        d.set_attribute(second, "name", "none").unwrap();
        assert!(d.assigned_nodes(second).is_empty());
        assert_eq!(d.assigned_slot(a), None);
        // A child taken out leaves its slot; one appended joins it last.
        d.remove_child(host, text).unwrap();
        assert!(d.assigned_nodes(default).is_empty());
        d.take_slot_changes();
        d.append_child(host, text).unwrap();
        assert_eq!(d.assigned_nodes(default), [text]);
        assert_eq!(d.take_slot_changes(), [default]);
        // Fallback content changing in a slot with none is signaled.
        let empty = element(&mut d, root, "slot", Some(("name", "x")));
        d.take_slot_changes();
        element(&mut d, empty, "b", None);
        assert_eq!(d.take_slot_changes(), [empty]);
        d.validate().unwrap();
    }

    #[test]
    fn manual_slots_take_the_nodes_assigned_while_they_are_the_hosts() {
        let (mut d, host) = page();
        let mut init = ShadowRootInit::open();
        init.slot_assignment = SlotAssignmentMode::Manual;
        let root = d.attach_shadow(host, init, false).unwrap();
        let c1 = element(&mut d, host, "span", Some(("slot", "s")));
        let c2 = element(&mut d, host, "span", None);
        let s1 = element(&mut d, root, "slot", Some(("name", "s")));
        let s2 = element(&mut d, root, "slot", None);
        assert!(d.assigned_nodes(s1).is_empty(), "names do not count");
        let elsewhere = d.create_element("p").unwrap();
        d.assign_slot(s1, &[c2, elsewhere, c1, c2]).unwrap();
        assert_eq!(d.manually_assigned(s1), [c2, elsewhere, c1]);
        assert_eq!(d.assigned_nodes(s1), [c2, c1], "the host's alone");
        // Assigned to another slot, a node leaves the first.
        d.assign_slot(s2, &[c1]).unwrap();
        assert_eq!(d.assigned_nodes(s1), [c2]);
        assert_eq!(d.assigned_nodes(s2), [c1]);
        // Out of the host and back, the assignment holds.
        d.remove_child(host, c1).unwrap();
        assert!(d.assigned_nodes(s2).is_empty());
        d.append_child(host, c1).unwrap();
        assert_eq!(d.assigned_nodes(s2), [c1]);
        // A slot out of the tree loses its nodes, and has them back in it.
        d.remove_child(root, s2).unwrap();
        assert_eq!(d.assigned_slot(c1), None);
        d.append_child(root, s2).unwrap();
        assert_eq!(d.assigned_slot(c1), Some(s2));
        d.validate().unwrap();
        // Freed, a node leaves every list.
        d.remove_child(host, c2).unwrap();
        d.free(c2).unwrap();
        assert_eq!(d.manually_assigned(s1), [elsewhere]);
        d.validate().unwrap();
    }

    #[test]
    fn flattened_slottables_go_through_nested_slots_and_fallback() {
        let (mut d, outer) = page();
        let outer_root = d
            .attach_shadow(outer, ShadowRootInit::open(), false)
            .unwrap();
        let light = element(&mut d, outer, "b", None);
        let inner = element(&mut d, outer_root, "div", None);
        let inner_root = d
            .attach_shadow(inner, ShadowRootInit::open(), false)
            .unwrap();
        let middle = element(&mut d, inner, "slot", None);
        let deepest = element(&mut d, inner_root, "slot", None);
        let fallback = element(&mut d, deepest, "i", None);
        assert_eq!(d.assigned_nodes(deepest), [middle]);
        assert_eq!(d.flattened_assigned_nodes(deepest), [light]);
        // With nothing in the outer host, the middle slot's fallback.
        d.remove_child(outer, light).unwrap();
        assert!(d.flattened_assigned_nodes(deepest).is_empty());
        let middle_fallback = element(&mut d, middle, "u", None);
        assert_eq!(d.flattened_assigned_nodes(deepest), [middle_fallback]);
        d.remove_child(inner, middle).unwrap();
        assert_eq!(d.flattened_assigned_nodes(deepest), [fallback]);
        // A slot outside a shadow tree flattens to nothing.
        assert!(d.flattened_assigned_nodes(middle).is_empty());
    }

    #[test]
    fn retargeting_goes_out_to_the_tree_of_the_other_node() {
        let (mut d, host) = page();
        let root = d
            .attach_shadow(host, ShadowRootInit::open(), false)
            .unwrap();
        let inner = element(&mut d, root, "span", None);
        let html = d.parent(host).unwrap();
        assert_eq!(d.retarget(inner, html), host);
        assert_eq!(d.retarget(inner, inner), inner);
        assert_eq!(d.retarget(html, inner), html);
    }

    #[test]
    fn the_focus_marks_the_hosts_around_it_and_the_flat_ancestors() {
        let (mut d, host) = page();
        let root = d
            .attach_shadow(host, ShadowRootInit::open(), false)
            .unwrap();
        let slot = element(&mut d, root, "slot", None);
        let input = element(&mut d, root, "input", None);
        let light = element(&mut d, host, "button", None);
        d.set_focus(Some(input), false);
        assert!(d.state(input).contains(ElementState::FOCUS));
        assert!(d.state(host).contains(ElementState::FOCUS));
        assert!(d.state(host).contains(ElementState::FOCUS_WITHIN));
        // A slotted element's focus is its own and reaches the slot.
        d.set_focus(Some(light), false);
        assert!(!d.state(host).contains(ElementState::FOCUS));
        assert!(!d.state(input).contains(ElementState::FOCUS));
        assert!(d.state(slot).contains(ElementState::FOCUS_WITHIN));
        assert!(d.state(host).contains(ElementState::FOCUS_WITHIN));
        // A host taken out takes the focus in its tree with it.
        d.set_focus(Some(input), false);
        let html = d.parent(host).unwrap();
        d.remove_child(html, host).unwrap();
        assert_eq!(d.focused(), None);
        assert!(!d.state(host).contains(ElementState::FOCUS));
    }

    #[test]
    fn custom_element_names_are_checked_as_html_says() {
        assert!(is_valid_custom_element_name("my-element"));
        assert!(is_valid_custom_element_name("a-\u{B7}"));
        assert!(!is_valid_custom_element_name("element"));
        assert!(!is_valid_custom_element_name("My-element"));
        assert!(!is_valid_custom_element_name("-x"));
        assert!(!is_valid_custom_element_name("font-face"));
        assert!(!is_valid_custom_element_name("a-B"));
        // What a valid element local name takes, the reference browser
        // takes in a custom element's name.
        assert!(is_valid_custom_element_name("a-!"));
        assert!(is_valid_custom_element_name("a-\u{1F171}"));
        assert!(!is_valid_custom_element_name("a- b"));
        assert!(!is_valid_custom_element_name("a-/"));
        assert!(!is_valid_custom_element_name("a->"));
    }

    use super::super::ElementState;
}
