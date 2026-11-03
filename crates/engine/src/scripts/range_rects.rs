//! A range's boxes (CSSOM View section 6.2, D65): `Range`'s
//! `getClientRects` and `getBoundingClientRect`. Each element the range
//! holds whole, whose parent it does not, gives its border boxes; each
//! text node it reaches, outside such an element, the part of its text
//! boxes its characters are drawn in, from the text fragments of the
//! element around it, their text matched to the node's by its
//! characters (white space the layout collapsed takes no room).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use dom::{Boundary, Document, NodeId};
use layout::{DocumentLayout, Fragment, FragmentKind};
use paint::{Rect, ScrollOffsets, Step, Viewport};
use style::transform::Matrix;

/// One text fragment of an element, where the walk drew it: its line
/// box in its group's coordinates and the matrix to the viewport's.
pub(crate) struct Drawn<'a> {
    pub(crate) fragment: &'a Fragment,
    pub(crate) rect: Rect,
    pub(crate) matrix: Matrix,
}

/// Where a character of a text node is drawn: a fragment's number in
/// its element's list and the character's number in its text.
pub(crate) type Place = (usize, usize);

/// Where a UTF-16 unit of a text node is: a character drawn; a space
/// the layout left as the room between the characters drawn before and
/// after it rather than drawing; or nowhere, collapsed away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Slot {
    At(Place),
    Gap(Option<Place>, Option<Place>),
    Nowhere,
}

/// The rectangles of the range from `start` to `end` in viewport
/// coordinates, in tree order: `[x, y, width, height]` each. An element
/// in `unboxed`, of `display: contents`, gives none of its own: the
/// range reaches its children as if they were its parent's (D77).
pub(super) fn range_rects(
    document: &Document,
    layout: &DocumentLayout,
    (viewport, offsets): (Viewport, &dyn ScrollOffsets),
    fonts: &text::FontCollection,
    unboxed: &HashSet<NodeId>,
    start: Boundary,
    end: Boundary,
) -> Vec<[f32; 4]> {
    if document.compare_boundaries(start, end) == Some(Ordering::Greater) {
        return Vec::new();
    }
    let mut root = start.node;
    while !document.is_inclusive_ancestor(root, end.node) {
        match document.parent(root) {
            Some(parent) => root = parent,
            None => return Vec::new(),
        }
    }
    // What the range holds, in tree order: elements whole, and text
    // nodes' parts in UTF-16 units.
    enum Held {
        Element(NodeId),
        Text(NodeId, u32, u32),
    }
    let mut held = Vec::new();
    let mut whole: HashSet<NodeId> = HashSet::new();
    for node in document.subtree(root) {
        let parent = document.parent(node);
        if parent.is_some_and(|p| whole.contains(&p)) {
            whole.insert(node);
            continue;
        }
        if let Some(data) = document.text(node) {
            let length = data.encode_utf16().count() as u32;
            let at = |offset| Boundary { node, offset };
            let from = match document.compare_boundaries(start, at(0)) {
                Some(Ordering::Greater) if start.node == node => start.offset.min(length),
                Some(Ordering::Greater) => continue,
                _ => 0,
            };
            let to = match document.compare_boundaries(end, at(length)) {
                Some(Ordering::Less) if end.node == node => end.offset.min(length),
                Some(Ordering::Less) => continue,
                _ => length,
            };
            held.push(Held::Text(node, from, to));
        } else if document.is_element(node)
            && let Some(parent) = parent
        {
            let index = document.index_of(node);
            let before = Boundary {
                node: parent,
                offset: index,
            };
            let after = Boundary {
                node: parent,
                offset: index + 1,
            };
            let contained = document.compare_boundaries(start, before) != Some(Ordering::Greater)
                && document.compare_boundaries(after, end) != Some(Ordering::Greater);
            if contained && !unboxed.contains(&node) {
                whole.insert(node);
                held.push(Held::Element(node));
            }
        }
    }
    let elements: HashSet<NodeId> = held
        .iter()
        .filter_map(|h| match h {
            Held::Element(node) => Some(*node),
            Held::Text(..) => None,
        })
        .collect();
    let mut around: HashSet<NodeId> = held
        .iter()
        .filter_map(|h| match h {
            Held::Text(node, ..) => document.parent(*node),
            Held::Element(_) => None,
        })
        .collect();
    let mut measured = crate::geometry::measure(layout, viewport, offsets, &elements);
    // An element held whole that has no box, as one of `display:
    // contents` has none, gives what is inside it instead (CSSOM View
    // section 6.2 includes such elements recursively): its children
    // with boxes, its text, and the same of its children without.
    let mut inner = HashSet::new();
    for &node in elements.iter().filter(|node| !measured.contains_key(node)) {
        for at in document.subtree(node).skip(1) {
            if document.is_element(at) {
                inner.insert(at);
            } else if document.text(at).is_some()
                && let Some(parent) = document.parent(at)
            {
                around.insert(parent);
            }
        }
    }
    if !inner.is_empty() {
        measured.extend(crate::geometry::measure(layout, viewport, offsets, &inner));
    }
    let mut expanded = Vec::with_capacity(held.len());
    for h in held {
        let node = match h {
            Held::Element(node) if !measured.contains_key(&node) => node,
            other => {
                expanded.push(other);
                continue;
            }
        };
        let mut stack: Vec<NodeId> = document.children(node).collect();
        stack.reverse();
        while let Some(at) = stack.pop() {
            if let Some(data) = document.text(at) {
                expanded.push(Held::Text(at, 0, data.encode_utf16().count() as u32));
            } else if measured.contains_key(&at) {
                expanded.push(Held::Element(at));
            } else if document.is_element(at) {
                let at_children = stack.len();
                stack.extend(document.children(at));
                stack[at_children..].reverse();
            }
        }
    }
    let drawn = text_fragments(layout, viewport, offsets, &around);
    let mut matched: HashMap<NodeId, HashMap<NodeId, Vec<Slot>>> = HashMap::new();
    let mut out = Vec::new();
    for h in expanded {
        match h {
            Held::Element(node) => {
                if let Some(m) = measured.get(&node) {
                    out.extend(m.rects.iter().copied());
                }
            }
            Held::Text(node, from, to) => {
                let Some(parent) = document.parent(node) else {
                    continue;
                };
                let Some(boxes) = drawn.get(&parent) else {
                    continue;
                };
                let places = matched
                    .entry(parent)
                    .or_insert_with(|| match_text(document, parent, boxes));
                if let Some(places) = places.get(&node) {
                    out.extend(text_rects(boxes, places, from, to, fonts));
                }
            }
        }
    }
    out
}

/// The text fragments of the elements in `elements`, each element's in
/// the order they are drawn.
pub(crate) fn text_fragments<'a>(
    layout: &'a DocumentLayout,
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
    elements: &HashSet<NodeId>,
) -> HashMap<NodeId, Vec<Drawn<'a>>> {
    let mut out: HashMap<NodeId, Vec<Drawn<'a>>> = HashMap::new();
    if elements.is_empty() {
        return out;
    }
    let mut matrices = vec![Matrix::IDENTITY];
    for step in paint::paint_order_everywhere(&layout.roots, viewport, offsets) {
        let matrix = *matrices.last().expect("the viewport's");
        match step {
            Step::PushGroup { matrix: inner, .. } => matrices.push(matrix.then(&inner)),
            Step::PopGroup => {
                if matrices.len() > 1 {
                    matrices.pop();
                }
            }
            Step::Text { fragment, rect, .. }
                if fragment.generated.is_none()
                    && matches!(fragment.kind, FragmentKind::Text(_))
                    && elements.contains(&fragment.node) =>
            {
                out.entry(fragment.node).or_default().push(Drawn {
                    fragment,
                    rect,
                    matrix,
                });
            }
            _ => {}
        }
    }
    out
}

/// Each of `element`'s text children's characters matched to where the
/// fragments draw them, by UTF-16 unit; the second unit of a pair is
/// nowhere.
pub(crate) fn match_text(
    document: &Document,
    element: NodeId,
    boxes: &[Drawn<'_>],
) -> HashMap<NodeId, Vec<Slot>> {
    let drawn = drawn_chars(boxes);
    let mut at = 0;
    // Whether the last unit was white space.
    let mut in_gap = false;
    let mut out = HashMap::new();
    for child in document.children(element) {
        let Some(data) = document.text(child) else {
            continue;
        };
        out.insert(child, match_data(data, &drawn, &mut at, &mut in_gap));
    }
    out
}

/// The characters `boxes` draw, each with its place.
pub(crate) fn drawn_chars(boxes: &[Drawn<'_>]) -> Vec<(Place, char)> {
    boxes
        .iter()
        .enumerate()
        .flat_map(|(i, d)| {
            let text = match &d.fragment.kind {
                FragmentKind::Text(text) => text.as_str(),
                _ => "",
            };
            text.chars().enumerate().map(move |(c, ch)| ((i, c), ch))
        })
        .collect()
}

/// `data`'s characters matched to where `drawn` draws them, by UTF-16
/// unit, from the character drawn `*at` on, `*in_gap` saying whether
/// the unit before was white space; both moved past it.
pub(crate) fn match_data(
    data: &str,
    drawn: &[(Place, char)],
    at: &mut usize,
    in_gap: &mut bool,
) -> Vec<Slot> {
    let mut slots = Vec::new();
    for ch in data.chars() {
        let slot = match drawn.get(*at) {
            Some(&(place, shown)) if alike(ch, shown) => {
                *at += 1;
                Slot::At(place)
            }
            // White space the layout took out: the first of a run
            // is the room between the words around it.
            _ if is_collapsible(ch) => {
                if *in_gap || *at == 0 {
                    Slot::Nowhere
                } else {
                    Slot::Gap(
                        at.checked_sub(1).and_then(|b| drawn.get(b)).map(|d| d.0),
                        drawn.get(*at).map(|d| d.0),
                    )
                }
            }
            // A soft hyphen or another character drawn only where a
            // line breaks at it, or never: none.
            _ if is_hidden(ch) => Slot::Nowhere,
            // Text the layout changed (a transform, a hyphen):
            // one for one.
            Some(&(place, _)) => {
                *at += 1;
                Slot::At(place)
            }
            None => Slot::Nowhere,
        };
        // After white space, drawn or not, more collapses.
        *in_gap = is_collapsible(ch);
        slots.push(slot);
        if ch.len_utf16() == 2 {
            slots.push(Slot::Nowhere);
        }
    }
    slots
}

/// Whether the source character `ch` is drawn as `shown`: a soft
/// hyphen as the hyphen a line broken at it ends with.
fn alike(ch: char, shown: char) -> bool {
    ch == shown
        || (is_collapsible(ch) && shown == ' ')
        || (ch == '\u{ad}' && matches!(shown, '-' | '\u{2010}'))
        || ch.to_lowercase().eq(shown.to_lowercase())
}

/// A character that takes no room unless a line breaks at it: the soft
/// hyphen, and the zero-width space and joiners.
fn is_hidden(ch: char) -> bool {
    matches!(
        ch,
        '\u{ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
    )
}

/// White space that collapses (CSS Text 3 section 4.1.1).
fn is_collapsible(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// The rectangles of the units `from` to `to` of a text node matched by
/// `slots` to `boxes`, those on a line together made one; a rectangle
/// of no width where they take no room, at the place of the next unit
/// drawn, or past the last.
pub(crate) fn text_rects(
    boxes: &[Drawn<'_>],
    slots: &[Slot],
    from: u32,
    to: u32,
    fonts: &text::FontCollection,
) -> Vec<[f32; 4]> {
    let (from, to) = (from as usize, (to as usize).min(slots.len()));
    let from = from.min(to);
    // The characters drawn, a run in one fragment at a time, and the
    // gaps, in order.
    enum Piece {
        Run(usize, usize, usize),
        Gap(Option<Place>, Option<Place>),
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for slot in &slots[from..to] {
        match *slot {
            Slot::At((fragment, c)) => match pieces.last_mut() {
                Some(Piece::Run(f, _, last)) if *f == fragment && *last == c => *last = c + 1,
                _ => pieces.push(Piece::Run(fragment, c, c + 1)),
            },
            Slot::Gap(before, after) => pieces.push(Piece::Gap(before, after)),
            Slot::Nowhere => {}
        }
    }
    if pieces.is_empty() {
        // A caret: before the next character drawn, else after the last.
        let next = slots[from..].iter().find_map(|s| match *s {
            Slot::At(place) => Some(place),
            _ => None,
        });
        let caret = match next {
            Some((fragment, c)) => (fragment, c),
            None => match slots[..from].iter().rev().find_map(|s| match *s {
                Slot::At(place) => Some(place),
                _ => None,
            }) {
                Some((fragment, c)) => (fragment, c + 1),
                None => return Vec::new(),
            },
        };
        pieces.push(Piece::Run(caret.0, caret.1, caret.1));
    }
    // A character's left and right edges in its group's coordinates.
    let edges = |(fragment, c): Place| -> Option<(&Drawn<'_>, f32, f32)> {
        let drawn = boxes.get(fragment)?;
        let a = advance(drawn.fragment, c, fonts);
        let b = advance(drawn.fragment, c + 1, fonts);
        Some((drawn, drawn.rect.x + a.min(b), drawn.rect.x + a.max(b)))
    };
    let mut out: Vec<[f32; 4]> = Vec::new();
    for piece in pieces {
        let rect = match piece {
            Piece::Run(fragment, first, last) => boxes.get(fragment).map(|drawn| {
                let a = advance(drawn.fragment, first, fonts);
                let b = advance(drawn.fragment, last, fonts);
                let r = drawn.rect;
                map(
                    &drawn.matrix,
                    Rect {
                        x: r.x + a.min(b),
                        y: r.y,
                        w: (a - b).abs(),
                        h: r.h,
                    },
                )
            }),
            Piece::Gap(before, after) => {
                let before = before.and_then(edges);
                let after = after.and_then(edges);
                match (before, after) {
                    // The room between two words on one line.
                    (Some((b, _, right)), Some((a, left, _)))
                        if b.rect.y == a.rect.y && b.matrix == a.matrix && left >= right =>
                    {
                        Some(map(
                            &b.matrix,
                            Rect {
                                x: right,
                                y: b.rect.y,
                                w: left - right,
                                h: b.rect.h,
                            },
                        ))
                    }
                    // At a line's end: none.
                    (Some((b, _, right)), _) => Some(map(
                        &b.matrix,
                        Rect {
                            x: right,
                            y: b.rect.y,
                            w: 0.0,
                            h: b.rect.h,
                        },
                    )),
                    _ => None,
                }
            }
        };
        let Some(rect) = rect else {
            continue;
        };
        // One with the rectangle before where it goes on along its line.
        match out.last_mut() {
            Some(last)
                if last[1] == rect[1]
                    && last[3] == rect[3]
                    && (last[0] + last[2] - rect[0]).abs() < 0.01 =>
            {
                last[2] = rect[0] + rect[2] - last[0];
            }
            _ => out.push(rect),
        }
    }
    out
}

/// How far from its left edge the text of `f` is drawn before its
/// character `n`: the font's advance of the characters before, the
/// fragment's spacing shared among its characters; from its right edge
/// in a right-to-left run.
pub(crate) fn advance(f: &Fragment, n: usize, fonts: &text::FontCollection) -> f32 {
    let FragmentKind::Text(text) = &f.kind else {
        return 0.0;
    };
    let count = text.chars().count();
    if count == 0 {
        return 0.0;
    }
    let n = n.min(count);
    let size = f.style.font_size_px;
    let byte = text.char_indices().nth(n).map_or(text.len(), |(b, _)| b);
    let natural = fonts.measure_in(f.font, text, size);
    let before = fonts.measure_in(f.font, &text[..byte], size);
    let spacing = (f.w - natural) * n as f32 / count as f32;
    let x = (before + spacing).clamp(0.0, f.w);
    if text.chars().any(right_to_left) {
        f.w - x
    } else {
        x
    }
}

/// Whether `c` is a strong right-to-left character: Hebrew, Arabic and
/// the scripts between and after them in the BMP's right-to-left blocks.
fn right_to_left(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// `r` through `m`: the box around its corners.
pub(super) fn map(m: &Matrix, r: Rect) -> [f32; 4] {
    let corners = [
        (r.x, r.y),
        (r.x + r.w, r.y),
        (r.x, r.y + r.h),
        (r.x + r.w, r.y + r.h),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        let [px, py, _, w] = m.apply(x, y, 0.0);
        let w = if w.abs() > f32::EPSILON { w } else { 1.0 };
        let (px, py) = (px / w, py / w);
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    [x0, y0, x1 - x0, y1 - y0]
}
