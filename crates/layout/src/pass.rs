//! One layout pass's context (M17): what every function that lays a box
//! out in a pass shares, made by `block::layout_document` and its
//! siblings and dropped when the pass ends. That is the font collection
//! the pass measures text with, the flex items' layouts kept for the
//! rest of the pass, and the content widths of the boxes measured so
//! far.
//!
//! A flex item is laid out more than once, to measure it and then at its
//! flexed or stretched size, and a flex container inside an item each
//! time its item is: a chain of nested containers would cost two to the
//! power of its depth. So `crate::flex` lays each item out through
//! [`Pass::item`], which keeps the result under the box and every input
//! it depends on, and gives a clone of it when asked again, or lets a
//! step that only measures the item read it where it is kept. Nothing kept
//! outlives the pass, so nothing kept outlives the box tree it was made
//! from; the layout cache (M13.2) is what keeps results across passes,
//! and a layout the pass does not have it asks the cache for, by the
//! box's node and the same inputs, before laying the item out, and gives
//! the cache what it lays out (M35.9): a flex or grid item, or a table
//! cell, an element's own box, which no ancestor's pseudo-element
//! styles.
//!
//! A box's min-content and max-content widths are the same for the
//! whole pass, as they depend on nothing but the box and the fonts,
//! and measuring them walks the box's subtree. A flex item's are
//! measured for its automatic minimum size, and an item's content
//! holds the next container's items: measured afresh each time, a
//! chain of nested containers would measure the box at its foot once
//! for every container above it, and cost the square of its depth. So
//! [`crate::intrinsic::content_widths`] keeps them through
//! [`Pass::widths`], under the box alone.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use dom::NodeId;
use text::FontCollection;

use crate::block::{BlockLayout, Imposed};
use crate::box_tree::LayoutBox;
use crate::cache::{ItemInputs, LayoutCache};

/// The context of one layout pass over a box tree.
pub(crate) struct Pass<'a> {
    /// The fonts the pass measures and shapes text with.
    pub(crate) fonts: &'a FontCollection,
    /// The layout cache the pass takes from and keeps in, if any.
    cache: Option<&'a LayoutCache>,
    /// The flex items' layouts so far, by box and inputs.
    items: RefCell<HashMap<ItemKey, BlockLayout<'a>>>,
    /// The boxes' content widths measured so far, by box address.
    widths: RefCell<HashMap<usize, (f32, f32)>>,
    /// How many item layouts were computed rather than taken.
    computed: Cell<usize>,
    /// How many boxes' content widths were measured.
    measured: Cell<usize>,
}

/// What a flex item's layout depends on (M17): the box, by its address,
/// which is fixed while the pass borrows the tree; its content width;
/// the height imposed on it and whether that is definite; and its
/// containing block's width and height. Sizes are kept by their bits:
/// two layouts are the same only for the very same inputs.
///
/// The containing block's height counts only for a box whose `height`,
/// `min-height` or `max-height` is a percentage: nothing else in a box's
/// layout reads it. Its children resolve their percentages against the
/// box's own height, which is `auto` for them unless it is a length, a
/// percentage, or imposed; its relative offset is applied by its parent;
/// and the absolutely positioned boxes inside it are laid out against
/// their containing block's size once that is known. Leaving it out
/// otherwise is what keeps a chain of nested flex containers linear: a
/// container laid out at an indefinite height and again at a definite
/// one asks for its items with the same key both times.
///
/// What the layout cache keeps an item's layout by across passes, the
/// box's node for its address, it carries too: its node, its place in
/// tree order, the fingerprint of its generated text, and whether it is
/// an element's own box no ancestor's pseudo-element styles, which the
/// cache keeps; and a cell's border in the collapsed model, which its
/// table gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ItemKey {
    b: usize,
    content_width: u32,
    imposed: Option<(u32, bool)>,
    cb_width: u32,
    cb_height: Option<u32>,
    border: Option<[u32; 4]>,
    node: NodeId,
    tree_order: u32,
    generated_text: u64,
    kept: bool,
}

impl ItemKey {
    /// The key of `b` laid out at `content_width`, at the height
    /// `imposed`, in a containing block `cb_width` by `cb_height`.
    pub(crate) fn of(
        b: &LayoutBox,
        content_width: f32,
        imposed: Option<Imposed>,
        cb_width: f32,
        cb_height: Option<f32>,
    ) -> Self {
        let style = &b.style;
        let reads_cb_height = style.height.has_percent()
            || style.min_height.has_percent()
            || style.max_height.has_percent();
        ItemKey {
            b: std::ptr::from_ref(b) as usize,
            content_width: content_width.to_bits(),
            imposed: imposed.map(|i| (i.height.to_bits(), i.definite)),
            cb_width: cb_width.to_bits(),
            cb_height: cb_height.filter(|_| reads_cb_height).map(f32::to_bits),
            border: None,
            node: b.node,
            tree_order: b.tree_order,
            generated_text: b.generated_text,
            kept: !b.anonymous && !b.styled_by_ancestor && b.generated.is_none(),
        }
    }

    /// The key with a cell's border in the collapsed model, `border`.
    pub(crate) fn with_border(mut self, border: Option<[f32; 4]>) -> Self {
        self.border = border.map(|b| b.map(f32::to_bits));
        self
    }

    /// What the layout cache keeps the layout by, beside the node.
    fn inputs(&self) -> ItemInputs {
        ItemInputs {
            content_width: self.content_width,
            imposed: self.imposed,
            cb_width: self.cb_width,
            cb_height: self.cb_height,
            border: self.border,
            generated_text: self.generated_text,
        }
    }
}

impl<'a> Pass<'a> {
    /// A pass measuring text with `fonts`.
    #[cfg(test)]
    pub(crate) fn new(fonts: &'a FontCollection) -> Self {
        Pass::with_cache(fonts, None)
    }

    /// A pass measuring text with `fonts`, taking from `cache` and
    /// keeping in it.
    pub(crate) fn with_cache(fonts: &'a FontCollection, cache: Option<&'a LayoutCache>) -> Self {
        Pass {
            fonts,
            cache,
            items: RefCell::new(HashMap::new()),
            widths: RefCell::new(HashMap::new()),
            computed: Cell::new(0),
            measured: Cell::new(0),
        }
    }

    /// The flex item layout `key` names: the one kept, or `lay_out`'s,
    /// which is then kept. `lay_out` may lay other items out through the
    /// pass, so no borrow of the kept layouts is held while it runs.
    pub(crate) fn item(
        &self,
        key: ItemKey,
        lay_out: impl FnOnce() -> BlockLayout<'a>,
    ) -> BlockLayout<'a> {
        if let Some(layout) = self.items.borrow().get(&key) {
            return layout.clone();
        }
        let layout = self
            .taken(key)
            .unwrap_or_else(|| self.laid_out(key, lay_out));
        self.items.borrow_mut().insert(key, layout.clone());
        layout
    }

    /// The layout cache the pass takes from and keeps in, if any.
    pub(crate) fn cache(&self) -> Option<&'a LayoutCache> {
        self.cache
    }

    /// The layout `key` names, from the layout cache, if it keeps one.
    fn taken(&self, key: ItemKey) -> Option<BlockLayout<'a>> {
        let cache = self.cache.filter(|_| key.kept)?;
        cache.take_item(key.node, key.inputs(), key.tree_order)
    }

    /// `lay_out`'s layout, counted, and kept in the layout cache.
    fn laid_out(&self, key: ItemKey, lay_out: impl FnOnce() -> BlockLayout<'a>) -> BlockLayout<'a> {
        let layout = lay_out();
        self.computed.set(self.computed.get() + 1);
        if let Some(cache) = self.cache {
            cache.count_item_laid_out();
            if key.kept {
                cache.keep_item(key.node, key.inputs(), &layout);
            }
        }
        layout
    }

    /// What `read` takes from the flex item layout `key` names, the one
    /// kept or `lay_out`'s, which is then kept: [`Pass::item`] for a
    /// step that wants the layout's sizes and baselines, which reads it
    /// where it is kept rather than copying its fragments. A copy costs
    /// as much as the item's subtree, and a container laid out more
    /// than once, at each level of a chain of nested ones, would copy
    /// what is below it each time. `read` runs with the kept layouts
    /// borrowed, so it reads and lays nothing out.
    pub(crate) fn read_item<T>(
        &self,
        key: ItemKey,
        lay_out: impl FnOnce() -> BlockLayout<'a>,
        read: impl FnOnce(&BlockLayout<'a>) -> T,
    ) -> T {
        if let Some(layout) = self.items.borrow().get(&key) {
            return read(layout);
        }
        let layout = self
            .taken(key)
            .unwrap_or_else(|| self.laid_out(key, lay_out));
        let value = read(&layout);
        self.items.borrow_mut().insert(key, layout);
        value
    }

    /// `b`'s `(min-content, max-content)` widths: the ones kept, or
    /// `measure`'s, which are then kept. The box is borrowed for the
    /// whole pass, so no other box takes its address while the pass
    /// lasts. `measure` may measure other boxes through the pass, so
    /// no borrow of the kept widths is held while it runs.
    pub(crate) fn widths(
        &self,
        b: &'a LayoutBox,
        measure: impl FnOnce() -> (f32, f32),
    ) -> (f32, f32) {
        let key = std::ptr::from_ref(b) as usize;
        if let Some(&widths) = self.widths.borrow().get(&key) {
            return widths;
        }
        // Across passes, an element's own box's, by its node (M35.9).
        let cache = self
            .cache
            .filter(|_| !b.anonymous && !b.styled_by_ancestor && b.generated.is_none());
        let widths = match cache.and_then(|cache| cache.widths(b.node, b.generated_text)) {
            Some(widths) => widths,
            None => {
                let widths = measure();
                if let Some(cache) = cache {
                    cache.keep_widths(b.node, b.generated_text, widths);
                }
                widths
            }
        };
        self.widths.borrow_mut().insert(key, widths);
        widths
    }

    /// Counts a box's content widths measured: what measures them says
    /// so, for the tests to see each box measured once a pass, however
    /// the measuring is reached.
    pub(crate) fn measuring(&self) {
        self.measured.set(self.measured.get() + 1);
    }

    /// How many item layouts the pass has computed so far.
    #[cfg(test)]
    pub(crate) fn computed(&self) -> usize {
        self.computed.get()
    }

    /// How many boxes' content widths the pass has measured so far.
    #[cfg(test)]
    pub(crate) fn measured(&self) -> usize {
        self.measured.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::boxes_of;

    #[test]
    fn a_key_tells_every_input_apart() {
        // Nodes: html, body, div (a percentage height), p.
        let doc = html::parse_document("<div style='height: 50%'></div><p></p>")
            .expect("html parsing never fails");
        let (_, boxes) = boxes_of(&doc, &[]);
        let body = &boxes[0].children[0];
        let (div, p) = (&body.children[0], &body.children[1]);
        let imposed = |height, definite| Some(Imposed { height, definite });
        let key = ItemKey::of(p, 100.0, None, 200.0, Some(300.0));
        assert_eq!(key, ItemKey::of(p, 100.0, None, 200.0, Some(300.0)));
        assert_ne!(key, ItemKey::of(div, 100.0, None, 200.0, Some(300.0)));
        assert_ne!(key, ItemKey::of(p, 101.0, None, 200.0, Some(300.0)));
        assert_ne!(key, ItemKey::of(p, 100.0, imposed(10.0, true), 200.0, None));
        assert_ne!(
            ItemKey::of(p, 100.0, imposed(10.0, true), 200.0, None),
            ItemKey::of(p, 100.0, imposed(10.0, false), 200.0, None)
        );
        assert_ne!(
            ItemKey::of(p, 100.0, imposed(10.0, true), 200.0, None),
            ItemKey::of(p, 100.0, imposed(11.0, true), 200.0, None)
        );
        assert_ne!(key, ItemKey::of(p, 100.0, None, 201.0, Some(300.0)));
        // The containing block's height counts only for a box with a
        // percentage height.
        assert_eq!(key, ItemKey::of(p, 100.0, None, 200.0, None));
        assert_ne!(
            ItemKey::of(div, 100.0, None, 200.0, Some(300.0)),
            ItemKey::of(div, 100.0, None, 200.0, None)
        );
    }
}
