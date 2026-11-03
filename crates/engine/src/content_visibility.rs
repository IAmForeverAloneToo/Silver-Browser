//! `content-visibility: auto` (CSS Containment 2 section 4, gap batch H):
//! at each rendering update, which of the document's elements with it
//! are relevant to the user, kept as their `RELEVANT` state, which the
//! cascade reads to skip their contents or not, and the
//! `contentvisibilityautostatechange` events a change sends; and the
//! last remembered sizes (CSS Sizing 4 section 5.2) of the elements
//! whose `contain-intrinsic-size` is `auto`, which their boxes take
//! while they have size containment.
//!
//! As in the reference browser, an element met for the first time is
//! found relevant or not at once, at the rendering update that first
//! lays it out, and hears it once.

use std::collections::{HashMap, HashSet};

use dom::{Document, ElementState, NodeId};
use layout::{DocumentLayout, FragmentKind};
use style::computed::ContentVisibility;

use crate::{Engine, scripts};

/// How far past the viewport an element is still near it, a share of
/// the viewport's size each way: the reference browser's 150%.
const MARGIN: f32 = 1.5;

/// The elements of a layout the rendering update watches: those with
/// `content-visibility: auto`, and those whose `contain-intrinsic-size`
/// is `auto` and that are laid out by their content, whose size is
/// remembered.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Watched {
    pub(crate) autos: Vec<NodeId>,
    pub(crate) remembering: Vec<NodeId>,
}

/// What `layout` holds of the [`Watched`] elements, in tree order, each
/// once.
pub(crate) fn watched(document: &Document, layout: &DocumentLayout) -> Watched {
    let mut out = Watched::default();
    let mut seen = HashSet::new();
    for f in layout.iter() {
        if !matches!(f.kind, FragmentKind::Box) || f.generated.is_some() {
            continue;
        }
        if !document.is_element(f.node) || !seen.insert(f.node) {
            continue;
        }
        let containment = f.style.containment();
        if containment.content_visibility == ContentVisibility::Auto {
            out.autos.push(f.node);
        }
        if containment.intrinsic.iter().any(|axis| axis.is_auto())
            && !f.style.contains_size()
            && !f.style.contains_inline_size()
        {
            out.remembering.push(f.node);
        }
    }
    out
}

/// Whether the selection's `range` holds some of `node` or of what is
/// in it: it starts before the point after the node and ends after the
/// point before it, and is not collapsed.
fn holds_selection(document: &Document, node: NodeId, range: dom::LiveRange) -> bool {
    use std::cmp::Ordering;
    let Some(parent) = document.parent(node) else {
        return false;
    };
    let index = document.index_of(node);
    let before = dom::Boundary {
        node: parent,
        offset: index,
    };
    let after = dom::Boundary {
        node: parent,
        offset: index + 1,
    };
    range.start != range.end
        && document.compare_boundaries(range.start, after) == Some(Ordering::Less)
        && document.compare_boundaries(range.end, before) == Some(Ordering::Greater)
}

/// Whether the viewport-coordinates `bounds`, `[x, y, width, height]`,
/// are near a viewport of `size` (section 4.3): within [`MARGIN`] of it.
pub(crate) fn near_viewport(bounds: [f32; 4], (width, height): (f32, f32)) -> bool {
    let [x, y, w, h] = bounds;
    let (mx, my) = (width * MARGIN, height * MARGIN);
    x <= width + mx && x + w >= -mx && y <= height + my && y + h >= -my
}

/// Whether the element `m` measures is near a viewport of `size`: its
/// box, clipped by each scroll container and clip around it, each grown
/// by [`MARGIN`] of its own size as the reference browser's observer
/// grows them (its scroll margin), near the viewport.
pub(crate) fn near(m: &scripts::Measured, size: (f32, f32)) -> bool {
    let mut seen = Some(m.bounds);
    for &(node, [x, y, w, h]) in &m.clips {
        if node.is_none() {
            continue;
        }
        let (mx, my) = (w * MARGIN, h * MARGIN);
        let grown = [x - mx, y - my, w + 2.0 * mx, h + 2.0 * my];
        seen = seen.and_then(|s| scripts::meet(s, grown));
    }
    seen.is_some_and(|bounds| near_viewport(bounds, size))
}

impl Engine {
    /// Finds which of the shown document's `content-visibility: auto`
    /// elements are relevant to the user (section 4.3) from the frame
    /// just laid out: near the viewport, or holding the focus. An
    /// element met for the first time is skipped, as its event says;
    /// one whose relevance changed has its state changed, which the
    /// next frame restyles it by, and hears it. The sizes of the
    /// elements laid out by their content whose intrinsic size is
    /// `auto` are remembered, and those of elements no longer so
    /// forgotten.
    pub(crate) fn update_relevance(&mut self) {
        let (width, height) = self.viewport;
        if width == 0 || self.shown.layout.is_none() {
            return;
        }
        self.layout(width as f32);
        // Found once a layout, the walk being of every box.
        let run = self.layout_runs;
        if self.shown.watched.as_ref().is_none_or(|(at, _)| *at != run) {
            let Some(cached) = self.shown.layout.as_ref() else {
                return;
            };
            self.shown.watched = Some((run, watched(&self.shown.document, &cached.layout)));
        }
        let Some((_, watched)) = self.shown.watched.take() else {
            return;
        };
        self.measure_watched(&watched, (width, height));
        self.shown.watched = Some((run, watched));
    }

    /// The `content-visibility: auto` elements met since the last
    /// rendering update: each found relevant to the user or not at once,
    /// from the frame just drawn, its state set, and its one event sent.
    /// Whether one was found relevant, the frame then to be drawn again.
    pub(crate) fn first_relevance(&mut self) -> bool {
        let (width, height) = self.viewport;
        if width == 0 {
            return false;
        }
        self.layout(width as f32);
        let Some(cached) = self.shown.layout.as_ref() else {
            return false;
        };
        let met: Vec<NodeId> = watched(&self.shown.document, &cached.layout)
            .autos
            .into_iter()
            .filter(|node| !self.shown.relevance.contains_key(node))
            .collect();
        if met.is_empty() {
            return false;
        }
        let measured = self.measure(&met.iter().copied().collect());
        let focused = self.shown.document.focused();
        let viewport = (width as f32, height as f32);
        let mut found = false;
        for node in met {
            let near = measured.get(&node).is_some_and(|m| near(m, viewport));
            let relevant =
                near || focused.is_some_and(|f| self.shown.document.is_inclusive_ancestor(node, f));
            self.shown.relevance.insert(node, relevant);
            if relevant {
                self.set_relevant(node, true);
                found = true;
            }
            self.fire_state_change(node, !relevant);
        }
        found
    }

    /// Makes the `content-visibility: auto` elements around `node` that
    /// skip their contents relevant to the user at once, as holding the
    /// focus makes them: what is in them laid out to be drawn, each told.
    pub(crate) fn relevant_around(&mut self, node: NodeId) {
        let Some(styles) = self.shown.styled.as_ref().map(|styled| &styled.styles) else {
            return;
        };
        let skipping: Vec<NodeId> = self
            .shown
            .document
            .ancestors(node)
            .filter(|&a| {
                styles.get(a).is_some_and(|style| {
                    style.skips_contents()
                        && style.containment().content_visibility == ContentVisibility::Auto
                })
            })
            .collect();
        for ancestor in skipping {
            let was = self.shown.relevance.insert(ancestor, true);
            self.set_relevant(ancestor, true);
            if was.is_some() {
                self.fire_state_change(ancestor, false);
            }
        }
    }

    /// Sends `node` its `contentvisibilityautostatechange`.
    fn fire_state_change(&mut self, node: NodeId, skipped: bool) {
        if self.shown.document.contains(node) {
            self.fire(
                js::Target::Node(scripts::handle(node)),
                js::EventInit::new(
                    "ContentVisibilityAutoStateChangeEvent",
                    "contentvisibilityautostatechange",
                )
                .with("bubbles", js::Field::Bool(true))
                .with("cancelable", js::Field::Bool(true))
                .with("skipped", js::Field::Bool(skipped)),
            );
        }
    }

    /// [`Engine::update_relevance`] for the `watched` elements of a
    /// viewport of `width` by `height`.
    fn measure_watched(&mut self, watched: &Watched, (width, height): (u32, u32)) {
        if watched.autos.is_empty()
            && watched.remembering.is_empty()
            && self.shown.relevance.is_empty()
            && !self.shown.images.remembers()
        {
            return;
        }
        let targets: HashSet<NodeId> = watched
            .autos
            .iter()
            .chain(&watched.remembering)
            .copied()
            .collect();
        let measured = self.measure(&targets);
        let remembered: HashMap<NodeId, [Option<f32>; 2]> = watched
            .remembering
            .iter()
            .filter_map(|node| {
                let (w, h) = measured.get(node)?.content();
                Some((*node, [Some(w), Some(h)]))
            })
            .collect();
        let shown = &mut self.shown;
        let styles = shown.styled.as_ref().map(|styled| &styled.styles);
        let document = &shown.document;
        let still_auto = |node: NodeId| {
            document.contains(node)
                && styles
                    .and_then(|styles| styles.get(node))
                    .is_some_and(|style| style.containment().intrinsic.iter().any(|a| a.is_auto()))
        };
        shown.images.set_remembered(remembered, still_auto);
        let focused = self.shown.document.focused();
        let selection = self
            .shown
            .scripts
            .selection()
            .and_then(|id| self.shown.document.range(id));
        let viewport = (width as f32, height as f32);
        let mut changed: Vec<(NodeId, bool)> = Vec::new();
        let autos: HashSet<NodeId> = watched.autos.iter().copied().collect();
        for &node in &watched.autos {
            let near = measured.get(&node).is_some_and(|m| near(m, viewport));
            let holds_focus =
                focused.is_some_and(|f| self.shown.document.is_inclusive_ancestor(node, f));
            let selected =
                selection.is_some_and(|range| holds_selection(&self.shown.document, node, range));
            let relevant = near || holds_focus || selected;
            match self.shown.relevance.get(&node) {
                None => {
                    self.shown.relevance.insert(node, false);
                    changed.push((node, true));
                }
                Some(&was) if was != relevant => {
                    self.shown.relevance.insert(node, relevant);
                    self.set_relevant(node, relevant);
                    changed.push((node, !relevant));
                }
                Some(_) => {}
            }
        }
        // An element no longer `auto`, or no longer laid out, starts
        // again when it is.
        let gone: Vec<NodeId> = self
            .shown
            .relevance
            .keys()
            .filter(|node| !autos.contains(node))
            .copied()
            .collect();
        for node in gone {
            self.shown.relevance.remove(&node);
            if self.shown.document.contains(node) {
                self.set_relevant(node, false);
            }
        }
        for (node, skipped) in changed {
            self.fire_state_change(node, skipped);
        }
    }

    /// Sets or clears `node`'s `RELEVANT` state, which restyles it.
    fn set_relevant(&mut self, node: NodeId, relevant: bool) {
        let document = &mut self.shown.document;
        let state = document.state(node).with(ElementState::RELEVANT, relevant);
        let _ = document.set_state(node, state);
        self.state_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_is_within_half_again_the_viewport_each_way() {
        let size = (800.0, 600.0);
        assert!(near_viewport([0.0, 0.0, 10.0, 10.0], size));
        // 900 below a viewport 600 tall is the edge.
        assert!(near_viewport([0.0, 1490.0, 10.0, 10.0], size));
        assert!(near_viewport([0.0, 1500.0, 10.0, 10.0], size));
        assert!(!near_viewport([0.0, 1505.0, 10.0, 10.0], size));
        assert!(!near_viewport([0.0, -930.0, 10.0, 10.0], size));
        assert!(near_viewport([0.0, -905.0, 10.0, 10.0], size));
        // 1200 across a viewport 800 wide.
        assert!(near_viewport([1800.0, 0.0, 10.0, 10.0], size));
        assert!(!near_viewport([2005.0, 0.0, 10.0, 10.0], size));
        // A box reaching into it from far off is near.
        assert!(near_viewport([0.0, -5000.0, 10.0, 6000.0], size));
    }
}
