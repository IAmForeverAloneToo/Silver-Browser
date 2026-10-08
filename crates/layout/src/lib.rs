//! Layout: box-tree construction and sizing algorithms.
//!
//! [`box_tree`] turns a `dom::Document` and its `style::Styles` into a
//! tree of [`LayoutBox`]es, each naming its node by its `dom::NodeId`
//! (M13), in which every block container's children are block-level (CSS2
//! §9.2.1.1's anonymous block boxes, M10.4); [`block`] (M6) lays
//! normal-flow block boxes out into a [`Fragment`] tree — real
//! width/margin resolution (CSS2.1 §10.3.3), margin collapsing (CSS2
//! §8.3.1), block formatting context roots; [`inline`] (M7) lays the
//! inline-level content out for real too — line boxes, greedy line
//! breaking against `text`-measured words, baseline alignment across
//! mixed font sizes, `text-align`, bidi — see that module's docs for
//! its own v1 scope (no `overflow-wrap`). The result is a
//! [`DocumentLayout`] (M9.1): a fragment tree in document coordinates
//! plus the content height the viewport scrolls over; `paint` consumes
//! it, applying the scroll offset. A [`LayoutCache`] (M13.2) keeps
//! block boxes' layouts from one pass to the next, so that a pass after
//! a change to the document lays out what the change reached and
//! reuses the rest.

mod align;
pub mod block;
pub mod box_tree;
mod cache;
mod composite;
pub mod decoration;
mod flex;
mod float;
mod grid;
pub mod inline;
mod intrinsic;
mod marker;
/// Pagination (M20): a laid-out document cut into pages.
pub mod paginate;
mod pass;
mod replaced;
mod table;

pub use block::{
    DocumentLayout, Fragment, FragmentKind, Preorder, StackingKey, flatten, layout,
    layout_document, layout_document_cached, layout_document_in, preorder,
};
pub use box_tree::{
    BoxKind, FirstLine, ImageState, Images, LayoutBox, MAX_GENERATED_TEXT, MarkerShape, Replaced,
    RowGroupKind, build_box_tree, build_box_tree_showing,
};
pub use cache::LayoutCache;
pub use composite::{Composited, EffectsChange};
pub use paginate::{MAX_PAGES, PageSlice, Paginated, paginate};
pub use replaced::ICON_SIZE;

#[cfg(test)]
pub(crate) mod testing {
    //! What the unit tests share: the pipeline from a tree to its boxes,
    //! and the numbers the tests name nodes by.

    use std::ops::Deref;

    use dom::{Document, Node, NodeId};

    use crate::{DocumentLayout, LayoutBox};

    /// The document `node` describes and its box tree, cascaded under
    /// the UA sheet and `sheets` with no viewport and no font.
    pub(crate) fn boxes_of(node: &Node, sheets: &[css::Stylesheet]) -> (Document, Vec<LayoutBox>) {
        let document = Document::from_node(node);
        let sheets = style::Stylesheets::new(sheets.to_vec());
        let context = style::CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let boxes = crate::build_box_tree(&document, &styles, &recascade);
        (document, boxes)
    }

    /// [`boxes_of`] with each `img` showing what `image` says of its
    /// `src`, and each image a style names what it says of its URL
    /// (M23.2, M23.3).
    pub(crate) fn boxes_with(
        node: &Node,
        sheets: &[css::Stylesheet],
        image: impl Fn(Option<&str>) -> crate::ImageState,
    ) -> (Document, Vec<LayoutBox>) {
        /// An `img`'s image by its `src`, a style's by its URL.
        struct Shown<'d, F>(&'d Document, F);
        impl<F: Fn(Option<&str>) -> crate::ImageState> crate::Images for Shown<'_, F> {
            fn image(&self, node: NodeId) -> crate::ImageState {
                (self.1)(self.0.attr(node, "src"))
            }

            fn url_image(&self, url: &str) -> crate::ImageState {
                (self.1)(Some(url))
            }
        }
        let document = Document::from_node(node);
        let sheets = style::Stylesheets::new(sheets.to_vec());
        let context = style::CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let boxes =
            crate::build_box_tree_showing(&document, &styles, &recascade, &Shown(&document, image));
        (document, boxes)
    }

    /// The document node and the elements, in tree order.
    fn numbered(document: &Document) -> impl Iterator<Item = NodeId> + '_ {
        document
            .subtree(document.root())
            .filter(|&id| id == document.root() || document.is_element(id))
    }

    /// The number the tests name `node` by: 0 for the document node,
    /// then the elements from 1 in tree order — the numbering nodes had
    /// before they had ids (M13), which the tests were written in.
    pub(crate) fn number(document: &Document, node: NodeId) -> usize {
        numbered(document)
            .position(|id| id == node)
            .unwrap_or_else(|| panic!("{node} is not numbered in the document"))
    }

    /// The node numbered `number`, as [`number`] counts.
    pub(crate) fn element(document: &Document, number: usize) -> NodeId {
        numbered(document)
            .nth(number)
            .unwrap_or_else(|| panic!("no node is numbered {number}"))
    }

    /// The node a box or fragment a test builds by hand names: the
    /// document node of an empty document.
    pub(crate) fn no_node() -> NodeId {
        Document::new().root()
    }

    /// A layout and the document it was laid out from, so that a test
    /// can name a fragment's node by its [`number`].
    pub(crate) struct Laid {
        pub(crate) document: Document,
        pub(crate) layout: DocumentLayout,
    }

    impl Laid {
        /// [`number`] in this layout's document.
        pub(crate) fn number(&self, node: NodeId) -> usize {
            number(&self.document, node)
        }

        /// [`element`] in this layout's document.
        pub(crate) fn element(&self, number: usize) -> NodeId {
            element(&self.document, number)
        }
    }

    impl Deref for Laid {
        type Target = DocumentLayout;

        fn deref(&self) -> &DocumentLayout {
            &self.layout
        }
    }
}
