//! The paint layer: fragment tree in, display list out.
//!
//! Owns the display-list types — the render backend's input.
//! [`paint_fragments`] (M6.8) walks a real `layout::Fragment` tree:
//! backgrounds and borders painted from actual box geometry, not a
//! guess. Since M9 it paints for a [`Viewport`]: the list is in
//! viewport coordinates (the scroll offset subtracted, M9.1), items the
//! viewport does not meet are left out, and a box whose `overflow`
//! clips wraps its content in a [`DisplayItem::PushClip`]/[`DisplayItem::
//! PopClip`] pair at its padding box (M9.3). Since M10.5 such a box is
//! a scroll container: [`paint_fragments_in`] takes each one's scroll
//! offset ([`ScrollOffsets`]), moves its content up by it inside the
//! clip, and draws an overlay scrollbar over its padding box's right
//! edge when it can scroll. Since M11 the walk itself lives in [`order`]:
//! [`paint_order`] lists the steps — boxes, texts, clips, scrollbars —
//! in viewport coordinates, and this module only turns them into
//! display items, while the engine's hit test reads the same steps
//! backwards. Two earlier stopgaps remain, retained rather than deleted
//! (each milestone's own exit criteria asked for the *active* pipeline
//! to move on, not for the previous stage's code to disappear):
//! [`paint_document`] (M1) walks a `dom::Node` directly with hardcoded
//! per-tag presentation, and [`paint_styled_document`] (M5) walks a
//! `dom::Document` with its `style::Styles`, real cascaded
//! colors/font-sizes but still no real box model.

/// Gradients rendered (M23.3).
pub mod gradient;
pub mod order;

use std::collections::HashMap;
use std::sync::Arc;

use dom::{Document, Node, NodeData, NodeId};
use layout::{Fragment, FragmentKind, Mark, MarkerShape, Replaced};
use style::Styles;
use style::computed::{
    Attachment, BOTTOM, BackgroundBox, BackgroundSize, BorderImageRepeat, BorderImageWidth,
    BorderStyle, Color, Display, Image, ImageOrientation, LEFT, LengthPercentage, ObjectFit,
    OutlineStyle, Outset, RIGHT, Repeat, SliceSide, TOP,
};
use style::transform::Matrix;

pub use order::{Rect, Step, padding_box, paint_order, paint_order_in};

/// What names a group across frames (M19), which a backend keeps a
/// layer under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupKey {
    /// A stacking context's group: its box, by the node and the
    /// pseudo-element that generated it, as [`layout::StackingKey`]
    /// names it.
    Box(layout::StackingKey),
    /// The scrolled content of the scroll container of this element,
    /// one whose `will-change` names `scroll-position`.
    Scroll(NodeId),
    /// The document's content, one piece of it: the content is cut
    /// where a `position: fixed` box, which does not scroll, is painted
    /// among it, and each piece is a group of its own, numbered from
    /// zero in paint order.
    Document(u32),
    /// The bitmap of this `canvas` element (M30.3), alone.
    Canvas(NodeId),
}

/// The bit a canvas's image number has (M30.3), apart from the engine's
/// images' (counted from 1) and gradients' (the top bit): the rest is
/// the element's node.
pub const CANVAS_IMAGE: u64 = 1 << 62;

/// The image number the bitmap of the canvas `node` is drawn by.
pub fn canvas_image(node: NodeId) -> u64 {
    CANVAS_IMAGE | node.to_bits()
}

/// The canvas an image number names, if it names one.
pub fn canvas_node(id: u64) -> Option<NodeId> {
    if id & (1 << 63) != 0 || id & CANVAS_IMAGE == 0 {
        return None;
    }
    NodeId::from_bits(id & !CANVAS_IMAGE)
}

/// The bit an outer `<svg>`'s image number has (M32), apart from the
/// canvases' and gradients' bits: the rest is the element's node.
pub const SVG_IMAGE: u64 = 1 << 61;

/// The image number the drawing of the outer `<svg>` `node` is shown by.
pub fn svg_image(node: NodeId) -> u64 {
    SVG_IMAGE | node.to_bits()
}

/// The outer `<svg>` an image number names, if it names one.
pub fn svg_node(id: u64) -> Option<NodeId> {
    if id & (3 << 62) != 0 || id & SVG_IMAGE == 0 {
        return None;
    }
    NodeId::from_bits(id & !SVG_IMAGE)
}

/// Why a group is a layer (M19): the compositing note's policy, which
/// is a function of the computed style and the matrix alone, so that a
/// page's layers are the same from frame to frame until its styles
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerReason {
    /// Its matrix rotates, skews, or is 3D: the clips inside stay
    /// axis-aligned in the layer's own coordinates.
    Transformed,
    /// Its box's `will-change` names a transform property, `opacity`,
    /// `perspective` or `transform-style`: the author's word that the
    /// matrix or the opacity is about to change.
    WillChange,
    /// It is the scrolled content of a scroll container: the viewport's
    /// always, an element's when its `will-change` names
    /// `scroll-position`. A scroll is then a composite.
    Scrolled,
    /// It is a canvas's bitmap (M30.3), which scripts draw again from
    /// frame to frame: drawn again alone, the content beneath it not.
    Canvas,
}

/// The height of a tile of a scrolled layer (M19): a layer of scrolled
/// content is kept as strips of its full width and this many pixels
/// tall, so that it never holds a page of sixteen thousand pixels at
/// once.
pub const TILE_HEIGHT: u32 = 256;

/// The scrollbar track's colour, the viewport's and an element's alike.
pub const SCROLLBAR_TRACK: Rgba8 = Rgba8::rgb(0x31, 0x32, 0x44);
/// The scrollbar thumb's colour.
pub const SCROLLBAR_THUMB: Rgba8 = Rgba8::rgb(0x6c, 0x70, 0x86);
/// A scrollbar thumb never gets shorter than this, however long the
/// content, so it stays something a pointer can see and grab.
pub const SCROLLBAR_MIN_THUMB: f32 = 24.0;
/// The width of a scroll container's overlay scrollbar (M10.5), drawn
/// over the right edge of its padding box; it takes no space from the
/// content.
pub const ELEMENT_SCROLLBAR_WIDTH: f32 = 8.0;

/// Where each scroll container (M10.5) is scrolled to, by its element's
/// [`NodeId`] (M13): zero for one not scrolled. The engine keeps the
/// offsets; `()` is a document scrolled nowhere.
pub trait ScrollOffsets {
    fn offset(&self, node: NodeId) -> f32;
}

impl ScrollOffsets for () {
    fn offset(&self, _node: NodeId) -> f32 {
        0.0
    }
}

impl ScrollOffsets for HashMap<NodeId, f32> {
    fn offset(&self, node: NodeId) -> f32 {
        self.get(&node).copied().unwrap_or(0.0)
    }
}

/// A scrollbar thumb's top and height within a track `track` px long,
/// for content that scrolls `max_scroll` px past it and is scrolled to
/// `offset`: the thumb is the track's share of the whole, never shorter
/// than [`SCROLLBAR_MIN_THUMB`] nor longer than the track, placed in
/// proportion to the offset. The viewport's scrollbar (the engine's)
/// and an element's (M10.5) are the same shape.
pub fn scrollbar_thumb(track: f32, max_scroll: f32, offset: f32) -> (f32, f32) {
    let thumb_h = (track * track / (track + max_scroll))
        .max(SCROLLBAR_MIN_THUMB)
        .min(track);
    let thumb_y = if max_scroll > 0.0 {
        (track - thumb_h) * (offset / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (thumb_y, thumb_h)
}

/// An 8-bit RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

impl Rgba8 {
    /// An opaque color from 8-bit channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba8 { r, g, b, a: 255 }
    }
}

/// The image a [`DisplayItem::Image`] draws (M23.2): its number in the
/// engine, the frame shown, how its pixels are turned before they are
/// drawn, the part of them drawn, and the pixels, which the engine gives
/// once they are decoded. Two compare equal when they draw the same: one
/// image's same frame, turned and cut the same way, both with pixels or
/// both without.
#[derive(Debug, Clone)]
pub struct ImageRef {
    pub id: u64,
    pub frame: u32,
    /// What turns the decoded pixels, which are oriented, before they
    /// are drawn: `Normal`, but under `image-orientation: none` the
    /// inverse of the image's orientation.
    pub orientation: images::Orientation,
    /// The part of the turned image drawn (M23.3), a border image's
    /// slice: its left, top, right and bottom edges as fractions of the
    /// image's width and height, which a backend puts on whole pixels;
    /// `None` for all of it.
    pub crop: Option<[f32; 4]>,
    /// The decoded image; `None` until it is, when nothing is drawn.
    pub pixels: Option<Arc<images::Image>>,
}

impl PartialEq for ImageRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.frame == other.frame
            && self.orientation == other.orientation
            && self.crop == other.crop
            && self.pixels.is_some() == other.pixels.is_some()
    }
}

/// What paint asks of the images a style names (M23.3): the number and
/// header of the image at a URL, as a computed `<image>` has it, once its
/// header is read; `None` for one still coming, broken, or never asked
/// for. The engine's image store answers; `()` knows none.
pub trait Pictures {
    fn picture(&self, url: &str) -> Option<(u64, images::Header)>;

    /// Whether the image at `url` is still coming: neither read nor
    /// broken yet.
    fn coming(&self, _url: &str) -> bool {
        false
    }

    /// `gradient` rendered for a tile `w` by `h`: rendered each time it
    /// is asked for, unless the implementation keeps it, as the engine
    /// does.
    fn gradient(&self, gradient: &style::computed::Gradient, w: f32, h: f32) -> Arc<images::Image> {
        Arc::new(gradient::render(gradient, w, h))
    }
}

impl Pictures for () {
    fn picture(&self, _url: &str) -> Option<(u64, images::Header)> {
        None
    }
}

/// How an image whose header is `header` is drawn under `style`'s
/// `image-orientation` (CSS Images 3 section 5.1, M23): its size as
/// drawn, and what turns its decoded pixels, which are oriented: nothing,
/// but under `none` the inverse of its orientation, its size then the
/// size it is stored at. An `img`'s, a background's, a border image's and
/// a marker's alike.
fn oriented(
    style: &style::ComputedStyle,
    header: &images::Header,
) -> ((f32, f32), images::Orientation) {
    let (w, h) = (header.width as f32, header.height as f32);
    if style.image().image_orientation != ImageOrientation::None {
        return ((w, h), images::Orientation::Normal);
    }
    let size = if header.orientation.swaps_sides() {
        (h, w)
    } else {
        (w, h)
    };
    (size, header.orientation.inverse())
}

/// The colour of the icon of an image that cannot be shown.
pub const ICON_COLOUR: Rgba8 = Rgba8::rgb(0x8c, 0x8c, 0x8c);
/// The colour of the border around a box the icon is in, when the box
/// is larger than the icon.
pub const ICON_BOX_COLOUR: Rgba8 = Rgba8::rgb(0xc0, 0xc0, 0xc0);

/// One drawable operation in a display list. Items compare by value:
/// two equal items draw the same pixels over the same pixels under the
/// same clip, which `render` relies on when it compares a frame's list
/// with the last one's to redraw only where they differ (M13.3).
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Fill an axis-aligned rectangle.
    Rect {
        /// Left edge, px from viewport left.
        x: f32,
        /// Top edge, px from viewport top.
        y: f32,
        /// Width in px.
        w: f32,
        /// Height in px.
        h: f32,
        /// Fill color.
        color: Rgba8,
    },
    /// Draw a line of text. `y` is the BASELINE, not the top.
    Text {
        /// The characters to draw (shaping happens at raster time for
        /// now; shaped runs move here once line breaking lands).
        text: String,
        /// Left edge of the run, px from viewport left.
        x: f32,
        /// Baseline, px from viewport top: a whole pixel.
        y: f32,
        /// Font size in px.
        size: f32,
        /// Text color.
        color: Rgba8,
    },
    /// Fill a convex quadrilateral, without anti-aliasing, so that
    /// quads sharing an edge tile it exactly: the bands of a border
    /// whose four sides are one colour (M10). Points in viewport
    /// coordinates, in order around the shape.
    Quad {
        points: [(f32, f32); 4],
        color: Rgba8,
    },
    /// Fill a simple polygon, anti-aliased: a border's run of sides in
    /// one colour, from the mitre to a side of another colour to the
    /// next (M11). Its axis-aligned edges lie on whole pixels, so only
    /// the mitres blend, as a browser draws them. Points in viewport
    /// coordinates, in order around the shape.
    Polygon {
        points: Vec<(f32, f32)>,
        color: Rgba8,
    },
    /// Fill the band between the rectangle at `(x, y)` of size `w` by
    /// `h` and the rectangle inset from it by `widths`, top, right,
    /// bottom and left, as one shape, anti-aliased (M19): a border
    /// solid in one colour all round. Its edges lie on whole pixels,
    /// where it is the four strips it was (M6.8); under a scale they
    /// may not, and a pixel at a corner is then covered once, by the
    /// share of it the band covers, where two strips over each other
    /// would cover it twice.
    Border {
        /// Left edge, px from viewport left.
        x: f32,
        /// Top edge, px from viewport top.
        y: f32,
        /// Width in px.
        w: f32,
        /// Height in px.
        h: f32,
        /// The band's widths, top, right, bottom and left, in px.
        widths: [f32; 4],
        /// Fill color.
        color: Rgba8,
    },
    /// Clip everything up to the matching [`DisplayItem::PopClip`] to
    /// this rectangle (M9.3), intersected with any clip already in
    /// force. Clips nest; an unmatched pop is ignored.
    PushClip {
        /// Left edge, px from viewport left.
        x: f32,
        /// Top edge, px from viewport top.
        y: f32,
        /// Width in px.
        w: f32,
        /// Height in px.
        h: f32,
    },
    /// End the innermost [`DisplayItem::PushClip`].
    PopClip,
    /// Everything up to the matching [`DisplayItem::PopGroup`] is one
    /// group (M19): a stacking context with a transform or an opacity
    /// below one. The items inside are in the group's own coordinates,
    /// the viewport coordinates the box would have had with no
    /// transform at all, and `matrix` maps them to the coordinates the
    /// group sits in, those of the group around it or the viewport's;
    /// the group is composited with `opacity`, so that its items do not
    /// show through one another (css-color-4 §3.2). A clip inside the
    /// group is in the group's coordinates too. Groups nest, their
    /// matrices composing from the inside out; an unmatched pop is
    /// ignored. The backend draws a group through the raster it sits
    /// in when the matrix only translates and scales and the opacity is
    /// one, and through a scratch of its own otherwise.
    PushGroup {
        /// From the group's coordinates to those it sits in.
        matrix: Matrix,
        /// In `[0, 1]`.
        opacity: f32,
        /// What names the group across frames.
        key: GroupKey,
        /// Whether the group is a layer, kept from frame to frame under
        /// its key and composited by its matrix, and why; `None` for a
        /// group drawn through, or from a scratch that is not kept. A
        /// backend may still draw a layer the other way, when its
        /// budget has no room for it.
        layer: Option<LayerReason>,
    },
    /// End the innermost [`DisplayItem::PushGroup`].
    PopGroup,
    /// Draw an image (M23.2) into the rectangle at `(x, y)` of size `w`
    /// by `h`, stretched to it, each pixel of the raster whose centre is
    /// inside the rectangle and the clip taking the image's colour
    /// there, the pixels between texel centres blended, or the nearest
    /// texel taken where `sharp`; nothing until its pixels are decoded.
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        image: ImageRef,
        /// `image-rendering: pixelated` or `crisp-edges`.
        sharp: bool,
    },
    /// Tile an image over the rectangle at `(x, y)` of size `w` by `h`
    /// (M23.3): a tile at `tile`'s `(x, y)` of its size, the image
    /// stretched over it, repeated along x every tile width plus
    /// `gap[0]`, both ways, where `repeat[0]`, and along y likewise; each
    /// pixel whose centre is in the rectangle and a tile taking the
    /// image's colour there, as [`DisplayItem::Image`] does. One item
    /// however many tiles: a background layer.
    Pattern {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tile: [f32; 4],
        gap: [f32; 2],
        repeat: [bool; 2],
        image: ImageRef,
        sharp: bool,
    },
}

/// The window onto the document a display list is painted for (M9.1):
/// its size, and how far down the document it is scrolled. The list's
/// coordinates are viewport coordinates: a fragment at document `y`
/// paints at `y - scroll_y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    /// How far the document is scrolled down, in px; never negative.
    pub scroll_y: f32,
}

impl Viewport {
    /// A viewport at the top of the document.
    pub fn new(width: f32, height: f32) -> Self {
        Viewport {
            width,
            height,
            scroll_y: 0.0,
        }
    }
}

/// An ordered list of drawable operations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    /// Items in paint order (later items composite over earlier ones).
    pub items: Vec<DisplayItem>,
}

/// Collects `(parent_tag, text)` pairs from all text nodes, in document
/// order. `style` and `script` subtrees are skipped: their content is not
/// rendered text.
fn collect_text_nodes(node: &Node, out: &mut Vec<(String, String)>) {
    match node {
        Node::Document(children) => {
            for child in children {
                collect_text_nodes(child, out);
            }
        }
        Node::Element(el) => {
            if el.tag == "style" || el.tag == "script" {
                return;
            }
            for child in &el.children {
                match child {
                    Node::Text(t) => out.push((el.tag.clone(), t.data.clone())),
                    other => collect_text_nodes(other, out),
                }
            }
        }
        Node::Text(_) => {}
    }
}

/// Heading text color.
const HEADING: Rgba8 = Rgba8::rgb(0xcd, 0xd6, 0xf4);
/// Body text color.
const BODY: Rgba8 = Rgba8::rgb(0xa6, 0xad, 0xc8);

/// Pragmatic per-tag presentation: (font size, color).
fn style_for(tag: &str) -> (f32, Rgba8) {
    match tag {
        "h1" => (28.0, HEADING),
        "h2" | "h3" | "h4" | "h5" | "h6" => (22.0, HEADING),
        _ => (16.0, BODY),
    }
}

/// Paints a document into a display list for a `viewport_w` x `viewport_h`
/// viewport.
///
/// Milestone-2 presentation: dark background, an accent bar, and text
/// runs stacked top-to-bottom, sized and colored by their parent tag
/// (`h1` heading, other headings smaller, everything else body). Real
/// styling arrives with the `style` + `layout` milestones.
pub fn paint_document(root: &Node, viewport_w: f32, viewport_h: f32) -> DisplayList {
    let mut items = Vec::new();

    // Background.
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_w,
        h: viewport_h,
        color: Rgba8::rgb(0x1e, 0x1e, 0x2e),
    });
    // Accent bar.
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: 4.0,
        h: viewport_h,
        color: Rgba8::rgb(0x89, 0xb4, 0xfa),
    });

    let mut texts = Vec::new();
    collect_text_nodes(root, &mut texts);
    texts.retain(|(_, data)| !data.trim().is_empty());
    if texts.is_empty() {
        texts.push(("h1".to_string(), "Hello from Silver Browser".to_string()));
    }

    // Stack text runs top-to-bottom, sized by their tag.
    let mut baseline = 48.0;
    for (tag, data) in &texts {
        let (size, color) = style_for(tag);
        items.push(DisplayItem::Text {
            text: data.clone(),
            x: 40.0,
            y: baseline,
            size,
            color,
        });
        baseline += size * 1.6;
    }

    DisplayList { items }
}

/// The left edge every text run starts at — matches [`paint_document`]'s
/// fixed indent (there's no real inline layout yet to compute one).
const TEXT_X: f32 = 40.0;
/// Where the first line's baseline sits.
const TOP_INSET: f32 = 32.0;

/// Paints `document` with its cascaded `styles` (M5's cascade output)
/// into a display list for a `viewport_w` x `viewport_h` viewport: the
/// same dark background + accent bar chrome as [`paint_document`], then
/// every text run in document order, sized and colored by *its own*
/// cascaded style rather than a per-tag lookup table — real user CSS
/// visibly changes the screen here (roadmap M5's exit criterion).
/// `display: none` skips an element and everything under it, per spec,
/// and so does having no style in `styles`.
pub fn paint_styled_document(
    document: &Document,
    styles: &Styles,
    viewport_w: f32,
    viewport_h: f32,
) -> DisplayList {
    let mut items = Vec::new();
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_w,
        h: viewport_h,
        color: Rgba8::rgb(0x1e, 0x1e, 0x2e),
    });
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: 4.0,
        h: viewport_h,
        color: Rgba8::rgb(0x89, 0xb4, 0xfa),
    });

    let mut cursor_y = TOP_INSET;
    paint_children(document, styles, document.root(), &mut items, &mut cursor_y);

    if items.len() <= 2 {
        // Nothing at all rendered (e.g. a document with no visible
        // text): same fallback as `paint_document`, so an empty/failed
        // load is never a silently blank window.
        items.push(DisplayItem::Text {
            text: "Hello from Silver Browser".to_string(),
            x: TEXT_X,
            y: TOP_INSET,
            size: 16.0,
            color: Rgba8::rgb(0xa6, 0xad, 0xc8),
        });
    }

    DisplayList { items }
}

/// Walks `idx`'s children in document order, emitting a `Text` item per
/// non-blank text run (colored/sized by `idx`'s own computed style — the
/// nearest element ancestor's is what a text node visually takes) and
/// recursing into element children, applying `margin` top/bottom as
/// simple additive vertical spacing (no collapsing, no box model — a
/// placeholder for the real thing layout builds in M6).
fn paint_children(
    document: &Document,
    styles: &Styles,
    idx: NodeId,
    items: &mut Vec<DisplayItem>,
    cursor_y: &mut f32,
) {
    for child in document.children(idx) {
        match document.data(child) {
            Some(NodeData::Element { .. }) => {
                let Some(style) = styles.get(child) else {
                    continue; // never styled: nothing to paint it with
                };
                if style.display == Display::None {
                    continue; // the element and its whole subtree: skipped
                }
                // `margin: auto` has no meaning in this simplified,
                // horizontal-layout-free flow (real auto-margin
                // centering needs `layout`'s box-width math, M6) — treat
                // it as 0 here, same as an unset margin.
                *cursor_y += style.margin[style::computed::TOP].resolve(0.0, 0.0);
                paint_children(document, styles, child, items, cursor_y);
                *cursor_y += style.margin[style::computed::BOTTOM].resolve(0.0, 0.0);
            }
            Some(NodeData::Text(text)) => {
                if text.trim().is_empty() {
                    continue;
                }
                let Some(style) = styles.get(idx) else {
                    continue;
                };
                items.push(DisplayItem::Text {
                    text: text.clone(),
                    x: TEXT_X,
                    y: *cursor_y,
                    size: style.font_size_px,
                    color: to_rgba8(style.color),
                });
                *cursor_y += style.font_size_px * 1.6; // matches paint_document's line spacing
            }
            Some(NodeData::Document) | None => {}
        }
    }
}

/// Converts `style`'s 0.0–1.0-channel color to `paint`'s 8-bit form —
/// `style` owns its own color type (the layering contract: types belong
/// to their producer), so this is the one place that boundary is crossed.
fn to_rgba8(c: Color) -> Rgba8 {
    let scale = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8 {
        r: scale(c.r),
        g: scale(c.g),
        b: scale(c.b),
        a: scale(c.a),
    }
}

/// Paints a real `layout::Fragment` tree (M6.7) into a display list for
/// `viewport` — "paint v2" (roadmap M6.8): backgrounds and borders
/// painted from actual fragment geometry (box position/size resolved by
/// real width/margin/collapsing math), not a per-tag guess or a flat
/// vertical list. A transparent background paints nothing (an `Rgba8`
/// with `a: 0` would be a no-op in `render` anyway, but skipping it
/// here avoids pushing dead display items for the overwhelming majority
/// of boxes, which have no background at all).
///
/// The list is in viewport coordinates (M9.1): `viewport.scroll_y` is
/// subtracted from every document `y`. A fragment the viewport does not
/// meet produces no item (M9.5's cheap half: paint is proportional to
/// the document, raster to the viewport), though its content is still
/// walked, since content may overflow a box into view — except under
/// a box whose `overflow` clips (M9.3), whose content is wrapped in a
/// clip at its padding box and skipped altogether when that box is out
/// of view. `roots` is the tree `layout::layout_document` returns; a
/// flat list works too, as a forest of childless fragments. No scroll
/// container is scrolled; [`paint_fragments_in`] takes their offsets.
pub fn paint_fragments(roots: &[Fragment], viewport: Viewport) -> DisplayList {
    paint_fragments_in(roots, viewport, &())
}

/// [`paint_fragments`] with each scroll container (M10.5) scrolled as
/// `offsets` says: its content is moved up by its offset inside its
/// clip, and when it can scroll — `overflow: scroll` always, `auto`
/// when its content overflows, `hidden` never — an overlay scrollbar is
/// drawn over its padding box's right edge, after its content, as an
/// indicator that takes no space from it (vertical only, like the
/// viewport's; a control from M26). The list follows [`paint_order`]
/// step for step (M11): what is painted, where and in which order is
/// decided there, once for painting and hit testing both.
pub fn paint_fragments_in(
    roots: &[Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
) -> DisplayList {
    paint_fragments_with(roots, viewport, offsets, &())
}

/// [`paint_fragments_in`] with the background images `pictures` knows
/// (M23.3).
pub fn paint_fragments_with(
    roots: &[Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
    pictures: &dyn Pictures,
) -> DisplayList {
    // CSS2 §14.2: the root element's background paints the canvas, the
    // whole viewport; when the root's is transparent, the body's — the
    // root's first box child, in an HTML document — does instead and is
    // not painted on the body itself. The canvas is white otherwise.
    let canvas = canvas_background(roots);
    let canvas_node = canvas.map(|f| f.node);
    let white = Rgba8::rgb(0xff, 0xff, 0xff);
    let mut items = vec![DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport.width,
        h: viewport.height,
        color: canvas
            .map(|f| to_rgba8(f.style.background_color))
            .filter(|c| c.a > 0)
            .unwrap_or(white),
    }];
    // The viewport in the document's coordinates, which a fixed
    // background is placed in (M23.3), and the canvas the canvas's
    // images cover: the document from its top to the bottom of its last
    // top-level box, and a viewport more for the margins after it. It is
    // the same whatever the scroll offset, so that a scroll draws them
    // from the document's layer (M19) rather than anew.
    let mut backdrop = Backdrop {
        pictures,
        offsets,
        viewport: Rect {
            x: 0.0,
            y: viewport.scroll_y,
            w: viewport.width,
            h: viewport.height,
        },
        in_transform: false,
    };
    // Whether each group open is a transformed box's.
    let mut transformed: Vec<bool> = Vec::new();
    let bottom = roots.iter().map(|f| f.top() + f.h).fold(0.0, f32::max);
    let canvas_area = Rect {
        x: 0.0,
        y: 0.0,
        w: viewport.width,
        h: bottom + viewport.height,
    };
    // The document's content comes in groups that scroll it (M19), the
    // items inside in document coordinates and the group's matrix the
    // scroll offset, and the content a viewport's height above and
    // below the viewport is listed too, for the tiles a scroll brings
    // in next. A `position: fixed` box, which does not scroll, is
    // outside them, in viewport coordinates.
    let steps = paint_order_in(roots, viewport, offsets, viewport.height);
    // A fixed root is painted outside the document's groups, which are
    // then empty and left out: its canvas's images go under everything,
    // over the viewport, which it does not scroll with.
    let document = steps.iter().any(|step| {
        matches!(
            step,
            Step::PushGroup {
                key: GroupKey::Document(0),
                ..
            }
        )
    });
    if !document && let (Some(canvas), Some(root)) = (canvas, roots.first()) {
        let root_rect = snapped(Rect {
            x: root.x,
            y: root.top(),
            w: root.w,
            h: root.h,
        });
        push_layers(
            &mut items,
            canvas,
            Placed {
                by: root,
                border: root_rect,
                cb_width: viewport.width,
                area: Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: viewport.width,
                    h: viewport.height,
                }),
            },
            &backdrop,
        );
    }
    for step in steps {
        match step {
            Step::Box {
                fragment,
                rect,
                cb_width,
            } => push_box(&mut items, fragment, rect, cb_width, canvas_node, &backdrop),
            Step::Text {
                fragment,
                rect,
                baseline,
            } => match &fragment.kind {
                FragmentKind::Text(text) => {
                    // The baseline on the pixel grid, as a browser puts it
                    // and as a box's edges are (M11): a glyph's edge then
                    // falls in the same pixel rows whatever fraction the
                    // layout gave the line (M17). Across, the run keeps
                    // its fraction. Its decorations (M21.7) are drawn from
                    // it too: the underlines and overlines below the text,
                    // the line-throughs over it.
                    let baseline = snap(baseline);
                    let decorated = fragment.decorated.as_deref();
                    if let Some(decorated) = decorated {
                        push_decorations(&mut items, decorated, rect, baseline, false);
                    }
                    if !text.trim().is_empty() {
                        items.push(DisplayItem::Text {
                            text: text.clone(),
                            x: rect.x,
                            y: baseline,
                            size: fragment.style.font_size_px,
                            color: to_rgba8(fragment.style.color),
                        });
                    }
                    if let Some(decorated) = decorated {
                        push_decorations(&mut items, decorated, rect, baseline, true);
                    }
                }
                FragmentKind::Marker(shape) => {
                    push_marker(&mut items, *shape, rect, to_rgba8(fragment.style.color));
                }
                FragmentKind::Replaced(replaced) => {
                    push_replaced(&mut items, fragment, replaced, rect, pictures);
                }
                _ => {}
            },
            Step::PushClip { rect, .. } => {
                let rect = snapped(rect);
                items.push(DisplayItem::PushClip {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                })
            }
            Step::PopClip => items.push(DisplayItem::PopClip),
            Step::PushGroup {
                fragment,
                matrix,
                opacity,
                key,
                layer,
            } => {
                transformed.push(fragment.is_some_and(|f| f.style.effects().has_transform()));
                backdrop.in_transform = transformed.contains(&true);
                items.push(DisplayItem::PushGroup {
                    matrix,
                    opacity,
                    key,
                    layer,
                });
                // The canvas's images (CSS Backgrounds 3 section 2.11.2):
                // under the whole document, which they scroll with, the
                // root's box placing them.
                if key == GroupKey::Document(0)
                    && let (Some(canvas), Some(root)) = (canvas, roots.first())
                {
                    let root_rect = snapped(Rect {
                        x: root.x,
                        y: root.top(),
                        w: root.w,
                        h: root.h,
                    });
                    push_layers(
                        &mut items,
                        canvas,
                        Placed {
                            by: root,
                            border: root_rect,
                            cb_width: viewport.width,
                            area: Some(canvas_area),
                        },
                        &backdrop,
                    );
                }
            }
            Step::PopGroup => {
                transformed.pop();
                backdrop.in_transform = transformed.contains(&true);
                items.push(DisplayItem::PopGroup)
            }
            Step::Scrollbar {
                fragment,
                rect,
                offset,
            } => push_scrollbar(&mut items, fragment, snapped(rect), offset),
        }
    }

    DisplayList { items }
}

/// The fragment whose background the canvas takes (CSS2 §14.2): the
/// root element's when it has one, else the body's — the first box
/// among the root's children that names another element, in an HTML
/// document. The root's box is the first of `roots`, a block or a table
/// (the cascade blockifies the root); among its children, an anonymous
/// box and a box the root's `::before` or `::after` generated (M16)
/// name the root, and are passed over.
fn canvas_background(roots: &[Fragment]) -> Option<&Fragment> {
    fn is_box(f: &Fragment) -> bool {
        matches!(f.kind, FragmentKind::Box)
    }
    fn with_background(f: &Fragment) -> Option<&Fragment> {
        let images = f
            .style
            .background
            .as_ref()
            .is_some_and(|b| b.image.iter().any(Option::is_some));
        (f.style.background_color.a > 0.0 || images).then_some(f)
    }
    let root = roots.first().filter(|f| is_box(f))?;
    with_background(root).or_else(|| {
        root.children
            .iter()
            .find(|f| is_box(f) && f.node != root.node)
            .and_then(with_background)
    })
}

/// A scroll container's overlay scrollbar (M10.5) over the right edge
/// of its padding box `rect`, in viewport coordinates. The walk says
/// which boxes have one: `overflow: scroll` always, `auto` only with
/// content past the box, `hidden` never.
fn push_scrollbar(items: &mut Vec<DisplayItem>, f: &Fragment, rect: Rect, offset: f32) {
    let max = f.max_scroll();
    let Rect { x, y, w, h } = rect;
    if w < ELEMENT_SCROLLBAR_WIDTH || h <= 0.0 {
        return;
    }
    let bar_x = x + w - ELEMENT_SCROLLBAR_WIDTH;
    items.push(DisplayItem::Rect {
        x: bar_x,
        y,
        w: ELEMENT_SCROLLBAR_WIDTH,
        h,
        color: SCROLLBAR_TRACK,
    });
    let (thumb_y, thumb_h) = scrollbar_thumb(h, max, offset);
    items.push(DisplayItem::Rect {
        x: bar_x + 1.0,
        y: y + thumb_y,
        w: ELEMENT_SCROLLBAR_WIDTH - 2.0,
        h: thumb_h,
        color: SCROLLBAR_THUMB,
    });
}

/// A position on the pixel grid (M11): rounded to the nearest whole
/// device pixel, half up.
fn snap(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// The text decoration lines of a text fragment whose line box is
/// `rect` and whose baseline is `baseline`, on the pixel grid (M21.7):
/// its line-throughs when `through`, else its underlines and overlines,
/// outermost element's first, from `decorated.left` before the fragment
/// to `decorated.right` after it. A solid line is a rectangle as thick as
/// the line; a double one two of them; a dotted one squares of its
/// thickness a thickness apart; a dashed one dashes three times as long
/// as it is thick and as far apart; and a wavy one a wave of its
/// thickness, its crests two thicknesses and a pixel apart.
fn push_decorations(
    items: &mut Vec<DisplayItem>,
    decorated: &layout::decoration::Decorated,
    rect: Rect,
    baseline: f32,
    through: bool,
) {
    use layout::decoration::LineKind;
    use style::computed::DecorationStyle;
    let (x0, x1) = (
        snap(rect.x - decorated.left),
        snap(rect.x + rect.w + decorated.right),
    );
    if x1 <= x0 {
        return;
    }
    for line in decorated.lines.iter() {
        if (line.kind == LineKind::LineThrough) != through {
            continue;
        }
        let color = to_rgba8(line.color);
        let t = line.thickness;
        let top = baseline + line.top;
        let bar = |items: &mut Vec<DisplayItem>, x: f32, w: f32, y: f32, h: f32| {
            let w = w.min(x1 - x);
            if w > 0.0 {
                items.push(DisplayItem::Rect { x, y, w, h, color });
            }
        };
        match line.style {
            DecorationStyle::Solid => bar(items, x0, x1 - x0, top, t),
            DecorationStyle::Double => {
                bar(items, x0, x1 - x0, top, t);
                bar(
                    items,
                    x0,
                    x1 - x0,
                    baseline + line.second.unwrap_or(line.top + t + 1.0),
                    t,
                );
            }
            DecorationStyle::Dotted | DecorationStyle::Dashed => {
                let (on, off) = if line.style == DecorationStyle::Dotted {
                    (t, t)
                } else {
                    (3.0 * t, 3.0 * t)
                };
                let mut x = x0;
                while x < x1 {
                    bar(items, x, on, top, t);
                    x += on + off;
                }
            }
            DecorationStyle::Wavy => {
                // Half a wavelength between a crest and a trough, the
                // wave's middle at the line's.
                let half = 2.0 * t + 1.0;
                let amplitude = t;
                let middle = top + t / 2.0;
                let wave =
                    |x: f32| middle - amplitude * (std::f32::consts::PI * (x - x0) / half).sin();
                let steps = ((x1 - x0) * 2.0).ceil().max(1.0) as usize;
                let at = |i: usize| x0 + (x1 - x0) * i as f32 / steps as f32;
                let mut points: Vec<(f32, f32)> = (0..=steps)
                    .map(|i| (at(i), wave(at(i)) - t / 2.0))
                    .collect();
                points.extend((0..=steps).rev().map(|i| (at(i), wave(at(i)) + t / 2.0)));
                items.push(DisplayItem::Polygon { points, color });
            }
        }
    }
}

/// A list item's marker drawn as `shape` in `color` (M21.6), in the
/// square `rect` snapped to whole pixels, as the reference browser draws
/// it: a filled circle in the square, a circle stroked a pixel wide
/// along the square's inscribed circle, half a pixel either side of it,
/// or the square filled. A circle is a polygon of 32 sides,
/// anti-aliased; the stroked one goes round the outer circle and back
/// round the inner, which the fill's signed area leaves open.
/// The size an image `natural` in size is drawn at in a content box
/// `room` in size under `fit` (CSS Images 3 section 4.5): the box's,
/// the largest that fits in it or the smallest that covers it with the
/// image's ratio kept, the natural size, or the smaller of that and the
/// contained size.
pub fn fitted(fit: ObjectFit, natural: (f32, f32), room: (f32, f32)) -> (f32, f32) {
    let (nw, nh) = natural;
    let (bw, bh) = room;
    let scaled = |by: fn(f32, f32) -> f32| {
        if nw <= 0.0 || nh <= 0.0 {
            return room;
        }
        let s = by(bw / nw, bh / nh);
        (nw * s, nh * s)
    };
    match fit {
        ObjectFit::Fill => room,
        ObjectFit::Contain => scaled(f32::min),
        ObjectFit::Cover => scaled(f32::max),
        ObjectFit::None => natural,
        ObjectFit::ScaleDown => {
            let contained = scaled(f32::min);
            if contained.0 < nw { contained } else { natural }
        }
    }
}

/// Paints what a replaced box shows (M23.2) in its content box `rect`:
/// its image, placed by `object-fit` and `object-position` (CSS Images 3
/// sections 4.5 and 4.6) and clipped to the box where it reaches past
/// it; or the icon of one that cannot be shown, at the box's top left,
/// a light border round a box larger than it; or nothing.
fn push_replaced(
    items: &mut Vec<DisplayItem>,
    fragment: &Fragment,
    replaced: &Replaced,
    rect: Rect,
    pictures: &dyn Pictures,
) {
    match replaced {
        Replaced::Empty => {}
        Replaced::Mark(mark) => push_mark(items, *mark, rect, to_rgba8(fragment.style.color)),
        // An outer `<svg>` (M32) fills its content box, its `viewBox`
        // fitted in it by its own `preserveAspectRatio`: an image the
        // engine draws at the size it lands on.
        Replaced::Svg { node, .. } => {
            if rect.w > 0.0 && rect.h > 0.0 {
                items.push(DisplayItem::Image {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                    image: ImageRef {
                        id: svg_image(*node),
                        frame: 0,
                        orientation: images::Orientation::Normal,
                        crop: None,
                        pixels: None,
                    },
                    sharp: false,
                });
            }
        }
        // A gradient (M23.3) fills the box, rendered for its size.
        Replaced::Gradient(gradient) => {
            if rect.w > 0.0 && rect.h > 0.0 {
                items.push(DisplayItem::Image {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                    image: ImageRef {
                        id: gradient::number(gradient, rect.w, rect.h),
                        frame: 0,
                        orientation: images::Orientation::Normal,
                        crop: None,
                        pixels: Some(pictures.gradient(gradient, rect.w, rect.h)),
                    },
                    sharp: false,
                });
            }
        }
        Replaced::Icon => {
            let size = layout::ICON_SIZE;
            if rect.w > size || rect.h > size {
                let r = snapped(rect);
                items.push(DisplayItem::Border {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                    widths: [1.0; 4],
                    color: ICON_BOX_COLOUR,
                });
            }
            push_icon(
                items,
                snapped(Rect {
                    w: size,
                    h: size,
                    ..rect
                }),
            );
        }
        // A canvas (M30.3): its bitmap placed as an image's is, in a
        // layer of its own; the engine gives the pixels and, as the
        // frame, the bitmap's generation.
        Replaced::Canvas {
            node,
            width,
            height,
        } => {
            let image = fragment.style.image();
            let natural = (*width as f32, *height as f32);
            let (w, h) = fitted(image.object_fit, natural, (rect.w, rect.h));
            let x = rect.x + image.object_position.x.resolve(rect.w - w);
            let y = rect.y + image.object_position.y.resolve(rect.h - h);
            if w <= 0.0 || h <= 0.0 || natural.0 <= 0.0 || natural.1 <= 0.0 {
                return;
            }
            items.push(DisplayItem::PushGroup {
                matrix: Matrix::IDENTITY,
                opacity: 1.0,
                key: GroupKey::Canvas(*node),
                layer: Some(LayerReason::Canvas),
            });
            let clipped =
                x < rect.x || y < rect.y || x + w > rect.x + rect.w || y + h > rect.y + rect.h;
            if clipped {
                let r = snapped(rect);
                items.push(DisplayItem::PushClip {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                });
            }
            items.push(DisplayItem::Image {
                x,
                y,
                w,
                h,
                image: ImageRef {
                    id: canvas_image(*node),
                    frame: 0,
                    orientation: images::Orientation::Normal,
                    crop: None,
                    pixels: None,
                },
                sharp: image.image_rendering.is_sharp(),
            });
            if clipped {
                items.push(DisplayItem::PopClip);
            }
            items.push(DisplayItem::PopGroup);
        }
        Replaced::Image { id, header } => {
            let image = fragment.style.image();
            let (natural, orientation) = oriented(&fragment.style, header);
            let (w, h) = fitted(image.object_fit, natural, (rect.w, rect.h));
            let x = rect.x + image.object_position.x.resolve(rect.w - w);
            let y = rect.y + image.object_position.y.resolve(rect.h - h);
            let clipped =
                x < rect.x || y < rect.y || x + w > rect.x + rect.w || y + h > rect.y + rect.h;
            if clipped {
                let r = snapped(rect);
                items.push(DisplayItem::PushClip {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                });
            }
            items.push(DisplayItem::Image {
                x,
                y,
                w,
                h,
                image: ImageRef {
                    id: *id,
                    frame: 0,
                    orientation,
                    crop: None,
                    pixels: None,
                },
                sharp: image.image_rendering.is_sharp(),
            });
            if clipped {
                items.push(DisplayItem::PopClip);
            }
        }
    }
}

/// The icon of an image that cannot be shown, in the 16 by 16 square
/// `rect`: the engine's own, a square with a cross in it.
fn push_icon(items: &mut Vec<DisplayItem>, rect: Rect) {
    items.push(DisplayItem::Border {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        widths: [1.0; 4],
        color: ICON_COLOUR,
    });
    let (x0, y0) = (rect.x + 4.0, rect.y + 4.0);
    let (x1, y1) = (rect.x + rect.w - 4.0, rect.y + rect.h - 4.0);
    let t = 0.75;
    for (a, b) in [((x0, y0), (x1, y1)), ((x1, y0), (x0, y1))] {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (-dy / length * t, dx / length * t);
        items.push(DisplayItem::Polygon {
            points: vec![
                (a.0 + nx, a.1 + ny),
                (b.0 + nx, b.1 + ny),
                (b.0 - nx, b.1 - ny),
                (a.0 - nx, a.1 - ny),
            ],
            color: ICON_COLOUR,
        });
    }
}

/// The accent colour a checked checkbox or radio button is drawn in
/// (M26.7), the reference browser's.
const ACCENT: Rgba8 = Rgba8 {
    r: 0,
    g: 117,
    b: 255,
    a: 255,
};
/// The grey of an unchecked one's edge.
const MARK_EDGE: Rgba8 = Rgba8 {
    r: 118,
    g: 118,
    b: 118,
    a: 255,
};
const WHITE: Rgba8 = Rgba8 {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// A control's mark in `rect` (M26.7), as the reference browser draws
/// one: a checkbox a white box with a grey edge, filled with the accent
/// colour and a white tick when checked or a white bar when
/// indeterminate; a radio button a white ring with a grey edge, the
/// edge and a dot in the accent colour when checked; a drop-down's
/// arrow a chevron in the text's `color`, in the middle of its box. A
/// disabled one is faded, and grey where it would be the accent.
fn push_mark(items: &mut Vec<DisplayItem>, mark: Mark, rect: Rect, color: Rgba8) {
    let r = snapped(rect);
    let fade = |c: Rgba8, disabled: bool| Rgba8 {
        a: if disabled { c.a / 3 + 10 } else { c.a },
        ..c
    };
    let circle = |cx: f32, cy: f32, radius: f32| -> Vec<(f32, f32)> {
        (0..32)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 32.0;
                (cx + radius * angle.cos(), cy + radius * angle.sin())
            })
            .collect()
    };
    match mark {
        Mark::Checkbox {
            checked,
            indeterminate,
            disabled,
        } => {
            let on = checked || indeterminate;
            let fill = match (on, disabled) {
                (true, false) => ACCENT,
                (true, true) => MARK_EDGE,
                (false, _) => WHITE,
            };
            items.push(DisplayItem::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                color: fade(fill, disabled),
            });
            if !on {
                items.push(DisplayItem::Border {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                    widths: [1.0; 4],
                    color: fade(MARK_EDGE, disabled),
                });
                return;
            }
            // In a 13 by 13 box, scaled to the box.
            let (sx, sy) = (r.w / 13.0, r.h / 13.0);
            let at = |x: f32, y: f32| (r.x + x * sx, r.y + y * sy);
            let points = if indeterminate {
                vec![at(3.0, 5.5), at(10.0, 5.5), at(10.0, 7.5), at(3.0, 7.5)]
            } else {
                vec![
                    at(2.3, 7.2),
                    at(5.5, 10.4),
                    at(10.7, 4.7),
                    at(9.3, 3.3),
                    at(5.5, 7.6),
                    at(3.7, 5.8),
                ]
            };
            items.push(DisplayItem::Polygon {
                points,
                color: fade(WHITE, disabled),
            });
        }
        Mark::Radio { checked, disabled } => {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let radius = r.w.min(r.h) / 2.0;
            let edge = if checked && !disabled {
                ACCENT
            } else {
                MARK_EDGE
            };
            items.push(DisplayItem::Polygon {
                points: circle(cx, cy, radius),
                color: fade(edge, disabled),
            });
            items.push(DisplayItem::Polygon {
                points: circle(cx, cy, (radius - 1.0).max(0.0)),
                color: fade(WHITE, disabled),
            });
            if checked {
                items.push(DisplayItem::Polygon {
                    points: circle(cx, cy, (radius - 3.0).max(0.0)),
                    color: fade(edge, disabled),
                });
            }
        }
        Mark::Arrow => {
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            items.push(DisplayItem::Polygon {
                points: vec![
                    (cx - 4.0, cy - 2.5),
                    (cx, cy + 1.5),
                    (cx + 4.0, cy - 2.5),
                    (cx + 4.0, cy - 1.0),
                    (cx, cy + 3.0),
                    (cx - 4.0, cy - 1.0),
                ],
                color,
            });
        }
    }
}

fn push_marker(items: &mut Vec<DisplayItem>, shape: MarkerShape, rect: Rect, color: Rgba8) {
    let rect = snapped(rect);
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let radius = rect.w.min(rect.h) / 2.0;
    let circle = |r: f32| -> Vec<(f32, f32)> {
        (0..32)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 32.0;
                (cx + r * angle.cos(), cy + r * angle.sin())
            })
            .collect()
    };
    match shape {
        MarkerShape::Square => items.push(DisplayItem::Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            color,
        }),
        MarkerShape::Disc => items.push(DisplayItem::Polygon {
            points: circle(radius),
            color,
        }),
        MarkerShape::Circle => {
            let outer = circle(radius + 0.5);
            let inner = circle((radius - 0.5).max(0.0));
            let mut points = outer.clone();
            points.push(outer[0]);
            points.push(inner[0]);
            points.extend(inner.iter().skip(1).rev());
            points.push(inner[0]);
            items.push(DisplayItem::Polygon { points, color });
        }
    }
}

/// Snaps a box's edges to whole device pixels (M11): each edge rounded
/// to the nearest pixel on its own, half up, so that neighbouring boxes
/// still meet without a seam, as browsers do; layout keeps its
/// fractions. A text run's baseline is snapped the same way (M17).
fn snapped(rect: Rect) -> Rect {
    let (x, y) = (snap(rect.x), snap(rect.y));
    let (right, bottom) = (snap(rect.x + rect.w), snap(rect.y + rect.h));
    Rect {
        x,
        y,
        w: (right - x).max(0.0),
        h: (bottom - y).max(0.0),
    }
}

/// A box's own background and border over its border box `rect`, in
/// viewport coordinates, snapped to whole pixels: the border box's
/// edges and the padding box's each on their own, which keeps the
/// border's widths, whole pixels already, as they are. `canvas_node`
/// is the element whose background the canvas already took (CSS2
/// §14.2), which is not painted on its own box again — but is on the
/// boxes its `::before` and `::after` generate (M16).
fn push_box(
    items: &mut Vec<DisplayItem>,
    f: &Fragment,
    rect: Rect,
    cb_width: f32,
    canvas_node: Option<NodeId>,
    backdrop: &Backdrop<'_>,
) {
    let outer = snapped(rect);
    let inner = snapped(padding_box(f, rect));
    let widths = [
        (inner.y - outer.y).max(0.0),
        (outer.x + outer.w - (inner.x + inner.w)).max(0.0),
        (outer.y + outer.h - (inner.y + inner.h)).max(0.0),
        (inner.x - outer.x).max(0.0),
    ];
    let canvas_took = f.generated.is_none() && canvas_node == Some(f.node);
    if !canvas_took {
        let placed = Placed {
            by: f,
            border: outer,
            cb_width,
            area: None,
        };
        // The colour under every layer, in the bottom one's clip box
        // (CSS Backgrounds 3 section 2.7).
        if f.style.background_color.a > 0.0 {
            let r = placed.box_of(f.style.background().color_clip());
            items.push(DisplayItem::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                color: to_rgba8(f.style.background_color),
            });
        }
        push_layers(items, f, placed, backdrop);
    }
    if !push_border_image(items, f, outer, widths, backdrop.pictures)
        && f.style.border_style.iter().any(|s| s.paints())
    {
        push_border(items, f, outer, widths);
    }
    push_outline(items, f, outer);
}

/// A box's outline (CSS UI 4 section 5, M26.4): a band of its width
/// outside the border box, `outline-offset` away from it, in its colour;
/// `auto`, the focus ring, two pixels wide whatever the width says, as
/// the reference browser draws it. It takes no room and is drawn with
/// the box, over what came before. `double` draws its outer and inner
/// thirds; the other styles are drawn solid, an approximation.
fn push_outline(items: &mut Vec<DisplayItem>, f: &Fragment, border: Rect) {
    let outline = f.style.outline();
    let width = match outline.style {
        OutlineStyle::None => return,
        OutlineStyle::Auto => 2.0,
        _ => f.style.used_outline_width(),
    };
    if width <= 0.0 {
        return;
    }
    let color = to_rgba8(f.style.outline_color());
    if color.a == 0 {
        return;
    }
    let offset = outline.offset.round();
    let x = border.x - offset - width;
    let y = border.y - offset - width;
    let w = border.w + 2.0 * (offset + width);
    let h = border.h + 2.0 * (offset + width);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let mut ring = |inset: f32, thickness: f32| {
        let (x, y, w, h) = (x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset);
        if thickness <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let bands = [
            (x, y, w, thickness),
            (x, y + h - thickness, w, thickness),
            (x, y + thickness, thickness, h - 2.0 * thickness),
            (
                x + w - thickness,
                y + thickness,
                thickness,
                h - 2.0 * thickness,
            ),
        ];
        for (x, y, w, h) in bands {
            if w > 0.0 && h > 0.0 {
                items.push(DisplayItem::Rect { x, y, w, h, color });
            }
        }
    };
    if outline.style == OutlineStyle::Double && width >= 3.0 {
        let third = (width / 3.0).round().max(1.0);
        ring(0.0, third);
        ring(width - third, third);
    } else {
        ring(0.0, width);
    }
}

/// Where the tiles of one of a border image's slices go along one axis
/// (CSS Backgrounds 3 section 6.5), the room `start` and `len` long, a
/// tile `tile` long as scaled: the first tile's start, its length, the
/// gap between tiles and whether they repeat. `stretch` makes one tile
/// of the room; `repeat` tiles from one centred; `round` fits a whole
/// number; `space` as many whole as fit with even gaps around them, and
/// nothing where none fits.
fn slice_axis(
    rule: BorderImageRepeat,
    start: f32,
    len: f32,
    tile: f32,
) -> Option<(f32, f32, f32, bool)> {
    if !(tile > 0.0 && tile.is_finite()) {
        return None;
    }
    match rule {
        BorderImageRepeat::Stretch => Some((start, len, 0.0, false)),
        BorderImageRepeat::Repeat => Some((start + (len - tile) / 2.0, tile, 0.0, true)),
        BorderImageRepeat::Round => {
            let n = (len / tile).round().max(1.0);
            Some((start, len / n, 0.0, true))
        }
        BorderImageRepeat::Space => {
            let n = (len / tile).floor();
            if n < 1.0 {
                return None;
            }
            let gap = (len - n * tile) / (n + 1.0);
            Some((start + gap, tile, gap, true))
        }
    }
}

/// Paints `f`'s border image (CSS Backgrounds 3 section 6, M23.3) over
/// its border image area, the border box `outer` grown by the outset,
/// `widths` the border's: the image cut in nine by the slices, the
/// corners stretched over the corners of the area the widths leave, the
/// edges and, with `fill`, the middle scaled and stretched, repeated,
/// rounded or spaced along them. Whether it takes the border's place: it
/// does while it is coming too, when the reference browser draws
/// neither; `none`, or one broken, leaves it to the border.
fn push_border_image(
    items: &mut Vec<DisplayItem>,
    f: &Fragment,
    outer: Rect,
    widths: [f32; 4],
    pictures: &dyn Pictures,
) -> bool {
    let Some(border_image) = &f.style.border_image else {
        return false;
    };
    let Some(source) = &border_image.source else {
        return false;
    };
    let out = |side: usize| match border_image.outset[side] {
        Outset::Number(n) => n * widths[side],
        Outset::Px(v) => v,
    };
    let area = Rect {
        x: outer.x - out(LEFT),
        y: outer.y - out(TOP),
        w: outer.w + out(LEFT) + out(RIGHT),
        h: outer.h + out(TOP) + out(BOTTOM),
    };
    if !(area.w > 0.0 && area.h > 0.0) {
        return true;
    }
    // The image's size in its pixels, the CSS pixels one is, and what
    // draws it; a gradient is rendered for the area, its pixels CSS
    // pixels.
    let (iw, ih, scale, image) = match source {
        Image::Url { url, resolution } => {
            let Some((id, header)) = pictures.picture(url) else {
                return pictures.coming(url);
            };
            let (size, orientation) = oriented(&f.style, &header);
            let image = ImageRef {
                id,
                frame: 0,
                orientation,
                crop: None,
                pixels: None,
            };
            (size.0, size.1, Some(1.0 / resolution), image)
        }
        Image::Gradient(g) => {
            let image = ImageRef {
                id: gradient::number(g, area.w, area.h),
                frame: 0,
                orientation: images::Orientation::Normal,
                crop: None,
                pixels: Some(pictures.gradient(g, area.w, area.h)),
            };
            (area.w, area.h, None, image)
        }
    };
    if !(iw > 0.0 && ih > 0.0) {
        return true;
    }
    // The slices, in the image's pixels, whole ones for a raster image;
    // one past the image is all of it.
    let slice = |side: usize, len: f32| {
        let v = match border_image.slice[side] {
            SliceSide::Number(n) => n,
            SliceSide::Percent(p) => p / 100.0 * len,
        };
        let v = if scale.is_some() { v.round() } else { v };
        v.min(len)
    };
    let s = [
        slice(TOP, ih),
        slice(RIGHT, iw),
        slice(BOTTOM, ih),
        slice(LEFT, iw),
    ];
    // The widths of the area's sides (section 6.3), scaled down
    // together where two opposite ones overlap.
    let css = scale.unwrap_or(1.0);
    let width = |side: usize| match &border_image.width[side] {
        BorderImageWidth::Number(n) => n * widths[side],
        BorderImageWidth::Length(v) if side == TOP || side == BOTTOM => v.resolve(area.h),
        BorderImageWidth::Length(v) => v.resolve(area.w),
        // A gradient's slices have sizes too, cut from it at the area's
        // size (section 6.2).
        BorderImageWidth::Auto => s[side] * css,
    };
    let mut w = [width(TOP), width(RIGHT), width(BOTTOM), width(LEFT)];
    let fit = (area.w / (w[LEFT] + w[RIGHT])).min(area.h / (w[TOP] + w[BOTTOM]));
    if fit < 1.0 {
        for v in &mut w {
            *v *= fit;
        }
    }
    let xs = [
        area.x,
        area.x + w[LEFT],
        area.x + area.w - w[RIGHT],
        area.x + area.w,
    ];
    let ys = [
        area.y,
        area.y + w[TOP],
        area.y + area.h - w[BOTTOM],
        area.y + area.h,
    ];
    let sx = [0.0, s[LEFT], iw - s[RIGHT], iw];
    let sy = [0.0, s[TOP], ih - s[BOTTOM], ih];
    // The middle's scale along x is the top's, else the bottom's, along
    // y the left's, else the right's; else the image's own.
    let factor = |a: usize, b: usize| {
        [a, b]
            .into_iter()
            .map(|side| w[side] / s[side])
            .find(|k| *k > 0.0 && k.is_finite())
            .unwrap_or(css)
    };
    let middle = (factor(TOP, BOTTOM), factor(LEFT, RIGHT));
    let sharp = f.style.image().image_rendering.is_sharp();
    for row in 0..3 {
        for col in 0..3 {
            if row == 1 && col == 1 && !border_image.fill {
                continue;
            }
            let (src_w, src_h) = (sx[col + 1] - sx[col], sy[row + 1] - sy[row]);
            let (x, y) = (xs[col], ys[row]);
            let (dst_w, dst_h) = (xs[col + 1] - x, ys[row + 1] - y);
            if !(src_w > 0.0 && src_h > 0.0 && dst_w > 0.0 && dst_h > 0.0) {
                continue;
            }
            // A tile's size as scaled: an edge keeps its ratio, its
            // breadth the side's width; the middle by its factors.
            let (tile_w, tile_h) = match (col, row) {
                (1, 1) => (src_w * middle.0, src_h * middle.1),
                (1, _) => (src_w * dst_h / src_h, dst_h),
                (_, 1) => (dst_w, src_h * dst_w / src_w),
                _ => (dst_w, dst_h),
            };
            let rule_x = if col == 1 {
                border_image.repeat[0]
            } else {
                BorderImageRepeat::Stretch
            };
            let rule_y = if row == 1 {
                border_image.repeat[1]
            } else {
                BorderImageRepeat::Stretch
            };
            let (Some((tx, tw, gx, rx)), Some((ty, th, gy, ry))) = (
                slice_axis(rule_x, x, dst_w, tile_w),
                slice_axis(rule_y, y, dst_h, tile_h),
            ) else {
                continue;
            };
            let image = ImageRef {
                crop: Some([
                    sx[col] / iw,
                    sy[row] / ih,
                    sx[col + 1] / iw,
                    sy[row + 1] / ih,
                ]),
                ..image.clone()
            };
            if !rx && !ry {
                items.push(DisplayItem::Image {
                    x,
                    y,
                    w: dst_w,
                    h: dst_h,
                    image,
                    sharp,
                });
            } else {
                items.push(DisplayItem::Pattern {
                    x,
                    y,
                    w: dst_w,
                    h: dst_h,
                    tile: [tx, ty, tw, th],
                    gap: [gx, gy],
                    repeat: [rx, ry],
                    image,
                    sharp,
                });
            }
        }
    }
    true
}

/// What a box's background images are painted against (M23.3): the
/// images, the viewport in the document's coordinates, which a `fixed`
/// layer is placed in, and the scroll offsets, which a `local` one moves
/// with.
struct Backdrop<'p> {
    pictures: &'p dyn Pictures,
    offsets: &'p dyn ScrollOffsets,
    viewport: Rect,
    /// Whether what is painted is inside a transformed box's group.
    in_transform: bool,
}

/// Where a background's layers go: the box `by` whose boxes place them,
/// its border box `border`, its containing block's width, and the area
/// they cover when it is not their clip box, the canvas's.
struct Placed<'f> {
    by: &'f Fragment,
    border: Rect,
    cb_width: f32,
    area: Option<Rect>,
}

impl Placed<'_> {
    /// The box `which` names, on whole pixels: the border box, the
    /// padding box, or the content box, whose padding may be a
    /// percentage of the containing block's width.
    fn box_of(&self, which: BackgroundBox) -> Rect {
        let padding = padding_box(self.by, self.border);
        match which {
            BackgroundBox::BorderBox => self.border,
            BackgroundBox::PaddingBox => snapped(padding),
            BackgroundBox::ContentBox => {
                let p = self
                    .by
                    .style
                    .padding
                    .each_ref()
                    .map(|v| v.resolve(self.cb_width));
                snapped(Rect {
                    x: padding.x + p[LEFT],
                    y: padding.y + p[TOP],
                    w: (padding.w - p[LEFT] - p[RIGHT]).max(0.0),
                    h: (padding.h - p[TOP] - p[BOTTOM]).max(0.0),
                })
            }
        }
    }
}

/// Paints the image layers of `f`'s background (CSS Backgrounds 3 section
/// 2, M23.3), the bottom one first: each image whose header is read, one
/// [`DisplayItem::Pattern`], over its clip box, or the canvas's area;
/// sized by `background-size` against its positioning area, its origin
/// box, the viewport for `fixed`, a scroll container's content for
/// `local`; placed in it by `background-position`; repeated, spaced or
/// rounded by `background-repeat`. A layer whose image has no size, or
/// is `none`, paints nothing.
fn push_layers(
    items: &mut Vec<DisplayItem>,
    f: &Fragment,
    placed: Placed<'_>,
    backdrop: &Backdrop<'_>,
) {
    let Some(background) = &f.style.background else {
        return;
    };
    let layers: Vec<_> = background.layers().collect();
    for layer in layers.iter().rev() {
        // An image's natural size, its resolution applied, and its
        // number; a gradient has neither.
        let (natural, picture) = match layer.image {
            Some(Image::Url { url, resolution }) => {
                let Some((id, header)) = backdrop.pictures.picture(url) else {
                    continue;
                };
                let ((w, h), orientation) = oriented(&f.style, &header);
                (
                    Some((w / resolution, h / resolution)),
                    Some((id, orientation)),
                )
            }
            Some(Image::Gradient(_)) => (None, None),
            None => continue,
        };
        let area = placed.area.unwrap_or_else(|| placed.box_of(layer.clip));
        let mut positioning = placed.box_of(layer.origin);
        match layer.attachment {
            // Inside a transformed box it is `scroll` (CSS Transforms 1
            // section 6).
            Attachment::Fixed if !backdrop.in_transform => positioning = backdrop.viewport,
            // A scroll container's local layer is placed in its content,
            // as tall as it scrolls, and moves with it.
            Attachment::Local if placed.by.clips() => {
                let padding = padding_box(placed.by, placed.border);
                positioning.y -= backdrop.offsets.offset(placed.by.node);
                positioning.h += (placed.by.scroll_height - padding.h).max(0.0);
            }
            Attachment::Fixed | Attachment::Local | Attachment::Scroll => {}
        }
        let (tile_w, tile_h) = tile_size(layer.size, natural, positioning, layer.repeat);
        if !(tile_w > 0.0 && tile_h > 0.0 && area.w > 0.0 && area.h > 0.0) {
            continue;
        }
        // A gradient is rendered for its tile, which names it.
        let image = match (picture, layer.image) {
            (Some((id, orientation)), _) => ImageRef {
                id,
                frame: 0,
                orientation,
                crop: None,
                pixels: None,
            },
            (None, Some(Image::Gradient(g))) => ImageRef {
                id: gradient::number(g, tile_w, tile_h),
                frame: 0,
                orientation: images::Orientation::Normal,
                crop: None,
                pixels: Some(backdrop.pictures.gradient(g, tile_w, tile_h)),
            },
            _ => continue,
        };
        let (x, gap_x, repeat_x) = place(
            layer.repeat[0],
            &layer.position[0],
            positioning.x,
            positioning.w,
            tile_w,
        );
        let (y, gap_y, repeat_y) = place(
            layer.repeat[1],
            &layer.position[1],
            positioning.y,
            positioning.h,
            tile_h,
        );
        items.push(DisplayItem::Pattern {
            x: area.x,
            y: area.y,
            w: area.w,
            h: area.h,
            tile: [x, y, tile_w, tile_h],
            gap: [gap_x, gap_y],
            repeat: [repeat_x, repeat_y],
            image,
            sharp: f.style.image().image_rendering.is_sharp(),
        });
    }
}

/// A layer's tile size (CSS Backgrounds 3 sections 2.4 and 2.3): what
/// `size` makes of the image's `natural` size in the positioning `area`,
/// `auto` keeping its ratio; then, for `round`, as wide or tall as fits a
/// whole number of times, the other side, where `auto`, keeping the
/// ratio. An image with no natural size, a gradient, is the area's size
/// where `size` leaves it `auto` (CSS Images 3 section 4.3).
pub fn tile_size(
    size: &BackgroundSize,
    natural: Option<(f32, f32)>,
    area: Rect,
    repeat: [Repeat; 2],
) -> (f32, f32) {
    let (nw, nh) = natural.unwrap_or((area.w, area.h));
    let ratio = natural.and_then(|(nw, nh)| (nw > 0.0 && nh > 0.0).then(|| nw / nh));
    let (w, h, auto) = match size {
        BackgroundSize::Cover | BackgroundSize::Contain => {
            if ratio.is_none() {
                return (area.w, area.h);
            }
            let (sx, sy) = (area.w / nw, area.h / nh);
            let scale = if *size == BackgroundSize::Cover {
                sx.max(sy)
            } else {
                sx.min(sy)
            };
            (nw * scale, nh * scale, [false, false])
        }
        BackgroundSize::Sized(sw, sh) => {
            let sw = sw.as_ref().map(|v| v.resolve(area.w));
            let sh = sh.as_ref().map(|v| v.resolve(area.h));
            let (w, h) = match (sw, sh) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, ratio.map_or(nh, |r| w / r)),
                (None, Some(h)) => (ratio.map_or(nw, |r| h * r), h),
                (None, None) => (nw, nh),
            };
            (w, h, [sw.is_none(), sh.is_none()])
        }
    };
    let fitted = |room: f32, tile: f32| {
        if tile > 0.0 {
            room / (room / tile).round().max(1.0)
        } else {
            tile
        }
    };
    let round = [repeat[0] == Repeat::Round, repeat[1] == Repeat::Round];
    let (mut fw, mut fh) = (w, h);
    if round[0] {
        fw = fitted(area.w, w);
    }
    if round[1] {
        fh = fitted(area.h, h);
    }
    if round[0] && !round[1] && auto[1] && w > 0.0 {
        fh = h * fw / w;
    }
    if round[1] && !round[0] && auto[0] && h > 0.0 {
        fw = w * fh / h;
    }
    (fw, fh)
}

/// Where a layer's tiles go along one axis (sections 2.3 and 2.5): the
/// first tile's start, the gap between tiles and whether they repeat.
/// `position` places a tile in the positioning area, `start` and `len`
/// long, a percentage of the room beside it; `space` puts as many as fit
/// whole from edge to edge with even gaps, or one, placed, where two do
/// not fit.
pub fn place(
    repeat: Repeat,
    position: &LengthPercentage,
    start: f32,
    len: f32,
    tile: f32,
) -> (f32, f32, bool) {
    let placed = start + position.resolve(len - tile);
    match repeat {
        Repeat::Space => {
            let n = (len / tile).floor();
            if n >= 2.0 {
                (start, (len - n * tile) / (n - 1.0), true)
            } else {
                (placed, 0.0, false)
            }
        }
        Repeat::NoRepeat => (placed, 0.0, false),
        Repeat::Repeat | Repeat::Round => (placed, 0.0, true),
    }
}

/// The colour a border's darker tone is: two thirds of each channel.
fn darker(c: Rgba8) -> Rgba8 {
    Rgba8 {
        r: (c.r as u16 * 2 / 3) as u8,
        g: (c.g as u16 * 2 / 3) as u8,
        b: (c.b as u16 * 2 / 3) as u8,
        a: c.a,
    }
}

/// Paints a box fragment's border, side by side (M10), over the border
/// box `rect` with the four `widths`, both in whole pixels. The four
/// sides meet at mitred corners, as in CSS2 §8.5: each side's band is
/// the trapezoid between the border box's edge and the padding box's.
/// When all four sides are solid in one colour the mitres are
/// invisible and the border is one band, which paints without seams
/// and covers each corner once (M19).
/// Otherwise a run of neighbouring sides whose band is one colour is
/// one polygon, drawn anti-aliased, so that only the mitres between
/// sides of different colours blend, as a browser draws them (M11):
/// the first run takes its corners whole, and each run after it is cut
/// along the mitre to its neighbour, so that a mitre's pixels blend the
/// two sides' colours and nothing behind them. Four such sides are four
/// plain quads, their mitres invisible too.
///
/// Per style: `solid` is the whole band; `double` the outer and inner
/// thirds; `dashed` and `dotted` are dashes of three times the width
/// and dots of the width, with equal gaps, from each side's start, drawn
/// as plain strips without the fitting adjustment real browsers make;
/// `inset`, `outset`, `groove` and `ridge` use the colour and its
/// darker tone — the last four are approximations, said so here.
fn push_border(items: &mut Vec<DisplayItem>, f: &Fragment, rect: Rect, widths: [f32; 4]) {
    let uniform = f
        .style
        .border_style
        .iter()
        .all(|s| *s == BorderStyle::Solid)
        && f.style
            .border_color
            .iter()
            .all(|c| *c == f.style.border_color[TOP]);
    if uniform {
        push_solid_border(items, f, rect, widths);
        return;
    }
    let geometry = (rect.x, rect.y, rect.w, rect.h, widths);
    // The bands of each side, outer first: none for a side that has no
    // width or is dashed, whose dashes are strips of their own.
    let mut bands: [Vec<Band>; 4] = Default::default();
    for side in [TOP, RIGHT, BOTTOM, LEFT] {
        let width = widths[side];
        if width <= 0.0 {
            continue;
        }
        let color = to_rgba8(f.style.border_color[side]);
        let dark = darker(color);
        // Which tone the outer and inner halves of a 3D style get: the
        // top and left sides of an `outset` box catch the light.
        let lit = side == TOP || side == LEFT;
        let band = |a: f32, b: f32, color: Rgba8| Band { a, b, color };
        bands[side] = match f.style.border_style[side] {
            BorderStyle::None | BorderStyle::Hidden => Vec::new(),
            BorderStyle::Solid => vec![band(0.0, 1.0, color)],
            BorderStyle::Double => vec![band(0.0, 1.0 / 3.0, color), band(2.0 / 3.0, 1.0, color)],
            BorderStyle::Dashed => {
                push_dashes(items, side, geometry, width * 3.0, color);
                Vec::new()
            }
            BorderStyle::Dotted => {
                push_dashes(items, side, geometry, width, color);
                Vec::new()
            }
            BorderStyle::Inset => vec![band(0.0, 1.0, if lit { dark } else { color })],
            BorderStyle::Outset => vec![band(0.0, 1.0, if lit { color } else { dark })],
            BorderStyle::Groove => {
                let (outer, inner) = if lit { (dark, color) } else { (color, dark) };
                vec![band(0.0, 0.5, outer), band(0.5, 1.0, inner)]
            }
            BorderStyle::Ridge => {
                let (outer, inner) = if lit { (color, dark) } else { (dark, color) };
                vec![band(0.0, 0.5, outer), band(0.5, 1.0, inner)]
            }
        };
    }
    for k in 0..2 {
        let band_of = |side: usize| bands[side].get(k).copied();
        // Whether a side's band carries on the one before it, clockwise.
        let continues = |side: usize| match (band_of(side), band_of((side + 3) % 4)) {
            (Some(own), Some(before)) => own == before,
            _ => false,
        };
        // The cycle starts at a side that does not continue the one
        // before it, so no run is cut at the wrap from left to top.
        let Some(start) = (0..4).find(|&side| !continues(side)) else {
            // Every side continues the last: one band all round, whose
            // mitres are invisible.
            if let Some(band) = band_of(TOP) {
                for side in [TOP, RIGHT, BOTTOM, LEFT] {
                    items.push(DisplayItem::Quad {
                        points: quad(side, geometry, band.a, band.b),
                        color: band.color,
                    });
                }
            }
            continue;
        };
        let mut runs: Vec<Vec<usize>> = Vec::new();
        for i in 0..4 {
            let side = (start + i) % 4;
            if band_of(side).is_none() {
                continue;
            }
            match runs.last_mut() {
                Some(run) if continues(side) => run.push(side),
                _ => runs.push(vec![side]),
            }
        }
        let count = runs.len();
        for (i, run) in runs.iter().enumerate() {
            let Some(band) = band_of(run[0]) else {
                continue;
            };
            // A run's end is cut where the side beyond it has a band of
            // this slot, which a run painted before this one holds: the
            // run before it at its start, and at the last run's end the
            // first run, round the corner; and where the side beyond it
            // is transparent, since nothing paints over the corner then
            // and the mitre is the band's own edge (a transparent side
            // keeps its width, CSS2 §8.5.2).
            let (first, last) = (run[0], run[run.len() - 1]);
            let (before, beyond) = ((first + 3) % 4, (last + 1) % 4);
            let clear = |side: usize| band_of(side).is_some_and(|b| b.color.a == 0);
            let cuts = (
                band_of(before).is_some() && (i > 0 || clear(before)),
                band_of(beyond).is_some() && (i + 1 == count || clear(beyond)),
            );
            if band.color.a == 0 {
                continue;
            }
            items.push(DisplayItem::Polygon {
                points: run_polygon(run, geometry, band.a, band.b, cuts),
                color: band.color,
            });
        }
    }
}

/// One band of a border side: between the fractions `a` and `b` of
/// the side's width (0 its outer edge, 1 its inner), in one colour.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Band {
    a: f32,
    b: f32,
    color: Rgba8,
}

/// A solid border in one colour all round: one band, the border box
/// less the padding box (M19; M6.8 drew it as four strips, which
/// overlap at the corners).
fn push_solid_border(items: &mut Vec<DisplayItem>, f: &Fragment, rect: Rect, widths: [f32; 4]) {
    if widths.iter().all(|w| *w <= 0.0) {
        return;
    }
    let Rect { x, y, w, h } = rect;
    items.push(DisplayItem::Border {
        x,
        y,
        w,
        h,
        widths: widths.map(|v| v.max(0.0)),
        color: to_rgba8(f.style.border_color[TOP]),
    });
}

/// The border geometry a side's bands are computed from: the border
/// box's `(x, y, w, h)` in viewport coordinates and the four widths.
type BorderGeometry = (f32, f32, f32, f32, [f32; 4]);

/// A point on `side`'s band edge at fraction `own` of the side's width
/// (0 the outer edge, 1 the inner), at the side's start or end, where
/// the neighbouring side's width is taken at fraction `other`:
/// `other == own` is the point on the mitre line, `other` at the
/// band's outer fraction the corner of the band's plain rectangle. The
/// sides run clockwise: the top side left to right, the right side
/// downwards, and so on.
fn edge_point(
    side: usize,
    geometry: BorderGeometry,
    own: f32,
    other: f32,
    start: bool,
) -> (f32, f32) {
    let (x, y, w, h, [top, right, bottom, left]) = geometry;
    match (side, start) {
        (TOP, true) => (x + left * other, y + top * own),
        (TOP, false) => (x + w - right * other, y + top * own),
        (RIGHT, true) => (x + w - right * own, y + top * other),
        (RIGHT, false) => (x + w - right * own, y + h - bottom * other),
        (BOTTOM, true) => (x + w - right * other, y + h - bottom * own),
        (BOTTOM, false) => (x + left * other, y + h - bottom * own),
        (_, true) => (x + left * own, y + h - bottom * other),
        (_, false) => (x + left * own, y + top * other),
    }
}

/// The quad of `side`'s band between the fractions `a` and `b` of its
/// width, its ends on the mitre lines to the neighbouring sides.
fn quad(side: usize, geometry: BorderGeometry, a: f32, b: f32) -> [(f32, f32); 4] {
    [
        edge_point(side, geometry, a, a, true),
        edge_point(side, geometry, a, a, false),
        edge_point(side, geometry, b, b, false),
        edge_point(side, geometry, b, b, true),
    ]
}

/// The polygon of the band between the fractions `a` and `b` along
/// `sides`, neighbours in clockwise order: its outer edge along them,
/// then its inner edge back. `cuts` says whether its start and its end
/// are cut along the mitre to the neighbouring side; an end not cut
/// takes the whole corner, out to the band's outer fraction.
fn run_polygon(
    sides: &[usize],
    geometry: BorderGeometry,
    a: f32,
    b: f32,
    (cut_start, cut_end): (bool, bool),
) -> Vec<(f32, f32)> {
    let (first, last) = (sides[0], sides[sides.len() - 1]);
    let mut points = Vec::with_capacity(2 * sides.len() + 2);
    points.push(edge_point(first, geometry, a, a, true));
    for &side in sides {
        points.push(edge_point(side, geometry, a, a, false));
    }
    points.push(edge_point(
        last,
        geometry,
        b,
        if cut_end { b } else { a },
        false,
    ));
    for &side in sides.iter().rev().skip(1) {
        points.push(edge_point(side, geometry, b, b, false));
    }
    points.push(edge_point(
        first,
        geometry,
        b,
        if cut_start { b } else { a },
        true,
    ));
    points
}

/// Dashes of `dash` px with `dash` px gaps along `side`'s strip, from
/// its start (the top-left corner for the top and left sides).
fn push_dashes(
    items: &mut Vec<DisplayItem>,
    side: usize,
    geometry: BorderGeometry,
    dash: f32,
    color: Rgba8,
) {
    let (x, y, w, h, [top, right, bottom, left]) = geometry;
    if dash.is_nan() || dash <= 0.0 {
        return;
    }
    let horizontal = side == TOP || side == BOTTOM;
    let length = if horizontal { w } else { h };
    let mut at = 0.0;
    while at < length {
        let len = dash.min(length - at);
        items.push(match side {
            TOP => DisplayItem::Rect {
                x: x + at,
                y,
                w: len,
                h: top,
                color,
            },
            BOTTOM => DisplayItem::Rect {
                x: x + at,
                y: y + h - bottom,
                w: len,
                h: bottom,
                color,
            },
            RIGHT => DisplayItem::Rect {
                x: x + w - right,
                y: y + at,
                w: right,
                h: len,
                color,
            },
            _ => DisplayItem::Rect {
                x,
                y: y + at,
                w: left,
                h: len,
                color,
            },
        });
        at += dash * 2.0;
    }
}

#[cfg(test)]
mod testing {
    //! What the unit tests share: the pipeline from a tree to its styles
    //! and layout, and the numbers the tests name nodes by.

    use std::ops::Deref;

    use dom::{Document, Node, NodeId};
    use style::Styles;

    /// A document and its styles.
    pub(crate) struct Styled {
        pub(crate) document: Document,
        pub(crate) styles: Styles,
    }

    /// The document `node` describes, cascaded under the UA sheet and
    /// `sheets` with no viewport and no font.
    pub(crate) fn styled(node: &Node, sheets: &[css::Stylesheet]) -> Styled {
        let document = Document::from_node(node);
        let styles = style::cascade_document(&document, sheets, &style::CascadeContext::default());
        Styled { document, styles }
    }

    /// A layout and the document it was laid out from, so that a test
    /// can name a fragment's node by its number: 0 for the document
    /// node, then the elements from 1 in tree order — the numbering
    /// nodes had before they had ids (M13), which the tests were
    /// written in.
    pub(crate) struct Laid {
        pub(crate) document: Document,
        pub(crate) layout: layout::DocumentLayout,
    }

    /// `node`'s document cascaded under `sheets`, its box tree laid out
    /// `width` px wide with `fonts`.
    pub(crate) fn laid_out(
        node: &Node,
        sheets: &[css::Stylesheet],
        width: f32,
        fonts: &text::FontCollection,
    ) -> Laid {
        let document = Document::from_node(node);
        let sheets = style::Stylesheets::new(sheets.to_vec());
        let context = style::CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let boxes = layout::build_box_tree(&document, &styles, &recascade);
        let layout = layout::layout_document(&boxes, width, fonts);
        Laid { document, layout }
    }

    impl Laid {
        /// The document node and the elements, in tree order.
        fn numbered(&self) -> impl Iterator<Item = NodeId> + '_ {
            let document = &self.document;
            document
                .subtree(document.root())
                .filter(|&id| id == document.root() || document.is_element(id))
        }

        /// The number the tests name `node` by.
        pub(crate) fn number(&self, node: NodeId) -> usize {
            self.numbered()
                .position(|id| id == node)
                .unwrap_or_else(|| panic!("{node} is not numbered in the document"))
        }

        /// The node numbered `number`.
        pub(crate) fn element(&self, number: usize) -> NodeId {
            self.numbered()
                .nth(number)
                .unwrap_or_else(|| panic!("no node is numbered {number}"))
        }
    }

    impl Deref for Laid {
        type Target = layout::DocumentLayout;

        fn deref(&self) -> &layout::DocumentLayout {
            &self.layout
        }
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    use crate::testing::laid_out;
    use dom::Element;

    const VIEWPORT: f32 = 800.0;

    /// The bundled test font, so that every machine measures the same
    /// and no test is skipped for want of a system font.
    fn test_fonts() -> text::FontCollection {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    }

    /// The full `style` -> `box_tree` -> `block` -> `paint` pipeline, for
    /// one element and some CSS — exactly as `engine` drives it.
    fn display_list_of(root: Element, css_src: &str) -> DisplayList {
        display_list_for(root, css_src, Viewport::new(VIEWPORT, 600.0))
    }

    /// The fixture is wrapped in `html > body` with no margins, as the
    /// parser would wrap a page, unless it is an `html` element itself;
    /// the root's background is the canvas's (CSS2 §14.2), so a bare
    /// `div` at the root would paint differently from one in a page.
    fn display_list_for(root: Element, css_src: &str, viewport: Viewport) -> DisplayList {
        display_list_scrolled(root, css_src, viewport, &[])
    }

    /// [`display_list_for`] with scroll containers scrolled (M10.5), by
    /// node number. Nodes: html 1, body 2, the fixture's root 3.
    fn display_list_scrolled(
        root: Element,
        css_src: &str,
        viewport: Viewport,
        offsets: &[(usize, f32)],
    ) -> DisplayList {
        let fonts = test_fonts();
        let (root, css_src) = if root.tag == "html" {
            (root, css_src.to_string())
        } else {
            (
                Element::new("html").push(Element::new("body").push(root)),
                format!("html, body {{ margin: 0; padding: 0 }} {css_src}"),
            )
        };
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(&css_src).expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        let offsets: HashMap<NodeId, f32> = offsets
            .iter()
            .map(|&(node, offset)| (layout.element(node), offset))
            .collect();
        paint_fragments_in(&layout.roots, viewport, &offsets)
    }

    fn rects(list: &DisplayList) -> Vec<&DisplayItem> {
        list.items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Rect { .. }))
            .collect()
    }

    /// The display list of an `img` whose `src` is `src`, styled by
    /// `css`, its image 200 by 100 and turned by `orientation` when
    /// `src` is `a.png`, broken otherwise (M23.2).
    fn img_list(src: &str, orientation: images::Orientation, css: &str) -> DisplayList {
        struct Shown(images::Orientation);
        impl layout::Images for Shown {
            fn image(&self, _node: NodeId) -> layout::ImageState {
                let (width, height) = if self.0.swaps_sides() {
                    (100, 200)
                } else {
                    (200, 100)
                };
                layout::ImageState::Ready {
                    id: 9,
                    header: images::Header {
                        format: images::Format::Png,
                        width,
                        height,
                        orientation: self.0,
                        vector: None,
                    },
                }
            }
        }
        struct Broken;
        impl layout::Images for Broken {
            fn image(&self, _node: NodeId) -> layout::ImageState {
                layout::ImageState::Broken
            }
        }
        let fonts = test_fonts();
        let root = Element::new("html")
            .push(Element::new("body").push(Element::new("img").attr("id", "i").attr("src", src)));
        let document = Document::from_node(&dom::Node::document(vec![root.into()]));
        let sheet = css::parse_stylesheet(&format!(
            "html, body {{ margin: 0 }} img {{ display: block }} {css}"
        ))
        .expect("test css is valid");
        let sheets = style::Stylesheets::new(vec![sheet]);
        let context = style::CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let shown = Shown(orientation);
        let images: &dyn layout::Images = if src == "a.png" { &shown } else { &Broken };
        let boxes = layout::build_box_tree_showing(&document, &styles, &recascade, images);
        let layout = layout::layout_document(&boxes, VIEWPORT, &fonts);
        paint_fragments(&layout.roots, Viewport::new(VIEWPORT, 600.0))
    }

    /// The items after the canvas, inside the document's group.
    fn after_canvas(list: &DisplayList) -> Vec<DisplayItem> {
        list.items[1..]
            .iter()
            .filter(|i| !matches!(i, DisplayItem::PushGroup { .. } | DisplayItem::PopGroup))
            .cloned()
            .collect()
    }

    fn image_at(item: &DisplayItem) -> (f32, f32, f32, f32, images::Orientation, bool) {
        match item {
            DisplayItem::Image {
                x,
                y,
                w,
                h,
                image,
                sharp,
            } => (*x, *y, *w, *h, image.orientation, *sharp),
            other => panic!("not an image: {other:?}"),
        }
    }

    #[test]
    fn an_image_is_placed_by_object_fit_and_object_position_and_clipped_to_its_box() {
        use images::Orientation::{Normal, Rotate90, Rotate270};
        let box_css = "img { width: 100px; height: 100px }";
        let items =
            |css: &str| after_canvas(&img_list("a.png", Normal, &format!("{box_css} {css}")));
        // `fill`: stretched over the box.
        let fill = items("");
        assert_eq!(fill.len(), 1);
        assert_eq!(image_at(&fill[0]), (0.0, 0.0, 100.0, 100.0, Normal, false));
        // `contain`: as wide as the box, centred down it.
        let contain = items("img { object-fit: contain }");
        assert_eq!(
            image_at(&contain[0]),
            (0.0, 25.0, 100.0, 50.0, Normal, false)
        );
        // `cover`: as tall as the box, at its left, clipped to it.
        let cover = items("img { object-fit: cover; object-position: 0 0 }");
        assert_eq!(cover.len(), 3);
        assert!(matches!(
            cover[0],
            DisplayItem::PushClip {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0
            }
        ));
        assert_eq!(image_at(&cover[1]), (0.0, 0.0, 200.0, 100.0, Normal, false));
        assert_eq!(cover[2], DisplayItem::PopClip);
        // `none`, at the right: its natural size, clipped.
        let none = items("img { object-fit: none; object-position: right 0 }");
        assert_eq!(
            image_at(&none[1]),
            (-100.0, 0.0, 200.0, 100.0, Normal, false)
        );
        // `scale-down` of an image larger than the box is `contain`.
        let down = items("img { object-fit: scale-down }");
        assert_eq!(image_at(&down[0]).3, 50.0);
        // `pixelated` keeps texels sharp.
        let sharp = items("img { image-rendering: pixelated }");
        assert!(image_at(&sharp[0]).5);
        // Turned back where `image-orientation` is `none`: its natural
        // size, and its pixels turned, the other way.
        let turned = after_canvas(&img_list(
            "a.png",
            Rotate90,
            "img { image-orientation: none }",
        ));
        assert_eq!(
            image_at(&turned[0]),
            (0.0, 0.0, 200.0, 100.0, Rotate270, false)
        );
        let applied = after_canvas(&img_list("a.png", Rotate90, ""));
        assert_eq!(
            image_at(&applied[0]),
            (0.0, 0.0, 100.0, 200.0, Normal, false)
        );
    }

    #[test]
    fn an_image_that_cannot_be_shown_is_its_icon_in_a_light_box() {
        let icon = after_canvas(&img_list("missing.png", images::Orientation::Normal, ""));
        assert_eq!(icon.len(), 3, "{icon:?}");
        assert!(matches!(
            icon[0],
            DisplayItem::Border {
                x: 0.0,
                y: 0.0,
                w: 16.0,
                h: 16.0,
                color: ICON_COLOUR,
                ..
            }
        ));
        assert!(matches!(icon[1], DisplayItem::Polygon { .. }));
        let sized = after_canvas(&img_list(
            "missing.png",
            images::Orientation::Normal,
            "img { width: 50px; height: 40px }",
        ));
        assert!(matches!(
            sized[0],
            DisplayItem::Border {
                w: 50.0,
                h: 40.0,
                color: ICON_BOX_COLOUR,
                ..
            }
        ));
        assert!(matches!(sized[1], DisplayItem::Border { w: 16.0, .. }));
    }

    /// Every image 20 by 10, numbered by its URL's length.
    struct Twenty;
    impl Pictures for Twenty {
        fn picture(&self, url: &str) -> Option<(u64, images::Header)> {
            Some((
                url.len() as u64,
                images::Header {
                    format: images::Format::Png,
                    width: 20,
                    height: 10,
                    orientation: images::Orientation::Normal,
                    vector: None,
                },
            ))
        }
    }

    /// The patterns of a page whose `div`, 100 by 50 inside a 10px
    /// border and 5px padding at (0, 0), is styled by `css`, scrolled to
    /// `scroll_y`, its images [`Twenty`]'s.
    fn patterns(css: &str, scroll_y: f32) -> Vec<DisplayItem> {
        patterns_of(css, scroll_y, &Twenty)
    }

    /// [`patterns`], its images `pictures`'.
    fn patterns_of(css: &str, scroll_y: f32, pictures: &dyn Pictures) -> Vec<DisplayItem> {
        let fonts = test_fonts();
        let root = Element::new("html").push(Element::new("body").push(Element::new("div")));
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(&format!(
            "html, body {{ margin: 0 }} div {{ width: 100px; height: 50px; padding: 5px;
             border: 10px solid transparent }} {css}"
        ))
        .expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        let viewport = Viewport {
            width: VIEWPORT,
            height: 600.0,
            scroll_y,
        };
        paint_fragments_with(&layout.roots, viewport, &(), pictures)
            .items
            .into_iter()
            .filter(|i| matches!(i, DisplayItem::Pattern { .. }))
            .collect()
    }

    /// Every image 20 by 10 as shown, stored 10 by 20 and turned a
    /// quarter.
    struct Turned;
    impl Pictures for Turned {
        fn picture(&self, _: &str) -> Option<(u64, images::Header)> {
            Some((
                1,
                images::Header {
                    format: images::Format::Jpeg,
                    width: 20,
                    height: 10,
                    orientation: images::Orientation::Rotate90,
                    vector: None,
                },
            ))
        }
    }

    #[test]
    fn image_orientation_none_draws_a_background_and_a_border_image_as_stored() {
        let image = |item: &DisplayItem| match item {
            DisplayItem::Pattern { image, .. } | DisplayItem::Image { image, .. } => image.clone(),
            other => panic!("not an image: {other:?}"),
        };
        let shown = patterns_of("div { background: url(a.png) no-repeat }", 0.0, &Turned);
        assert_eq!(laid(&shown[0]).1[2..], [20.0, 10.0]);
        assert_eq!(image(&shown[0]).orientation, images::Orientation::Normal);
        let stored = patterns_of(
            "div { background: url(a.png) no-repeat; image-orientation: none }",
            0.0,
            &Turned,
        );
        assert_eq!(laid(&stored[0]).1[2..], [10.0, 20.0]);
        assert_eq!(
            image(&stored[0]).orientation,
            images::Orientation::Rotate90.inverse()
        );
        // A border image's slices are of the image as stored: 3 of its
        // 10 wide.
        let pieces = border_image(
            "div { border-image: url(a.png) 3; image-orientation: none }",
            &Turned,
        );
        assert_eq!(pieces[0].1, [0.0, 0.0, 3.0 / 10.0, 3.0 / 20.0]);
    }

    #[test]
    fn a_fixed_background_in_a_transformed_box_is_placed_as_scroll() {
        let at = |attachment: &str| {
            let css = format!(
                "div {{ transform: translateX(1px); background: url(a.png) no-repeat {attachment} right top }}"
            );
            laid(&patterns(&css, 30.0)[0]).1
        };
        assert_eq!(at("fixed"), at("scroll"));
        // Outside one, in the viewport.
        let fixed = patterns(
            "div { background: url(a.png) no-repeat fixed right top }",
            30.0,
        );
        assert_ne!(laid(&fixed[0]).1, at("scroll"));
    }

    #[test]
    fn a_fixed_roots_canvas_images_cover_the_viewport_under_it() {
        let fonts = test_fonts();
        let root = Element::new("html").push(Element::new("body").push(Element::new("p")));
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(
            "html { position: fixed; margin: 0 } body { margin: 0; background: url(a.png) }",
        )
        .expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        let list =
            paint_fragments_with(&layout.roots, Viewport::new(VIEWPORT, 600.0), &(), &Twenty);
        let patterns: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Pattern { .. }))
            .collect();
        assert_eq!(patterns.len(), 1);
        assert_eq!(laid(patterns[0]).0, [0.0, 0.0, VIEWPORT, 600.0]);
        assert!(
            matches!(list.items[1], DisplayItem::Pattern { .. }),
            "on the canvas's colour, under everything"
        );
    }

    /// The images the border image of a page's `div` draws, as
    /// [`patterns`] lays it out, under `css`, its images `pictures`':
    /// each one's area, crop, and its tile, gap and repeat if a pattern.
    fn border_image(css: &str, pictures: &dyn Pictures) -> Vec<BorderPiece> {
        let fonts = test_fonts();
        let root = Element::new("html").push(Element::new("body").push(Element::new("div")));
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(&format!(
            "html, body {{ margin: 0 }} div {{ width: 100px; height: 50px; padding: 5px;
             border: 10px solid red }} {css}"
        ))
        .expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        paint_fragments_with(&layout.roots, Viewport::new(VIEWPORT, 600.0), &(), pictures)
            .items
            .into_iter()
            .filter_map(|i| match i {
                DisplayItem::Image {
                    x, y, w, h, image, ..
                } => Some(([x, y, w, h], image.crop.expect("cut"), None)),
                DisplayItem::Pattern {
                    x,
                    y,
                    w,
                    h,
                    tile,
                    gap,
                    repeat,
                    image,
                    ..
                } => Some((
                    [x, y, w, h],
                    image.crop.expect("cut"),
                    Some((tile, gap, repeat)),
                )),
                DisplayItem::Border { .. } => Some(([0.0; 4], [0.0; 4], None)),
                _ => None,
            })
            .collect()
    }

    /// A border image's piece: its area, crop, and its tile, gap and
    /// repeat if a pattern; a border's, zeros.
    type BorderPiece = ([f32; 4], [f32; 4], Option<([f32; 4], [f32; 2], [bool; 2])>);

    #[test]
    fn a_border_image_is_cut_in_nine_and_its_pieces_stretched_over_the_border() {
        // Twenty's 20 by 10 cut 3 from each edge, over a border box
        // 130 by 80 with 10px sides.
        let pieces = border_image("div { border-image: url(a.png) 3 }", &Twenty);
        let (x, y) = ([0.0, 3.0 / 20.0, 17.0 / 20.0, 1.0], [0.0, 0.3, 0.7, 1.0]);
        let want: Vec<BorderPiece> = vec![
            ([0.0, 0.0, 10.0, 10.0], [x[0], y[0], x[1], y[1]], None),
            ([10.0, 0.0, 110.0, 10.0], [x[1], y[0], x[2], y[1]], None),
            ([120.0, 0.0, 10.0, 10.0], [x[2], y[0], x[3], y[1]], None),
            ([0.0, 10.0, 10.0, 60.0], [x[0], y[1], x[1], y[2]], None),
            ([120.0, 10.0, 10.0, 60.0], [x[2], y[1], x[3], y[2]], None),
            ([0.0, 70.0, 10.0, 10.0], [x[0], y[2], x[1], y[3]], None),
            ([10.0, 70.0, 110.0, 10.0], [x[1], y[2], x[2], y[3]], None),
            ([120.0, 70.0, 10.0, 10.0], [x[2], y[2], x[3], y[3]], None),
        ];
        assert_eq!(pieces, want, "the border's place taken");
        // `fill` draws the middle too.
        let filled = border_image("div { border-image: url(a.png) 3 fill }", &Twenty);
        assert_eq!(filled.len(), 9);
        assert_eq!(
            filled[4],
            ([10.0, 10.0, 110.0, 60.0], [x[1], y[1], x[2], y[2]], None)
        );
        // Slices past the image leave the corners alone.
        let corners = border_image("div { border-image: url(a.png) 12 }", &Twenty);
        assert_eq!(corners.len(), 4);
        assert_eq!(corners[1].1, [8.0 / 20.0, 0.0, 1.0, 1.0]);
        // An outset grows the area; widths too wide are scaled down
        // together; `auto` is the slice's own size.
        let outset = border_image("div { border-image: url(a.png) 3 / 10px / 5px }", &Twenty);
        assert_eq!(outset[0].0, [-5.0, -5.0, 10.0, 10.0]);
        assert_eq!(outset[7].0, [125.0, 75.0, 10.0, 10.0]);
        let times = border_image("div { border-image: url(a.png) 3 / 10px / 1 }", &Twenty);
        assert_eq!(
            times[0].0,
            [-10.0, -10.0, 10.0, 10.0],
            "the border's width once"
        );
        let wide = border_image("div { border-image: url(a.png) 3 / 50px }", &Twenty);
        assert_eq!(wide[0].0, [0.0, 0.0, 40.0, 40.0]);
        let auto = border_image("div { border-image: url(a.png) 3 / auto }", &Twenty);
        assert_eq!(auto[0].0, [0.0, 0.0, 3.0, 3.0]);
        // A gradient's slices have sizes too.
        let gradient = border_image(
            "div { border-image: linear-gradient(red, blue) 3 / auto }",
            &Twenty,
        );
        assert_eq!(gradient[0].0, [0.0, 0.0, 3.0, 3.0]);
    }

    #[test]
    fn a_border_images_edges_repeat_round_and_space_along_them() {
        // The top edge, 14 by 3, is 10 high: a tile 140 / 3 wide over
        // 110; the left, 3 by 4, 10 wide: a tile 40 / 3 high over 60.
        let tile = 140.0 / 3.0;
        let at = |css: &str, i: usize| {
            border_image(
                &format!("div {{ border-image: url(a.png) 3 {css} }}"),
                &Twenty,
            )[i]
                .2
                .expect("a pattern")
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let (t, g, r) = at("repeat", 1);
        assert!(
            close(t[0], 10.0 + (110.0 - tile) / 2.0) && close(t[2], tile),
            "{t:?}"
        );
        assert_eq!((t[1], t[3], g, r), (0.0, 10.0, [0.0, 0.0], [true, false]));
        let (t, _, _) = at("repeat", 3);
        assert!(close(t[1], 10.0 + (60.0 - 40.0 / 3.0) / 2.0) && close(t[3], 40.0 / 3.0));
        let (t, _, r) = at("round", 1);
        assert_eq!((t[0], t[2], r), (10.0, 55.0, [true, false]));
        let (t, g, _) = at("space", 1);
        let gap = (110.0 - 2.0 * tile) / 3.0;
        assert!(close(g[0], gap) && close(t[0], 10.0 + gap) && close(t[2], tile));
        // The middle scaled as the top along x, the left along y: 14 by
        // 4 made 140 / 3 by 40 / 3, rounded over 110 by 60.
        let (t, _, r) = at("fill round", 4);
        assert_eq!((t[2], t[3], r), (55.0, 12.0, [true, true]));
        // Stretched along x, repeated along y: the corners stay put.
        let pieces = border_image("div { border-image: url(a.png) 3 stretch repeat }", &Twenty);
        assert!(pieces[1].2.is_none() && pieces[3].2.is_some() && pieces[0].2.is_none());
        // Space with no room for one tile draws none.
        let none = border_image(
            "div { border-image: url(a.png) 3 / 10px space; width: 10px }",
            &Twenty,
        );
        assert_eq!(none.len(), 6, "{none:?}");
    }

    #[test]
    fn a_border_image_coming_draws_nothing_and_one_broken_the_border() {
        struct Coming(bool);
        impl Pictures for Coming {
            fn picture(&self, _url: &str) -> Option<(u64, images::Header)> {
                None
            }
            fn coming(&self, _url: &str) -> bool {
                self.0
            }
        }
        let css = "div { border-image: url(a.png) 3 }";
        assert_eq!(border_image(css, &Coming(true)), []);
        let border = border_image(css, &Coming(false));
        assert!(!border.is_empty() && border.iter().all(|p| p.0 == [0.0; 4]));
        let none = border_image("div { border-image: none 3 }", &Twenty);
        assert!(!none.is_empty() && none.iter().all(|p| p.0 == [0.0; 4]));
        // A gradient is rendered for the area.
        let gradient = border_image("div { border-image: linear-gradient(red, blue) 10% }", &());
        assert_eq!(gradient.len(), 8);
        assert_eq!(gradient[0].1, [0.0, 0.0, 0.1, 0.1]);
    }

    /// A pattern's area, tile, gap and repeat.
    type Laid = ([f32; 4], [f32; 4], [f32; 2], [bool; 2]);

    fn laid(item: &DisplayItem) -> Laid {
        match item {
            DisplayItem::Pattern {
                x,
                y,
                w,
                h,
                tile,
                gap,
                repeat,
                ..
            } => ([*x, *y, *w, *h], *tile, *gap, *repeat),
            other => panic!("not a pattern: {other:?}"),
        }
    }

    #[test]
    fn a_background_image_tiles_its_clip_box_from_its_origin_box() {
        let border = [0.0, 0.0, 130.0, 80.0];
        // Repeated over the border box from the padding box's corner.
        let one = patterns("div { background-image: url(a.png) }", 0.0);
        assert_eq!(
            laid(&one[0]),
            (border, [10.0, 10.0, 20.0, 10.0], [0.0; 2], [true; 2])
        );
        // Placed alone at the padding box's centre.
        let one = patterns("div { background: url(a.png) no-repeat center }", 0.0);
        assert_eq!(laid(&one[0]).1, [10.0 + 45.0, 10.0 + 25.0, 20.0, 10.0]);
        assert_eq!(laid(&one[0]).3, [false; 2]);
        // From the far edges, in the content box.
        let one = patterns(
            "div { background: url(a.png) no-repeat right 4px bottom 2px; background-clip: content-box;
             background-origin: content-box }",
            0.0,
        );
        assert_eq!(laid(&one[0]).0, [15.0, 15.0, 100.0, 50.0]);
        assert_eq!(
            laid(&one[0]).1,
            [
                15.0 + 100.0 - 4.0 - 20.0,
                15.0 + 50.0 - 2.0 - 10.0,
                20.0,
                10.0
            ]
        );
        // Sized: one side keeps the ratio, cover and contain fill the
        // positioning area.
        let size = |css: &str| {
            laid(
                &patterns(
                    &format!("div {{ background: url(a.png); background-size: {css} }}"),
                    0.0,
                )[0],
            )
            .1
        };
        assert_eq!(size("40px")[2..], [40.0, 20.0]);
        assert_eq!(size("auto 30px")[2..], [60.0, 30.0]);
        assert_eq!(size("50% 50%")[2..], [55.0, 30.0]);
        assert_eq!(size("cover")[2..], [120.0, 60.0]);
        assert_eq!(size("contain")[2..], [110.0, 55.0]);
        // Rounded to a whole number of tiles, the other side keeping the
        // ratio; spaced from edge to edge.
        let round = laid(&patterns("div { background: url(a.png) round }", 0.0)[0]);
        assert_eq!(round.1[2..], [110.0 / 6.0, 60.0 / 6.0]);
        let space = laid(&patterns("div { background: url(a.png) space }", 0.0)[0]);
        assert_eq!(space.1[..2], [10.0, 10.0]);
        assert_eq!(
            space.2,
            [(110.0 - 5.0 * 20.0) / 4.0, (60.0 - 6.0 * 10.0) / 5.0]
        );
        // Layers, the last at the bottom, painted first.
        let two = patterns("div { background: url(a.png), url(bb.png) no-repeat }", 0.0);
        assert_eq!(two.len(), 2);
        assert!(matches!(&two[0], DisplayItem::Pattern { image, .. } if image.id == 6));
        assert!(matches!(&two[1], DisplayItem::Pattern { image, .. } if image.id == 5));
        // `fixed` is placed in the viewport, where the document is
        // scrolled to.
        let fixed = patterns(
            "div { background: url(a.png) no-repeat fixed right top }",
            30.0,
        );
        assert_eq!(laid(&fixed[0]).1, [VIEWPORT - 20.0, 30.0, 20.0, 10.0]);
        // A percentage padding is of the containing block's width.
        let one = patterns(
            "div { padding: 1%; background: url(a.png) no-repeat content-box }",
            0.0,
        );
        assert_eq!(laid(&one[0]).1[..2], [10.0 + 8.0, 10.0 + 8.0]);
        // No image, or none of a size: nothing.
        assert!(patterns("div { background-image: none }", 0.0).is_empty());
        assert!(patterns("div { background: url(a.png); background-size: 0 }", 0.0).is_empty());
    }

    #[test]
    fn the_colour_is_painted_in_the_bottom_layers_clip_box() {
        let fonts = test_fonts();
        let root = Element::new("html").push(Element::new("body").push(Element::new("div")));
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(
            "html, body { margin: 0 } div { width: 100px; height: 50px; padding: 5px;
             border: 10px solid transparent;
             background: url(a.png) content-box, url(bb.png) padding-box red }",
        )
        .expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        let list =
            paint_fragments_with(&layout.roots, Viewport::new(VIEWPORT, 600.0), &(), &Twenty);
        let red = list
            .items
            .iter()
            .find_map(|i| match i {
                DisplayItem::Rect { x, y, w, h, color } if *color == Rgba8::rgb(255, 0, 0) => {
                    Some([*x, *y, *w, *h])
                }
                _ => None,
            })
            .expect("the colour");
        assert_eq!(red, [10.0, 10.0, 110.0, 60.0]);
    }

    #[test]
    fn the_root_backgrounds_images_cover_the_canvas_under_the_document() {
        let fonts = test_fonts();
        let root = Element::new("html").push(Element::new("body").push(Element::new("p")));
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(
            "html { margin: 0; border: 7px solid transparent } body { margin: 0; background: url(a.png) }",
        )
        .expect("test css is valid");
        let layout = laid_out(&doc, &[sheet], VIEWPORT, &fonts);
        let list =
            paint_fragments_with(&layout.roots, Viewport::new(VIEWPORT, 600.0), &(), &Twenty);
        let group = list
            .items
            .iter()
            .position(|i| {
                matches!(
                    i,
                    DisplayItem::PushGroup {
                        key: GroupKey::Document(0),
                        ..
                    }
                )
            })
            .expect("the document's group");
        // The body's image, propagated, first in the group, over the
        // canvas, the document and a viewport more, placed by the root's
        // padding box.
        let (area, tile, _, repeat) = laid(&list.items[group + 1]);
        let bottom = layout.roots[0].top() + layout.roots[0].h;
        assert_eq!(area, [0.0, 0.0, VIEWPORT, bottom + 600.0]);
        // The same scrolled: the document's layer keeps it.
        let viewport = Viewport {
            width: VIEWPORT,
            height: 600.0,
            scroll_y: 250.0,
        };
        let scrolled = paint_fragments_with(&layout.roots, viewport, &(), &Twenty);
        let item = scrolled
            .items
            .iter()
            .find(|i| matches!(i, DisplayItem::Pattern { .. }))
            .expect("the canvas's image");
        assert_eq!(item, &list.items[group + 1]);
        assert_eq!(tile[..2], [7.0, 7.0]);
        assert_eq!(repeat, [true; 2]);
        let patterns = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Pattern { .. }))
            .count();
        assert_eq!(patterns, 1, "the body does not paint it again");
    }

    #[test]
    fn a_gradient_layer_is_rendered_for_its_tile_and_named_by_it() {
        let one = patterns("div { background: linear-gradient(red, blue) }", 0.0);
        let (area, tile, _, repeat) = laid(&one[0]);
        // As big as the padding box, over the border box, repeated.
        assert_eq!(area, [0.0, 0.0, 130.0, 80.0]);
        assert_eq!(tile, [10.0, 10.0, 110.0, 60.0]);
        assert_eq!(repeat, [true; 2]);
        let DisplayItem::Pattern { image, .. } = &one[0] else {
            unreachable!()
        };
        assert!(image.id >> 63 == 1);
        let pixels = image.pixels.as_ref().expect("rendered");
        assert_eq!((pixels.width, pixels.height), (1, 60));
        // Sized: one side given, the other the area's, no ratio kept.
        let one = patterns(
            "div { background: linear-gradient(red, blue) 0 0 / 20px }",
            0.0,
        );
        assert_eq!(laid(&one[0]).1[2..], [20.0, 60.0]);
    }

    #[test]
    fn tiles_are_sized_and_placed_as_backgrounds_3_says() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 40.0,
        };
        let auto = BackgroundSize::Sized(None, None);
        let no = [Repeat::NoRepeat; 2];
        assert_eq!(tile_size(&auto, Some((20.0, 10.0)), area, no), (20.0, 10.0));
        assert_eq!(
            tile_size(&BackgroundSize::Cover, Some((0.0, 0.0)), area, no),
            (100.0, 40.0)
        );
        assert_eq!(
            tile_size(
                &auto,
                Some((30.0, 10.0)),
                area,
                [Repeat::Round, Repeat::NoRepeat]
            ),
            (100.0 / 3.0, 100.0 / 9.0)
        );
        assert_eq!(
            tile_size(&auto, Some((30.0, 10.0)), area, [Repeat::Round; 2]),
            (100.0 / 3.0, 10.0)
        );
        let p = LengthPercentage::Percent;
        assert_eq!(
            place(Repeat::NoRepeat, &p(50.0), 0.0, 100.0, 20.0),
            (40.0, 0.0, false)
        );
        assert_eq!(
            place(Repeat::Repeat, &p(100.0), 10.0, 100.0, 20.0),
            (90.0, 0.0, true)
        );
        assert_eq!(
            place(Repeat::Space, &p(100.0), 0.0, 100.0, 30.0),
            (0.0, 5.0, true)
        );
        assert_eq!(
            place(Repeat::Space, &p(100.0), 0.0, 100.0, 60.0),
            (40.0, 0.0, false)
        );
    }

    #[test]
    fn a_gradient_an_element_shows_is_rendered_for_its_box() {
        let list = display_list_of(
            Element::new("div"),
            "div { content: linear-gradient(red, blue); width: 40px; height: 20px }",
        );
        let image = list
            .items
            .iter()
            .find_map(|i| match i {
                DisplayItem::Image { w, h, image, .. } => Some((*w, *h, image)),
                _ => None,
            })
            .expect("the gradient");
        assert_eq!((image.0, image.1), (40.0, 20.0));
        assert!(image.2.id >> 63 == 1);
        let pixels = image.2.pixels.as_ref().expect("rendered");
        assert_eq!((pixels.width, pixels.height), (1, 20));
    }

    #[test]
    fn fitting_keeps_the_ratio_but_for_fill() {
        let natural = (200.0, 100.0);
        assert_eq!(fitted(ObjectFit::Fill, natural, (50.0, 50.0)), (50.0, 50.0));
        assert_eq!(
            fitted(ObjectFit::Contain, natural, (50.0, 50.0)),
            (50.0, 25.0)
        );
        assert_eq!(
            fitted(ObjectFit::Cover, natural, (50.0, 50.0)),
            (100.0, 50.0)
        );
        assert_eq!(fitted(ObjectFit::None, natural, (50.0, 50.0)), natural);
        assert_eq!(
            fitted(ObjectFit::ScaleDown, natural, (50.0, 50.0)),
            (50.0, 25.0)
        );
        assert_eq!(
            fitted(ObjectFit::ScaleDown, natural, (500.0, 500.0)),
            natural
        );
        assert_eq!(
            fitted(ObjectFit::Contain, (0.0, 0.0), (5.0, 5.0)),
            (5.0, 5.0)
        );
    }

    #[test]
    fn image_refs_compare_by_what_they_draw() {
        let picture = |id, pixels: bool| ImageRef {
            id,
            frame: 0,
            orientation: images::Orientation::Normal,
            crop: None,
            pixels: pixels.then(|| {
                Arc::new(images::Image {
                    width: 1,
                    height: 1,
                    frames: vec![images::Frame {
                        pixels: Arc::from(vec![0u8; 4]),
                        delay: 100,
                    }],
                    plays: images::Plays::Times(1),
                })
            }),
        };
        assert_eq!(picture(1, true), picture(1, true));
        assert_ne!(picture(1, true), picture(1, false));
        assert_ne!(picture(1, true), picture(2, true));
        let mut turned = picture(1, true);
        turned.orientation = images::Orientation::Rotate90;
        assert_ne!(turned, picture(1, true));
        let mut later = picture(1, true);
        later.frame = 1;
        assert_ne!(later, picture(1, true));
    }

    #[test]
    fn background_color_paints_a_rect_at_the_fragment_geometry() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; background-color: #ff0000; }",
        );
        let backgrounds: Vec<&DisplayItem> = rects(&list)
            .into_iter()
            .filter(
                |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(255, 0, 0)),
            )
            .collect();
        assert_eq!(backgrounds.len(), 1);
        match backgrounds[0] {
            DisplayItem::Rect { x, y, w, h, .. } => {
                assert_eq!((*x, *y, *w, *h), (0.0, 0.0, 100.0, 50.0))
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn transparent_background_paints_nothing() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; }", // background-color: initial = transparent
        );
        // Just the viewport background rect — no second rect for the div.
        assert_eq!(rects(&list).len(), 1);
    }

    /// The border items of a list, as `(x, y, w, h, widths, color)`.
    #[allow(clippy::type_complexity)]
    fn borders(list: &DisplayList) -> Vec<(f32, f32, f32, f32, [f32; 4], Rgba8)> {
        list.items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Border {
                    x,
                    y,
                    w,
                    h,
                    widths,
                    color,
                } => Some((*x, *y, *w, *h, *widths, *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_border_in_one_colour_is_one_band_at_the_box_edges() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; margin: 0;
                   border-width: 4px 3px 2px 1px; border-style: solid; border-color: #00ff00; }",
        );
        // Border-box is 100+4=104 wide, 50+6=56 tall (border adds to
        // content), the band the widths given.
        assert_eq!(
            borders(&list),
            vec![(
                0.0,
                0.0,
                104.0,
                56.0,
                [4.0, 3.0, 2.0, 1.0],
                Rgba8::rgb(0, 255, 0)
            )]
        );
        assert_eq!(rects(&list).len(), 1); // just the viewport background
    }

    /// The rectangles of a list but the viewport's background, as
    /// `(x, y, w, h, color)`.
    fn outline_rects(list: &DisplayList) -> Vec<(f32, f32, f32, f32, Rgba8)> {
        rects(list)
            .into_iter()
            .skip(1)
            .filter_map(|i| match i {
                DisplayItem::Rect { x, y, w, h, color } => Some((*x, *y, *w, *h, *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_outline_is_a_band_outside_the_border_box_at_its_offset() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; margin: 20px;
                   outline: 2px solid #ff0000; outline-offset: 3px; }",
        );
        // The border box is at (20, 20), 100 by 50: the band starts 5px
        // out, 2px wide.
        let red = Rgba8::rgb(255, 0, 0);
        assert_eq!(
            outline_rects(&list),
            [
                (15.0, 15.0, 110.0, 2.0, red),
                (15.0, 73.0, 110.0, 2.0, red),
                (15.0, 17.0, 2.0, 56.0, red),
                (123.0, 17.0, 2.0, 56.0, red),
            ]
        );
        // None, or no width, draws nothing; `auto` is a 2px ring in the
        // colour, which is `currentcolor` unless set.
        for css in [
            "div { display: block; width: 10px; height: 10px; outline: 5px none red; }",
            "div { display: block; width: 10px; height: 10px; outline: 0 solid red; }",
        ] {
            assert!(outline_rects(&display_list_of(Element::new("div"), css)).is_empty());
        }
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 10px; height: 10px; margin: 10px; color: #0000ff;
                   outline: auto 1px; }",
        );
        let blue = Rgba8::rgb(0, 0, 255);
        assert_eq!(outline_rects(&list)[0], (8.0, 8.0, 14.0, 2.0, blue));
    }

    #[test]
    fn border_style_none_paints_no_border_even_with_width_set() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; border-width: 4px; }", // no border-style
        );
        assert_eq!(rects(&list).len(), 1); // just the viewport background
        assert!(borders(&list).is_empty());
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; border: 4px hidden red; }",
        );
        assert_eq!(rects(&list).len(), 1);
        assert!(borders(&list).is_empty());
    }

    /// The quad items of a list.
    fn quads(list: &DisplayList) -> Vec<([(f32, f32); 4], Rgba8)> {
        list.items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Quad { points, color } => Some((*points, *color)),
                _ => None,
            })
            .collect()
    }

    /// The polygon items of a list.
    fn polygons(list: &DisplayList) -> Vec<(Vec<(f32, f32)>, Rgba8)> {
        list.items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Polygon { points, color } => Some((points.clone(), *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sides_of_different_colours_meet_at_mitred_corners() {
        // A 100 by 50 box with a 10px top border and 20px left border in
        // two colours: each side is a trapezoid between the border edge
        // and the padding edge, meeting on the diagonal — a polygon of
        // its own, anti-aliased there (M11).
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border-top: 10px solid #ff0000; border-left: 20px solid #0000ff;
                   border-right: 0 solid #000000; border-bottom: 0 solid #000000; }",
        );
        let bands = polygons(&list);
        assert_eq!(bands.len(), 2);
        // The top side is painted first and takes the corner whole; the
        // left side, after it, is cut along the mitre.
        let (top, red) = &bands[0];
        assert_eq!(*red, Rgba8::rgb(255, 0, 0));
        assert_eq!(
            *top,
            vec![(0.0, 0.0), (100.0, 0.0), (100.0, 10.0), (0.0, 10.0)]
        );
        let (left, blue) = &bands[1];
        assert_eq!(*blue, Rgba8::rgb(0, 0, 255));
        assert_eq!(
            *left,
            vec![(0.0, 50.0), (0.0, 0.0), (20.0, 10.0), (20.0, 50.0)]
        );
        assert!(quads(&list).is_empty());
        // Four solid sides in one colour are one band.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; border: 4px solid #ff0000; }",
        );
        assert!(quads(&list).is_empty() && polygons(&list).is_empty());
        assert_eq!((rects(&list).len(), borders(&list).len()), (1, 1));
    }

    #[test]
    fn a_transparent_border_side_keeps_its_width_at_the_mitre() {
        // CSS2 §8.5.2: a transparent border has its width, so the side
        // next to it ends on the mitre to it, with nothing painted over
        // the corner: the top side is a trapezoid cut at its start, and
        // the transparent side paints nothing.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border-top: 10px solid #ff0000; border-left: 20px solid transparent;
                   border-right: 0 solid #000000; border-bottom: 0 solid #000000; }",
        );
        let bands = polygons(&list);
        assert_eq!(bands.len(), 1, "{bands:?}");
        let (top, red) = &bands[0];
        assert_eq!(*red, Rgba8::rgb(255, 0, 0));
        assert_eq!(
            *top,
            vec![(0.0, 0.0), (100.0, 0.0), (100.0, 10.0), (20.0, 10.0)]
        );
        // The triangle a border makes of a box with no size: its top
        // side cut at both ends by the transparent sides.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 0; height: 0; margin: 0;
                   border-top: 100px solid #00ff00; border-left: 100px solid transparent;
                   border-right: 100px solid transparent; border-bottom: 0 solid #000000; }",
        );
        let bands = polygons(&list);
        assert_eq!(bands.len(), 1, "{bands:?}");
        assert_eq!(
            bands[0].0,
            vec![(0.0, 0.0), (200.0, 0.0), (100.0, 100.0), (100.0, 100.0)]
        );
    }

    #[test]
    fn neighbouring_sides_of_one_colour_are_one_shape_and_four_are_quads() {
        // Top and left red, right and bottom blue: two L-shaped
        // polygons, the blue one first (the cycle starts where the
        // colour changes) with its corners whole, the red one cut along
        // the top-right and bottom-left mitres, each six points around
        // its outer and inner edges.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border: 4px solid; border-color: #ff0000 #0000ff #0000ff #ff0000; }",
        );
        let shapes = polygons(&list);
        assert_eq!(shapes.len(), 2);
        assert_eq!(shapes[0].1, Rgba8::rgb(0, 0, 255));
        assert_eq!(
            shapes[0].0,
            vec![
                (100.0, 0.0),
                (100.0, 50.0),
                (0.0, 50.0),
                (0.0, 46.0),
                (96.0, 46.0),
                (96.0, 0.0)
            ]
        );
        assert_eq!(shapes[1].1, Rgba8::rgb(255, 0, 0));
        assert_eq!(
            shapes[1].0,
            vec![
                (0.0, 50.0),
                (0.0, 0.0),
                (100.0, 0.0),
                (96.0, 4.0),
                (4.0, 4.0),
                (4.0, 46.0)
            ]
        );
        // `inset` lights two sides and darkens two: the same two shapes.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; border: 4px inset #c0c0c0; }",
        );
        let shapes = polygons(&list);
        assert_eq!(shapes.len(), 2);
        assert_eq!(
            shapes[0].1,
            Rgba8::rgb(0xc0, 0xc0, 0xc0),
            "right and bottom lit"
        );
        assert_eq!(shapes[1].1, darker(Rgba8::rgb(0xc0, 0xc0, 0xc0)));
        // Four double sides in one colour: two bands all round, eight
        // quads tiling exactly, no polygon.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; border: 9px double #000000; }",
        );
        assert!(polygons(&list).is_empty());
        assert_eq!(quads(&list).len(), 8);
    }

    #[test]
    fn box_edges_are_snapped_to_whole_pixels_when_painted() {
        // A box 10.4px in, 20.3px wide and 10.6px tall inside a 1.5px
        // border, a pixel wide once used: its border box runs from 10.4
        // to 32.7 and 0 to 12.6, its padding box from 11.4 to 31.7 and
        // 1 to 11.6. Painted, each edge is on the pixel nearest it: the
        // background over 10 to 33 and 0 to 13, the strips a pixel
        // wide, the padding box from 11 to 32 and 1 to 12. Layout keeps
        // the box's fractions.
        let list = display_list_of(
            Element::new("div"),
            "div { margin: 0 0 0 10.4px; width: 20.3px; height: 10.6px;
                   border: 1.5px solid #ff0000; background-color: #0000ff; }",
        );
        let boxes: Vec<(f32, f32, f32, f32, Rgba8)> = rects(&list)
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                DisplayItem::Rect { x, y, w, h, color } => Some((*x, *y, *w, *h, *color)),
                _ => None,
            })
            .collect();
        let (red, blue) = (Rgba8::rgb(255, 0, 0), Rgba8::rgb(0, 0, 255));
        assert_eq!(boxes, vec![(10.0, 0.0, 23.0, 13.0, blue)]);
        assert_eq!(borders(&list), vec![(10.0, 0.0, 23.0, 13.0, [1.0; 4], red)]);
        // A border thinner than a pixel is a pixel wide.
        let list = display_list_of(
            Element::new("div"),
            "div { margin: 0; width: 20px; height: 10px; border: 0.2px solid #ff0000; }",
        );
        assert_eq!(borders(&list)[0].4, [1.0; 4]);
        // A clip is snapped the same way: 0.4 to 50.6 is 0 to 51.
        let list = display_list_of(
            Element::new("div").push(Element::new("p")),
            "div { overflow: hidden; margin: 0 0 0 0.4px; width: 50.2px; height: 20.7px; }
             p { margin: 0; height: 100px; }",
        );
        let clip = list
            .items
            .iter()
            .find(|i| matches!(i, DisplayItem::PushClip { .. }))
            .expect("a clip");
        assert!(matches!(
            clip,
            DisplayItem::PushClip {
                x: 0.0,
                y: 0.0,
                w: 51.0,
                h: 21.0
            }
        ));
    }

    #[test]
    fn double_borders_are_two_bands_and_dashes_run_along_the_side() {
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border: 9px double #000000; border-bottom-style: none; border-left-style: none;
                   border-right-style: dashed; }",
        );
        let bands = polygons(&list);
        // Top: the outer third and the inner third, 3px each, 3px apart,
        // each a shape of its own beside the dashed and missing sides,
        // and the first run of its band, so with its corners whole.
        assert_eq!(bands.len(), 2);
        assert_eq!(
            bands[0].0,
            vec![(0.0, 0.0), (100.0, 0.0), (100.0, 3.0), (0.0, 3.0)]
        );
        assert_eq!(
            bands[1].0,
            vec![(0.0, 6.0), (94.0, 6.0), (94.0, 9.0), (0.0, 9.0)]
        );
        // Right: 27px dashes with 27px gaps down the 50px side: a dash
        // at 0 and a 23px remainder at 54 — none, the side ends at 50.
        let dashes: Vec<(f32, f32, f32, f32)> = rects(&list)
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                DisplayItem::Rect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
                _ => None,
            })
            .collect();
        assert_eq!(dashes, vec![(91.0, 0.0, 9.0, 27.0)]);
    }

    #[test]
    fn the_root_or_body_background_paints_the_canvas_once() {
        // The html element's background fills the viewport, and is not
        // painted on its own 0-height box again.
        let list = display_list_of(
            Element::new("html").push(Element::new("body").push(Element::new("p").text("x"))),
            "html { background-color: #ff0000; margin: 0; } body { margin: 0; }",
        );
        let reds: Vec<_> = rects(&list)
            .into_iter()
            .filter(
                |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(255, 0, 0)),
            )
            .collect();
        assert_eq!(reds.len(), 1);
        assert!(matches!(
            reds[0],
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
                ..
            }
        ));
        // With a transparent html, the body's background is the canvas.
        let list = display_list_of(
            Element::new("html").push(Element::new("body").push(Element::new("p").text("x"))),
            "body { background-color: #00ff00; margin: 0; }",
        );
        let greens: Vec<_> = rects(&list)
            .into_iter()
            .filter(
                |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(0, 255, 0)),
            )
            .collect();
        assert_eq!(greens.len(), 1);
        assert!(matches!(
            greens[0],
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
                ..
            }
        ));
        // Nothing propagates: the canvas is white.
        let list = display_list_of(Element::new("p").text("x"), "");
        assert!(matches!(
            rects(&list)[0],
            DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(255, 255, 255)
        ));
    }

    #[test]
    fn a_generated_box_is_never_the_canvas_and_keeps_its_background() {
        let page =
            || Element::new("html").push(Element::new("body").push(Element::new("p").text("x")));
        let count = |list: &DisplayList, rgb: Rgba8| {
            rects(list)
                .into_iter()
                .filter(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == rgb))
                .count()
        };
        let (red, green, blue) = (
            Rgba8::rgb(255, 0, 0),
            Rgba8::rgb(0, 255, 0),
            Rgba8::rgb(0, 0, 255),
        );
        // The root's `::before` before the body, a block or a cell in an
        // anonymous table: the body's green is still the canvas's, and
        // not painted on the body; the blue is painted on its box.
        for before in ["display: block", "display: table-cell; width: 10px"] {
            let list = display_list_of(
                page(),
                &format!(
                    "html, body {{ margin: 0 }} body {{ background-color: #00ff00 }}
                     html::before {{ {before}; content: ''; height: 10px; background-color: #0000ff }}"
                ),
            );
            assert!(
                matches!(rects(&list)[0], DisplayItem::Rect { w: 800.0, h: 600.0, color, .. } if *color == green),
                "{before}"
            );
            assert_eq!(
                (count(&list, green), count(&list, blue)),
                (1, 1),
                "{before}"
            );
        }
        // A `::before` of the element whose background the canvas took
        // has its own, painted.
        for (css, rgb) in [
            ("html { background-color: #ff0000 } html::before", red),
            ("body { background-color: #00ff00 } body::before", green),
        ] {
            let list = display_list_of(
                page(),
                &format!(
                    "html, body {{ margin: 0 }}
                     {css} {{ display: block; content: ''; height: 10px; background-color: {} }}",
                    if rgb == red { "#ff0000" } else { "#00ff00" }
                ),
            );
            assert_eq!(count(&list, rgb), 2, "{css}");
        }
    }

    #[test]
    fn text_fragment_paints_at_its_baseline_on_the_pixel_grid() {
        let fonts = test_fonts();
        let list = display_list_of(
            Element::new("p").text("hi"),
            "p { display: block; margin: 0; color: #0000ff; font-size: 12px; }",
        );
        let texts: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect();
        assert_eq!(texts.len(), 1);
        match texts[0] {
            DisplayItem::Text {
                text,
                x,
                y,
                size,
                color,
            } => {
                assert_eq!(text, "hi");
                // The baseline is the font's own ascent at this size
                // (M7.1/M7.4) — not simply the font-size — a whole
                // pixel, as the reference browser rounds it (M17).
                let ascent = fonts.primary().metrics(12.0).ascent;
                assert_eq!(ascent, 11.0);
                assert_eq!((*x, *y, *size), (0.0, 11.0, 12.0));
                assert_eq!(*color, Rgba8::rgb(0, 0, 255));
            }
            _ => unreachable!(),
        }
        // A box at a fraction of a pixel puts it on the pixel grid
        // (M17), half up: half a pixel lower, 11.5 is 12, and a quarter
        // lower, 11.25 is 11.
        for (padding, snapped) in [("0.5px", 12.0), ("0.25px", 11.0)] {
            let list = display_list_of(
                Element::new("p").text("hi"),
                &format!(
                    "p {{ display: block; margin: 0; padding-top: {padding}; font-size: 12px; }}"
                ),
            );
            let baseline = list.items.iter().find_map(|i| match i {
                DisplayItem::Text { y, .. } => Some(*y),
                _ => None,
            });
            assert_eq!(baseline, Some(snapped), "{padding}");
        }
    }

    #[test]
    fn a_layout_with_no_box_is_the_white_canvas_alone() {
        // A root of `display: none`, or no element at all: the canvas,
        // white, as the reference browser draws it, and nothing on it.
        let fonts = test_fonts();
        for doc in [
            dom::Node::document(vec![]),
            dom::Node::document(vec![
                Element::new("html").attr("style", "display: none").into(),
            ]),
        ] {
            let layout = laid_out(&doc, &[], VIEWPORT, &fonts);
            assert!(layout.roots.is_empty());
            let list = paint_fragments(&layout.roots, Viewport::new(VIEWPORT, 600.0));
            assert_eq!(
                list.items,
                [DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: VIEWPORT,
                    h: 600.0,
                    color: Rgba8::rgb(0xff, 0xff, 0xff),
                }]
            );
        }
    }

    /// `(y, h)` of every rect item after the viewport background.
    fn rect_rows(list: &DisplayList) -> Vec<(f32, f32)> {
        list.items
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                DisplayItem::Rect { y, h, .. } => Some((*y, *h)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_scroll_offset_is_the_documents_groups_matrix_and_the_margin_culls_the_rest() {
        // Three 100px boxes at document y = 0, 100, 200, and a fourth
        // 1000px further down, at 1300. The items are in document
        // coordinates inside the document's group, whose matrix is the
        // scroll offset (M19); a 150px-tall viewport lists what it and
        // a viewport's height on either side of it meet: unscrolled, the
        // first three (c, at 200, is in the margin below); scrolled to
        // 1200, the fourth alone.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "a"))
            .push(Element::new("div").attr("class", "b"))
            .push(Element::new("div").attr("class", "c"))
            .push(Element::new("div").attr("class", "d"));
        let css = "div { margin: 0; height: 100px; background-color: #ff0000; }
                   div div { height: 100px; }
                   .a, .b, .c, .d { background-color: #00ff00; }
                   .d { margin-top: 1000px; }";
        let unscrolled = display_list_for(root.clone(), css, Viewport::new(VIEWPORT, 150.0));
        assert_eq!(
            rect_rows(&unscrolled),
            vec![(0.0, 100.0), (0.0, 100.0), (100.0, 100.0), (200.0, 100.0)],
            "the outer div (100 tall), a, b, c; d at 1300 is past the margin"
        );
        let scroll = |scroll_y: f32| {
            display_list_for(
                root.clone(),
                css,
                Viewport {
                    width: VIEWPORT,
                    height: 150.0,
                    scroll_y,
                },
            )
        };
        let scrolled = scroll(1200.0);
        assert_eq!(rect_rows(&scrolled), vec![(1300.0, 100.0)]);
        let document_group = |list: &DisplayList| match list.items.get(1) {
            Some(DisplayItem::PushGroup {
                matrix,
                key: GroupKey::Document(0),
                layer: Some(LayerReason::Scrolled),
                opacity,
            }) if *opacity == 1.0 => matrix.scale_and_translation(),
            other => panic!("the document's group follows the canvas: {other:?}"),
        };
        assert_eq!(document_group(&unscrolled), Some((1.0, 1.0, 0.0, 0.0)));
        assert_eq!(document_group(&scrolled), Some((1.0, 1.0, 0.0, -1200.0)));
        assert_eq!(scrolled.items.last(), Some(&DisplayItem::PopGroup));
        // The items are the same from one offset to the next: a scroll
        // changes the group's matrix alone.
        let a = scroll(120.0);
        let b = scroll(121.0);
        assert_eq!(a.items[2..], b.items[2..]);
        assert_ne!(a.items[1], b.items[1]);
    }

    #[test]
    fn content_overflowing_an_out_of_view_box_still_paints() {
        // A zero-height box at the top: the viewport "misses" it (it has
        // no area), but its text overflows below it and shows.
        let root = Element::new("div").push(Element::new("p").text("shown"));
        let list = display_list_of(root, "div { height: 0px; } p { margin: 0; }");
        assert!(
            list.items
                .iter()
                .any(|i| matches!(i, DisplayItem::Text { text, .. } if text == "shown"))
        );
    }

    #[test]
    fn an_overflow_hidden_box_wraps_its_content_in_a_clip_at_its_padding_box() {
        let root = Element::new("div")
            .attr("class", "clip")
            .push(Element::new("p").text("hi"));
        let list = display_list_of(
            root,
            ".clip { overflow: hidden; width: 50px; height: 20px; padding: 2px;
                     border: 3px solid #000000; margin: 0; }
             p { margin: 0; }",
        );
        let items = &list.items;
        let first_push = items
            .iter()
            .position(|i| matches!(i, DisplayItem::PushClip { .. }))
            .expect("a clip is pushed");
        let text = items
            .iter()
            .position(|i| matches!(i, DisplayItem::Text { .. }))
            .expect("the text paints");
        // The text is inside a clip: the inline pass opens the clip
        // around it (the blocks pass opened it around the p's box).
        let push = items[..text]
            .iter()
            .rposition(|i| matches!(i, DisplayItem::PushClip { .. }))
            .expect("the clip is pushed before the text");
        let pop = text
            + items[text..]
                .iter()
                .position(|i| matches!(i, DisplayItem::PopClip))
                .expect("the clip is popped after the text");
        assert!(
            !items[push..pop]
                .iter()
                .any(|i| matches!(i, DisplayItem::PopClip)),
            "the text is inside the clip"
        );
        // The border box is 60 x 30; the padding box sits inside the
        // 3px border: 54 x 24 at (3, 3).
        for at in [first_push, push] {
            assert!(matches!(
                items[at],
                DisplayItem::PushClip {
                    x: 3.0,
                    y: 3.0,
                    w: 54.0,
                    h: 24.0
                }
            ));
        }
        // The box's own border paints outside the clip, before it.
        assert_eq!(
            items[1..first_push]
                .iter()
                .filter(|i| matches!(
                    i,
                    DisplayItem::Border {
                        widths: [3.0, 3.0, 3.0, 3.0],
                        ..
                    }
                ))
                .count(),
            1
        );
        // `scroll` and `auto` clip the same way; `visible` pushes nothing.
        for (value, clips) in [("scroll", true), ("auto", true), ("visible", false)] {
            let root = Element::new("div")
                .attr("class", "clip")
                .push(Element::new("p").text("hi"));
            let list = display_list_of(
                root,
                &format!(".clip {{ overflow: {value}; height: 20px; }}"),
            );
            assert_eq!(
                list.items
                    .iter()
                    .any(|i| matches!(i, DisplayItem::PushClip { .. })),
                clips,
                "{value}"
            );
        }
    }

    #[test]
    fn a_scroll_container_paints_its_content_moved_up_by_its_offset() {
        // A 50px box with a red 30px child and a blue 100px one,
        // scrolled by 20: the red box's rect is at y -20 and the blue
        // one's at 10, while the clip stays where the box is. Nodes:
        // html 1, body 2, the box 3.
        let root = Element::new("div")
            .attr("class", "s")
            .push(Element::new("p").attr("class", "r"))
            .push(Element::new("p").attr("class", "b"));
        let css = ".s { overflow: auto; height: 50px; width: 100px; }
                   .r { height: 30px; margin: 0; background-color: #ff0000; }
                   .b { height: 100px; margin: 0; background-color: #0000ff; }";
        let offsets = [(3, 20.0)];
        let list = display_list_scrolled(root, css, Viewport::new(VIEWPORT, 600.0), &offsets);
        let rect_y = |color: Rgba8| {
            list.items.iter().find_map(|i| match i {
                DisplayItem::Rect { y, color: c, .. } if *c == color => Some(*y),
                _ => None,
            })
        };
        assert_eq!(rect_y(Rgba8::rgb(255, 0, 0)), Some(-20.0));
        assert_eq!(rect_y(Rgba8::rgb(0, 0, 255)), Some(10.0));
        // Scrolled past the top of its clip, the red box is listed
        // still, at -40, since the viewport's margin meets it (M19);
        // the clip hides it.
        let offsets = [(3, 40.0)];
        let scrolled = display_list_scrolled(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p").attr("class", "r"))
                .push(Element::new("p").attr("class", "b")),
            css,
            Viewport::new(VIEWPORT, 600.0),
            &offsets,
        );
        assert!(scrolled.items.iter().any(|i| matches!(
            i,
            DisplayItem::Rect { y, color, .. } if *color == Rgba8::rgb(255, 0, 0) && *y == -40.0
        )));
        assert!(matches!(
            list.items
                .iter()
                .find(|i| matches!(i, DisplayItem::PushClip { .. })),
            Some(DisplayItem::PushClip {
                y: 0.0,
                h: 50.0,
                ..
            })
        ));
        // The overlay scrollbar, inside the clip after the content: the
        // 8px track at the padding box's right edge, the thumb in
        // proportion (50 of 130 is 19px, raised to 24; 20 of 80 down
        // the 26px of travel is 6.5).
        let track = list
            .items
            .iter()
            .position(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == SCROLLBAR_TRACK))
            .expect("a track");
        let pop = list
            .items
            .iter()
            .rposition(|i| matches!(i, DisplayItem::PopClip))
            .unwrap();
        let blue = list
            .items
            .iter()
            .position(
                |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(0, 0, 255)),
            )
            .unwrap();
        assert!(blue < track && track < pop);
        assert!(matches!(
            list.items[track],
            DisplayItem::Rect {
                x: 92.0,
                y: 0.0,
                w: 8.0,
                h: 50.0,
                ..
            }
        ));
        assert!(matches!(
            list.items[track + 1],
            DisplayItem::Rect {
                x: 93.0,
                y: 6.5,
                w: 6.0,
                h: 24.0,
                color: SCROLLBAR_THUMB
            }
        ));
    }

    #[test]
    fn a_scrollbar_is_drawn_only_where_the_box_can_scroll() {
        let has_track = |value: &str, child_height: &str| {
            let root = Element::new("div")
                .attr("class", "s")
                .push(Element::new("p"));
            let list = display_list_of(
                root,
                &format!(
                    ".s {{ overflow: {value}; height: 50px; width: 100px; }}
                     p {{ height: {child_height}; margin: 0; }}"
                ),
            );
            list.items
                .iter()
                .any(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == SCROLLBAR_TRACK))
        };
        assert!(has_track("auto", "200px"));
        assert!(!has_track("auto", "20px"), "auto: nothing overflows");
        assert!(has_track("scroll", "20px"), "scroll: always");
        assert!(!has_track("hidden", "200px"), "hidden: never");
        assert!(!has_track("visible", "200px"));
        // `scroll` with nothing to scroll: the thumb fills the track.
        assert_eq!(scrollbar_thumb(50.0, 0.0, 0.0), (0.0, 50.0));
        assert_eq!(scrollbar_thumb(50.0, 80.0, 80.0), (26.0, 24.0));
        assert_eq!(scrollbar_thumb(500.0, 500.0, 250.0), (125.0, 250.0));
    }

    #[test]
    fn a_clipping_box_out_of_view_is_skipped_with_its_content() {
        // Pushed 1000px down by a spacer; the 100px viewport never
        // meets it, so neither the clip nor the text is emitted.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "spacer"))
            .push(
                Element::new("div")
                    .attr("class", "clip")
                    .push(Element::new("p").text("hidden")),
            );
        let list = display_list_for(
            root,
            ".spacer { height: 1000px; } .clip { overflow: hidden; height: 50px; } p { margin: 0; }",
            Viewport::new(VIEWPORT, 100.0),
        );
        assert!(
            !list
                .items
                .iter()
                .any(|i| matches!(i, DisplayItem::PushClip { .. } | DisplayItem::Text { .. })),
            "{:?}",
            list.items
        );
    }
}

#[cfg(test)]
mod styled_tests {
    use super::*;
    use crate::testing::{Styled, styled};
    use dom::Element;

    /// A document holding an already-built element, cascaded under CSS
    /// source.
    fn styled_from(root: Element, css_src: &str) -> Styled {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        styled(&doc, &[sheet])
    }

    fn text_items(list: &DisplayList) -> Vec<&DisplayItem> {
        list.items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect()
    }

    #[test]
    fn cascaded_color_reaches_the_display_list() {
        let root = Element::new("p").text("hello");
        let styled = styled_from(root, "p { color: #ff0000; }");
        let list = paint_styled_document(&styled.document, &styled.styles, 800.0, 600.0);
        let texts = text_items(&list);
        assert_eq!(texts.len(), 1);
        match texts[0] {
            DisplayItem::Text { color, text, .. } => {
                assert_eq!(*color, Rgba8::rgb(255, 0, 0));
                assert_eq!(text, "hello");
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn cascaded_font_size_reaches_the_display_list() {
        let root = Element::new("p").text("hi");
        let styled = styled_from(root, "p { font-size: 40px; }");
        let list = paint_styled_document(&styled.document, &styled.styles, 800.0, 600.0);
        match text_items(&list)[0] {
            DisplayItem::Text { size, .. } => assert_eq!(*size, 40.0),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn display_none_hides_the_element_and_its_subtree() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden"))
            .push(Element::new("span").text("visible"));
        let styled = styled_from(root, "p { display: none; }");
        let list = paint_styled_document(&styled.document, &styled.styles, 800.0, 600.0);
        let texts: Vec<&str> = text_items(&list)
            .into_iter()
            .filter_map(|i| match i {
                DisplayItem::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["visible"]);
    }

    #[test]
    fn margin_advances_vertical_position_between_siblings() {
        let root = Element::new("div")
            .push(Element::new("p").text("first"))
            .push(Element::new("p").text("second"));
        let styled = styled_from(root, "p { margin: 50px 0; font-size: 10px; }");
        let list = paint_styled_document(&styled.document, &styled.styles, 800.0, 600.0);
        let texts = text_items(&list);
        let ys: Vec<f32> = texts
            .iter()
            .map(|i| match i {
                DisplayItem::Text { y, .. } => *y,
                _ => unreachable!(),
            })
            .collect();
        // second's margin-top (50) plus first's own line advance (10*1.6)
        // separate them by well over the plain per-line spacing alone.
        assert!(ys[1] - ys[0] > 50.0);
    }

    #[test]
    fn empty_document_falls_back_to_hint_text() {
        let doc = dom::Node::document(vec![]);
        let styled = styled(&doc, &[]);
        let list = paint_styled_document(&styled.document, &styled.styles, 100.0, 100.0);
        assert!(
            text_items(&list)
                .iter()
                .any(|i| matches!(i, DisplayItem::Text { .. }))
        );
    }

    #[test]
    fn background_and_accent_chrome_still_present() {
        let root = Element::new("p").text("x");
        let styled = styled_from(root, "");
        let list = paint_styled_document(&styled.document, &styled.styles, 800.0, 600.0);
        assert!(matches!(
            list.items[0],
            DisplayItem::Rect {
                w: 800.0,
                h: 600.0,
                ..
            }
        ));
        assert!(matches!(list.items[1], DisplayItem::Rect { w: 4.0, .. }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canvas_image_number_names_its_node_and_no_other_image() {
        let node = NodeId::from_bits((5 << 32) | 9).expect("an id");
        let id = canvas_image(node);
        assert_eq!(canvas_node(id), Some(node));
        assert_eq!(canvas_node(1), None);
        assert_eq!(canvas_node(42 | 1 << 63), None);
        assert_eq!(canvas_node(CANVAS_IMAGE | 1 << 63), None);
    }
    use dom::Element;

    #[test]
    fn paints_background_accent_and_texts() {
        let body = Element::new("body")
            .push(Element::new("h1").text("Heading"))
            .push(Element::new("p").text("Body line"));
        let doc = Node::document(vec![Element::new("html").push(body).into()]);

        let list = paint_document(&doc, 800.0, 600.0);
        assert!(list.items.len() >= 4, "expected bg + accent + 2 texts");
        match &list.items[0] {
            DisplayItem::Rect { w, h, color, .. } => {
                assert_eq!((*w, *h), (800.0, 600.0));
                assert_eq!(*color, Rgba8::rgb(0x1e, 0x1e, 0x2e));
            }
            other => panic!("first item must be the background rect, got {other:?}"),
        }
        let text_items: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect();
        assert_eq!(text_items.len(), 2);
    }

    #[test]
    fn empty_document_gets_fallback_text() {
        let list = paint_document(&Node::document(vec![]), 100.0, 100.0);
        assert!(
            list.items
                .iter()
                .any(|i| matches!(i, DisplayItem::Text { .. })),
            "expected a fallback text item"
        );
    }

    #[test]
    fn text_sized_by_tag_stacks_top_to_bottom() {
        let body = Element::new("body")
            .push(Element::new("h1").text("Heading"))
            .push(Element::new("p").text("Body line"));
        let doc = Node::document(vec![Element::new("html").push(body).into()]);
        let list = paint_document(&doc, 800.0, 600.0);
        let texts: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect();
        match (texts[0], texts[1]) {
            (
                DisplayItem::Text {
                    size: s1, y: y1, ..
                },
                DisplayItem::Text {
                    size: s2, y: y2, ..
                },
            ) => {
                assert_eq!((*s1, *s2), (28.0, 16.0));
                assert!(*y2 > *y1, "body must render below the heading");
            }
            _ => panic!("expected two text items"),
        }
    }

    #[test]
    fn whitespace_only_texts_are_skipped() {
        let doc = Node::document(vec![Element::new("p").text("   \n  ").into()]);
        let list = paint_document(&doc, 100.0, 100.0);
        // No visible text anywhere: the fallback heading fires.
        match list.items.get(2) {
            Some(DisplayItem::Text { text, .. }) => {
                assert_eq!(text, "Hello from Silver Browser");
            }
            other => panic!("expected fallback text, got {other:?}"),
        }
    }

    #[test]
    fn style_and_script_text_is_not_rendered() {
        let doc = Node::document(vec![
            Element::new("style").text("h1 { color: red; }").into(),
            Element::new("script").text("alert(1)").into(),
            Element::new("p").text("Visible").into(),
        ]);
        let list = paint_document(&doc, 100.0, 100.0);
        let texts: Vec<&str> = list
            .items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Visible"]);
    }
}
