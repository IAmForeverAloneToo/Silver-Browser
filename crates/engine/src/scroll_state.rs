//! The scroll states scroll-state container queries read (CSS
//! Conditional 5 section 7.4.4, D72).
//!
//! A layout's [`DocumentLayout::container_sizes`] gives the size
//! containers' sizes and the scroll containers' `scrollable` sides; this
//! adds the rest, as the engine's frames and a script's flush both read
//! them: the root element's states, which are the viewport's; the sides
//! the last relative scroll of each scroll container moved it towards
//! (`scrolled`), which the engine records as scrolls happen; the sides
//! each sticky box is held off by its insets (`stuck`), as the paint
//! order shifts it; and the axes each snap target is snapped in, as
//! scroll snapping ([`crate::scroll_snap`]) leaves them.

use std::collections::HashMap;

use dom::NodeId;
use layout::{DocumentLayout, Fragment, FragmentKind};
use style::computed::Position;
use style::container::{ContainerSizes, ScrollState};

use crate::scroll_snap::Snaps;

/// A scroll container: an element's, or the viewport (`None`).
pub(crate) type Scroller = Option<NodeId>;

/// The direction of the last relative scroll that moved each scroll
/// container in each axis, across and down: -1 left or up, 1 right or
/// down, 0 none yet.
pub(crate) type Scrolled = HashMap<Scroller, [i8; 2]>;

/// What the states are read from beside a layout: the viewport's size
/// and scroll offset across and down, each scroll container's offset,
/// and the scrolls and snaps so far.
pub(crate) struct Inputs<'a> {
    pub(crate) viewport: (f32, f32),
    pub(crate) viewport_scroll: (f32, f32),
    pub(crate) offsets: &'a dyn Fn(NodeId) -> (f32, f32),
    pub(crate) scrolled: &'a Scrolled,
    pub(crate) snaps: &'a Snaps,
}

/// The sides, top, right, bottom and left, a direction across and down
/// points to.
fn sides([across, down]: [i8; 2]) -> [bool; 4] {
    [down < 0, across > 0, down > 0, across < 0]
}

/// The sizes and scroll states of `layout`'s containers for `inputs`.
pub(crate) fn container_states(layout: &DocumentLayout, inputs: &Inputs<'_>) -> ContainerSizes {
    let mut sizes = layout.container_sizes(inputs.viewport.0);
    let scroll_state = |f: &Fragment| {
        matches!(f.kind, FragmentKind::Box)
            && f.generated.is_none()
            && f.style.container_type.scroll_state
    };
    if !layout.iter().any(scroll_state) {
        return sizes;
    }
    let snapped: HashMap<NodeId, [bool; 2]> = inputs
        .snaps
        .values()
        .flat_map(|snap| {
            snap.areas
                .iter()
                .enumerate()
                .filter_map(|(axis, area)| area.map(|area| (area, axis)))
        })
        .fold(HashMap::new(), |mut map, (area, axis)| {
            map.entry(area.node).or_insert([false; 2])[axis] = true;
            map
        });
    let sticky = if layout
        .iter()
        .any(|f| scroll_state(f) && f.style.position == Position::Sticky)
    {
        let offsets = Offsets(inputs.offsets);
        paint::order::sticky_shifts(
            &layout.roots,
            paint::Viewport {
                width: inputs.viewport.0,
                height: inputs.viewport.1,
                scroll_y: inputs.viewport_scroll.1,
            },
            &offsets,
        )
    } else {
        HashMap::new()
    };
    let (max_x, max_y) = viewport_max(layout, inputs.viewport);
    let (vx, vy) = inputs.viewport_scroll;
    for f in layout.iter().filter(|f| scroll_state(f)) {
        let mut state = ScrollState::default();
        if f.style.root {
            // The root element's scroll states are the viewport's.
            state.scrollable = [vy > 0.0, vx < max_x, vy < max_y, vx > 0.0];
            state.scrolled = sides(inputs.scrolled.get(&None).copied().unwrap_or_default());
        } else if f.style.overflow().scrolls() {
            // Towards the sides it can still go: across from its scroll
            // origin, the right edge right to left.
            let (x, y) = (inputs.offsets)(f.node);
            let (min_x, max_x) = crate::geometry::scroll_range_x(f);
            let max_y = f.max_scroll();
            state.scrollable = [y > 0.0, x < max_x, y < max_y, x > min_x];
            state.scrolled = sides(
                inputs
                    .scrolled
                    .get(&Some(f.node))
                    .copied()
                    .unwrap_or_default(),
            );
        }
        if let Some(&(sx, sy)) = sticky.get(&f.node) {
            state.stuck = [sy > 0.0, sx < 0.0, sy < 0.0, sx > 0.0];
        }
        if let Some(&axes) = snapped.get(&f.node) {
            state.snapped = axes;
        }
        sizes.insert_scroll(f.node, state);
    }
    sizes
}

/// The furthest the viewport scrolls across and down in `layout`.
pub(crate) fn viewport_max(layout: &DocumentLayout, (width, height): (f32, f32)) -> (f32, f32) {
    (
        (crate::geometry::document_width(layout) - width).max(0.0),
        (crate::geometry::document_height(layout) - height).max(0.0),
    )
}

/// The scroll offsets of [`Inputs::offsets`] as the paint order reads
/// them.
struct Offsets<'a>(&'a dyn Fn(NodeId) -> (f32, f32));

impl paint::ScrollOffsets for Offsets<'_> {
    fn offset(&self, node: NodeId) -> f32 {
        (self.0)(node).1
    }

    fn offset_x(&self, node: NodeId) -> f32 {
        (self.0)(node).0
    }
}
