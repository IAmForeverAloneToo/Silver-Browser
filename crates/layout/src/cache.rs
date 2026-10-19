//! The layout cache (M13.2): what a layout pass keeps for the next, so
//! that a change to the document lays out what it touches and reuses
//! the rest.
//!
//! An entry is kept by node for an element's principal box placed in
//! flow by [`crate::block`] — a block, a table and its wrapper, or a
//! flex container (M17) — and holds the box's whole result: its fragment
//! subtree relative to the box, its height, the margins it exposes and
//! its baselines, with the inputs it was computed from. A later pass
//! takes the entry instead of laying the box out when:
//!
//! - nothing in the element's subtree changed since: the caller forgets
//!   the entries of every node a change can affect and of its
//!   ancestors ([`LayoutCache::invalidate`]);
//! - the inputs are the same: the containing block's width and height
//!   (the latter for percentage heights), the width available to the
//!   box, and the fingerprint of the generated text in it (M21.4), whose
//!   quote marks and counters depend on what comes before the box — a
//!   fingerprint that took the place of M16's quote depth;
//! - no float was in the box's formatting context, placed or waiting,
//!   when it was laid out or when it is reused, and the box placed
//!   none: its layout then neither met a float nor moved one, and its
//!   place in the flow cannot have mattered to it;
//! - no absolutely positioned box was carried out of it: such a box is
//!   laid out against an ancestor, which lays it out again;
//! - its layout reads no ancestor's pseudo-element style (M16): an
//!   ancestor's `::first-letter` taking its text, whose change restyles
//!   the ancestor alone.
//!
//! An entry holds the box's fragment subtree whole, shared (M35.9,
//! [`crate::Children`]): the lists of children in it are the very ones
//! the layout that made or took it holds, and the entries of the boxes
//! inside it hold the same, so the cache costs the document's fragments
//! once however deep its boxes nest, and taking an entry copies
//! nothing.
//!
//! The fragments carry their boxes' places in tree order (M17), which
//! a change before a box moves without changing the box. Taking an
//! entry moves its box's place, and its children's list's shift every
//! place under it.
//!
//! A flex or grid item and a table cell are kept too (M35.9), under
//! their node beside a block's entries, a few each, by the inputs their
//! container lays them out with (`crate::pass::ItemKey`): a container
//! laid out again takes the items nothing changed from the cache, and
//! the boxes inside an item are kept and taken as a block's are.
//!
//! The box tree is kept too (M35.9, [`LayoutCache::keep_boxes`]): a pass
//! builds anew the boxes of the elements a change reached and takes
//! the rest whole from the last tree, an element's own box with its
//! subtree, where nothing in it is generated, a marker, a control, or
//! styled by an ancestor's pseudo-element (what the builder's later
//! steps write into a tree), and nothing was forgotten in it since: the
//! elements a change reached, their ancestors, and everything under a
//! restyled element, whose decorations and first lines reach its
//! descendants' boxes.
//!
//! Boxes laid out in other ways, inside an inline-block, a float, or
//! an absolutely positioned box, are laid out again with the box that
//! holds them, which is itself kept when it can be.

use std::collections::{HashMap, HashSet};

use dom::{Change, Document, NodeId};

use crate::block::{AdjoiningMargins, BlockLayout, Fragment};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::composite::Patch;
use crate::parallel::{Count, Locked};

/// The layout results kept between passes, and what the last pass did.
#[derive(Debug, Default)]
pub struct LayoutCache {
    entries: Locked<HashMap<NodeId, (Inputs, Entry)>>,
    /// The flex and grid items' and the table cells' layouts, a few by
    /// node, the latest last (M35.9).
    items: Locked<HashMap<NodeId, Vec<(ItemInputs, Entry)>>>,
    /// The boxes' content widths (min-content, max-content), by node,
    /// with the fingerprint of the generated text they were measured
    /// with (M35.9): what a box's widths depend on is its subtree and
    /// the fonts, so what a change does not reach measures the same.
    widths: Locked<HashMap<NodeId, (u64, Widths)>>,
    /// The last box tree built, and where in it each element's own box
    /// that may be taken whole is, by the places among their parents'
    /// children of the boxes down to it (M35.9).
    tree: Locked<Vec<LayoutBox>>,
    boxes: Locked<HashMap<NodeId, Box<[u32]>>>,
    boxes_taken: Count,
    laid_out: Count,
    reused: Count,
    items_laid_out: Count,
    items_reused: Count,
    widths_measured: Count,
    widths_reused: Count,
}

/// What a box's layout was computed from, beyond its subtree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Inputs {
    pub(crate) cb_width: f32,
    pub(crate) available_width: f32,
    pub(crate) cb_height: Option<f32>,
    pub(crate) generated_text: u64,
}

/// What an item's or a cell's layout was computed from, beyond its
/// subtree and its node (M35.9, `crate::pass::ItemKey`): the sizes by
/// their bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ItemInputs {
    pub(crate) content_width: u32,
    pub(crate) imposed: Option<(u32, bool)>,
    pub(crate) cb_width: u32,
    pub(crate) cb_height: Option<u32>,
    pub(crate) border: Option<[u32; 4]>,
    pub(crate) generated_text: u64,
}

/// A box's content widths: min-content, max-content.
type Widths = (f32, f32);

/// The most layouts of one item or cell kept: a container lays an item
/// out to measure it and again at its final size, and a few times more
/// for a nested container's sizes.
const MAX_ITEM_LAYOUTS: usize = 4;

/// One box's kept result: a [`BlockLayout`] with nothing carried out.
#[derive(Debug)]
struct Entry {
    /// The box's fragment and its subtree, shared.
    root: Fragment,
    height: f32,
    margin_left: f32,
    margin_top: AdjoiningMargins,
    margin_bottom: AdjoiningMargins,
    top_edge: AdjoiningMargins,
    bottom_edge: AdjoiningMargins,
    self_collapsing: bool,
    last_baseline: Option<f32>,
    first_baseline: Option<f32>,
    first_line: Option<(f32, f32)>,
    inline_block_baseline: Option<f32>,
    auto_content_height: f32,
}

impl LayoutCache {
    /// An empty cache.
    pub fn new() -> Self {
        LayoutCache::default()
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    /// Forgets every entry.
    pub fn clear(&mut self) {
        self.entries.get_mut().clear();
        self.items.get_mut().clear();
        self.widths.get_mut().clear();
        self.tree.get_mut().clear();
        self.boxes.get_mut().clear();
    }

    /// The bytes `layout` and the cache hold between them (M35.5): each
    /// list of fragments once however many share it, each fragment with
    /// its text, the boxes of the tree kept, and the content widths.
    pub fn heap_bytes(&self, layout: &crate::DocumentLayout) -> usize {
        let mut seen = HashSet::new();
        let mut bytes = layout.roots.capacity() * std::mem::size_of::<Fragment>();
        for f in &layout.roots {
            bytes += fragment_bytes(f, &mut seen);
        }
        for (_, entry) in self.entries.borrow().values() {
            bytes += std::mem::size_of::<Entry>() + fragment_bytes(&entry.root, &mut seen);
        }
        for kept in self.items.borrow().values() {
            for (_, entry) in kept {
                bytes += std::mem::size_of::<Entry>() + fragment_bytes(&entry.root, &mut seen);
            }
        }
        bytes += box_bytes(&self.tree.borrow());
        bytes += self.widths.borrow().len() * std::mem::size_of::<(NodeId, (u64, Widths))>();
        bytes
    }

    /// How many elements' boxes the last build took whole from the box
    /// tree kept (M35.9).
    pub fn boxes_taken(&self) -> usize {
        self.boxes_taken.get()
    }

    /// Forgets the kept boxes of `nodes`, elements whose new styles a
    /// restyle wrote into the layout without laying it out (M22.4,
    /// M35.9), and their ancestors' and descendants': a box carries its
    /// element's style, and the next build takes a box whole, its
    /// descendants with it.
    pub fn forget_boxes(&self, document: &Document, nodes: &[NodeId]) {
        let mut boxes = self.boxes.borrow_mut();
        if boxes.is_empty() {
            return;
        }
        for &node in nodes {
            if !document.contains(node) {
                continue;
            }
            for ancestor in document.ancestors(node) {
                boxes.remove(&ancestor);
            }
            for below in document.subtree(node) {
                boxes.remove(&below);
            }
        }
    }

    /// Keeps `boxes`, the box tree a pass was laid out from, for the next
    /// build to take what nothing changed from (M35.9).
    pub fn keep_boxes(&self, boxes: Vec<LayoutBox>) {
        let mut places = HashMap::new();
        mark(&boxes, &mut Vec::new(), &mut places);
        *self.tree.borrow_mut() = boxes;
        *self.boxes.borrow_mut() = places;
    }

    /// The box kept for the element `node`, taken out of the tree kept,
    /// if there is one nothing changed in.
    pub(crate) fn take_box(&self, node: NodeId) -> Option<LayoutBox> {
        let place = self.boxes.borrow_mut().remove(&node)?;
        let mut tree = self.tree.borrow_mut();
        let (&last, way) = place.split_last()?;
        let mut list: &mut Vec<LayoutBox> = &mut tree;
        for &i in way {
            list = &mut list.get_mut(i as usize)?.children;
        }
        let slot = list.get_mut(last as usize).filter(|b| b.node == node)?;
        let hollow = LayoutBox::hollow(slot);
        self.boxes_taken.add();
        Some(std::mem::replace(slot, hollow))
    }

    /// What the last pass did with the flex and grid items and the table
    /// cells (M35.9): how many layouts it made, and how many it took
    /// from the cache instead.
    pub fn item_counts(&self) -> (usize, usize) {
        (self.items_laid_out.get(), self.items_reused.get())
    }

    /// What the last pass did with the boxes' content widths (M35.9):
    /// how many it measured, and how many it took from the cache.
    pub fn width_counts(&self) -> (usize, usize) {
        (self.widths_measured.get(), self.widths_reused.get())
    }

    /// What the last pass did with the boxes it placed in flow: how
    /// many it laid out, and how many it took from the cache instead.
    pub fn counts(&self) -> (usize, usize) {
        (self.laid_out.get(), self.reused.get())
    }

    /// Forgets what `changes`, made to `document` since the last pass,
    /// and the elements whose computed style changed with them,
    /// `restyled`, can have changed: the entries of every node whose
    /// layout may differ and of all its ancestors, whose layouts hold
    /// it. That is a parent whose children changed, the parent of a
    /// text node whose data changed, an element whose `colspan`,
    /// `rowspan` or `span` changed (what table layout reads), an `img`
    /// whose `src` or `alt` changed (what its box shows, M23.2), and
    /// every restyled element. An overflowed journal forgets everything; the
    /// entries of nodes freed since go too.
    pub fn invalidate(&mut self, document: &Document, changes: &[Change], restyled: &[NodeId]) {
        let entries = self.entries.get_mut();
        let items = self.items.get_mut();
        let widths = self.widths.get_mut();
        let boxes = self.boxes.get_mut();
        let mut forget = |node: NodeId| {
            entries.remove(&node);
            items.remove(&node);
            widths.remove(&node);
            boxes.remove(&node);
            for ancestor in document.ancestors(node) {
                entries.remove(&ancestor);
                items.remove(&ancestor);
                widths.remove(&ancestor);
                boxes.remove(&ancestor);
            }
        };
        let mut controls = false;
        // The nodes inserted, moved among them: their boxes were built
        // where they were, under other ancestors' decorations, and are
        // built again (found by the mutation fuzz target, M35.9).
        let mut inserted = Vec::new();
        for change in changes {
            if let Change::Inserted { child, .. } = change {
                inserted.push(*child);
            }
            match change {
                Change::Everything => {
                    entries.clear();
                    items.clear();
                    widths.clear();
                    boxes.clear();
                    return;
                }
                Change::Inserted { parent, child } | Change::Removed { parent, child, .. } => {
                    forget(*parent);
                    // Controls moved change their groups and fieldsets.
                    controls |= document.contains(*child)
                        && document.subtree(*child).any(|node| {
                            matches!(
                                document.tag(node),
                                Some("input" | "select" | "textarea" | "option" | "legend")
                            )
                        });
                }
                Change::TextChanged { node } => {
                    if let Some(parent) = document.parent(*node) {
                        forget(parent);
                    }
                }
                // A state's change lays out what it restyles, which the
                // restyled list holds (M26.4).
                Change::StateChanged { .. } => {}
                // A control's state changes what it shows (M26.7), and
                // an option's what its select shows.
                Change::ControlChanged { element } => {
                    forget(*element);
                    if let Some(select) = dom::forms::owner_select(document, *element) {
                        forget(select);
                    }
                }
                Change::AttributeChanged { element, name, .. } => {
                    let img = document.tag(*element) == Some("img");
                    // A canvas's size is its attributes', no style's
                    // (found by dom_api at M33).
                    let canvas = document.tag(*element) == Some("canvas");
                    if matches!(name.as_str(), "colspan" | "rowspan" | "span")
                        || (img && matches!(name.as_str(), "src" | "alt"))
                        || (canvas && matches!(name.as_str(), "width" | "height"))
                    {
                        forget(*element);
                    }
                    controls |= shown_by_controls(name);
                }
            }
        }
        // What a control shows reads attributes and nodes far from it
        // (M26.7): a fieldset's `disabled`, a radio group's `name`s and
        // members, an option's `label`. One of those changed, every
        // control is laid out again.
        if controls {
            let all: Vec<NodeId> = document
                .subtree(document.root())
                .filter(|&node| {
                    matches!(
                        document.tag(node),
                        Some("input" | "select" | "textarea" | "option")
                    )
                })
                .collect();
            for node in all {
                forget(node);
            }
        }
        for &node in restyled {
            forget(node);
        }
        // A restyled element's decorations and first line reach the boxes
        // of its descendants, which are built again.
        let boxes = self.boxes.get_mut();
        if !boxes.is_empty() {
            for &node in restyled.iter().chain(&inserted) {
                if document.contains(node) {
                    for below in document.subtree(node) {
                        boxes.remove(&below);
                    }
                }
            }
        }
        entries.retain(|node, _| document.contains(*node));
        items.retain(|node, _| document.contains(*node));
        widths.retain(|node, _| document.contains(*node));
        self.boxes
            .get_mut()
            .retain(|node, _| document.contains(*node));
    }

    /// Gives the fragments of the kept entries that a restyle's
    /// composited changes reach their new effects, so that a later
    /// layout taking them has them (M22.4, `crate::composite`): through
    /// `patch`, which wrote them into the layout shown first, so that
    /// each list of children shared with it is the copy it made.
    pub fn composite(&self, patch: &mut Patch<'_, '_>) {
        for (_, entry) in self.entries.borrow_mut().values_mut() {
            patch.fragment(&mut entry.root);
        }
        for kept in self.items.borrow_mut().values_mut() {
            for (_, entry) in kept {
                patch.fragment(&mut entry.root);
            }
        }
    }

    /// Starts a pass: its counts from zero.
    pub(crate) fn begin_pass(&self) {
        self.laid_out.set(0);
        self.reused.set(0);
        self.items_laid_out.set(0);
        self.items_reused.set(0);
        self.widths_measured.set(0);
        self.widths_reused.set(0);
    }

    /// The content widths kept for `node`'s box with the generated text
    /// `generated_text`, if there are.
    pub(crate) fn widths(&self, node: NodeId, generated_text: u64) -> Option<(f32, f32)> {
        let widths = self.widths.borrow();
        let (_, widths) = widths
            .get(&node)
            .filter(|(text, _)| *text == generated_text)?;
        self.widths_reused.add();
        Some(*widths)
    }

    /// Keeps `widths`, `node`'s box's content widths with the generated
    /// text `generated_text`.
    pub(crate) fn keep_widths(&self, node: NodeId, generated_text: u64, widths: (f32, f32)) {
        self.widths_measured.add();
        self.widths
            .borrow_mut()
            .insert(node, (generated_text, widths));
    }

    /// Counts an item's or a cell's layout made.
    pub(crate) fn count_item_laid_out(&self) {
        self.items_laid_out.add();
    }

    /// The kept layout of `node`'s item or cell laid out with `inputs`,
    /// its place in tree order now `tree_order`, if there is one.
    pub(crate) fn take_item<'a>(
        &self,
        node: NodeId,
        inputs: ItemInputs,
        tree_order: u32,
    ) -> Option<BlockLayout<'a>> {
        let items = self.items.borrow();
        let (_, entry) = items.get(&node)?.iter().find(|(i, _)| *i == inputs)?;
        self.items_reused.add();
        Some(entry.layout(tree_order))
    }

    /// Keeps `layout`, `node`'s item or cell laid out with `inputs`, when
    /// nothing was carried out of it, the oldest of its layouts let go
    /// past [`MAX_ITEM_LAYOUTS`].
    pub(crate) fn keep_item(&self, node: NodeId, inputs: ItemInputs, layout: &BlockLayout<'_>) {
        if !layout.absolutes.is_empty() {
            return;
        }
        let mut items = self.items.borrow_mut();
        let kept = items.entry(node).or_default();
        kept.retain(|(i, _)| *i != inputs);
        if kept.len() >= MAX_ITEM_LAYOUTS {
            kept.remove(0);
        }
        kept.push((inputs, Entry::of(layout)));
    }

    /// Counts a box placed in flow and laid out.
    pub(crate) fn count_laid_out(&self) {
        self.laid_out.add();
    }

    /// The kept result for `node`'s box laid out with `inputs`, its
    /// place in tree order now `tree_order`, if there is one: its
    /// fragment subtree shared, the box's place moved to `tree_order`
    /// and every place under it with it.
    pub(crate) fn take<'a>(
        &self,
        node: NodeId,
        inputs: Inputs,
        tree_order: u32,
    ) -> Option<BlockLayout<'a>> {
        let entries = self.entries.borrow();
        let (_, entry) = entries.get(&node).filter(|(i, _)| *i == inputs)?;
        self.reused.add();
        Some(entry.layout(tree_order))
    }

    /// Keeps `layout`, `node`'s box laid out with `inputs`, its fragment
    /// subtree shared, when `keep` says it may be (see the module
    /// documentation) and nothing was carried out of it; otherwise
    /// forgets any entry `node` had.
    pub(crate) fn keep(&self, node: NodeId, inputs: Inputs, layout: &BlockLayout<'_>, keep: bool) {
        let mut entries = self.entries.borrow_mut();
        if !keep || !layout.absolutes.is_empty() {
            entries.remove(&node);
            return;
        }
        entries.insert(node, (inputs, Entry::of(layout)));
    }
}

impl Entry {
    /// `layout` kept, its fragment subtree shared.
    fn of(layout: &BlockLayout<'_>) -> Entry {
        Entry {
            root: layout.root.clone(),
            height: layout.height,
            margin_left: layout.margin_left,
            margin_top: layout.margin_top,
            margin_bottom: layout.margin_bottom,
            top_edge: layout.top_edge,
            bottom_edge: layout.bottom_edge,
            self_collapsing: layout.self_collapsing,
            last_baseline: layout.last_baseline,
            first_baseline: layout.first_baseline,
            first_line: layout.first_line,
            inline_block_baseline: layout.inline_block_baseline,
            auto_content_height: layout.auto_content_height,
        }
    }

    /// The layout kept, its fragment subtree shared, its box's place in
    /// tree order moved to `tree_order` and every place under it with
    /// it.
    fn layout<'a>(&self, tree_order: u32) -> BlockLayout<'a> {
        let mut root = self.root.clone();
        let by = tree_order.wrapping_sub(root.tree_order);
        root.tree_order = tree_order;
        root.children.move_along(by);
        BlockLayout {
            root,
            height: self.height,
            margin_left: self.margin_left,
            margin_top: self.margin_top,
            margin_bottom: self.margin_bottom,
            top_edge: self.top_edge,
            bottom_edge: self.bottom_edge,
            self_collapsing: self.self_collapsing,
            last_baseline: self.last_baseline,
            first_baseline: self.first_baseline,
            first_line: self.first_line,
            inline_block_baseline: self.inline_block_baseline,
            absolutes: Vec::new(),
            auto_content_height: self.auto_content_height,
        }
    }
}

/// The bytes of what `f` holds beyond itself: its text and decoration,
/// and its children's list, when `seen` has not counted it.
fn fragment_bytes(f: &Fragment, seen: &mut HashSet<*const Vec<Fragment>>) -> usize {
    let mut bytes = match &f.kind {
        crate::FragmentKind::Text(text) => text.capacity(),
        _ => 0,
    };
    if f.decorated.is_some() {
        bytes += std::mem::size_of::<crate::decoration::Decorated>();
    }
    if let Some(list) = f.children.shared()
        && seen.insert(std::sync::Arc::as_ptr(list))
    {
        bytes += list.capacity() * std::mem::size_of::<Fragment>();
        for child in list.iter() {
            bytes += fragment_bytes(child, seen);
        }
    }
    bytes
}

/// The bytes of `boxes` and everything in them.
fn box_bytes(boxes: &[LayoutBox]) -> usize {
    boxes
        .iter()
        .map(|b| {
            let text = match &b.kind {
                BoxKind::Text(text) => text.capacity(),
                _ => 0,
            };
            std::mem::size_of::<LayoutBox>() + text + box_bytes(&b.children)
        })
        .sum()
}

/// Records in `places` where each element's own box under `boxes` that
/// a build may take whole is, `path` the way to `boxes`: whether every
/// box there may be.
fn mark(
    boxes: &[LayoutBox],
    path: &mut Vec<u32>,
    places: &mut HashMap<NodeId, Box<[u32]>>,
) -> bool {
    let mut all = true;
    for (i, b) in boxes.iter().enumerate() {
        path.push(i as u32);
        let inside = mark(&b.children, path, places);
        let whole = inside
            && b.generated.is_none()
            && !b.styled_by_ancestor
            && b.first_line.is_none()
            && b.control_size.is_none()
            && !matches!(b.kind, BoxKind::Marker { .. });
        let own = !b.anonymous
            && matches!(
                b.kind,
                BoxKind::Block
                    | BoxKind::InlineBlock
                    | BoxKind::Float
                    | BoxKind::Absolute
                    | BoxKind::Flex
                    | BoxKind::Grid
                    | BoxKind::Table
            );
        if whole && own {
            places.insert(b.node, path.clone().into_boxed_slice());
        }
        all &= whole;
        path.pop();
    }
    all
}

/// Whether an attribute named `name` may change what a form control
/// shows (M26.7): its value, placeholder or type, a checkbox's or a
/// radio button's state and group, being disabled, a select's kind, an
/// option's label or selectedness; or its own size (M26.11), a text
/// field's `size`, a textarea's `cols` and `rows`.
pub fn shown_by_controls(name: &str) -> bool {
    matches!(
        name,
        "value"
            | "placeholder"
            | "type"
            | "checked"
            | "name"
            | "form"
            | "id"
            | "disabled"
            | "multiple"
            | "size"
            | "cols"
            | "rows"
            | "label"
            | "selected"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{DocumentLayout, layout_document_cached, layout_document_in};
    use crate::composite::EffectsChange;
    use crate::{Children, FragmentKind};
    use crate::{build_box_tree, build_box_tree_kept};
    use std::collections::HashSet;
    use style::{CascadeContext, Styles, Stylesheets};
    use text::FontCollection;

    const HEIGHT: f32 = 300.0;

    /// A document laid out pass after pass with a cache, as the engine
    /// does it: changed through its mutation API, restyled, the cache
    /// brought up to date, laid out again.
    struct Session {
        document: Document,
        sheets: Stylesheets,
        styles: Styles,
        cache: LayoutCache,
        fonts: FontCollection,
        width: f32,
        /// The last pass's layout.
        layout: DocumentLayout,
    }

    impl Session {
        /// `html` under `css`, at 400px wide; nothing laid out yet.
        fn new(html: &str, css: &str) -> Session {
            let parsed = html::parse_document(html).expect("html parsing never fails");
            let document = Document::from_node(&parsed);
            let sheet = css::parse_stylesheet(css).expect("test css is valid");
            let sheets = Stylesheets::new(vec![sheet]);
            let styles = sheets.cascade(&document, &CascadeContext::default());
            Session {
                document,
                sheets,
                styles,
                cache: LayoutCache::new(),
                fonts: FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                    .expect("the bundled font parses"),
                width: 400.0,
                layout: DocumentLayout::default(),
            }
        }

        /// The element whose `id` is `id`.
        fn id(&self, id: &str) -> NodeId {
            self.document
                .subtree(self.document.root())
                .find(|&n| self.document.attr(n, "id") == Some(id))
                .unwrap_or_else(|| panic!("no element has id {id}"))
        }

        /// The `body` element.
        fn body(&self) -> NodeId {
            self.document
                .subtree(self.document.root())
                .find(|&n| self.document.tag(n) == Some("body"))
                .expect("the parser implies a body")
        }

        /// Whether the cache holds an entry for `id`'s box.
        fn kept(&self, id: &str) -> bool {
            self.cache.entries.borrow().contains_key(&self.id(id))
        }

        /// A layout of the document as it stands, over a full cascade,
        /// with no cache.
        fn fresh(&self) -> DocumentLayout {
            let context = CascadeContext::default();
            let styles = self.sheets.cascade(&self.document, &context);
            let recascade = self.sheets.recascader(&self.document, &styles, &context);
            let boxes = build_box_tree(&self.document, &styles, &recascade);
            layout_document_in(&boxes, self.width, Some(HEIGHT), &self.fonts)
        }

        /// The box fragment of the element `id` in the last layout.
        fn box_of(&self, id: &str) -> Fragment {
            let node = self.id(id);
            self.layout
                .iter()
                .find(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
                .cloned()
                .expect("a box")
        }

        /// Whether every list of children the cache's entries hold is
        /// one the last layout holds: shared, no copy (M35.9).
        fn shares_its_lists(&self) -> bool {
            let mut layout = HashSet::new();
            lists(&self.layout.roots, &mut layout);
            let mut kept = HashSet::new();
            for (_, entry) in self.cache.entries.borrow().values() {
                lists(std::slice::from_ref(&entry.root), &mut kept);
            }
            for kept_items in self.cache.items.borrow().values() {
                for (_, entry) in kept_items {
                    lists(std::slice::from_ref(&entry.root), &mut kept);
                }
            }
            !kept.is_empty() && kept.is_subset(&layout)
        }

        /// Changes the document with `f`, then restyles it and brings
        /// the cache up to date with what changed.
        fn change(&mut self, f: impl FnOnce(&mut Document)) {
            f(&mut self.document);
            let changes = self.document.take_changes();
            let context = CascadeContext::default();
            let restyle = self
                .sheets
                .restyle(&self.document, &context, &mut self.styles, &changes);
            if restyle.full {
                self.cache.clear();
            } else {
                // A composited change (M22.4) is laid out again, as the
                // engine lays it out when it does not keep its layout.
                let mut restyled = restyle.changed;
                restyled.extend(restyle.composited.iter().map(|c| c.node));
                self.cache.invalidate(&self.document, &changes, &restyled);
            }
        }

        /// Changes the document with `f`, then restyles it, every change
        /// of style composited (M22.4), and writes them into the cache's
        /// entries rather than forget them, as the engine does when it
        /// keeps its layout.
        fn composite(&mut self, f: impl FnOnce(&mut Document)) {
            f(&mut self.document);
            let changes = self.document.take_changes();
            let restyle = self.sheets.restyle(
                &self.document,
                &CascadeContext::default(),
                &mut self.styles,
                &changes,
            );
            assert!(
                !restyle.full && restyle.changed.is_empty() && !restyle.composited.is_empty(),
                "{restyle:?}"
            );
            let changes: Vec<EffectsChange<'_>> = restyle
                .composited
                .iter()
                .map(|c| EffectsChange {
                    node: c.node,
                    pseudo: c.pseudo,
                    old: &c.old,
                    new: &c.new,
                    transformed: c.transformed,
                    styles: Some((&c.old_style, &c.new_style)),
                })
                .collect();
            // Into the layout, then the cache, as the engine does: one
            // patch, the lists they share copied once.
            let mut patch = Patch::new(&changes);
            self.layout.composite(&mut patch, self.width);
            self.cache.composite(&mut patch);
        }

        /// A pass with the cache, held to a pass over a full cascade
        /// without one: the boxes it laid out and those it reused.
        fn pass(&mut self) -> (usize, usize) {
            let context = CascadeContext::default();
            let recascade = self
                .sheets
                .recascader(&self.document, &self.styles, &context);
            // The box tree from the one kept, as the engine builds it.
            let boxes =
                build_box_tree_kept(&self.document, &self.styles, &recascade, &(), &self.cache);
            let cached =
                layout_document_cached(&boxes, self.width, Some(HEIGHT), &self.fonts, &self.cache);
            self.cache.keep_boxes(boxes);
            let fresh = self.fresh();
            assert_eq!(
                snapshot(&cached),
                snapshot(&fresh),
                "the cached layout differs from a fresh one"
            );
            let orders = |layout: &DocumentLayout| -> Vec<u32> {
                crate::flatten(layout.roots.clone())
                    .iter()
                    .map(|f| f.tree_order)
                    .collect()
            };
            assert_eq!(
                orders(&cached),
                orders(&fresh),
                "the flat view's tree order"
            );
            self.layout = cached;
            self.cache.counts()
        }
    }

    /// Everything a layout says, to compare two by: the places in tree
    /// order with the shifts above them added (M35.9).
    fn snapshot(layout: &DocumentLayout) -> String {
        format!(
            "{} {} {:?}",
            layout.height,
            layout.depends_on_viewport_height,
            settled(&layout.roots, 0)
        )
    }

    /// A copy of `fragments`, `shift` along in tree order, with every
    /// list's shift added to the places under it.
    fn settled(fragments: &[Fragment], shift: u32) -> Vec<Fragment> {
        fragments
            .iter()
            .map(|f| {
                let mut f = f.clone();
                if !matches!(f.kind, FragmentKind::Text(_)) {
                    f.tree_order = f.tree_order.wrapping_add(shift);
                }
                let below = shift.wrapping_add(f.children.shift());
                f.children = settled(&f.children, below).into();
                f
            })
            .collect()
    }

    /// Appends a paragraph saying `text` to `parent`.
    fn append_paragraph(d: &mut Document, parent: NodeId, text: &str) -> NodeId {
        let p = d.create_element("p").unwrap();
        let data = d.create_text(text).unwrap();
        d.append_child(p, data).unwrap();
        d.append_child(parent, p).unwrap();
        p
    }

    /// Three paragraphs, the middle one moved from where the flow puts
    /// it, so that a box taken from the cache is put back where it was.
    const PAGE: &str = "<p id=a>one</p><p id=b>two</p><p id=c>three</p>";
    const PAGE_CSS: &str = "#b { position: relative; left: 5px; top: 3px }";

    #[test]
    fn a_pass_with_nothing_changed_takes_the_top_box_whole() {
        let mut s = Session::new(PAGE, PAGE_CSS);
        // html, body and the three paragraphs.
        assert_eq!(s.pass(), (5, 0));
        assert_eq!(s.cache.len(), 5);
        assert_eq!(s.pass(), (0, 1));
    }

    #[test]
    fn the_entries_a_composited_change_is_written_into_are_what_a_pass_takes() {
        // M22.4: a box turned and faded, its change written into the
        // entries that hold its fragments, the top box's and its own:
        // the pass that takes the top box whole has the new effects, as
        // a fresh layout does.
        let mut s = Session::new(
            "<div id=d><p id=t>turned</p><p>still</p></div><p id=after>after</p>",
            "#t { rotate: 5deg; opacity: 0.9 } #t.more { rotate: 30deg; opacity: 0.5 }",
        );
        s.pass();
        let t = s.id("t");
        let after = s.box_of("after").children.clone();
        s.composite(|d| d.set_attribute(t, "class", "more").unwrap());
        // The lists on the way to the change are copies the layout and
        // the cache share; those off it are as they were (M35.9).
        assert!(s.shares_its_lists());
        assert!(Children::ptr_eq(&s.box_of("after").children, &after));
        assert_eq!(s.pass(), (0, 1));
        // Laid out again, the change reaches its box and its ancestors.
        s.change(|d| d.set_attribute(t, "class", "").unwrap());
        assert_eq!(s.pass(), (4, 2));
    }

    #[test]
    fn an_appended_paragraph_is_laid_out_with_its_ancestors_and_no_other_box() {
        let mut s = Session::new(PAGE, PAGE_CSS);
        s.pass();
        let body = s.body();
        s.change(|d| {
            append_paragraph(d, body, "four");
        });
        assert_eq!(s.pass(), (3, 3));
    }

    #[test]
    fn a_text_change_lays_out_its_element_and_its_ancestors() {
        let mut s = Session::new(PAGE, PAGE_CSS);
        s.pass();
        let b = s.id("b");
        s.change(|d| {
            let text = d.first_child(b).unwrap();
            d.set_text(text, "two, and a good deal longer than it was")
                .unwrap();
        });
        assert_eq!(s.pass(), (3, 2));
    }

    #[test]
    fn a_restyled_element_is_laid_out_again_and_an_unread_attribute_changes_nothing() {
        let mut s = Session::new(PAGE, ".wide { padding: 10px }");
        s.pass();
        let b = s.id("b");
        s.change(|d| d.set_attribute(b, "class", "wide").unwrap());
        assert_eq!(s.pass(), (3, 2));
        s.change(|d| d.set_attribute(b, "title", "no selector reads it").unwrap());
        assert_eq!(s.pass(), (0, 1));
    }

    #[test]
    fn a_new_width_misses_but_boxes_in_a_fixed_width_container_hit() {
        let mut s = Session::new(
            "<div id=d><p>one</p><p>two</p></div>",
            "#d { width: 200px }",
        );
        s.pass();
        s.width = 500.0;
        // html, body and the div; the paragraphs' containing block is
        // the div's 200px, as before.
        assert_eq!(s.pass(), (3, 2));
    }

    #[test]
    fn boxes_a_float_can_reach_are_laid_out_and_a_context_roots_children_reused() {
        let mut s = Session::new(
            "<span id=f>float</span><p id=p>one</p><div id=r><p id=x>two</p><p>three</p></div>",
            "#f { float: left; width: 50px; height: 50px } #r { overflow: hidden }",
        );
        s.pass();
        assert!(!s.kept("p") && !s.kept("r"));
        assert!(s.kept("x"));
        let body = s.body();
        s.change(|d| {
            append_paragraph(d, body, "four");
        });
        // html, body, the paragraph beside the float, the context root
        // and the new paragraph; the root's two paragraphs, in a
        // context of its own, reused.
        assert_eq!(s.pass(), (5, 2));
    }

    #[test]
    fn a_box_that_placed_a_float_is_not_kept() {
        // The float reaches down beside the paragraph, whose line it
        // moves: taking the div from the cache would leave it out of
        // the context, and the line where it would be without it.
        let mut s = Session::new(
            "<div id=d><span class=f>float</span>text</div><p id=p>after</p>",
            ".f { float: left; width: 20px; height: 60px }",
        );
        assert_eq!(s.pass(), (4, 0));
        // Nor is anything the float was in the context of.
        assert!(s.cache.is_empty());
        let p = s.id("p");
        s.change(|d| {
            let text = d.first_child(p).unwrap();
            d.set_text(text, "later").unwrap();
        });
        assert_eq!(s.pass(), (4, 0));
    }

    #[test]
    fn a_box_carrying_an_absolute_box_up_is_not_kept_one_settling_it_is() {
        let mut s = Session::new(
            "<div id=a><span id=x>up</span></div><div id=r><span id=y>here</span></div>",
            "#x, #y { position: absolute; top: 4px; left: 6px } #r { position: relative }",
        );
        assert_eq!(s.pass(), (4, 0));
        // The ancestors carry `x` up too, to the initial containing
        // block.
        assert!(!s.kept("a") && s.kept("r"));
        assert_eq!(s.cache.len(), 1);
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn an_anonymous_box_is_not_kept_under_its_parents_node() {
        let mut s = Session::new(
            "<div id=d><span class=c>cell</span></div><p id=p>x</p>",
            ".c { display: table-cell; padding: 2px }",
        );
        s.pass();
        let p = s.id("p");
        s.change(|d| {
            let text = d.first_child(p).unwrap();
            d.set_text(text, "y").unwrap();
        });
        // html, body and the paragraph; the div, its anonymous table
        // inside, reused.
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn a_span_change_lays_the_table_out_again() {
        let mut s = Session::new(
            "<table id=t><tr><td id=c>one</td><td>two</td></tr><tr><td>three</td><td>four</td></tr></table><p>after</p>",
            "td { border: 1px solid #000 }",
        );
        s.pass();
        let c = s.id("c");
        s.change(|d| d.set_attribute(c, "colspan", "2").unwrap());
        // html, body and the table; the paragraph reused.
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn nested_boxes_are_kept_once_however_deep() {
        // Layout recurses once a level of nesting: an unoptimized build's
        // frames take more than a test thread's 2 MiB a hundred levels
        // down, so this runs on a stack the size of the main thread's,
        // where the engine lays out (8 MiB; an optimized build lays out
        // the DOM's 512 levels in under 3).
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                let mut s = Session::new(&"<div>x".repeat(100), "div { padding-left: 1px }");
                assert_eq!(s.pass(), (102, 0));
                // Every list of children an entry holds is the layout's,
                // shared, not a copy (M35.9): the cache costs the
                // fragments once however deep the boxes nest.
                assert!(s.shares_its_lists());
                // Taken whole, the next layout's are the same lists.
                let before: HashSet<_> = {
                    let mut set = HashSet::new();
                    lists(&s.layout.roots, &mut set);
                    set
                };
                assert_eq!(s.pass(), (0, 1));
                let mut after = HashSet::new();
                lists(&s.layout.roots, &mut after);
                assert_eq!(after, before);
            })
            .expect("the thread starts")
            .join()
            .expect("no panic");
    }

    /// The lists of children under `fragments`, by address.
    fn lists(fragments: &[Fragment], out: &mut HashSet<*const Vec<Fragment>>) {
        for f in fragments {
            if let Some(list) = f.children.shared()
                && out.insert(std::sync::Arc::as_ptr(list))
            {
                lists(list, out);
            }
        }
    }

    #[test]
    fn an_entry_whose_child_was_kept_again_since_is_whole_and_taken() {
        // A float before the context root narrows it: it is laid out
        // again, not kept, and its paragraph kept again at the narrower
        // width. With the float gone, the root's old entry fits its
        // inputs again, and holds its subtree whole as it was (M35.9),
        // its paragraph's at the width it has again: it is taken, and
        // the pass is a fresh one's.
        let mut s = Session::new(
            "<div id=r><p id=p>one two three four five six seven</p></div>",
            "#f { float: left; width: 100px; height: 20px } #r { overflow: hidden }",
        );
        s.pass();
        let (body, r) = (s.body(), s.id("r"));
        let mut float = None;
        s.change(|d| {
            let f = d.create_element("span").unwrap();
            d.set_attribute(f, "id", "f").unwrap();
            d.insert_before(body, f, Some(r)).unwrap();
            float = Some(f);
        });
        s.pass();
        assert!(s.kept("r") && s.kept("p"));
        let float = float.unwrap();
        s.change(|d| {
            d.remove_child(body, float).unwrap();
            d.free(float).unwrap();
        });
        assert_eq!(s.pass(), (2, 1));
    }

    #[test]
    fn freed_nodes_entries_go_and_an_overflowed_journal_clears_the_cache() {
        let mut s = Session::new(PAGE, PAGE_CSS);
        s.pass();
        let (body, c) = (s.body(), s.id("c"));
        s.change(|d| {
            d.remove_child(body, c).unwrap();
            d.free(c).unwrap();
        });
        // The body and the root forgotten, the freed paragraph dropped.
        assert_eq!(s.cache.len(), 2);
        s.cache.invalidate(&s.document, &[Change::Everything], &[]);
        assert!(s.cache.is_empty());
    }

    #[test]
    fn a_box_is_taken_only_with_the_generated_text_it_was_kept_with() {
        // #b's quote marks depend on the quote #a's class opens: with
        // it, #b's `q` is a quote inside a quote, and its last span
        // closes #a's quote, where without it the close-quote has none
        // to close — the depth after #b is 0 either way.
        let mut s = Session::new(
            "<p id=a class=o>one</p><p id=b><q>two</q><span class=c></span></p>",
            ".o::before { content: open-quote } .c::before { content: close-quote }",
        );
        assert_eq!(s.pass(), (4, 0));
        let a = s.id("a");
        // #a is restyled, and #b, unchanged but at another depth, laid
        // out again with it: `pass` holds the result to a fresh one.
        s.change(|d| d.remove_attribute(a, "class").unwrap());
        assert_eq!(s.pass(), (4, 0));
        // A change that moves no depth: #b is taken.
        s.change(|d| {
            let text = d.first_child(a).unwrap();
            d.set_text(text, "uno").unwrap();
        });
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn a_box_is_laid_out_again_when_a_counter_before_it_moves() {
        // #c counts the paragraphs before it, which it does not hold:
        // taking #a's class away moves its counter; #d holds no
        // counter, and is taken.
        let mut s = Session::new(
            "<p id=a class=n>one</p><p id=b>two</p><p id=c class=n>three</p><p id=d>four</p>",
            ".n { counter-increment: n } #c::before { content: counter(n) }",
        );
        assert_eq!(s.pass(), (6, 0));
        let a = s.id("a");
        s.change(|d| d.remove_attribute(a, "class").unwrap());
        // html, body, #a and #c; #b and #d taken.
        assert_eq!(s.pass(), (4, 2));
    }

    #[test]
    fn a_list_item_is_laid_out_again_when_its_number_moves() {
        // #a's `value` numbers the items after it, which it does not
        // hold; the paragraph after the list numbers nothing, and is
        // taken. `pass` holds each result to a fresh one.
        let mut s = Session::new(
            "<ol><li id=a value=5>one</li><li id=b>two</li><li id=c>three</li></ol><p id=d>x</p>",
            "",
        );
        assert_eq!(s.pass(), (7, 0));
        let a = s.id("a");
        s.change(|d| d.remove_attribute(a, "value").unwrap());
        // html, body, the list and its three items; #d taken.
        assert_eq!(s.pass(), (6, 1));
        assert!(s.kept("b") && s.kept("c"));
    }

    #[test]
    fn a_list_item_is_taken_with_its_marker_beside_its_content() {
        // #b has no line for #a's marker to go on, so the marker is
        // beside it, put before #b's fragment, which the cache holds
        // under #b: taking #a puts #b back in its own place and the
        // marker in the marker's. `pass` holds each result to a fresh
        // one.
        let mut s = Session::new("<ul><li id=a><p id=b></p></li></ul>", "");
        // html, body, the list, #a and #b.
        assert_eq!(s.pass(), (5, 0));
        let body = s.body();
        s.change(|d| {
            append_paragraph(d, body, "after");
        });
        // html, body and the new paragraph; the list taken, #a with it.
        assert_eq!(s.pass(), (3, 1));
        assert!(s.kept("a") && s.kept("b"));
    }

    #[test]
    fn a_box_is_laid_out_again_when_a_decoration_reaches_it() {
        // #a's underline is propagated to #b's text, which #b's own style
        // does not show; #c, after #a, is taken. `pass` holds each result
        // to a fresh one.
        let mut s = Session::new(
            "<div id=a><p id=b>one</p></div><p id=c>two</p>",
            ".u { text-decoration: underline }",
        );
        assert_eq!(s.pass(), (5, 0));
        let a = s.id("a");
        s.change(|d| d.set_attribute(a, "class", "u").unwrap());
        // html, body, #a and #b; #c taken.
        assert_eq!(s.pass(), (4, 1));
    }

    #[test]
    fn a_block_holding_an_ancestors_first_letter_is_neither_kept_nor_taken() {
        // #p's line holds #d's first letter: #d's restyle does not reach
        // #p, whose layout reads it, so #p is laid out each time.
        let mut s = Session::new(
            "<div id=d class=big><p id=p>Text</p></div><p id=q>after</p>",
            ".big::first-letter { font-size: 40px }",
        );
        s.pass();
        assert!(!s.kept("p"));
        assert!(s.kept("q"));
        let d = s.id("d");
        s.change(|doc| doc.remove_attribute(d, "class").unwrap());
        // html, body, #d and #p; #q taken. `pass` holds the result to a
        // fresh one, the letter back to the paragraph's size.
        assert_eq!(s.pass(), (4, 1));
    }

    #[test]
    fn a_block_holding_an_ancestors_first_line_is_neither_kept_nor_taken() {
        // #p's run holds #d's first line: #d's restyle does not reach #p,
        // whose layout reads it, so #p is laid out each time.
        let mut s = Session::new(
            "<div id=d class=big><p id=p>Text</p></div><p id=q>after</p>",
            ".big::first-line { font-size: 40px }",
        );
        s.pass();
        assert!(!s.kept("p"));
        assert!(s.kept("q"));
        let d = s.id("d");
        s.change(|doc| doc.remove_attribute(d, "class").unwrap());
        // html, body, #d and #p; #q taken. `pass` holds the result to a
        // fresh one, the line back to the paragraph's size.
        assert_eq!(s.pass(), (4, 1));
    }

    #[test]
    fn a_flex_container_is_kept_with_its_items() {
        // M17: a flex container placed in flow is kept as a block is; its
        // items, laid out with it, are not kept on their own.
        let mut s = Session::new(
            "<div id=f><p id=a>one</p><p>two</p></div><p id=q>after</p>",
            "#f { display: flex }",
        );
        // html, body, the container and the paragraph after it.
        assert_eq!(s.pass(), (4, 0));
        assert!(s.kept("f") && !s.kept("a") && s.kept("q"));
        let q = s.id("q");
        s.change(|d| {
            let text = d.first_child(q).unwrap();
            d.set_text(text, "later").unwrap();
        });
        // html, body and the paragraph; the container taken.
        assert_eq!(s.pass(), (3, 1));
        // A change inside an item lays the container out again.
        let a = s.id("a");
        s.change(|d| {
            let text = d.first_child(a).unwrap();
            d.set_text(text, "uno").unwrap();
        });
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn a_grid_container_is_kept_with_its_items() {
        // M18: a grid container placed in flow is kept as a flex container
        // is; its items, laid out with it, are not kept on their own.
        let mut s = Session::new(
            "<div id=g><p id=a>one</p><p>two</p></div><p id=q>after</p>",
            "#g { display: grid; grid-template-columns: 1fr 1fr }",
        );
        // html, body, the container and the paragraph after it.
        assert_eq!(s.pass(), (4, 0));
        assert!(s.kept("g") && !s.kept("a") && s.kept("q"));
        let q = s.id("q");
        s.change(|d| {
            let text = d.first_child(q).unwrap();
            d.set_text(text, "later").unwrap();
        });
        // html, body and the paragraph; the container taken.
        assert_eq!(s.pass(), (3, 1));
        // A change inside an item lays the container out again.
        let a = s.id("a");
        s.change(|d| {
            let text = d.first_child(a).unwrap();
            d.set_text(text, "uno").unwrap();
        });
        assert_eq!(s.pass(), (3, 1));
    }

    #[test]
    fn a_box_taken_after_an_insertion_before_it_moves_along_in_tree_order() {
        // M17: the fragments carry their boxes' places in tree order, which
        // an insertion before a kept box moves. The box taken from the
        // cache has its new ones, and so does the kept child it holds:
        // `pass` holds the layout to a fresh one.
        // Fragments of the kept boxes' own, not held: a positioned span
        // on a line, a flex container's items.
        let mut s = Session::new(
            "<p id=a>one</p><div id=d><p id=r>two <span>and</span></p>\
             <div id=i><p>three</p></div><div id=f><p id=g>four</p></div></div>",
            "#d { position: relative } #r { position: relative; top: 2px }
             span { position: relative; left: 1px } #f { display: flex }",
        );
        s.pass();
        assert!(s.kept("d") && s.kept("i") && s.kept("f") && !s.kept("g"));
        let (body, a) = (s.body(), s.id("a"));
        s.change(|d| {
            let p = d.create_element("p").unwrap();
            let data = d.create_text("zero").unwrap();
            d.append_child(p, data).unwrap();
            d.insert_before(body, p, Some(a)).unwrap();
        });
        // html, body and the new paragraph; #a and #d taken.
        assert_eq!(s.pass(), (3, 2));
        // #d's subtree is its entry's, shared, moved along by its list's
        // shift rather than copied with every place moved (M35.9).
        let d = s.box_of("d");
        assert_ne!(d.children.shift(), 0);
        let entries = s.cache.entries.borrow();
        let kept = &entries[&s.id("d")].1.root;
        assert!(Children::ptr_eq(&d.children, &kept.children));
    }

    #[test]
    fn a_composited_change_under_a_box_moved_along_keeps_its_places() {
        // M35.9: a box taken after an insertion before it has its list
        // moved along by a shift; a change written under it copies the
        // list, the shift kept: the layout is a fresh one's.
        let mut s = Session::new(
            "<p id=a>one</p><div id=d><p id=t>turned</p><p>still</p></div>",
            "#t { opacity: 0.9 } #t.more { opacity: 0.5 }",
        );
        s.pass();
        let (body, a) = (s.body(), s.id("a"));
        s.change(|d| {
            let p = d.create_element("p").unwrap();
            d.insert_before(body, p, Some(a)).unwrap();
        });
        s.pass();
        assert_ne!(s.box_of("d").children.shift(), 0);
        let t = s.id("t");
        s.composite(|d| d.set_attribute(t, "class", "more").unwrap());
        assert_eq!(snapshot(&s.layout), snapshot(&s.fresh()));
    }

    #[test]
    fn a_markers_walk_leaves_the_lists_without_it_shared() {
        // M35.9: an outside marker is moved onto its line by a walk that
        // goes only into the subtree holding it: a box before that line,
        // whose list the cache shares, is not written.
        // #k has no line, so the marker's line is the paragraph's.
        let mut s = Session::new(
            "<ul><li id=l><div id=k><div></div></div><p>x</p></li></ul>",
            "",
        );
        s.pass();
        assert!(s.kept("k"));
        assert!(s.shares_its_lists());
    }

    #[test]
    fn the_bytes_count_each_list_of_fragments_once() {
        // M35.5: the cache's entries are the layout's subtrees, their
        // lists shared: what the cache adds is its entries, its boxes and
        // its widths, not the lists again.
        let mut s = Session::new("<div id=k><p>a</p><p>b</p></div><p>c</p>", "");
        s.pass();
        assert!(s.kept("k") && s.shares_its_lists());
        let alone = LayoutCache::new().heap_bytes(&s.layout);
        assert!(alone > 0);
        let entries = s.cache.entries.borrow().len()
            + s.cache.items.borrow().values().map(Vec::len).sum::<usize>();
        let widths = s.cache.widths.borrow().len() * std::mem::size_of::<(NodeId, (u64, Widths))>();
        assert_eq!(
            s.cache.heap_bytes(&s.layout),
            alone
                + entries * std::mem::size_of::<Entry>()
                + box_bytes(&s.cache.tree.borrow())
                + widths
        );
    }

    /// The item and cell layouts `change` makes the next pass lay out
    /// and take, and the content widths it measures and takes, in a
    /// session of `html` under `css` after a first pass.
    type Counts = ((usize, usize), (usize, usize));
    fn items_after(html: &str, css: &str, change: impl FnOnce(&mut Session)) -> Counts {
        let mut s = Session::new(html, css);
        s.pass();
        change(&mut s);
        s.pass();
        (s.cache.item_counts(), s.cache.width_counts())
    }

    /// Sets the text of the element `id` to `text`.
    fn set_text(s: &mut Session, id: &str, text: &str) {
        let node = s.id(id);
        s.change(|d| {
            let child = d.first_child(node).unwrap();
            d.set_text(child, text).unwrap();
        });
    }

    #[test]
    fn a_containers_items_and_a_tables_cells_nothing_changed_are_taken() {
        // M35.9: one item's text changed, the container is laid out
        // again, that item with it, the others taken from the cache
        // (`pass` holds each layout to a fresh one's).
        let flex = items_after(
            "<div id=f><p id=a>one</p><p id=b>two</p><p id=c>three</p></div>",
            "#f { display: flex } p { margin: 0 }",
            |s| set_text(s, "b", "two and more"),
        );
        let grid = items_after(
            "<div id=g><p id=a>one</p><p id=b>two</p><p id=c>three</p></div>",
            "#g { display: grid; grid-template-columns: 1fr 1fr 1fr } p { margin: 0 }",
            |s| set_text(s, "b", "two and more"),
        );
        // A table whose columns the text sizes gives every cell another
        // width; one laid out by the fixed algorithm does not.
        let table = items_after(
            "<table><tr><td id=a>one</td><td id=b>two</td><td id=c>three</td></tr></table>",
            "table { table-layout: fixed; width: 300px }",
            |s| set_text(s, "b", "two and more"),
        );
        // An item twice, measured and laid out; a cell once. The
        // changed item's content widths are measured, the others' taken.
        assert_eq!(flex, ((2, 4), (1, 2)));
        assert_eq!(grid, ((2, 4), (1, 2)));
        assert_eq!(table.0, (1, 2));
    }

    #[test]
    fn what_an_item_or_a_cell_is_laid_out_by_is_held_to_a_fresh_layout() {
        // M35.9 (`pass` holds each layout to a fresh one's): anonymous
        // items, which name their container and lay out alike in equal
        // columns, are not kept, or one would be given the other's
        // layout; a cell's border in the collapsed model is its table's
        // to change; a change deep in an item reaches its widths.
        let mut s = Session::new(
            "<div id=g>one<p>x</p>two and two</div>",
            "#g { display: grid; grid-template-columns: 1fr 1fr 1fr } p { margin: 0 }",
        );
        s.pass();
        s.pass();
        // Nor are their content widths, which would be another's.
        let mut s = Session::new(
            "<div id=f>short<p>x</p>a run of anonymous text a great deal longer</div>",
            "#f { display: flex } p { margin: 0 }",
        );
        s.pass();
        s.pass();
        let mut s = Session::new(
            "<table id=t><tr><td>one</td><td>two</td></tr></table>",
            "table { border-collapse: collapse; table-layout: fixed; width: 200px }              td { border: 1px solid #000 } .thick { border-top: 9px solid #000 }",
        );
        s.pass();
        let t = s.id("t");
        s.change(|d| d.set_attribute(t, "class", "thick").unwrap());
        s.pass();
        let mut s = Session::new(
            "<div id=f><div><span id=s>one</span></div><p>two</p></div>",
            "#f { display: flex } p { margin: 0 }",
        );
        s.pass();
        set_text(&mut s, "s", "one and a great deal more");
        s.pass();
    }

    #[test]
    fn a_build_takes_the_boxes_nothing_changed_from_the_tree_kept() {
        // M35.9: a paragraph appended, html's and body's boxes are built
        // anew and the three paragraphs' taken whole; a restyled element's
        // box and its descendants' are built anew (`pass` holds each tree
        // laid out to a fresh one's).
        let mut s = Session::new(
            "<p id=a>one</p><p id=b>two</p><div id=c><p>three</p></div>",
            "",
        );
        s.pass();
        let taken = s.cache.boxes_taken();
        let body = s.body();
        s.change(|d| {
            append_paragraph(d, body, "four");
        });
        s.pass();
        assert_eq!(s.cache.boxes_taken() - taken, 3);
        let (taken, c) = (s.cache.boxes_taken(), s.id("c"));
        s.change(|d| d.set_attribute(c, "style", "padding-left: 9px").unwrap());
        s.pass();
        // #a, #b and the fourth paragraph; not #c nor the paragraph in it.
        assert_eq!(s.cache.boxes_taken() - taken, 3);
        // A box whose own first line was styled is built anew, or the
        // build would style it again.
        let mut s = Session::new(
            "<div id=d>first line, and a great deal more after it, enough to wrap</div>",
            "#d::first-line { letter-spacing: 2px }",
        );
        s.pass();
        let body = s.body();
        s.change(|d| {
            append_paragraph(d, body, "after");
        });
        s.pass();
        // A float moved out of a flex container, where it was an item, is
        // a float again, whatever box it had.
        let mut s = Session::new(
            "<div id=f><div id=x>x</div><p>y</p></div><div id=b><p>z</p></div>",
            "#f { display: flex } #x { float: left; width: 10px } p { margin: 0 }",
        );
        s.pass();
        let (f, x, b) = (s.id("f"), s.id("x"), s.id("b"));
        s.change(|d| {
            d.remove_child(f, x).unwrap();
            d.insert_before(b, x, None).unwrap();
        });
        s.pass();
    }

    #[test]
    fn a_float_after_kept_boxes_leaves_their_lists_shared() {
        // M35.9: the floats get their places at the end of a pass, under
        // whichever box holds their fragments; a box with none under it,
        // whose list the cache shares, is not written, which would copy
        // it.
        let mut s = Session::new(
            "<div id=k><p>one</p><p>two</p></div><div id=f>float</div>",
            "#f { float: left; width: 50px }",
        );
        s.pass();
        assert!(s.kept("k") && !s.kept("f"));
        assert!(s.shares_its_lists());
    }
}
