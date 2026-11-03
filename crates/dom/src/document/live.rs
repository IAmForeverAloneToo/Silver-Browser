//! Live ranges and node iterators (gap batch D, D59): boundary points
//! the document keeps and moves with every insertion, removal, change of
//! data and split, as DOM sections 4.2.3, 4.2.4, 4.10, 5.5 and 6.1 say;
//! and what reads and changes character data by the DOM's UTF-16
//! offsets. A range let go of by its script object is dropped
//! ([`Document::drop_range`]); at most [`MAX_RANGES`] live at once, and
//! [`MAX_ITERATORS`] node iterators.
//!
//! The ranges a script let go of are dropped when its realm finalizes
//! their objects, which a long script's run may not do before it ends,
//! so a document may hold thousands: each node's boundary points are
//! kept by the node, so that a change touches only those of the nodes it
//! changes, and a removal finds those under the node removed without
//! walking up from every boundary point.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use super::{Document, MutationError, NodeData, NodeId};

/// The most live ranges a document keeps at once.
pub const MAX_RANGES: usize = 1 << 12;

/// The most node iterators a document keeps at once.
pub const MAX_ITERATORS: usize = 1 << 12;

/// A boundary point: a node and an offset in it, a UTF-16 code unit
/// for character data, a child index for another node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    pub node: NodeId,
    pub offset: u32,
}

/// A live range's two boundary points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRange {
    pub start: Boundary,
    pub end: Boundary,
}

/// A node iterator's root, reference node and whether its pointer is
/// before the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IteratorState {
    pub root: NodeId,
    pub reference: NodeId,
    pub before: bool,
}

/// The live ranges and iterators, by the ids given out; and the rest of
/// what fewer documents use.
#[derive(Debug, Clone, Default)]
pub(super) struct Live {
    ranges: Vec<Option<LiveRange>>,
    free_ranges: Vec<u32>,
    /// The boundary points each node holds: each its range's id and
    /// whether it is the range's end.
    holding: HashMap<NodeId, Vec<(u32, bool)>>,
    iterators: Vec<Option<IteratorState>>,
    free_iterators: Vec<u32>,
    count: usize,
    /// A node is being moved (`moveBefore`): its removal leaves the
    /// focus.
    pub(super) moving: bool,
    pub(super) templates: super::Templates,
    /// An XML document (DOM's type "xml"), an XML parser's: its HTML
    /// elements' names and attributes keep their case.
    pub(super) xml: bool,
    /// The documents beside the page's (D67).
    pub(super) documents: super::documents::Documents,
    /// The namespaces of the attributes of the elements whose attributes
    /// are not all in those their names imply, one an attribute.
    pub(super) attr_ns: std::collections::HashMap<NodeId, Vec<Option<Box<str>>>>,
    /// The elements whose qualified names have a prefix: those
    /// `createElementNS` and the XML parser made of a name with a colon,
    /// which another's (`createElement`'s, the HTML parser's) is part of
    /// its local name.
    pub(super) prefixed: std::collections::HashSet<NodeId>,
    /// The text nodes that are CDATA sections (an XML parser's, or
    /// `createCDATASection`'s): text to everything after the DOM, with
    /// their own node type and markup.
    pub(super) cdata: std::collections::HashSet<NodeId>,
    /// The shadow roots and their slots' assignments (D68).
    pub(super) shadows: super::shadow::Shadows,
    /// The top layer, the showing popovers, the open dialogs and the
    /// close watchers (gap batch H).
    pub(super) top_layer: super::top_layer::TopLayer,
    /// The elements scripts set for element-reflecting attributes, by
    /// element and attribute (gap batch H).
    pub(super) references: HashMap<(NodeId, Box<str>), Vec<NodeId>>,
    /// The `details` elements' toggles owed (gap batch H).
    pub(super) details: super::details::DetailsState,
    /// The custom elements' states, definitions and reactions (D70).
    pub(super) custom: super::custom::Custom,
    /// The form owners the parser's form element pointer gave controls it
    /// inserted outside their forms (HTML section 13.2.6.1, "create an
    /// element for a token" step 12), until their ancestors change.
    pub(super) parser_forms: HashMap<NodeId, NodeId>,
}

impl Live {
    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Range `id`'s start, or its end, is now in `node`.
    fn hold(&mut self, node: NodeId, point: (u32, bool)) {
        self.holding.entry(node).or_default().push(point);
    }

    /// Range `id`'s start, or its end, is no longer in `node`.
    fn unhold(&mut self, node: NodeId, point: (u32, bool)) {
        if let Some(points) = self.holding.get_mut(&node) {
            if let Some(at) = points.iter().position(|&p| p == point) {
                points.swap_remove(at);
            }
            if points.is_empty() {
                self.holding.remove(&node);
            }
        }
    }
}

/// The byte index in `text` of the UTF-16 offset `offset`, a pair's
/// middle taken as its start; the end for one past it.
fn byte_at(text: &str, offset: u32) -> usize {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        let next = units + c.len_utf16() as u32;
        if next > offset {
            return at;
        }
        units = next;
    }
    text.len()
}

/// `text`'s length in UTF-16 code units.
fn units(text: &str) -> u32 {
    text.chars().map(|c| c.len_utf16() as u32).sum()
}

impl Document {
    /// A node's length (DOM section 4.2): a doctype's 0, character
    /// data's in UTF-16 code units, another's its children.
    pub fn node_length(&self, node: NodeId) -> u32 {
        match self.data(node) {
            Some(NodeData::DocumentType { .. }) | None => 0,
            Some(data) => match data.character_data() {
                Some(text) => units(text),
                None => self.children(node).count() as u32,
            },
        }
    }

    /// The child index of `node` in its parent; 0 for a parentless one.
    pub fn index_of(&self, node: NodeId) -> u32 {
        std::iter::successors(self.prev_sibling(node), |&n| self.prev_sibling(n)).count() as u32
    }

    /// The root of `node`'s tree: its topmost ancestor, or itself.
    pub fn root_of(&self, node: NodeId) -> NodeId {
        self.ancestors(node).last().unwrap_or(node)
    }

    /// The UTF-16 code units `offset` to `offset + count` of character
    /// data, as `substringData` reads them; `None` past its length.
    pub fn substring_data(&self, node: NodeId, offset: u32, count: u32) -> Option<String> {
        let text = self.character_data(node)?;
        if offset > units(text) {
            return None;
        }
        let start = byte_at(text, offset);
        let end = byte_at(text, offset.saturating_add(count));
        Some(text[start..end].to_owned())
    }

    /// The DOM's "replace data" (section 4.10): `count` UTF-16 code units
    /// of character data from `offset` replaced by `data`, the live
    /// ranges in it moved.
    pub fn replace_data(
        &mut self,
        node: NodeId,
        offset: u32,
        count: u32,
        data: &str,
    ) -> Result<(), MutationError> {
        let text = self.character_data(node).ok_or(MutationError::WrongKind)?;
        let length = units(text);
        if offset > length {
            return Err(MutationError::IndexSize);
        }
        let count = count.min(length - offset);
        let start = byte_at(text, offset);
        let end = byte_at(text, offset + count);
        let mut new = String::with_capacity(text.len() - (end - start) + data.len());
        new.push_str(&text[..start]);
        new.push_str(data);
        new.push_str(&text[end..]);
        self.store_data(node, &new)?;
        let added = units(data);
        self.each_boundary(&[node], |b| {
            if b.node != node {
                return;
            }
            if b.offset > offset && b.offset <= offset + count {
                b.offset = offset;
            } else if b.offset > offset + count {
                b.offset = b.offset + added - count;
            }
        });
        Ok(())
    }

    /// The DOM's "split a Text node" (section 4.11): the data after
    /// `offset` moved into a new text node after it, the live ranges
    /// with it.
    pub fn split_text(&mut self, node: NodeId, offset: u32) -> Result<NodeId, MutationError> {
        if !self.is_text(node) {
            return Err(MutationError::WrongKind);
        }
        let length = self.node_length(node);
        if offset > length {
            return Err(MutationError::IndexSize);
        }
        let tail = self
            .substring_data(node, offset, length - offset)
            .unwrap_or_default();
        let made = self.create_text(&tail)?;
        if let Some(parent) = self.parent(node) {
            let index = self.index_of(node);
            self.insert_before(parent, made, self.next_sibling(node))?;
            self.each_boundary(&[node, parent], |b| {
                if b.node == node && b.offset > offset {
                    *b = Boundary {
                        node: made,
                        offset: b.offset - offset,
                    };
                } else if b.node == parent && b.offset == index + 1 {
                    b.offset += 1;
                }
            });
        }
        self.replace_data(node, offset, length - offset, "")?;
        Ok(made)
    }

    /// The DOM's `normalize()` (section 4.4): the empty text nodes under
    /// `node` removed and each run of adjacent ones joined into its
    /// first, the live ranges kept where their text went.
    pub fn normalize(&mut self, node: NodeId) -> Result<(), MutationError> {
        let texts: Vec<NodeId> = self.subtree(node).filter(|&n| self.is_text(n)).collect();
        for text in texts {
            if !self.contains(text) || self.parent(text).is_none() {
                continue;
            }
            // Only the first of a run of text nodes gathers the rest.
            if self.prev_sibling(text).is_some_and(|p| self.is_text(p)) {
                continue;
            }
            let mut length = self.node_length(text);
            if length == 0 {
                let parent = self.parent(text).ok_or(MutationError::NoSuchNode)?;
                self.remove_child(parent, text)?;
                continue;
            }
            let parent = self.parent(text).ok_or(MutationError::NoSuchNode)?;
            while let Some(next) = self.next_sibling(text).filter(|&n| self.is_text(n)) {
                let data = self.character_data(next).unwrap_or_default().to_owned();
                self.replace_data(text, length, 0, &data)?;
                let index = self.index_of(next);
                self.each_boundary(&[next, parent], |b| {
                    if b.node == next {
                        *b = Boundary {
                            node: text,
                            offset: b.offset + length,
                        };
                    } else if b.node == parent && b.offset == index {
                        *b = Boundary {
                            node: text,
                            offset: length,
                        };
                    }
                });
                length += units(&data);
                self.remove_child(parent, next)?;
            }
        }
        Ok(())
    }

    // ---- Comparing ------------------------------------------------------

    /// Where boundary point `a` is against `b` (DOM section 5.2); `None`
    /// when they are in different trees.
    pub fn compare_boundaries(&self, a: Boundary, b: Boundary) -> Option<Ordering> {
        if self.root_of(a.node) != self.root_of(b.node) {
            return None;
        }
        if a.node == b.node {
            return Some(a.offset.cmp(&b.offset));
        }
        if self.precedes(b.node, a.node) {
            return self.compare_boundaries(b, a).map(Ordering::reverse);
        }
        // `a`'s node is first: if it holds `b`'s, compare by the child of
        // `a`'s node on the way to `b`'s.
        if self.is_inclusive_ancestor(a.node, b.node) {
            let mut child = b.node;
            while let Some(parent) = self.parent(child)
                && parent != a.node
            {
                child = parent;
            }
            if self.index_of(child) < a.offset {
                return Some(Ordering::Greater);
            }
        }
        Some(Ordering::Less)
    }

    /// Whether `a` comes before `b` in tree order, in one tree.
    pub fn precedes(&self, a: NodeId, b: NodeId) -> bool {
        if a == b {
            return false;
        }
        let path = |n: NodeId| {
            let mut path: Vec<NodeId> = std::iter::once(n).chain(self.ancestors(n)).collect();
            path.reverse();
            path
        };
        let (pa, pb) = (path(a), path(b));
        let shared = pa.iter().zip(&pb).take_while(|(x, y)| x == y).count();
        match (pa.get(shared), pb.get(shared)) {
            // One holds the other: the ancestor is first.
            (None, _) => true,
            (_, None) => false,
            (Some(&x), Some(&y)) => {
                std::iter::successors(self.next_sibling(x), |&n| self.next_sibling(n))
                    .any(|n| n == y)
            }
        }
    }

    /// The DOM's `compareDocumentPosition` bits (section 4.4) of `other`
    /// against `node`, attributes aside.
    pub fn compare_position(&self, node: NodeId, other: NodeId) -> u16 {
        const DISCONNECTED: u16 = 1;
        const PRECEDING: u16 = 2;
        const FOLLOWING: u16 = 4;
        const CONTAINS: u16 = 8;
        const CONTAINED_BY: u16 = 16;
        const IMPLEMENTATION_SPECIFIC: u16 = 32;
        if node == other {
            return 0;
        }
        if self.root_of(node) != self.root_of(other) {
            // A consistent order, by slot.
            let order = if other.index() < node.index() {
                PRECEDING
            } else {
                FOLLOWING
            };
            return DISCONNECTED | IMPLEMENTATION_SPECIFIC | order;
        }
        if self.is_inclusive_ancestor(other, node) {
            return CONTAINS | PRECEDING;
        }
        if self.is_inclusive_ancestor(node, other) {
            return CONTAINED_BY | FOLLOWING;
        }
        if self.precedes(other, node) {
            PRECEDING
        } else {
            FOLLOWING
        }
    }

    /// The DOM's "equals" (section 4.4): the same kind, names, data and
    /// attributes, and equal children in order.
    pub fn nodes_equal(&self, a: NodeId, b: NodeId) -> bool {
        let mut pending = vec![(a, b)];
        while let Some((a, b)) = pending.pop() {
            let same = match (self.data(a), self.data(b)) {
                (
                    Some(NodeData::Element { tag, attrs, ns }),
                    Some(NodeData::Element {
                        tag: tag2,
                        attrs: attrs2,
                        ns: ns2,
                    }),
                ) => {
                    // Each attribute has one of the same namespace, local
                    // name and value, whatever its prefix.
                    tag == tag2
                        && ns == ns2
                        && attrs.len() == attrs2.len()
                        && attrs.iter().enumerate().all(|(i, (name, value))| {
                            let namespace = self.attribute_namespace(a, i);
                            let local = super::attributes::attribute_local_name(namespace, name);
                            self.find_attribute_ns(b, namespace, local)
                                .is_some_and(|j| attrs2[j].1 == *value)
                        })
                }
                (Some(x), Some(y)) => x == y,
                _ => false,
            };
            if !same {
                return false;
            }
            let (ka, kb): (Vec<NodeId>, Vec<NodeId>) =
                (self.children(a).collect(), self.children(b).collect());
            if ka.len() != kb.len() {
                return false;
            }
            pending.extend(ka.into_iter().zip(kb));
        }
        true
    }

    // ---- Live ranges and iterators ----------------------------------------

    /// A new live range, collapsed at the start of `node`.
    pub fn create_range(&mut self, node: NodeId) -> Result<u32, MutationError> {
        if !self.contains(node) {
            return Err(MutationError::NoSuchNode);
        }
        let start = Boundary { node, offset: 0 };
        let range = Some(LiveRange { start, end: start });
        let live = &mut self.rare;
        if live.ranges.len() - live.free_ranges.len() >= MAX_RANGES {
            return Err(MutationError::TooManyNodes);
        }
        live.count += 1;
        let id = match live.free_ranges.pop() {
            Some(id) => {
                live.ranges[id as usize] = range;
                id
            }
            None => {
                live.ranges.push(range);
                live.ranges.len() as u32 - 1
            }
        };
        live.hold(node, (id, false));
        live.hold(node, (id, true));
        Ok(id)
    }

    /// Lets go of range `id`.
    pub fn drop_range(&mut self, id: u32) {
        if let Some(slot) = self.rare.ranges.get_mut(id as usize)
            && let Some(range) = slot.take()
        {
            self.rare.free_ranges.push(id);
            self.rare.count -= 1;
            self.rare.unhold(range.start.node, (id, false));
            self.rare.unhold(range.end.node, (id, true));
        }
    }

    /// Range `id`'s boundary points.
    pub fn range(&self, id: u32) -> Option<LiveRange> {
        self.rare.ranges.get(id as usize).copied().flatten()
    }

    /// Sets range `id`'s boundary points, as they are: the caller has
    /// checked them (DOM section 5.5's "set the start or end").
    pub fn set_range(&mut self, id: u32, range: LiveRange) -> Result<(), MutationError> {
        if !self.contains(range.start.node) || !self.contains(range.end.node) {
            return Err(MutationError::NoSuchNode);
        }
        match self.rare.ranges.get_mut(id as usize) {
            Some(Some(slot)) => {
                let old = std::mem::replace(slot, range);
                let live = &mut self.rare;
                live.unhold(old.start.node, (id, false));
                live.unhold(old.end.node, (id, true));
                live.hold(range.start.node, (id, false));
                live.hold(range.end.node, (id, true));
                Ok(())
            }
            _ => Err(MutationError::NoSuchNode),
        }
    }

    /// A new node iterator over `root`'s subtree, its pointer before it.
    pub fn create_iterator(&mut self, root: NodeId) -> Result<u32, MutationError> {
        if !self.contains(root) {
            return Err(MutationError::NoSuchNode);
        }
        let state = Some(IteratorState {
            root,
            reference: root,
            before: true,
        });
        let live = &mut self.rare;
        if live.iterators.len() - live.free_iterators.len() >= MAX_ITERATORS {
            return Err(MutationError::TooManyNodes);
        }
        live.count += 1;
        Ok(match live.free_iterators.pop() {
            Some(id) => {
                live.iterators[id as usize] = state;
                id
            }
            None => {
                live.iterators.push(state);
                live.iterators.len() as u32 - 1
            }
        })
    }

    /// Lets go of iterator `id`.
    pub fn drop_iterator(&mut self, id: u32) {
        if let Some(slot) = self.rare.iterators.get_mut(id as usize)
            && slot.take().is_some()
        {
            self.rare.free_iterators.push(id);
            self.rare.count -= 1;
        }
    }

    /// Iterator `id`'s state.
    pub fn iterator(&self, id: u32) -> Option<IteratorState> {
        self.rare.iterators.get(id as usize).copied().flatten()
    }

    /// Moves iterator `id` to `reference`, its pointer before it or not.
    pub fn set_iterator(&mut self, id: u32, reference: NodeId, before: bool) {
        if let Some(Some(state)) = self.rare.iterators.get_mut(id as usize) {
            state.reference = reference;
            state.before = before;
        }
    }

    /// The nodes the live ranges and iterators hold, which are not to be
    /// freed while they do.
    pub fn live_nodes(&self) -> Vec<NodeId> {
        let ranges = self.rare.ranges.iter().flatten();
        let iterators = self.rare.iterators.iter().flatten();
        ranges
            .flat_map(|r| [r.start.node, r.end.node])
            .chain(iterators.flat_map(|i| [i.root, i.reference]))
            .collect()
    }

    /// The live ranges' boundary points in `nodes`, which are distinct,
    /// changed by `f`, the nodes keeping those that are theirs.
    fn each_boundary(&mut self, nodes: &[NodeId], mut f: impl FnMut(&mut Boundary)) {
        let live = &mut *self.rare;
        let mut points: Vec<(u32, bool)> = Vec::new();
        for node in nodes {
            if let Some(held) = live.holding.get(node) {
                points.extend_from_slice(held);
            }
        }
        for (id, end) in points {
            let Some(Some(range)) = live.ranges.get_mut(id as usize) else {
                continue;
            };
            let b = if end {
                &mut range.end
            } else {
                &mut range.start
            };
            let was = b.node;
            f(b);
            let now = b.node;
            if now != was {
                live.unhold(was, (id, end));
                live.hold(now, (id, end));
            }
        }
    }

    /// The nodes holding boundary points that are `child` or under it:
    /// found by walking its subtree when that is no larger than the
    /// number of nodes holding any, else by asking of each of those.
    fn held_under(&self, child: NodeId) -> HashSet<NodeId> {
        let holding = &self.rare.holding;
        let mut found = HashSet::new();
        for (walked, node) in self.subtree(child).enumerate() {
            if walked >= holding.len() {
                return holding
                    .keys()
                    .copied()
                    .filter(|&node| self.is_inclusive_ancestor(child, node))
                    .collect();
            }
            if holding.contains_key(&node) {
                found.insert(node);
            }
        }
        found
    }

    /// Before `child` is taken out of `parent`: the live ranges in it go
    /// to where it was, those after it in `parent` back one (the
    /// "remove" steps 4 to 7), and the iterators' pre-removing steps
    /// (section 6.1).
    pub(super) fn live_removing(&mut self, parent: NodeId, child: NodeId) {
        if self.rare.is_empty() {
            return;
        }
        let index = self.index_of(child);
        let under = self.held_under(child);
        let mut nodes: Vec<NodeId> = under.iter().copied().collect();
        nodes.push(parent);
        self.each_boundary(&nodes, |b| {
            if under.contains(&b.node) {
                *b = Boundary {
                    node: parent,
                    offset: index,
                };
            } else if b.node == parent && b.offset > index {
                b.offset -= 1;
            }
        });
        let mut iterators = std::mem::take(&mut self.rare.iterators);
        for state in iterators.iter_mut().flatten() {
            // An iterator whose root goes with `child`, or is not above
            // it, keeps its reference.
            if !self.is_inclusive_ancestor(child, state.reference)
                || child == state.root
                || !self.is_inclusive_ancestor(state.root, child)
            {
                continue;
            }
            if state.before {
                // The first node after `child`'s subtree, under the root.
                let mut at = child;
                let next = loop {
                    if let Some(next) = self.next_sibling(at) {
                        break Some(next);
                    }
                    match self.parent(at) {
                        Some(up) if up != state.root => at = up,
                        _ => break None,
                    }
                };
                if let Some(next) = next {
                    state.reference = next;
                    continue;
                }
                state.before = false;
            }
            state.reference = match self.prev_sibling(child) {
                None => parent,
                Some(mut last) => {
                    while let Some(deepest) = self.last_child(last) {
                        last = deepest;
                    }
                    last
                }
            };
        }
        self.rare.iterators = iterators;
    }

    /// After `child` went in under `parent`: the live ranges after it in
    /// `parent` on one (the "insert" step 6).
    pub(super) fn live_inserted(&mut self, parent: NodeId, child: NodeId) {
        if !self.rare.holding.contains_key(&parent) {
            return;
        }
        let index = self.index_of(child);
        self.each_boundary(&[parent], |b| {
            if b.node == parent && b.offset > index {
                b.offset += 1;
            }
        });
    }

    /// After a whole replacement of `node`'s data, which was `old`
    /// units long ([`Document::set_text`]): the live ranges in it go to
    /// its start.
    pub(super) fn live_replaced(&mut self, node: NodeId, old: u32) {
        self.each_boundary(&[node], |b| {
            if b.node == node && b.offset > 0 && b.offset <= old {
                b.offset = 0;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document of `<p>` holding the texts `texts`.
    fn paragraph(texts: &[&str]) -> (Document, NodeId, Vec<NodeId>) {
        let mut doc = Document::new();
        let root = doc.root();
        let p = doc.create_element("p").unwrap();
        doc.append_child(root, p).unwrap();
        let nodes = texts
            .iter()
            .map(|t| {
                let n = doc.create_text(t).unwrap();
                doc.append_child(p, n).unwrap();
                n
            })
            .collect();
        (doc, p, nodes)
    }

    fn at(node: NodeId, offset: u32) -> Boundary {
        Boundary { node, offset }
    }

    #[test]
    fn ranges_move_with_insertions_removals_and_data() {
        let (mut doc, p, t) = paragraph(&["ab", "cd", "ef"]);
        let id = doc.create_range(p).unwrap();
        doc.set_range(
            id,
            LiveRange {
                start: at(t[0], 1),
                end: at(p, 3),
            },
        )
        .unwrap();
        // An insertion before the end moves it on; a removal of what
        // holds the start brings it to where that was.
        let x = doc.create_element("x").unwrap();
        doc.insert_before(p, x, Some(t[1])).unwrap();
        assert_eq!(doc.range(id).unwrap().end, at(p, 4));
        doc.remove_child(p, t[0]).unwrap();
        assert_eq!(
            doc.range(id).unwrap(),
            LiveRange {
                start: at(p, 0),
                end: at(p, 3)
            }
        );
        // Replacing data: a point in what went goes to its start, one
        // after it moves by the change in length.
        doc.set_range(
            id,
            LiveRange {
                start: at(t[1], 1),
                end: at(t[1], 2),
            },
        )
        .unwrap();
        doc.replace_data(t[1], 0, 1, "xyz").unwrap();
        assert_eq!(
            doc.range(id).unwrap(),
            LiveRange {
                start: at(t[1], 0),
                end: at(t[1], 4)
            }
        );
        assert_eq!(doc.character_data(t[1]), Some("xyzd"));
        // A split takes the points past it into the new node.
        let tail = doc.split_text(t[1], 2).unwrap();
        assert_eq!(doc.range(id).unwrap().end, at(tail, 2));
        // Normalizing joins them back, the points with them.
        doc.normalize(p).unwrap();
        assert_eq!(doc.range(id).unwrap().end, at(t[1], 4));
        assert_eq!(doc.character_data(t[1]), Some("xyzdef"));
        assert_eq!(
            doc.replace_data(t[1], 9, 0, ""),
            Err(MutationError::IndexSize)
        );
        doc.drop_range(id);
        assert_eq!(doc.range(id), None);
        doc.validate().unwrap();
    }

    #[test]
    fn a_removal_moves_the_points_under_it_among_many_ranges() {
        let (mut doc, p, t) = paragraph(&["ab", "cd", "ef"]);
        let root = doc.root();
        // The paragraph moved into a body.
        doc.remove_child(root, p).unwrap();
        let body = doc.create_element("body").unwrap();
        doc.append_child(root, body).unwrap();
        doc.append_child(body, p).unwrap();
        // Many ranges in a tree of their own, which no change here moves.
        let other = doc.create_element("div").unwrap();
        let far = doc.create_text("far").unwrap();
        doc.append_child(other, far).unwrap();
        let mut ids = Vec::new();
        for i in 0..40 {
            let id = doc.create_range(far).unwrap();
            let point = at(far, i % 3);
            doc.set_range(
                id,
                LiveRange {
                    start: point,
                    end: point,
                },
            )
            .unwrap();
            ids.push(id);
        }
        let inside = doc.create_range(p).unwrap();
        doc.set_range(
            inside,
            LiveRange {
                start: at(t[1], 1),
                end: at(p, 3),
            },
        )
        .unwrap();
        // The paragraph's subtree is smaller than the nodes holding points:
        // it is walked. The point in the text goes to where the paragraph
        // was, the one in the paragraph itself too.
        doc.remove_child(body, p).unwrap();
        assert_eq!(
            doc.range(inside).unwrap(),
            LiveRange {
                start: at(body, 0),
                end: at(body, 0)
            }
        );
        // A subtree larger than the nodes holding points is not walked:
        // each node holding one is asked about instead.
        let holder = doc.create_element("section").unwrap();
        doc.append_child(body, holder).unwrap();
        let big = doc.create_element("ul").unwrap();
        doc.append_child(holder, big).unwrap();
        let mut last = big;
        for _ in 0..100 {
            last = doc.create_element("li").unwrap();
            doc.append_child(big, last).unwrap();
        }
        let deep = doc.create_range(last).unwrap();
        doc.remove_child(holder, big).unwrap();
        let point = at(holder, 0);
        assert_eq!(
            doc.range(deep).unwrap(),
            LiveRange {
                start: point,
                end: point
            }
        );
        for (i, id) in ids.iter().enumerate() {
            let point = at(far, i as u32 % 3);
            assert_eq!(doc.range(*id).unwrap().start, point);
        }
        // Moved there, or set there, the points follow the changes of
        // their new node: on past a node inserted before them, not one at
        // or after them.
        let before = doc.create_element("p").unwrap();
        doc.insert_before(holder, before, None).unwrap();
        assert_eq!(doc.range(deep).unwrap().end, at(holder, 0));
        let point = at(holder, 1);
        doc.set_range(
            deep,
            LiveRange {
                start: point,
                end: point,
            },
        )
        .unwrap();
        doc.insert_before(holder, big, Some(before)).unwrap();
        assert_eq!(doc.range(deep).unwrap().end, at(holder, 2));
        doc.replace_data(far, 0, 3, "").unwrap();
        assert!(
            ids.iter()
                .all(|&id| doc.range(id).unwrap().end == at(far, 0))
        );
        for id in ids {
            doc.drop_range(id);
        }
        doc.drop_range(inside);
        doc.drop_range(deep);
        assert!(doc.rare.holding.is_empty(), "every point let go of");
        doc.validate().unwrap();
    }

    #[test]
    fn points_in_the_parent_follow_a_split_and_a_join() {
        let (mut doc, p, t) = paragraph(&["abcd", "ef"]);
        // After the text that splits: on past the new node.
        let id = doc.create_range(p).unwrap();
        doc.set_range(
            id,
            LiveRange {
                start: at(p, 1),
                end: at(p, 1),
            },
        )
        .unwrap();
        doc.split_text(t[0], 2).unwrap();
        assert_eq!(doc.range(id).unwrap().start, at(p, 2));
        // Before the text a join takes: into the text it joins.
        doc.normalize(p).unwrap();
        assert_eq!(doc.range(id).unwrap().start, at(t[0], 4));
        // A whole replacement brings a point in the text to its start.
        doc.set_text(t[0], "x").unwrap();
        assert_eq!(doc.range(id).unwrap().start, at(t[0], 0));
    }

    #[test]
    fn offsets_count_utf16_units() {
        let (mut doc, _, t) = paragraph(&["a\u{1f600}b"]);
        assert_eq!(doc.node_length(t[0]), 4);
        assert_eq!(doc.substring_data(t[0], 1, 2).as_deref(), Some("\u{1f600}"));
        doc.replace_data(t[0], 3, 1, "c").unwrap();
        assert_eq!(doc.character_data(t[0]), Some("a\u{1f600}c"));
    }

    #[test]
    fn boundaries_and_positions_compare_in_tree_order() {
        let (doc, p, t) = paragraph(&["ab", "cd"]);
        assert_eq!(
            doc.compare_boundaries(at(t[0], 2), at(t[1], 0)),
            Some(Ordering::Less)
        );
        // A point in a child against one in its parent, by the index.
        assert_eq!(
            doc.compare_boundaries(at(p, 1), at(t[0], 1)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            doc.compare_boundaries(at(t[0], 1), at(p, 1)),
            Some(Ordering::Less)
        );
        assert_eq!(
            doc.compare_boundaries(at(p, 0), at(t[0], 0)),
            Some(Ordering::Less)
        );
        assert_eq!(doc.compare_position(t[0], t[1]), 4);
        assert_eq!(doc.compare_position(t[1], p), 10);
        assert_eq!(doc.compare_position(p, t[1]), 20);
    }

    #[test]
    fn an_iterator_moves_off_a_node_taken_out() {
        let (mut doc, p, t) = paragraph(&["a", "b", "c"]);
        let id = doc.create_iterator(p).unwrap();
        // Before `b`: it goes to the next node, `c`.
        doc.set_iterator(id, t[1], true);
        doc.remove_child(p, t[1]).unwrap();
        assert_eq!(doc.iterator(id).unwrap().reference, t[2]);
        // After `c`, the last: to the node before it, `a`.
        doc.set_iterator(id, t[2], false);
        doc.remove_child(p, t[2]).unwrap();
        let state = doc.iterator(id).unwrap();
        assert_eq!((state.reference, state.before), (t[0], false));
        // Before `a`, with nothing after it: after the node before, `p`.
        doc.set_iterator(id, t[0], true);
        doc.remove_child(p, t[0]).unwrap();
        let state = doc.iterator(id).unwrap();
        assert_eq!((state.reference, state.before), (p, false));
        // One whose root goes with the node taken out keeps its reference.
        let (mut doc, p, t) = paragraph(&["a", "b"]);
        let id = doc.create_iterator(t[0]).unwrap();
        let parent = doc.parent(p).unwrap();
        doc.remove_child(parent, p).unwrap();
        assert_eq!(doc.iterator(id).unwrap().reference, t[0]);
    }
}
