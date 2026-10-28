//! Resize and intersection observers (gap batch A, D45;
//! `docs/design/scripting-essentials.md`): what each observer watches
//! and last reported, and the entries a frame's measurements make.
//!
//! The engine measures every observed element in one walk of the
//! frame's paint order after layout ([`Measured`]); this decides, as the
//! standards' steps do, which observations are to be told: a resize
//! observation whose box's size is not the one last reported, an
//! intersection whose threshold index or whether it intersects is not.

use std::collections::{BTreeMap, HashMap, HashSet};

use devtools::Json;
use dom::NodeId;

/// An element's box as the frame laid it out: its border box's size,
/// its border and padding widths (top, right, bottom, left), its border
/// box's bounds in viewport coordinates through the transforms around
/// it, and the clips in force around it, each with its element, in
/// viewport coordinates, outermost first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measured {
    pub(crate) size: (f32, f32),
    pub(crate) border: [f32; 4],
    pub(crate) padding: [f32; 4],
    pub(crate) bounds: [f32; 4],
    pub(crate) clips: Vec<(Option<NodeId>, [f32; 4])>,
}

impl Measured {
    /// The content box's size.
    fn content(&self) -> (f32, f32) {
        let [bt, br, bb, bl] = self.border;
        let [pt, pr, pb, pl] = self.padding;
        (
            (self.size.0 - bl - br - pl - pr).max(0.0),
            (self.size.1 - bt - bb - pt - pb).max(0.0),
        )
    }

    /// The padding box, in viewport coordinates: an element root's.
    fn padding_box(&self) -> [f32; 4] {
        let [x, y, w, h] = self.bounds;
        let [bt, br, bb, bl] = self.border;
        [
            x + bl,
            y + bt,
            (w - bl - br).max(0.0),
            (h - bt - bb).max(0.0),
        ]
    }
}

/// Which box a resize observation watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedBox {
    Content,
    Border,
    DevicePixelContent,
}

struct ResizeTarget {
    node: NodeId,
    observed: ObservedBox,
    /// The size last reported, `(0, 0)` before any.
    last: (f32, f32),
}

struct Intersection {
    root: Option<NodeId>,
    /// `rootMargin`: each side's value and whether it is a percentage.
    margin: [(f32, bool); 4],
    thresholds: Vec<f64>,
    /// Each target, its previous threshold index (-1 before any) and
    /// whether it intersected.
    targets: Vec<(NodeId, i64, bool)>,
}

/// A document's resize and intersection observers, by the numbers the
/// realm gave them.
#[derive(Default)]
pub(crate) struct LayoutObservers {
    resize: BTreeMap<u32, Vec<ResizeTarget>>,
    intersection: BTreeMap<u32, Intersection>,
}

/// A rectangle as JSON: x, y, width and height.
fn rect(r: [f32; 4]) -> Json {
    Json::Array(r.iter().map(|&v| Json::from(f64::from(v))).collect())
}

/// The intersection of two rectangles, `None` when they do not meet
/// (edge-adjacent ones meet, in a rectangle of no area).
fn meet(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    (right >= left && bottom >= top).then_some([left, top, right - left, bottom - top])
}

impl LayoutObservers {
    /// Whether any observer watches an element, so that frames measure.
    pub(crate) fn observing(&self) -> bool {
        self.resize.values().any(|targets| !targets.is_empty())
            || self.intersection.values().any(|o| !o.targets.is_empty())
    }

    /// `ResizeObserver.observe()`: `node` watched for `observed`'s size,
    /// as if `(0, 0)` had been reported, which observing it again resets.
    pub(crate) fn resize_observe(&mut self, observer: u32, node: NodeId, observed: ObservedBox) {
        let targets = self.resize.entry(observer).or_default();
        targets.retain(|t| t.node != node);
        targets.push(ResizeTarget {
            node,
            observed,
            last: (0.0, 0.0),
        });
    }

    pub(crate) fn resize_unobserve(&mut self, observer: u32, node: NodeId) {
        if let Some(targets) = self.resize.get_mut(&observer) {
            targets.retain(|t| t.node != node);
        }
    }

    pub(crate) fn resize_disconnect(&mut self, observer: u32) {
        self.resize.remove(&observer);
    }

    /// `IntersectionObserver.observe()`: `node` watched by an observer
    /// of `root`, `margin` and `thresholds`, unless it is already.
    pub(crate) fn intersection_observe(
        &mut self,
        observer: u32,
        node: NodeId,
        root: Option<NodeId>,
        margin: [(f32, bool); 4],
        thresholds: Vec<f64>,
    ) {
        let entry = self.intersection.entry(observer).or_insert(Intersection {
            root,
            margin,
            thresholds,
            targets: Vec::new(),
        });
        if !entry.targets.iter().any(|t| t.0 == node) {
            entry.targets.push((node, -1, false));
        }
    }

    pub(crate) fn intersection_unobserve(&mut self, observer: u32, node: NodeId) {
        if let Some(o) = self.intersection.get_mut(&observer) {
            o.targets.retain(|t| t.0 != node);
        }
    }

    pub(crate) fn intersection_disconnect(&mut self, observer: u32) {
        self.intersection.remove(&observer);
    }

    /// The elements a frame is to measure: the targets and the roots.
    pub(crate) fn targets(&self) -> HashSet<NodeId> {
        let mut out = HashSet::new();
        for targets in self.resize.values() {
            out.extend(targets.iter().map(|t| t.node));
        }
        for o in self.intersection.values() {
            out.extend(o.root);
            out.extend(o.targets.iter().map(|t| t.0));
        }
        out
    }

    /// The entries a frame's measurements make, as JSON for the realm to
    /// deliver (`None` when there are none): the viewport's size,
    /// `scale` device pixels to a CSS pixel, the time on the document's
    /// clock, and whether a node is the inclusive ancestor of another.
    pub(crate) fn observed(
        &mut self,
        measured: &HashMap<NodeId, Measured>,
        viewport: (f32, f32),
        scale: f32,
        now: f64,
        is_ancestor: impl Fn(NodeId, NodeId) -> bool,
    ) -> Option<String> {
        let mut resize = Vec::new();
        for (&observer, targets) in &mut self.resize {
            let mut entries = Vec::new();
            for t in targets.iter_mut() {
                let m = measured.get(&t.node);
                let content = m.map_or((0.0, 0.0), Measured::content);
                let border = m.map_or((0.0, 0.0), |m| m.size);
                let device = ((content.0 * scale).round(), (content.1 * scale).round());
                let size = match t.observed {
                    ObservedBox::Content => content,
                    ObservedBox::Border => border,
                    ObservedBox::DevicePixelContent => device,
                };
                if size == t.last {
                    continue;
                }
                t.last = size;
                let (pt, pl) = m.map_or((0.0, 0.0), |m| (m.padding[0], m.padding[3]));
                entries.push(Json::object([
                    ("target", Json::from(super::handle(t.node).0)),
                    ("content", rect([pl, pt, content.0, content.1])),
                    ("border", rect([border.0, border.1, 0.0, 0.0])),
                    ("device", rect([device.0, device.1, 0.0, 0.0])),
                ]));
            }
            if !entries.is_empty() {
                resize.push(Json::object([
                    ("observer", Json::from(observer)),
                    ("entries", Json::Array(entries)),
                ]));
            }
        }
        let mut intersection = Vec::new();
        for (&observer, o) in &mut self.intersection {
            // The root's rectangle, grown by the margin.
            let root = match o.root {
                None => Some([0.0, 0.0, viewport.0, viewport.1]),
                Some(root) => measured.get(&root).map(Measured::padding_box),
            };
            let root = root.map(|[x, y, w, h]| {
                let side = |i: usize, basis: f32| match o.margin[i] {
                    (v, true) => v / 100.0 * basis,
                    (v, false) => v,
                };
                let (top, right, bottom, left) = (side(0, h), side(1, w), side(2, h), side(3, w));
                [x - left, y - top, w + left + right, h + top + bottom]
            });
            let mut entries = Vec::new();
            for t in o.targets.iter_mut() {
                let m = measured.get(&t.0);
                let bounds = m.map_or([0.0; 4], |m| m.bounds);
                let within = o.root.is_none_or(|r| r != t.0 && is_ancestor(r, t.0));
                // The target's box clipped by the clips around it (an
                // element root's own and those inside it alone) and by
                // the root's rectangle.
                let mut seen = (m.is_some() && within).then_some(bounds);
                if let Some(m) = m {
                    for (node, clip) in &m.clips {
                        let inside = match (o.root, node) {
                            (None, _) => true,
                            (Some(r), Some(node)) => *node != r && is_ancestor(r, *node),
                            (Some(_), None) => false,
                        };
                        if inside {
                            seen = seen.and_then(|s| meet(s, *clip));
                        }
                    }
                }
                seen = match root {
                    Some(root) => seen.and_then(|s| meet(s, root)),
                    None => None,
                };
                let intersecting = seen.is_some();
                let area = |r: [f32; 4]| f64::from(r[2]) * f64::from(r[3]);
                let ratio = match (area(bounds) > 0.0, seen) {
                    (true, Some(s)) => (area(s) / area(bounds)).min(1.0),
                    (false, Some(_)) => 1.0,
                    (_, None) => 0.0,
                };
                let index = o
                    .thresholds
                    .iter()
                    .position(|&v| v > ratio)
                    .unwrap_or(o.thresholds.len()) as i64;
                if index == t.1 && intersecting == t.2 {
                    continue;
                }
                t.1 = index;
                t.2 = intersecting;
                entries.push(Json::object([
                    ("target", Json::from(super::handle(t.0).0)),
                    ("time", Json::from(now)),
                    ("root", root.map_or(Json::Null, rect)),
                    ("bounds", rect(bounds)),
                    ("rect", rect(seen.unwrap_or([0.0; 4]))),
                    ("intersecting", Json::from(intersecting)),
                    ("ratio", Json::from(ratio)),
                ]));
            }
            if !entries.is_empty() {
                intersection.push(Json::object([
                    ("observer", Json::from(observer)),
                    ("entries", Json::Array(entries)),
                ]));
            }
        }
        (!resize.is_empty() || !intersection.is_empty()).then(|| {
            Json::object([
                ("resize", Json::Array(resize)),
                ("intersection", Json::Array(intersection)),
            ])
            .write()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(doc: &mut dom::Document) -> NodeId {
        let n = doc.create_element("div").unwrap();
        doc.append_child(doc.root(), n).unwrap();
        n
    }

    fn boxed(bounds: [f32; 4]) -> Measured {
        Measured {
            size: (bounds[2], bounds[3]),
            border: [1.0; 4],
            padding: [2.0; 4],
            bounds,
            clips: Vec::new(),
        }
    }

    #[test]
    fn a_resize_is_told_once_each_time_the_size_changes() {
        let mut doc = dom::Document::new();
        let a = node(&mut doc);
        let mut observers = LayoutObservers::default();
        observers.resize_observe(1, a, ObservedBox::Content);
        let mut tell = |m: &HashMap<NodeId, Measured>| {
            observers.observed(m, (100.0, 100.0), 2.0, 0.0, |_, _| true)
        };
        let measured: HashMap<_, _> = [(a, boxed([0.0, 0.0, 20.0, 10.0]))].into();
        let told = tell(&measured).expect("the first size");
        assert!(told.contains("\"content\":[2,2,14,4]"), "{told}");
        assert!(told.contains("\"device\":[28,8,0,0]"), "{told}");
        assert_eq!(tell(&measured), None);
        // No box is a size of none, which is told too.
        assert!(tell(&HashMap::new()).is_some());
        assert_eq!(tell(&HashMap::new()), None);
    }

    #[test]
    fn an_intersection_is_told_as_its_threshold_index_changes() {
        let mut doc = dom::Document::new();
        let a = node(&mut doc);
        let mut observers = LayoutObservers::default();
        observers.intersection_observe(
            1,
            a,
            None,
            [(10.0, false), (0.0, false), (0.0, false), (0.0, false)],
            vec![0.0, 0.5, 1.0],
        );
        let mut at = |bounds: [f32; 4]| {
            let measured: HashMap<_, _> = [(a, boxed(bounds))].into();
            observers.observed(&measured, (100.0, 100.0), 1.0, 5.0, |_, _| true)
        };
        // Wholly in view: index 3.
        let told = at([10.0, 10.0, 20.0, 20.0]).unwrap();
        assert!(
            told.contains("\"intersecting\":true") && told.contains("\"ratio\":1"),
            "{told}"
        );
        assert!(told.contains("\"root\":[0,-10,100,110]"), "{told}");
        assert_eq!(at([10.0, 20.0, 20.0, 20.0]), None);
        // Half below the bottom: index 2.
        let told = at([10.0, 90.0, 20.0, 20.0]).unwrap();
        assert!(told.contains("\"ratio\":0.5"), "{told}");
        // Out of view, then just touching the root margin's edge above.
        assert!(
            at([10.0, 200.0, 20.0, 20.0])
                .unwrap()
                .contains("\"intersecting\":false")
        );
        let told = at([10.0, -30.0, 20.0, 20.0]).unwrap();
        assert!(
            told.contains("\"intersecting\":true") && told.contains("\"ratio\":0"),
            "{told}"
        );
        // A margin of a percentage is of the root's size.
        observers.intersection_observe(
            2,
            a,
            None,
            [(0.0, false), (0.0, false), (50.0, true), (0.0, false)],
            vec![0.0],
        );
        let measured: HashMap<_, _> = [(a, boxed([10.0, 110.0, 20.0, 20.0]))].into();
        let told = observers
            .observed(&measured, (100.0, 80.0), 1.0, 5.0, |_, _| true)
            .unwrap();
        assert!(
            told.contains("\"root\":[0,0,100,120]") && told.contains("\"intersecting\":true"),
            "{told}"
        );
    }
}
