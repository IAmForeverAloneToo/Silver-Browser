//! Caret positions from points (CSSOM View section 6.1, gap batch H):
//! what `caretPositionFromPoint` and `caretRangeFromPoint` find at a
//! viewport point, over the layout the hit test reads. The box hit is
//! found as `elementFromPoint` finds it; in a text the boundary nearest
//! the point is taken, its character matched to the text node's as a
//! range's boxes are (`range_rects`); in a box with no text under the
//! point the reference browser's "position for point" is followed: a
//! block container's line nearest the point, else its child box the
//! point is in or after, a flex or grid container's child nearest it,
//! and an empty box's start.

use std::collections::HashMap;

use dom::{Boundary, Document, NodeId, forms};
use layout::{DocumentLayout, Fragment, FragmentKind};
use paint::{Rect, ScrollOffsets, Step, Viewport};
use style::computed::{Position, Visibility};
use style::transform::Matrix;

use super::range_rects::{Drawn, Slot, advance, drawn_chars, map, match_data, match_text};

/// What a point's caret position is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Caret {
    /// A boundary point of the tree.
    At(Boundary),
    /// A place in a text control's value, in UTF-16 units: the control,
    /// which the reference browser's `caretRangeFromPoint` stands for
    /// with the point before it.
    Control(NodeId, u32),
}

/// What the walk drew of `layout`: every box and text not in contents a
/// box skips, its bounds in viewport coordinates by its address.
struct Drawing<'a> {
    seen: HashMap<usize, [f32; 4]>,
    /// The text fragments of each element, in the order drawn.
    texts: HashMap<NodeId, Vec<Drawn<'a>>>,
}

/// A fragment's address, which names it while the layout is borrowed.
fn key(f: &Fragment) -> usize {
    std::ptr::from_ref(f) as usize
}

impl<'a> Drawing<'a> {
    fn of(layout: &'a DocumentLayout, viewport: Viewport, offsets: &dyn ScrollOffsets) -> Self {
        let mut seen = HashMap::new();
        let mut texts: HashMap<NodeId, Vec<Drawn<'a>>> = HashMap::new();
        let mut matrices = vec![Matrix::IDENTITY];
        for step in paint::paint_order_drawn_everywhere(&layout.roots, viewport, offsets) {
            let matrix = *matrices.last().expect("the viewport's");
            match step {
                Step::PushGroup { matrix: inner, .. } => matrices.push(matrix.then(&inner)),
                Step::PopGroup => {
                    if matrices.len() > 1 {
                        matrices.pop();
                    }
                }
                Step::Box { fragment, rect, .. } => {
                    seen.insert(key(fragment), map(&matrix, rect));
                }
                Step::Text { fragment, rect, .. } => {
                    if fragment.generated.is_none()
                        && matches!(fragment.kind, FragmentKind::Text(_))
                    {
                        texts.entry(fragment.node).or_default().push(Drawn {
                            fragment,
                            rect,
                            matrix,
                        });
                    }
                    seen.insert(key(fragment), map(&matrix, rect));
                }
                _ => {}
            }
        }
        Drawing { seen, texts }
    }

    fn bounds(&self, f: &Fragment) -> Option<[f32; 4]> {
        self.seen.get(&key(f)).copied()
    }
}

/// Whether `[x, y, w, h]` holds the point.
fn holds([x, y, w, h]: [f32; 4], (px, py): (f32, f32)) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// How far the point is from `[x, y, w, h]`, squared; zero inside.
fn distance([x, y, w, h]: [f32; 4], (px, py): (f32, f32)) -> f32 {
    let dx = (x - px).max(px - (x + w)).max(0.0);
    let dy = (y - py).max(py - (y + h)).max(0.0);
    dx * dx + dy * dy
}

/// The boundary of `fragment`, a text drawn at viewport `bounds`, whose
/// edge is nearest `x`: its number among the fragment's characters'.
fn nearest_boundary(
    fragment: &Fragment,
    bounds: [f32; 4],
    x: f32,
    fonts: &text::FontCollection,
) -> usize {
    let FragmentKind::Text(text) = &fragment.kind else {
        return 0;
    };
    let count = text.chars().count();
    let scale = if fragment.w > 0.0 {
        bounds[2] / fragment.w
    } else {
        1.0
    };
    (0..=count)
        .min_by(|&a, &b| {
            let at = |n| (bounds[0] + advance(fragment, n, fonts) * scale - x).abs();
            at(a).total_cmp(&at(b))
        })
        .unwrap_or(0)
}

/// The caret position at the viewport point `(x, y)`: in what `hit`,
/// the element the hit test answers there, shows; `svg` the shape of an
/// outer `<svg>` under the point, when the hit is one.
#[allow(clippy::too_many_arguments)]
pub(super) fn caret_at(
    document: &Document,
    layout: &DocumentLayout,
    (viewport, offsets): (Viewport, &dyn ScrollOffsets),
    fonts: &text::FontCollection,
    hit: NodeId,
    svg: Option<NodeId>,
    (x, y): (f32, f32),
) -> Option<Caret> {
    if let Some(shape) = svg {
        return Some(Caret::At(Boundary {
            node: shape,
            offset: 0,
        }));
    }
    let drawing = Drawing::of(layout, viewport, offsets);
    let point = (x, y);
    // A text control's place in its value.
    if forms::is_text_control(document, hit) {
        let offset = control_offset(document, &drawing, hit, point, fonts);
        return Some(Caret::Control(hit, offset));
    }
    // A text of the element hit under the point, the last drawn.
    let texts = drawing.texts.get(&hit);
    if let Some(drawn) = texts.and_then(|texts| {
        texts
            .iter()
            .rev()
            .find(|d| drawing.bounds(d.fragment).is_some_and(|b| holds(b, point)))
    }) {
        return text_caret(document, &drawing, drawn.fragment, point, fonts)
            .or(Some(start_of(hit)));
    }
    // The element's box under the point, else its first.
    let boxes: Vec<&Fragment> = layout
        .iter()
        .filter(|f| {
            f.node == hit
                && f.generated.is_none()
                && matches!(f.kind, FragmentKind::Box | FragmentKind::InlineBox)
                && drawing.bounds(f).is_some()
        })
        .collect();
    let at = boxes
        .iter()
        .copied()
        .find(|f| drawing.bounds(f).is_some_and(|b| holds(b, point)))
        .or(boxes.first().copied());
    match at {
        Some(f) => Some(position_in(document, &drawing, f, point, fonts)),
        None => Some(start_of(hit)),
    }
}

/// The start of `node`.
fn start_of(node: NodeId) -> Caret {
    Caret::At(Boundary { node, offset: 0 })
}

/// The caret position in the text fragment `fragment` nearest the
/// point: the boundary nearest it, in the text node it is of; `None`
/// where its characters are no text node's (generated text).
fn text_caret(
    document: &Document,
    drawing: &Drawing<'_>,
    fragment: &Fragment,
    point: (f32, f32),
    fonts: &text::FontCollection,
) -> Option<Caret> {
    let bounds = drawing.bounds(fragment)?;
    let n = nearest_boundary(fragment, bounds, point.0, fonts);
    let element = fragment.node;
    let boxes = drawing.texts.get(&element)?;
    let index = boxes
        .iter()
        .position(|d| std::ptr::eq(d.fragment, fragment))?;
    let count = match &fragment.kind {
        FragmentKind::Text(text) => text.chars().count(),
        _ => 0,
    };
    let matched = match_text(document, element, boxes);
    // The unit drawn at the character after the boundary, else just
    // past the one before it.
    let find = |place: (usize, usize)| {
        document.children(element).find_map(|child| {
            let slots = matched.get(&child)?;
            let unit = slots.iter().position(|s| *s == Slot::At(place))?;
            Some((child, unit))
        })
    };
    if n < count
        && let Some((node, unit)) = find((index, n))
    {
        return Some(Caret::At(Boundary {
            node,
            offset: unit as u32,
        }));
    }
    let (node, unit) = find((index, n.checked_sub(1)?))?;
    let data = document.text(node)?;
    let width = data
        .encode_utf16()
        .nth(unit)
        .map_or(1, |u| if (0xd800..0xdc00).contains(&u) { 2 } else { 1 });
    Some(Caret::At(Boundary {
        node,
        offset: (unit + width) as u32,
    }))
}

/// Where a line item of a block container's lines is.
struct LineItem<'a> {
    fragment: &'a Fragment,
    bounds: [f32; 4],
}

/// The line items of the block container `f` drawn: its texts, and its
/// atomic inlines, inside its inline boxes too.
fn line_items<'a>(f: &'a Fragment, drawing: &Drawing<'_>, out: &mut Vec<LineItem<'a>>) {
    for child in f.children.iter() {
        match &child.kind {
            FragmentKind::Text(_) => {
                if let Some(bounds) = drawing.bounds(child) {
                    out.push(LineItem {
                        fragment: child,
                        bounds,
                    });
                }
            }
            FragmentKind::InlineBox => line_items(child, drawing, out),
            FragmentKind::Box if child.style.display.is_atomic_inline() => {
                if let Some(bounds) = drawing.bounds(child) {
                    out.push(LineItem {
                        fragment: child,
                        bounds,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Whether a child box is one the point can be placed in by the block
/// direction (the reference browser's hit test candidates): in flow,
/// not floating, visible, and of some height.
fn candidate(f: &Fragment, drawing: &Drawing<'_>) -> Option<[f32; 4]> {
    if !matches!(f.kind, FragmentKind::Box)
        || f.style.position.is_out_of_flow()
        || f.style.float != style::computed::Float::None
        || f.style.visibility != Visibility::Visible
        || matches!(f.style.position, Position::Fixed)
    {
        return None;
    }
    drawing.bounds(f).filter(|b| b[3] > 0.0)
}

/// The caret position in the box `f` nearest the point.
fn position_in(
    document: &Document,
    drawing: &Drawing<'_>,
    f: &Fragment,
    point: (f32, f32),
    fonts: &text::FontCollection,
) -> Caret {
    let node = f.node;
    // An element whose box is replaced is a point before or after it,
    // but for a canvas, which is its own.
    if let Some(caret) = replaced(document, drawing, f, point) {
        return caret;
    }
    if forms::is_text_control(document, node) {
        return Caret::Control(node, control_offset(document, drawing, node, point, fonts));
    }
    // The contents a box skips, or a box drawn as its own and none of
    // it known, are its start.
    if f.style.skips_contents() {
        return start_of(node);
    }
    // Its lines: the one the point is across, else the nearest, and on
    // it the item nearest.
    let mut items = Vec::new();
    line_items(f, drawing, &mut items);
    if !items.is_empty() {
        let line_of = |item: &LineItem<'_>| {
            let [_, y, _, h] = item.bounds;
            if point.1 < y {
                y - point.1
            } else if point.1 >= y + h {
                point.1 - (y + h)
            } else {
                0.0
            }
        };
        let nearest = items
            .iter()
            .min_by(|a, b| {
                line_of(a)
                    .total_cmp(&line_of(b))
                    .then(distance(a.bounds, point).total_cmp(&distance(b.bounds, point)))
            })
            .expect("not empty");
        return match nearest.fragment.kind {
            FragmentKind::Text(_) => text_caret(document, drawing, nearest.fragment, point, fonts)
                .unwrap_or_else(|| start_of(node)),
            _ if holds(nearest.bounds, point) => {
                position_in(document, drawing, nearest.fragment, point, fonts)
            }
            // Beside an atomic inline: before it or after it.
            _ => beside(document, nearest.fragment.node, nearest.bounds, point),
        };
    }
    let children: Vec<(&Fragment, [f32; 4])> = f
        .children
        .iter()
        .filter_map(|child| candidate(child, drawing).map(|b| (child, b)))
        .collect();
    let Some(&(last, last_bounds)) = children.last() else {
        return start_of(node);
    };
    // A flex or grid container's child nearest the point.
    if f.style.display.has_items() {
        let (child, _) = children
            .iter()
            .min_by(|a, b| distance(a.1, point).total_cmp(&distance(b.1, point)))
            .expect("not empty");
        return position_in(document, drawing, child, point, fonts);
    }
    // A block container's child the point is in or after, the first
    // below it when it is above them all.
    if point.1 >= last_bounds[1] {
        return position_in(document, drawing, last, point, fonts);
    }
    let child = children
        .iter()
        .find(|(_, b)| point.1 < b[1] + b[3])
        .map_or(last, |(child, _)| *child);
    position_in(document, drawing, child, point, fonts)
}

/// The caret position at a replaced box `f`, if it is one: a canvas's
/// start; for an image, a medium, a frame or an outer `<svg>`, the point
/// before it in its parent where the point is over its left half, else
/// after it.
fn replaced(
    document: &Document,
    drawing: &Drawing<'_>,
    f: &Fragment,
    point: (f32, f32),
) -> Option<Caret> {
    let node = f.node;
    match document.local_name(node)? {
        "canvas" => Some(start_of(node)),
        "img" | "video" | "audio" | "iframe" | "embed" | "object" => {
            let bounds = drawing.bounds(f)?;
            Some(beside(document, node, bounds, point))
        }
        "svg" if document.svg_tag(node).is_some() => {
            let bounds = drawing.bounds(f)?;
            Some(beside(document, node, bounds, point))
        }
        _ => None,
    }
}

/// The point before `node` in its parent where the point is left of
/// its middle, else the point after it.
fn beside(document: &Document, node: NodeId, bounds: [f32; 4], point: (f32, f32)) -> Caret {
    let Some(parent) = document.parent(node) else {
        return start_of(node);
    };
    let index = document.index_of(node);
    let after = point.0 > bounds[0] + bounds[2] / 2.0;
    Caret::At(Boundary {
        node: parent,
        offset: index + u32::from(after),
    })
}

/// The place in the text control `control`'s value nearest the point:
/// on the line of its text the point is across, or nearest, the
/// boundary nearest it, its character matched to the value's.
fn control_offset(
    document: &Document,
    drawing: &Drawing<'_>,
    control: NodeId,
    point: (f32, f32),
    fonts: &text::FontCollection,
) -> u32 {
    let value = forms::value(document, control);
    if value.is_empty() {
        return 0;
    }
    let Some(boxes) = drawing.texts.get(&control) else {
        return 0;
    };
    let nearest = boxes
        .iter()
        .enumerate()
        .filter_map(|(i, d)| drawing.bounds(d.fragment).map(|b| (i, d, b)))
        .min_by(|a, b| distance(a.2, point).total_cmp(&distance(b.2, point)));
    let Some((index, drawn, bounds)) = nearest else {
        return 0;
    };
    let n = nearest_boundary(drawn.fragment, bounds, point.0, fonts);
    // A password's bullets stand one for each character.
    let shown: String = if document.local_name(control) == Some("input")
        && forms::input_type(document, control) == "password"
    {
        value.chars().map(|_| '\u{2022}').collect()
    } else {
        value.clone()
    };
    let slots = match_data(&shown, &drawn_chars(boxes), &mut 0, &mut false);
    let unit = |place| slots.iter().position(|s| *s == Slot::At(place));
    if let Some(at) = unit((index, n)) {
        return at as u32;
    }
    match n.checked_sub(1).and_then(|before| unit((index, before))) {
        Some(at) => at as u32 + 1,
        None => value.encode_utf16().count() as u32,
    }
}

/// The boxes of the units `start` to `end` of the text control
/// `control`'s value, those on a line together one, in viewport
/// coordinates (an `OpaqueRange`'s, gap batch H); none when the control
/// shows no text.
pub(super) fn control_rects(
    document: &Document,
    layout: &DocumentLayout,
    (viewport, offsets): (Viewport, &dyn ScrollOffsets),
    fonts: &text::FontCollection,
    control: NodeId,
    (start, end): (u32, u32),
) -> Vec<[f32; 4]> {
    let drawing = Drawing::of(layout, viewport, offsets);
    let Some(boxes) = drawing.texts.get(&control) else {
        return Vec::new();
    };
    let value = forms::value(document, control);
    let slots = match_data(&value, &drawn_chars(boxes), &mut 0, &mut false);
    super::range_rects::text_rects(boxes, &slots, start, end, fonts)
}

/// The caret's box of a text control's place `offset`: a line of no
/// width at the boundary of its text there, in viewport coordinates;
/// `None` when the control shows no text.
pub(super) fn control_caret(
    document: &Document,
    layout: &DocumentLayout,
    (viewport, offsets): (Viewport, &dyn ScrollOffsets),
    fonts: &text::FontCollection,
    control: NodeId,
    offset: u32,
) -> Option<[f32; 4]> {
    let drawing = Drawing::of(layout, viewport, offsets);
    let boxes = drawing.texts.get(&control)?;
    let value = forms::value(document, control);
    let slots = match_data(&value, &drawn_chars(boxes), &mut 0, &mut false);
    let offset = (offset as usize).min(slots.len());
    // The character drawn at or after the place, else the end of the
    // one before it.
    let (place, end) = match slots[offset..].iter().find_map(|s| match *s {
        Slot::At(place) => Some(place),
        _ => None,
    }) {
        Some(place) => (place, false),
        None => match slots[..offset].iter().rev().find_map(|s| match *s {
            Slot::At(place) => Some(place),
            _ => None,
        }) {
            Some(place) => (place, true),
            // An empty value: the start of its line.
            None => ((0, 0), false),
        },
    };
    let drawn = boxes.get(place.0)?;
    let x = advance(drawn.fragment, place.1 + usize::from(end), fonts);
    let r = drawn.rect;
    Some(map(
        &drawn.matrix,
        Rect {
            x: r.x + x,
            y: r.y,
            w: 0.0,
            h: r.h,
        },
    ))
}
