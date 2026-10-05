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
//!   box, and the quote depth it starts at (M16), which the quote marks
//!   in it are chosen by;
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
//! An entry does not copy the fragments of the children the cache
//! holds too: it names each by its node and the serial its entry was
//! kept with, and the place its fragment had, and taking the entry
//! puts the children's fragments back from theirs. The cache's size is
//! so the document's, however deep its boxes nest. A child whose entry
//! has since gone or been replaced makes the entry useless, and the box
//! is laid out again.
//!
//! The fragments carry their boxes' places in tree order (M17), which
//! a change before a box moves without changing the box. Taking an
//! entry moves them along: every place in an unchanged subtree moves by
//! as much as the box's own, and a held child's is the box's and the
//! distance between the two when the entry was kept.
//!
//! Boxes laid out in other ways — inside an inline-block, a float, an
//! absolutely positioned box, a table cell, or a flex or grid item —
//! are laid out again with the box that holds them, which is itself
//! kept when it can be.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use dom::{Change, Document, NodeId};

use crate::block::{AdjoiningMargins, BlockLayout, Fragment, FragmentKind};

/// The layout results kept between passes, and what the last pass did.
#[derive(Debug, Default)]
pub struct LayoutCache {
    entries: RefCell<HashMap<NodeId, Entry>>,
    /// The serial the next entry kept gets: what tells a child's entry
    /// from one kept for the same node later.
    next_serial: Cell<u64>,
    laid_out: Cell<usize>,
    reused: Cell<usize>,
}

/// What a box's layout was computed from, beyond its subtree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Inputs {
    pub(crate) cb_width: f32,
    pub(crate) available_width: f32,
    pub(crate) cb_height: Option<f32>,
    pub(crate) quote_depth: u32,
}

/// A child of a box whose fragment the cache holds under the child's
/// node: one taken from the cache, or kept in it, as the box was laid
/// out.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Held {
    /// Its fragment's place among the box's fragment's children.
    pub(crate) index: usize,
    pub(crate) node: NodeId,
    /// The serial of the entry its fragment is in.
    pub(crate) serial: u64,
}

/// One box's kept result: a [`BlockLayout`] with nothing carried out,
/// its held children's fragments left out.
#[derive(Debug)]
struct Entry {
    serial: u64,
    inputs: Inputs,
    /// The box's fragment without the held children's.
    root: Fragment,
    /// The held children, in the order of their places.
    holes: Vec<Hole>,
    height: f32,
    margin_left: f32,
    margin_top: AdjoiningMargins,
    margin_bottom: AdjoiningMargins,
    top_edge: AdjoiningMargins,
    bottom_edge: AdjoiningMargins,
    self_collapsing: bool,
    last_baseline: Option<f32>,
    first_baseline: Option<f32>,
    inline_block_baseline: Option<f32>,
    auto_content_height: f32,
}

/// A held child left out of an entry: its place, its entry, where its
/// fragment was, and how far after the box it comes in tree order.
#[derive(Debug, Clone, Copy)]
struct Hole {
    index: usize,
    node: NodeId,
    serial: u64,
    x: f32,
    y: f32,
    after: u32,
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
    /// `rowspan` or `span` changed (what table layout reads), and every
    /// restyled element. An overflowed journal forgets everything; the
    /// entries of nodes freed since go too.
    pub fn invalidate(&mut self, document: &Document, changes: &[Change], restyled: &[NodeId]) {
        let entries = self.entries.get_mut();
        let mut forget = |node: NodeId| {
            entries.remove(&node);
            for ancestor in document.ancestors(node) {
                entries.remove(&ancestor);
            }
        };
        for change in changes {
            match change {
                Change::Everything => {
                    entries.clear();
                    return;
                }
                Change::Inserted { parent, .. } | Change::Removed { parent, .. } => forget(*parent),
                Change::TextChanged { node } => {
                    if let Some(parent) = document.parent(*node) {
                        forget(parent);
                    }
                }
                Change::AttributeChanged { element, name, .. } => {
                    if matches!(name.as_str(), "colspan" | "rowspan" | "span") {
                        forget(*element);
                    }
                }
            }
        }
        for &node in restyled {
            forget(node);
        }
        entries.retain(|node, _| document.contains(*node));
    }

    /// Starts a pass: its counts from zero.
    pub(crate) fn begin_pass(&self) {
        self.laid_out.set(0);
        self.reused.set(0);
    }

    /// Counts a box placed in flow and laid out.
    pub(crate) fn count_laid_out(&self) {
        self.laid_out.set(self.laid_out.get() + 1);
    }

    /// The kept result for `node`'s box laid out with `inputs`, its
    /// place in tree order now `tree_order`, and the serial of its
    /// entry, if there is one whose held children's entries are all
    /// still there.
    pub(crate) fn take<'a>(
        &self,
        node: NodeId,
        inputs: Inputs,
        tree_order: u32,
    ) -> Option<(BlockLayout<'a>, u64)> {
        let entries = self.entries.borrow();
        let entry = entries.get(&node).filter(|e| e.inputs == inputs)?;
        let root = rebuild(&entries, entry, tree_order)?;
        self.reused.set(self.reused.get() + 1);
        let layout = BlockLayout {
            root,
            height: entry.height,
            margin_left: entry.margin_left,
            margin_top: entry.margin_top,
            margin_bottom: entry.margin_bottom,
            top_edge: entry.top_edge,
            bottom_edge: entry.bottom_edge,
            self_collapsing: entry.self_collapsing,
            last_baseline: entry.last_baseline,
            first_baseline: entry.first_baseline,
            inline_block_baseline: entry.inline_block_baseline,
            absolutes: Vec::new(),
            auto_content_height: entry.auto_content_height,
            held: Vec::new(),
        };
        Some((layout, entry.serial))
    }

    /// Keeps `layout`, `node`'s box laid out with `inputs`, when `keep`
    /// says it may be (see the module documentation) and nothing was
    /// carried out of it, and returns its entry's serial; otherwise
    /// forgets any entry `node` had.
    pub(crate) fn keep(
        &self,
        node: NodeId,
        inputs: Inputs,
        layout: &BlockLayout<'_>,
        keep: bool,
    ) -> Option<u64> {
        let mut entries = self.entries.borrow_mut();
        if !keep || !layout.absolutes.is_empty() {
            entries.remove(&node);
            return None;
        }
        let serial = self.next_serial.get();
        self.next_serial.set(serial + 1);
        let mut root = childless(&layout.root);
        let mut holes = Vec::with_capacity(layout.held.len());
        let mut held = layout.held.iter().peekable();
        for (index, child) in layout.root.children.iter().enumerate() {
            match held.next_if(|h| h.index == index) {
                Some(h) => holes.push(Hole {
                    index,
                    node: h.node,
                    serial: h.serial,
                    x: child.x,
                    y: child.y,
                    after: child.tree_order.wrapping_sub(layout.root.tree_order),
                }),
                None => root.children.push(child.clone()),
            }
        }
        entries.insert(
            node,
            Entry {
                serial,
                inputs,
                root,
                holes,
                height: layout.height,
                margin_left: layout.margin_left,
                margin_top: layout.margin_top,
                margin_bottom: layout.margin_bottom,
                top_edge: layout.top_edge,
                bottom_edge: layout.bottom_edge,
                self_collapsing: layout.self_collapsing,
                last_baseline: layout.last_baseline,
                first_baseline: layout.first_baseline,
                inline_block_baseline: layout.inline_block_baseline,
                auto_content_height: layout.auto_content_height,
            },
        );
        Some(serial)
    }
}

/// `entry`'s fragment subtree whole, its box's place in tree order now
/// `tree_order`: its own fragments, moved along in tree order with it,
/// with each held child's put back in its place from the child's entry,
/// where it was. `None` when a held child's entry is gone or was
/// replaced.
fn rebuild(entries: &HashMap<NodeId, Entry>, entry: &Entry, tree_order: u32) -> Option<Fragment> {
    let by = tree_order.wrapping_sub(entry.root.tree_order);
    let mut root = childless(&entry.root);
    root.tree_order = tree_order;
    root.children
        .reserve(entry.root.children.len() + entry.holes.len());
    let mut own = entry.root.children.iter().map(|f| moved_along(f, by));
    for hole in &entry.holes {
        while root.children.len() < hole.index {
            root.children.push(own.next()?);
        }
        let child = entries
            .get(&hole.node)
            .filter(|e| e.serial == hole.serial)?;
        let mut fragment = rebuild(entries, child, tree_order.wrapping_add(hole.after))?;
        fragment.x = hole.x;
        fragment.y = hole.y;
        root.children.push(fragment);
    }
    root.children.extend(own);
    Some(root)
}

/// A copy of `fragment` and its subtree, every box's place in tree order
/// moved `by` on; a text fragment's, which is no box's, stays zero. A
/// subtree that has not moved, as when a box is appended after it, is
/// copied whole, which is cheaper than rebuilding it a fragment at a time.
fn moved_along(fragment: &Fragment, by: u32) -> Fragment {
    if by == 0 {
        return fragment.clone();
    }
    let mut moved = childless(fragment);
    if !matches!(moved.kind, FragmentKind::Text(_)) {
        moved.tree_order = moved.tree_order.wrapping_add(by);
    }
    moved.children = fragment
        .children
        .iter()
        .map(|child| moved_along(child, by))
        .collect();
    moved
}

/// A copy of `fragment` without its children.
fn childless(fragment: &Fragment) -> Fragment {
    Fragment {
        x: fragment.x,
        y: fragment.y,
        w: fragment.w,
        h: fragment.h,
        style: fragment.style.clone(),
        kind: fragment.kind.clone(),
        node: fragment.node,
        generated: fragment.generated,
        ascent: fragment.ascent,
        scroll_height: fragment.scroll_height,
        children: Vec::new(),
        tree_order: fragment.tree_order,
        stacking_parent: fragment.stacking_parent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{DocumentLayout, layout_document_cached, layout_document_in, preorder};
    use crate::build_box_tree;
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
                self.cache
                    .invalidate(&self.document, &changes, &restyle.changed);
            }
        }

        /// A pass with the cache, held to a pass over a full cascade
        /// without one: the boxes it laid out and those it reused.
        fn pass(&mut self) -> (usize, usize) {
            let context = CascadeContext::default();
            let recascade = self
                .sheets
                .recascader(&self.document, &self.styles, &context);
            let boxes = build_box_tree(&self.document, &self.styles, &recascade);
            let cached =
                layout_document_cached(&boxes, self.width, Some(HEIGHT), &self.fonts, &self.cache);
            let styles = self
                .sheets
                .cascade(&self.document, &CascadeContext::default());
            let recascade = self.sheets.recascader(&self.document, &styles, &context);
            let boxes = build_box_tree(&self.document, &styles, &recascade);
            let fresh = layout_document_in(&boxes, self.width, Some(HEIGHT), &self.fonts);
            assert_eq!(
                snapshot(&cached),
                snapshot(&fresh),
                "the cached layout differs from a fresh one"
            );
            self.layout = cached;
            self.cache.counts()
        }
    }

    /// Everything a layout says, to compare two by.
    fn snapshot(layout: &DocumentLayout) -> String {
        format!(
            "{} {} {:?}",
            layout.height, layout.depends_on_viewport_height, layout.roots
        )
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
        let mut s = Session::new(&"<div>x".repeat(100), "div { padding-left: 1px }");
        assert_eq!(s.pass(), (102, 0));
        // Every fragment of the layout is in exactly one entry.
        assert_eq!(stored(&s.cache), preorder(&s.layout.roots).count());
        assert_eq!(s.pass(), (0, 1));
    }

    /// How many fragments the cache's entries hold between them.
    fn stored(cache: &LayoutCache) -> usize {
        cache
            .entries
            .borrow()
            .values()
            .map(|e| preorder(std::slice::from_ref(&e.root)).count())
            .sum()
    }

    #[test]
    fn an_entry_whose_child_was_kept_again_since_is_not_taken() {
        // The context root's entry holds its paragraph's by serial. A
        // float before it narrows it: it is laid out again, not kept,
        // and its paragraph kept again at the narrower width. With the
        // float gone, the root's old entry fits its inputs again, but
        // not its paragraph's.
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
        assert_eq!(s.pass(), (4, 0));
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
    fn a_box_is_taken_only_at_the_quote_depth_it_was_kept_at() {
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
    }
}
