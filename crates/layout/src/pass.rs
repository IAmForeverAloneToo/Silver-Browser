//! One layout pass's context (M17): what every function that lays a box
//! out in a pass shares, made by `block::layout_document` and its
//! siblings and dropped when the pass ends. Today that is the font
//! collection the pass measures text with.

use text::FontCollection;

/// The context of one layout pass over a box tree.
pub(crate) struct Pass<'a> {
    /// The fonts the pass measures and shapes text with.
    pub(crate) fonts: &'a FontCollection,
}

impl<'a> Pass<'a> {
    /// A pass measuring text with `fonts`.
    pub(crate) fn new(fonts: &'a FontCollection) -> Self {
        Pass { fonts }
    }
}
