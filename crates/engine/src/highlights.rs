//! Custom highlights (CSS Custom Highlight API 1, gap batch H): the
//! registry a document's scripts keep in `CSS.highlights`, as the
//! bindings hand it over, and what it paints. A highlight's ranges are
//! the document's live ranges, by their numbers, static ones, or a text
//! control's value's (an `OpaqueRange`'s); each text node a range holds,
//! and a control's value, is matched to the text fragments its element
//! is drawn in, as a range's boxes are, and each fragment it reaches is
//! painted with the `::highlight()` of the highlight's name over the
//! characters held. The highlights stack by priority, then by the order
//! they were registered in, the last on top (CSS Pseudo 4 section 3.5);
//! the selection, which this engine does not paint (G59), and the
//! spelling and grammar errors, which nothing here marks, would go over
//! them.

use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dom::{Boundary, Document, NodeId};
use layout::{DocumentLayout, FragmentKind};
use paint::{HighlightOverlay, ScrollOffsets, Viewport};
use style::selector_matching::HighlightName;
use style::{ComputedStyle, Recascader, Styles};

use crate::scripts::{Slot, advance, drawn_chars, match_data, match_text, text_fragments};

/// The most highlights a registry hands over, and the most ranges in
/// them all (`docs/security.md`): what is past them paints nothing.
pub(crate) const MAX_HIGHLIGHTS: usize = 1 << 10;
pub(crate) const MAX_HIGHLIGHT_RANGES: usize = 1 << 14;

/// One of a highlight's ranges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HighlightRange {
    /// A live range, by its number in the document.
    Live(u32),
    /// A static range's boundary points.
    Static(Boundary, Boundary),
    /// An `OpaqueRange`'s text control and the units of its value it
    /// holds, from and to.
    Control(NodeId, u32, u32),
}

/// A highlight registered in `CSS.highlights`, under its name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Registered {
    pub(crate) name: String,
    pub(crate) priority: i32,
    pub(crate) ranges: Vec<HighlightRange>,
}

/// The registry from the bindings' numbers: for each name in turn, the
/// highlight's priority and its count of ranges, then each range, `0`
/// and a live range's number, `1` and a static range's start node and
/// offset and end node and offset, or `2` and a text control and the
/// units of its value from and to, its nodes by `node`; `None` for
/// numbers that do not read so.
pub(crate) fn read_registry(
    names: Vec<String>,
    numbers: &[f64],
    mut node: impl FnMut(f64) -> Option<NodeId>,
) -> Option<Vec<Registered>> {
    let mut at = numbers.iter().copied();
    let mut next = || at.next();
    let mut out = Vec::new();
    let mut ranges_left = MAX_HIGHLIGHT_RANGES;
    for name in names.into_iter().take(MAX_HIGHLIGHTS) {
        let priority = next()? as i32;
        let count = next()? as usize;
        let mut ranges = Vec::new();
        for _ in 0..count {
            let range = match next()? as u32 {
                0 => HighlightRange::Live(next()? as u32),
                2 => {
                    let (a, b, c) = (next()?, next()?, next()?);
                    HighlightRange::Control(node(a)?, b as u32, c as u32)
                }
                _ => {
                    let (a, b, c, d) = (next()?, next()?, next()?, next()?);
                    HighlightRange::Static(
                        Boundary {
                            node: node(a)?,
                            offset: b as u32,
                        },
                        Boundary {
                            node: node(c)?,
                            offset: d as u32,
                        },
                    )
                }
            };
            if ranges_left > 0 {
                ranges_left -= 1;
                ranges.push(range);
            }
        }
        out.push(Registered {
            name,
            priority,
            ranges,
        });
    }
    Some(out)
}

/// The boundary points of `range` in `document`, if it still has them
/// and they are in order and in one tree.
pub(crate) fn bounds_of(
    document: &Document,
    range: HighlightRange,
) -> Option<(Boundary, Boundary)> {
    let (start, end) = match range {
        HighlightRange::Live(id) => {
            let live = document.range(id)?;
            (live.start, live.end)
        }
        HighlightRange::Static(start, end) => (start, end),
        HighlightRange::Control(..) => return None,
    };
    if !document.contains(start.node) || !document.contains(end.node) {
        return None;
    }
    match document.compare_boundaries(start, end)? {
        Ordering::Less => Some((start, end)),
        _ => None,
    }
}

/// The style of `element`'s `::highlight(name)` (CSS Pseudo 4 section
/// 3.5): its own rules over the same highlight's style of its parent,
/// the root's over [`style::cascade::highlight_root`], each kept in
/// `memo` for the elements after.
pub(crate) fn highlight_style(
    document: &Document,
    styles: &Styles,
    recascade: &Recascader<'_>,
    element: NodeId,
    name: HighlightName,
    memo: &mut HashMap<NodeId, Arc<ComputedStyle>>,
) -> Option<Arc<ComputedStyle>> {
    if let Some(style) = memo.get(&element) {
        return Some(Arc::clone(style));
    }
    let own = styles.get(element)?;
    let parent = match document.parent_element(element) {
        Some(parent) => {
            // Font-relative lengths are the originating element's (CSS
            // Pseudo 4 section 3.5): its font, under its parent's
            // highlight.
            let mut parent =
                (*highlight_style(document, styles, recascade, parent, name, memo)?).clone();
            parent.font_size_px = own.font_size_px;
            parent.font.clone_from(&own.font);
            Arc::new(parent)
        }
        None => Arc::new(style::cascade::highlight_root(own)),
    };
    let style = match recascade.highlight(element, name, &parent) {
        Some(style) => Arc::new(style),
        None => Arc::new(ComputedStyle::inherited_from(&parent)),
    };
    memo.insert(element, Arc::clone(&style));
    Some(style)
}

/// What a highlight's style paints over the text of an element styled
/// `own`: its background, its colour unless it leaves the text's, its
/// shadows, and its decoration's lines at the element's font size in
/// that colour.
fn overlay_paint(
    highlight: &ComputedStyle,
    own: &ComputedStyle,
    fonts: &text::FontCollection,
) -> HighlightOverlay {
    let color = (!highlight.color.is_current()).then_some(highlight.color);
    let mut decorating = highlight.clone();
    decorating.color = color.unwrap_or(own.color);
    decorating.font_size_px = own.font_size_px;
    HighlightOverlay {
        from: 0.0,
        to: 0.0,
        background: paint::to_rgba8(highlight.background_color.resolve(decorating.color)),
        color: color.map(paint::to_rgba8),
        shadows: highlight.decoration().text_shadow.clone(),
        decorated: layout::decoration::decorated_by(&decorating, fonts),
    }
}

/// The highlights' overlays over `layout`'s text fragments, by each
/// fragment's address, bottom first: what paint draws them by
/// (`paint::Pictures::highlights`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn overlays(
    document: &Document,
    styles: &Styles,
    recascade: &Recascader<'_>,
    layout: &DocumentLayout,
    (viewport, offsets): (Viewport, &dyn ScrollOffsets),
    fonts: &text::FontCollection,
    registered: &[Registered],
) -> HashMap<usize, Vec<HighlightOverlay>> {
    let mut out: HashMap<usize, Vec<HighlightOverlay>> = HashMap::new();
    // Bottom first: by priority, then in the order registered.
    let mut order: Vec<(usize, &Registered)> = registered.iter().enumerate().collect();
    order.sort_by_key(|&(i, h)| (h.priority, i));
    // The text each highlight holds, by text node: from and to, UTF-16.
    let mut held: Vec<(HighlightName, Vec<Held>)> = Vec::new();
    let mut elements = HashSet::new();
    for (_, highlight) in order {
        // A name no selector names has no style, and paints nothing.
        let Some(name) = HighlightName::find(&highlight.name) else {
            continue;
        };
        let mut texts = Vec::new();
        for &range in &highlight.ranges {
            if let HighlightRange::Control(control, from, to) = range {
                if from < to && document.is_element(control) {
                    texts.push((control, from, to));
                }
                continue;
            }
            let Some((start, end)) = bounds_of(document, range) else {
                continue;
            };
            texts_in(document, start, end, &mut texts);
        }
        for &(node, _, _) in &texts {
            match document.text(node) {
                Some(_) => elements.extend(document.parent(node)),
                None => {
                    elements.insert(node);
                }
            }
        }
        held.push((name, texts));
    }
    if elements.is_empty() {
        return out;
    }
    let drawn = text_fragments(layout, viewport, offsets, &elements);
    let mut matched = HashMap::new();
    for (name, texts) in held {
        let mut memo = HashMap::new();
        let mut paints: HashMap<NodeId, HighlightOverlay> = HashMap::new();
        for (node, from, to) in texts {
            // A text node's element, or a control itself, its value.
            let control = document.text(node).is_none();
            let Some(element) = (if control {
                Some(node)
            } else {
                document.parent(node)
            }) else {
                continue;
            };
            let Some(boxes) = drawn.get(&element) else {
                continue;
            };
            let in_value;
            let slots = if control {
                let value = dom::forms::value(document, node);
                in_value = match_data(&value, &drawn_chars(boxes), &mut 0, &mut false);
                &in_value
            } else {
                let slots = matched
                    .entry(element)
                    .or_insert_with(|| match_text(document, element, boxes));
                let Some(slots) = slots.get(&node) else {
                    continue;
                };
                slots
            };
            let paint = match paints.entry(element) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(place) => {
                    let (Some(own), Some(style)) = (
                        styles.get(element),
                        highlight_style(document, styles, recascade, element, name, &mut memo),
                    ) else {
                        continue;
                    };
                    place.insert(overlay_paint(&style, own, fonts))
                }
            };
            // The characters held, a run in one fragment at a time.
            let mut runs: Vec<(usize, usize, usize)> = Vec::new();
            let (from, to) = (from as usize, (to as usize).min(slots.len()));
            for slot in &slots[from.min(to)..to] {
                if let Slot::At((fragment, c)) = *slot {
                    match runs.last_mut() {
                        Some((f, _, last)) if *f == fragment && *last == c => *last = c + 1,
                        _ => runs.push((fragment, c, c + 1)),
                    }
                }
            }
            for (fragment, first, last) in runs {
                let Some(d) = boxes.get(fragment) else {
                    continue;
                };
                if !matches!(d.fragment.kind, FragmentKind::Text(_)) {
                    continue;
                }
                let a = advance(d.fragment, first, fonts);
                let b = advance(d.fragment, last, fonts);
                out.entry(std::ptr::from_ref(d.fragment) as usize)
                    .or_default()
                    .push(HighlightOverlay {
                        from: a.min(b),
                        to: a.max(b),
                        ..paint.clone()
                    });
            }
        }
    }
    out
}

/// A text node a highlight holds some of, or a text control some of
/// whose value it holds, and the UTF-16 units held, from and to.
type Held = (NodeId, u32, u32);

/// Each text node the range from `start` to `end` holds some of, with
/// the units held, pushed to `out`.
fn texts_in(document: &Document, start: Boundary, end: Boundary, out: &mut Vec<Held>) {
    let mut root = start.node;
    while !document.is_inclusive_ancestor(root, end.node) {
        match document.parent(root) {
            Some(parent) => root = parent,
            None => return,
        }
    }
    for node in document.subtree(root) {
        let Some(data) = document.text(node) else {
            continue;
        };
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
        if from < to {
            out.push((node, from, to));
        }
    }
}

/// What paint asks while it paints a frame: the document's images, and
/// the highlights' overlays over its text.
pub(crate) struct Painting<'a> {
    pub(crate) images: &'a crate::image_store::ImageStore,
    pub(crate) overlays: &'a HashMap<usize, Vec<HighlightOverlay>>,
}

impl paint::Pictures for Painting<'_> {
    fn picture(&self, url: &str) -> Option<(u64, images::Header)> {
        self.images.picture(url)
    }

    fn media(&self, node: NodeId) -> layout::MediaState {
        paint::Pictures::media(self.images, node)
    }

    fn coming(&self, url: &str) -> bool {
        self.images.coming(url)
    }

    fn gradient(&self, gradient: &style::computed::Gradient, w: f32, h: f32) -> Arc<images::Image> {
        self.images.gradient(gradient, w, h)
    }

    fn highlights(&self, fragment: &layout::Fragment) -> &[HighlightOverlay] {
        self.overlays
            .get(&(std::ptr::from_ref(fragment) as usize))
            .map_or(&[], Vec::as_slice)
    }
}

impl crate::Engine {
    /// The overlays of the custom highlights the shown document's
    /// scripts registered over its layout, as of now: none where none
    /// is registered.
    pub(crate) fn highlight_overlays(
        &mut self,
        viewport: Viewport,
        offsets: &dyn ScrollOffsets,
    ) -> HashMap<usize, Vec<HighlightOverlay>> {
        if self.shown.scripts.highlights().is_empty() || self.shown.layout.is_none() {
            return HashMap::new();
        }
        self.cached_styles();
        let Some(styled) = self.shown.styled.as_ref() else {
            return HashMap::new();
        };
        let fonts = crate::FontMeasurer(crate::web_fonts::fonts_of(
            self.shown.web_fonts.as_ref(),
            &self.layout_fonts,
        ));
        let context = crate::cascade_context(
            self.viewport,
            &fonts,
            &self.shown.page,
            self.media(self.viewport),
            &self.shown.animations,
            self.frame_margins,
        );
        let recascade =
            self.shown
                .stylesheets
                .recascader(&self.shown.document, &styled.styles, &context);
        let Some(cached) = self.shown.layout.as_ref() else {
            return HashMap::new();
        };
        overlays(
            &self.shown.document,
            &styled.styles,
            &recascade,
            &cached.layout,
            (viewport, offsets),
            self.document_fonts(),
            self.shown.scripts.highlights(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_reads_from_the_bindings_numbers() {
        let node = |n: f64| NodeId::from_bits(n as u64);
        let names = vec!["a".to_owned(), "b".to_owned()];
        // `a`: priority 2, a live range 7; `b`: priority -1, no range.
        let numbers = [2.0, 1.0, 0.0, 7.0, -1.0, 0.0];
        let read = read_registry(names.clone(), &numbers, node).expect("reads");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].priority, 2);
        assert_eq!(read[0].ranges, vec![HighlightRange::Live(7)]);
        assert_eq!(read[1].priority, -1);
        assert!(read[1].ranges.is_empty());
        // Numbers cut short do not read.
        assert_eq!(read_registry(names, &[2.0, 1.0, 0.0], node), None);
    }
}
