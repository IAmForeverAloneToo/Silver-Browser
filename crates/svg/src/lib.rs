//! SVG (M32, decision D23; see `docs/design/svg.md`): SVG 2's
//! microsyntaxes, the render tree an `<svg>` element's document and
//! computed styles give, and its drawing through the `canvas` crate's
//! 2D context, at any size and scale. It knows no layout and no script:
//! the engine gives it a document, its styles and a viewport, and
//! shows the bitmap it draws, for an inline `<svg>`, an SVG image and
//! an SVG document alike.

#![forbid(unsafe_code)]

mod draw;
mod syntax;
mod tree;

#[cfg(test)]
mod tests;

pub use draw::draw;
pub use syntax::{
    Align, AspectRatio, Axis, Length, Unit, ViewBox, Viewport, parse_length, parse_number,
    parse_numbers, parse_points, parse_preserve_aspect_ratio, parse_view_box, view_box_matrix,
};
pub use tree::{Fill, Group, Item, Paint, Shape, Stroke, Tree, build};

/// The most elements a render tree is built from, `<use>`s' instances
/// counted; past it, what was built is drawn.
pub const MAX_SVG_ELEMENTS: usize = 1 << 18;
/// The most groups drawn into layers of their own, nested; past it, an
/// opacity goes to each child.
pub const MAX_SVG_LAYERS: usize = 16;
/// The most pixels the layers of one drawing take, together; past it,
/// an opacity goes to each child, as past [`MAX_SVG_LAYERS`].
pub const MAX_SVG_LAYER_PIXELS: u64 = 1 << 28;
