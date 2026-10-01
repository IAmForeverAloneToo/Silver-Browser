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
//! edge when it can scroll. Two earlier stopgaps remain, retained rather than deleted
//! (each milestone's own exit criteria asked for the *active* pipeline
//! to move on, not for the previous stage's code to disappear):
//! [`paint_document`] (M1) walks a `dom::Node` directly with hardcoded
//! per-tag presentation, and [`paint_styled_document`] (M5) walks a
//! `style::StyledDom` with real cascaded colors/font-sizes but still no
//! real box model.

use std::collections::HashMap;

use dom::Node;
use layout::{Fragment, FragmentKind};
use style::StyledDom;
use style::computed::{BOTTOM, BorderStyle, Color, Display, LEFT, Overflow, RIGHT, TOP};
use style::selector_matching::FlatChild;

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
/// `FlatDom` index: zero for one not scrolled. The engine keeps the
/// offsets; `()` is a document scrolled nowhere.
pub trait ScrollOffsets {
    fn offset(&self, node: usize) -> f32;
}

impl ScrollOffsets for () {
    fn offset(&self, _node: usize) -> f32 {
        0.0
    }
}

impl ScrollOffsets for HashMap<usize, f32> {
    fn offset(&self, node: usize) -> f32 {
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

/// One drawable operation in a display list.
#[derive(Debug, Clone)]
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
        /// Baseline, px from viewport top.
        y: f32,
        /// Font size in px.
        size: f32,
        /// Text color.
        color: Rgba8,
    },
    /// Fill a convex quadrilateral, without anti-aliasing, so that
    /// quads sharing an edge tile it exactly: a border's mitred sides
    /// (M10). Points in viewport coordinates, in order around the shape.
    Quad {
        points: [(f32, f32); 4],
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
#[derive(Debug, Clone, Default)]
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

/// Paints a [`StyledDom`] (M5's cascade output) into a display list for
/// a `viewport_w` x `viewport_h` viewport: the same dark background +
/// accent bar chrome as [`paint_document`], then every text run in
/// document order, sized and colored by *its own* cascaded style rather
/// than a per-tag lookup table — real user CSS visibly changes the
/// screen here (roadmap M5's exit criterion). `display: none` skips an
/// element and everything under it, per spec.
pub fn paint_styled_document(styled: &StyledDom, viewport_w: f32, viewport_h: f32) -> DisplayList {
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
    paint_flat_children(styled, 0, &mut items, &mut cursor_y);

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
fn paint_flat_children(
    styled: &StyledDom,
    idx: usize,
    items: &mut Vec<DisplayItem>,
    cursor_y: &mut f32,
) {
    for child in &styled.flat.nodes[idx].children {
        match child {
            FlatChild::Element(child_idx) => {
                let style = &styled.styles[*child_idx];
                if style.display == Display::None {
                    continue; // the element and its whole subtree: skipped
                }
                // `margin: auto` has no meaning in this simplified,
                // horizontal-layout-free flow (real auto-margin
                // centering needs `layout`'s box-width math, M6) — treat
                // it as 0 here, same as an unset margin.
                *cursor_y += style.margin[style::computed::TOP].resolve(0.0, 0.0);
                paint_flat_children(styled, *child_idx, items, cursor_y);
                *cursor_y += style.margin[style::computed::BOTTOM].resolve(0.0, 0.0);
            }
            FlatChild::Text(text) => {
                if text.trim().is_empty() {
                    continue;
                }
                let style = &styled.styles[idx];
                items.push(DisplayItem::Text {
                    text: text.clone(),
                    x: TEXT_X,
                    y: *cursor_y,
                    size: style.font_size_px,
                    color: to_rgba8(style.color),
                });
                *cursor_y += style.font_size_px * 1.6; // matches paint_document's line spacing
            }
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
/// viewport's; a control from M26).
pub fn paint_fragments_in(
    roots: &[Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
) -> DisplayList {
    // CSS2 §14.2: the root element's background paints the canvas, the
    // whole viewport; when the root's is transparent, the body's — the
    // root's first box child, in an HTML document — does instead and is
    // not painted on the body itself. The canvas is white otherwise.
    let canvas = canvas_background(roots);
    let mut painter = Painter {
        items: vec![DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w: viewport.width,
            h: viewport.height,
            color: canvas.map_or(Rgba8::rgb(0xff, 0xff, 0xff), |f| {
                to_rgba8(f.style.background_color)
            }),
        }],
        viewport,
        dy: -viewport.scroll_y,
        offsets,
        canvas_node: canvas.map(|f| f.node),
    };
    painter.paint_all(roots);
    let mut items = painter.items;

    // A document with nothing in it at all (an empty or failed load) is
    // never a silently blank window. A document whose content merely
    // lies outside this viewport is blank here, as it should be.
    if roots.is_empty() {
        items.push(DisplayItem::Text {
            text: "Hello from Silver Browser".to_string(),
            x: 40.0,
            y: 32.0,
            size: 16.0,
            color: Rgba8::rgb(0x33, 0x33, 0x33),
        });
    }

    DisplayList { items }
}

/// The fragment whose background the canvas takes (CSS2 §14.2): the
/// root element's when it has one, else the body's — the root's first
/// box child, in an HTML document.
fn canvas_background(roots: &[Fragment]) -> Option<&Fragment> {
    fn is_box(f: &Fragment) -> bool {
        matches!(f.kind, FragmentKind::Box)
    }
    fn with_background(f: &Fragment) -> Option<&Fragment> {
        (f.style.background_color.a > 0.0).then_some(f)
    }
    let root = roots.first().filter(|f| is_box(f))?;
    with_background(root).or_else(|| {
        root.children
            .iter()
            .find(|f| is_box(f))
            .and_then(with_background)
    })
}

/// One walk over a fragment tree for one viewport.
struct Painter<'a> {
    items: Vec<DisplayItem>,
    viewport: Viewport,
    /// From a document `y` to the viewport's: the viewport's scroll
    /// offset and, inside a scroll container, its offset too (M10.5),
    /// negated.
    dy: f32,
    offsets: &'a dyn ScrollOffsets,
    /// The element whose background became the canvas's, and so is not
    /// painted on its own box again.
    canvas_node: Option<usize>,
}

impl Painter<'_> {
    fn paint_all(&mut self, fragments: &[Fragment]) {
        for f in fragments {
            self.paint(f);
        }
    }

    /// Whether a document-space rectangle, as scrolled so far, meets
    /// the viewport at all.
    fn meets_viewport(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let v = self.viewport;
        let y = y + self.dy;
        x < v.width && x + w > 0.0 && y < v.height && y + h > 0.0
    }

    fn paint(&mut self, f: &Fragment) {
        match &f.kind {
            FragmentKind::Box => {
                let clips = f.style.overflow.clips();
                let in_view = self.meets_viewport(f.x, f.y, f.w, f.h);
                if clips && !in_view {
                    // Nothing under a clip can show where the clip
                    // itself does not.
                    return;
                }
                if in_view {
                    self.paint_box(f);
                }
                if clips {
                    let (x, y, w, h) = padding_box(f);
                    let y = y + self.dy;
                    self.items.push(DisplayItem::PushClip { x, y, w, h });
                    // The content is scrolled; the box, its clip and its
                    // scrollbar are not.
                    let offset = self.offsets.offset(f.node);
                    self.dy -= offset;
                    self.paint_all(&f.children);
                    self.dy += offset;
                    self.push_scrollbar(f, (x, y, w, h), offset);
                    self.items.push(DisplayItem::PopClip);
                } else {
                    self.paint_all(&f.children);
                }
            }
            FragmentKind::Text(text) => {
                if !text.trim().is_empty() && self.meets_viewport(f.x, f.top(), f.w, f.h) {
                    self.items.push(DisplayItem::Text {
                        text: text.clone(),
                        x: f.x,
                        // A text fragment's `y` is already its baseline.
                        y: f.y + self.dy,
                        size: f.style.font_size_px,
                        color: to_rgba8(f.style.color),
                    });
                }
            }
        }
    }

    /// A scroll container's overlay scrollbar (M10.5) over the right
    /// edge of its padding box `(x, y, w, h)`, in viewport coordinates,
    /// when it can scroll: `overflow: scroll` always, `auto` only with
    /// content past the box, `hidden` never.
    fn push_scrollbar(&mut self, f: &Fragment, padding_box: (f32, f32, f32, f32), offset: f32) {
        let max = f.max_scroll();
        let shows = match f.style.overflow {
            Overflow::Scroll => true,
            Overflow::Auto => max > 0.0,
            Overflow::Hidden | Overflow::Visible => false,
        };
        let (x, y, w, h) = padding_box;
        if !shows || w < ELEMENT_SCROLLBAR_WIDTH || h <= 0.0 {
            return;
        }
        let bar_x = x + w - ELEMENT_SCROLLBAR_WIDTH;
        self.items.push(DisplayItem::Rect {
            x: bar_x,
            y,
            w: ELEMENT_SCROLLBAR_WIDTH,
            h,
            color: SCROLLBAR_TRACK,
        });
        let (thumb_y, thumb_h) = scrollbar_thumb(h, max, offset);
        self.items.push(DisplayItem::Rect {
            x: bar_x + 1.0,
            y: y + thumb_y,
            w: ELEMENT_SCROLLBAR_WIDTH - 2.0,
            h: thumb_h,
            color: SCROLLBAR_THUMB,
        });
    }

    /// A box's own background and border, in viewport coordinates.
    fn paint_box(&mut self, f: &Fragment) {
        let y = f.y + self.dy;
        if f.style.background_color.a > 0.0 && self.canvas_node != Some(f.node) {
            self.items.push(DisplayItem::Rect {
                x: f.x,
                y,
                w: f.w,
                h: f.h,
                color: to_rgba8(f.style.background_color),
            });
        }
        if f.style.border_style.iter().any(|s| s.paints()) {
            push_border(&mut self.items, f, y);
        }
    }
}

/// The padding box of a box fragment, in document coordinates: the
/// border box inset by the border, zero-width on a side whose style
/// paints nothing (CSS2 §8.5.3). What `overflow` clips to (CSS2
/// §11.1.1).
fn padding_box(f: &Fragment) -> (f32, f32, f32, f32) {
    let [top, right, bottom, left] = f.style.used_border_width();
    (
        f.x + left,
        f.y + top,
        (f.w - left - right).max(0.0),
        (f.h - top - bottom).max(0.0),
    )
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

/// Paints a box fragment's border, side by side (M10). The four sides
/// meet at mitred corners, as in CSS2 §8.5: each side is drawn as the
/// trapezoid between the border box's edge and the padding box's,
/// which the quads of [`band`] follow. When all four sides are solid in
/// one colour the mitres are invisible and the sides are plain strips,
/// which paint without the seams anti-aliased diagonals leave. `y` is
/// the box's top in viewport coordinates.
///
/// Per style: `solid` is the whole band; `double` the outer and inner
/// thirds; `dashed` and `dotted` are dashes of three times the width
/// and dots of the width, with equal gaps, from each side's start, drawn
/// as plain strips without the fitting adjustment real browsers make;
/// `inset`, `outset`, `groove` and `ridge` use the colour and its
/// darker tone — the last four are approximations, said so here.
fn push_border(items: &mut Vec<DisplayItem>, f: &Fragment, y: f32) {
    let widths = f.style.used_border_width();
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
        push_solid_strips(items, f, y, widths);
        return;
    }
    let geometry = (f.x, y, f.w, f.h, widths);
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
        let quad = |a: f32, b: f32, color: Rgba8| DisplayItem::Quad {
            points: band(side, geometry, a, b),
            color,
        };
        match f.style.border_style[side] {
            BorderStyle::None | BorderStyle::Hidden => {}
            BorderStyle::Solid => items.push(quad(0.0, 1.0, color)),
            BorderStyle::Double => {
                items.push(quad(0.0, 1.0 / 3.0, color));
                items.push(quad(2.0 / 3.0, 1.0, color));
            }
            BorderStyle::Dashed => push_dashes(items, side, geometry, width * 3.0, color),
            BorderStyle::Dotted => push_dashes(items, side, geometry, width, color),
            BorderStyle::Inset => items.push(quad(0.0, 1.0, if lit { dark } else { color })),
            BorderStyle::Outset => items.push(quad(0.0, 1.0, if lit { color } else { dark })),
            BorderStyle::Groove => {
                let (outer, inner) = if lit { (dark, color) } else { (color, dark) };
                items.push(quad(0.0, 0.5, outer));
                items.push(quad(0.5, 1.0, inner));
            }
            BorderStyle::Ridge => {
                let (outer, inner) = if lit { (color, dark) } else { (dark, color) };
                items.push(quad(0.0, 0.5, outer));
                items.push(quad(0.5, 1.0, inner));
            }
        }
    }
}

/// The four strips of a solid border in one colour (M6.8's shape:
/// adjacent strips overlap at the corners, which a single colour makes
/// invisible).
fn push_solid_strips(items: &mut Vec<DisplayItem>, f: &Fragment, y: f32, widths: [f32; 4]) {
    let [top, right, bottom, left] = widths;
    let color = to_rgba8(f.style.border_color[TOP]);
    if top > 0.0 {
        items.push(DisplayItem::Rect {
            x: f.x,
            y,
            w: f.w,
            h: top,
            color,
        });
    }
    if bottom > 0.0 {
        items.push(DisplayItem::Rect {
            x: f.x,
            y: y + f.h - bottom,
            w: f.w,
            h: bottom,
            color,
        });
    }
    if left > 0.0 {
        items.push(DisplayItem::Rect {
            x: f.x,
            y,
            w: left,
            h: f.h,
            color,
        });
    }
    if right > 0.0 {
        items.push(DisplayItem::Rect {
            x: f.x + f.w - right,
            y,
            w: right,
            h: f.h,
            color,
        });
    }
}

/// The border geometry a side's bands are computed from: the border
/// box's `(x, y, w, h)` in viewport coordinates and the four widths.
type BorderGeometry = (f32, f32, f32, f32, [f32; 4]);

/// The quad of `side`'s band between the fractions `a` and `b` of its
/// width (0 the outer edge, 1 the inner), its ends on the mitre lines
/// to the neighbouring sides.
fn band(side: usize, geometry: BorderGeometry, a: f32, b: f32) -> [(f32, f32); 4] {
    let (x, y, w, h, [top, right, bottom, left]) = geometry;
    // The corner of the band at fraction `f` along the mitre, per side.
    let top_left = |f: f32| (x + left * f, y + top * f);
    let top_right = |f: f32| (x + w - right * f, y + top * f);
    let bottom_right = |f: f32| (x + w - right * f, y + h - bottom * f);
    let bottom_left = |f: f32| (x + left * f, y + h - bottom * f);
    match side {
        TOP => [top_left(a), top_right(a), top_right(b), top_left(b)],
        RIGHT => [top_right(a), bottom_right(a), bottom_right(b), top_right(b)],
        BOTTOM => [
            bottom_left(a),
            bottom_right(a),
            bottom_right(b),
            bottom_left(b),
        ],
        _ => [top_left(a), bottom_left(a), bottom_left(b), top_left(b)],
    }
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
mod fragment_tests {
    use super::*;
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
        display_list_scrolled(root, css_src, viewport, &())
    }

    /// [`display_list_for`] with scroll containers scrolled (M10.5).
    /// Nodes: html 1, body 2, the fixture's root 3.
    fn display_list_scrolled(
        root: Element,
        css_src: &str,
        viewport: Viewport,
        offsets: &dyn ScrollOffsets,
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
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = layout::build_box_tree(&styled);
        let layout = layout::layout_document(&boxes, VIEWPORT, &fonts);
        paint_fragments_in(&layout.roots, viewport, offsets)
    }

    fn rects(list: &DisplayList) -> Vec<&DisplayItem> {
        list.items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Rect { .. }))
            .collect()
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

    #[test]
    fn border_paints_four_strips_at_the_box_edges() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; margin: 0;
                   border-width: 4px; border-style: solid; border-color: #00ff00; }",
        );
        let green: Vec<&DisplayItem> = rects(&list)
            .into_iter()
            .filter(
                |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(0, 255, 0)),
            )
            .collect();
        assert_eq!(green.len(), 4); // top, bottom, left, right
        // Border-box is 100+8=108 wide, 50+8=58 tall (border adds to content).
        let has = |want: (f32, f32, f32, f32)| {
            green.iter().any(
                |i| matches!(i, DisplayItem::Rect { x, y, w, h, .. } if (*x, *y, *w, *h) == want),
            )
        };
        assert!(has((0.0, 0.0, 108.0, 4.0)), "top strip");
        assert!(has((0.0, 54.0, 108.0, 4.0)), "bottom strip");
        assert!(has((0.0, 0.0, 4.0, 58.0)), "left strip");
        assert!(has((104.0, 0.0, 4.0, 58.0)), "right strip");
    }

    #[test]
    fn border_style_none_paints_no_border_even_with_width_set() {
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; border-width: 4px; }", // no border-style
        );
        assert_eq!(rects(&list).len(), 1); // just the viewport background
        let list = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; border: 4px hidden red; }",
        );
        assert_eq!(rects(&list).len(), 1);
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

    #[test]
    fn sides_of_different_colours_meet_at_mitred_corners() {
        // A 100 by 50 box with a 10px top border and 20px left border in
        // two colours: each side is a trapezoid between the border edge
        // and the padding edge, meeting on the diagonal.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border-top: 10px solid #ff0000; border-left: 20px solid #0000ff;
                   border-right: 0 solid #000000; border-bottom: 0 solid #000000; }",
        );
        let bands = quads(&list);
        assert_eq!(bands.len(), 2);
        let (top, red) = bands[0];
        assert_eq!(red, Rgba8::rgb(255, 0, 0));
        assert_eq!(top, [(0.0, 0.0), (100.0, 0.0), (100.0, 10.0), (20.0, 10.0)]);
        let (left, blue) = bands[1];
        assert_eq!(blue, Rgba8::rgb(0, 0, 255));
        assert_eq!(left, [(0.0, 0.0), (0.0, 50.0), (20.0, 50.0), (20.0, 10.0)]);
        // Four solid sides in one colour are the plain strips of before.
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; border: 4px solid #ff0000; }",
        );
        assert!(quads(&list).is_empty());
        assert_eq!(rects(&list).len(), 5);
    }

    #[test]
    fn double_borders_are_two_bands_and_dashes_run_along_the_side() {
        let list = display_list_of(
            Element::new("div"),
            "div { width: 100px; height: 50px; margin: 0; box-sizing: border-box;
                   border: 9px double #000000; border-bottom-style: none; border-left-style: none;
                   border-right-style: dashed; }",
        );
        let quads = quads(&list);
        // Top: the outer third and the inner third, 3px each, 3px apart.
        assert_eq!(quads.len(), 2);
        assert_eq!(
            quads[0].0,
            [(0.0, 0.0), (100.0, 0.0), (97.0, 3.0), (0.0, 3.0)]
        );
        assert_eq!(
            quads[1].0,
            [(0.0, 6.0), (94.0, 6.0), (91.0, 9.0), (0.0, 9.0)]
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
    fn text_fragment_paints_at_its_baseline() {
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
                // (M7.1/M7.4) — not simply the font-size.
                assert_eq!(
                    (*x, *y, *size),
                    (0.0, fonts.primary().metrics(12.0).ascent, 12.0)
                );
                assert_eq!(*color, Rgba8::rgb(0, 0, 255));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn empty_document_falls_back_to_hint_text() {
        let fonts = test_fonts();
        let doc = dom::Node::document(vec![]);
        let styled = style::style_document(&doc, &[]);
        let boxes = layout::build_box_tree(&styled);
        let layout = layout::layout_document(&boxes, VIEWPORT, &fonts);
        let list = paint_fragments(&layout.roots, Viewport::new(VIEWPORT, 600.0));
        assert!(
            list.items
                .iter()
                .any(|i| matches!(i, DisplayItem::Text { .. }))
        );
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
    fn the_scroll_offset_moves_items_up_and_culls_what_the_viewport_misses() {
        // Three 100px boxes at document y = 0, 100, 200. A 150px-tall
        // viewport scrolled to 120 sees the second (at -20) and the
        // third (at 80); the first ends at 100, above the viewport.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "a"))
            .push(Element::new("div").attr("class", "b"))
            .push(Element::new("div").attr("class", "c"));
        let css = "div { margin: 0; height: 100px; background-color: #ff0000; }
                   div div { height: 100px; }
                   .a, .b, .c { background-color: #00ff00; }";
        let unscrolled = display_list_for(root.clone(), css, Viewport::new(VIEWPORT, 150.0));
        assert_eq!(
            rect_rows(&unscrolled),
            vec![(0.0, 100.0), (0.0, 100.0), (100.0, 100.0)],
            "the outer div (100 tall), a, b; c at 200 is out of view"
        );
        let scrolled = display_list_for(
            root,
            css,
            Viewport {
                width: VIEWPORT,
                height: 150.0,
                scroll_y: 120.0,
            },
        );
        assert_eq!(rect_rows(&scrolled), vec![(-20.0, 100.0), (80.0, 100.0)]);
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
        let push = items
            .iter()
            .position(|i| matches!(i, DisplayItem::PushClip { .. }))
            .expect("a clip is pushed");
        let pop = items
            .iter()
            .position(|i| matches!(i, DisplayItem::PopClip))
            .expect("the clip is popped");
        // The border box is 60 x 30; the padding box sits inside the
        // 3px border: 54 x 24 at (3, 3).
        assert!(matches!(
            items[push],
            DisplayItem::PushClip {
                x: 3.0,
                y: 3.0,
                w: 54.0,
                h: 24.0
            }
        ));
        let text = items
            .iter()
            .position(|i| matches!(i, DisplayItem::Text { .. }))
            .expect("the text paints");
        assert!(push < text && text < pop, "the text is inside the clip");
        // The box's own border paints outside the clip, before it.
        assert!(
            items[1..push]
                .iter()
                .filter(|i| matches!(i, DisplayItem::Rect { .. }))
                .count()
                >= 4
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
        let offsets: HashMap<usize, f32> = [(3, 20.0)].into_iter().collect();
        let list = display_list_scrolled(root, css, Viewport::new(VIEWPORT, 600.0), &offsets);
        let rect_y = |color: Rgba8| {
            list.items.iter().find_map(|i| match i {
                DisplayItem::Rect { y, color: c, .. } if *c == color => Some(*y),
                _ => None,
            })
        };
        assert_eq!(rect_y(Rgba8::rgb(255, 0, 0)), Some(-20.0));
        assert_eq!(rect_y(Rgba8::rgb(0, 0, 255)), Some(10.0));
        // Scrolled past the top of the viewport, the red box is culled
        // like anything else out of view.
        let offsets: HashMap<usize, f32> = [(3, 40.0)].into_iter().collect();
        let culled = display_list_scrolled(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p").attr("class", "r"))
                .push(Element::new("p").attr("class", "b")),
            css,
            Viewport::new(VIEWPORT, 600.0),
            &offsets,
        );
        assert!(!culled.items.iter().any(
            |i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(255, 0, 0))
        ));
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
            .position(|i| matches!(i, DisplayItem::PopClip))
            .unwrap();
        assert!(track < pop);
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
    use dom::Element;

    /// Builds a `StyledDom` from an already-built element and CSS source.
    fn styled_from(root: Element, css_src: &str) -> StyledDom {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        style::style_document(&doc, &[sheet])
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
        let list = paint_styled_document(&styled, 800.0, 600.0);
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
        let list = paint_styled_document(&styled, 800.0, 600.0);
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
        let list = paint_styled_document(&styled, 800.0, 600.0);
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
        let list = paint_styled_document(&styled, 800.0, 600.0);
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
        let styled = style::style_document(&doc, &[]);
        let list = paint_styled_document(&styled, 100.0, 100.0);
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
        let list = paint_styled_document(&styled, 800.0, 600.0);
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
