//! Grid layout (M18): a grid container's items placed in its rows and
//! columns and laid out there, by css-grid-2 §12's grid layout
//! algorithm. [`placement`] makes the explicit grid and places the items
//! in it (§7, §8); [`tracks`] sizes the tracks of one axis (§12.3 to
//! §12.8); this module ties them together: the columns sized from the
//! items' widths, the rows from their heights at those widths, and each
//! item laid out in its grid area.
//!
//! Every axis runs left to right and top to bottom, as the rest of
//! layout does. An item is laid out as a formatting context root
//! through the pass's kept item layouts ([`Pass::item`]), its containing
//! block its grid area, whose width and height are definite (§12).

pub(crate) mod placement;
pub(crate) mod tracks;

use std::ops::Range;

use style::computed::{
    BOTTOM, BaselinePosition, BoxSizing, ComputedStyle, ContentAlignment, LEFT, LengthPercentage,
    LengthPercentageAuto, MaxSize, Position, RIGHT, SelfAlignment, Size, TOP, TrackBreadth,
    TrackList, TrackSize,
};

use crate::align::{Alignment, align_content, justify_content, self_alignment};
use crate::block::{
    AdjoiningMargins, BlockLayout, Content, Deferred, Edges, Imposed, Intrinsic, box_fragment,
    carried, clamp_height, definite_height, extent_bottom, height_bounds, layout_contents,
    relative_offset, settle_absolutes_in, width_constraints,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::pass::{ItemKey, Pass};
use placement::{Axis, ExplicitAxis, Room, place, repetitions, resolve, resolve_absolute};
use tracks::{Available, Contribution, Sizing, Space, Track, size_tracks};

/// An item placed in the grid: its box, its place among the
/// container's children, and the tracks its area spans in each axis.
struct GridItem<'a> {
    b: &'a LayoutBox,
    child: usize,
    rows: Range<usize>,
    columns: Range<usize>,
}

/// One axis of the grid once the items are placed: each track's size as
/// written, and whether it is an `auto-fit` track no item occupies,
/// which collapses (§7.2.3.2).
struct AxisTracks {
    sizes: Vec<TrackSize>,
    collapsed: Vec<bool>,
}

impl AxisTracks {
    /// The tracks to size, percentages of `percent_of` when it is known
    /// and `auto` otherwise (§7.2.1).
    fn tracks(&self, percent_of: Option<f32>) -> Vec<Track> {
        self.sizes
            .iter()
            .zip(&self.collapsed)
            .map(|(&size, &collapsed)| track(size, percent_of, collapsed))
            .collect()
    }

    /// Whether any track's size has a percentage in it.
    fn has_percentages(&self) -> bool {
        let percent =
            |b: TrackBreadth| matches!(b, TrackBreadth::Fixed(LengthPercentage::Percent(_)));
        self.sizes.iter().any(|size| match *size {
            TrackSize::Breadth(b) => percent(b),
            TrackSize::MinMax(min, max) => percent(min) || percent(max),
            TrackSize::FitContent(limit) => matches!(limit, LengthPercentage::Percent(_)),
        })
    }

    /// The gutter after each track but the last (§11.1): `gap`, but
    /// that the gutters on either side of a run of collapsed tracks
    /// coincide, and are none at all when the run reaches an edge of the
    /// grid (§7.2.3.2).
    fn gutters(&self, gap: f32) -> Vec<f32> {
        let n = self.sizes.len();
        let mut gutters = vec![gap; n.saturating_sub(1)];
        let mut t = 0;
        while t < n {
            if !self.collapsed[t] {
                t += 1;
                continue;
            }
            let start = t;
            while t < n && self.collapsed[t] {
                t += 1;
            }
            // The run is `start..t`. Inside the grid, the gutter before it
            // stands for all of them; at an edge, none does.
            let first = if start == 0 || t == n {
                start.saturating_sub(1)
            } else {
                start
            };
            for g in gutters.iter_mut().take(t.min(n - 1)).skip(first) {
                *g = 0.0;
            }
        }
        gutters
    }
}

/// A sizing function from a track breadth, a percentage of an unknown
/// size being `auto`.
fn sizing(breadth: TrackBreadth, percent_of: Option<f32>) -> Sizing {
    match breadth {
        TrackBreadth::Fixed(LengthPercentage::Px(px)) => Sizing::Fixed(px),
        TrackBreadth::Fixed(LengthPercentage::Percent(p)) => {
            percent_of.map_or(Sizing::Auto, |of| Sizing::Fixed(of * p / 100.0))
        }
        TrackBreadth::Flex(f) => Sizing::Flex(f),
        TrackBreadth::MinContent => Sizing::MinContent,
        TrackBreadth::MaxContent => Sizing::MaxContent,
        TrackBreadth::Auto => Sizing::Auto,
    }
}

/// The track `size` makes (§12.2): a breadth alone is both functions, a
/// flex's minimum being `auto`; `fit-content()` is an `auto` minimum and
/// a `max-content` maximum limited by its argument; a collapsed track is
/// a fixed `0px`.
fn track(size: TrackSize, percent_of: Option<f32>, collapsed: bool) -> Track {
    if collapsed {
        return Track::new(Sizing::Fixed(0.0), Sizing::Fixed(0.0), None);
    }
    match size {
        TrackSize::Breadth(b) => {
            let max = sizing(b, percent_of);
            let min = if matches!(max, Sizing::Flex(_)) {
                Sizing::Auto
            } else {
                max
            };
            Track::new(min, max, None)
        }
        TrackSize::MinMax(min, max) => {
            Track::new(sizing(min, percent_of), sizing(max, percent_of), None)
        }
        TrackSize::FitContent(limit) => {
            let limit = match limit {
                LengthPercentage::Px(px) => Some(px),
                LengthPercentage::Percent(p) => percent_of.map(|of| of * p / 100.0),
            };
            Track::new(Sizing::Auto, Sizing::MaxContent, limit)
        }
    }
}

/// The grid: its items, placed, and its tracks in each axis; and, for
/// the absolutely positioned boxes it is the containing block of, each
/// axis's explicit grid and the line coordinate of its first track,
/// zero or negative, the explicit grid's first line being 0.
struct Grid<'a> {
    items: Vec<GridItem<'a>>,
    rows: AxisTracks,
    columns: AxisTracks,
    row_axis: ExplicitAxis,
    column_axis: ExplicitAxis,
    first_row: i32,
    first_column: i32,
}

/// The room an axis gives an automatic repetition (§7.2.3.2), from the
/// container's definite size, else its definite maximum, else its
/// definite minimum, each a content-box size.
fn room(size: Option<f32>, max: Option<f32>, min: Option<f32>) -> Room {
    match (size.or(max), min) {
        (Some(size), _) => Room::AtMost(size),
        (None, Some(min)) => Room::AtLeast(min),
        (None, None) => Room::Unknown,
    }
}

/// The room, gap and percentage base of one axis, for [`build`].
struct AxisInputs {
    room: Room,
    gap: f32,
    size: Option<f32>,
}

/// Places the grid container `b`'s items (§8.5) in the grid its
/// templates make, the automatic repetitions counted in the room each
/// axis gives.
fn build<'a>(b: &'a LayoutBox, columns: AxisInputs, rows: AxisInputs) -> Grid<'a> {
    let grid = b.style.grid();
    let areas = grid.template_areas.as_ref();
    let count = |list: Option<&TrackList>, inputs: &AxisInputs| {
        list.map_or(1, |list| {
            repetitions(list, inputs.room, inputs.gap, inputs.size)
        })
    };
    let column_axis = ExplicitAxis::new(
        grid.template_columns.as_ref(),
        &grid.auto_columns,
        areas,
        Axis::Columns,
        count(grid.template_columns.as_ref(), &columns),
    );
    let row_axis = ExplicitAxis::new(
        grid.template_rows.as_ref(),
        &grid.auto_rows,
        areas,
        Axis::Rows,
        count(grid.template_rows.as_ref(), &rows),
    );
    // §8.5: the items in order-modified document order (§6.3).
    let mut children: Vec<(usize, &'a LayoutBox)> = b
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.kind != BoxKind::Absolute)
        .collect();
    children.sort_by_key(|(_, child)| child.style.order);
    let placements: Vec<_> = children
        .iter()
        .map(|(_, child)| {
            let g = child.style.grid();
            (
                resolve(&g.row_start, &g.row_end, &row_axis),
                resolve(&g.column_start, &g.column_end, &column_axis),
            )
        })
        .collect();
    let placed = place(
        &placements,
        row_axis.len(),
        column_axis.len(),
        grid.auto_flow,
    );
    let span = |r: &Range<i32>| r.start as usize..r.end as usize;
    let items: Vec<GridItem<'a>> = children
        .iter()
        .zip(&placed.areas)
        .map(|(&(child, b), (rows, columns))| GridItem {
            b,
            child,
            rows: span(rows),
            columns: span(columns),
        })
        .collect();
    // Each track's size from its line coordinate; an `auto-fit` track no
    // item spans collapses.
    let tracks = |axis: &ExplicitAxis,
                  first: i32,
                  count: i32,
                  of: &dyn Fn(&GridItem<'a>) -> Range<usize>| {
        let mut occupied = vec![false; count as usize];
        for item in &items {
            for t in of(item) {
                occupied[t] = true;
            }
        }
        let fit = axis.auto_fit();
        AxisTracks {
            sizes: (0..count).map(|t| axis.size(first + t)).collect(),
            collapsed: (0..count)
                .map(|t| {
                    fit.as_ref().is_some_and(|fit| fit.contains(&(first + t)))
                        && !occupied[t as usize]
                })
                .collect(),
        }
    };
    let rows = tracks(&row_axis, placed.first_row, placed.rows, &|i| {
        i.rows.clone()
    });
    let columns = tracks(&column_axis, placed.first_column, placed.columns, &|i| {
        i.columns.clone()
    });
    Grid {
        items,
        rows,
        columns,
        row_axis,
        column_axis,
        first_row: placed.first_row,
        first_column: placed.first_column,
    }
}

/// Where each track of an axis is once sized: its start, from the
/// content box's edge, and its size.
struct Lines {
    starts: Vec<f32>,
    sizes: Vec<f32>,
}

impl Lines {
    /// The tracks of `sizes` laid end to end from `offset`, the gutters
    /// between them.
    fn new(sizes: Vec<f32>, gutters: &[f32], offset: f32) -> Self {
        let mut starts = Vec::with_capacity(sizes.len());
        let mut at = offset;
        for (t, size) in sizes.iter().enumerate() {
            starts.push(at);
            at += size + gutters.get(t).copied().unwrap_or(0.0);
        }
        Lines { starts, sizes }
    }

    /// The start and size of the area `span` covers: its tracks and the
    /// gutters between them.
    fn area(&self, span: &Range<usize>) -> (f32, f32) {
        let start = self.starts[span.start];
        let last = span.end - 1;
        (start, self.starts[last] + self.sizes[last] - start)
    }

    /// Where the line before track `k` is as an area's start edge: that
    /// track's start, or the last track's end for the last line.
    fn start_edge(&self, k: usize) -> f32 {
        self.starts.get(k).copied().unwrap_or_else(|| self.extent())
    }

    /// Where the line after track `k - 1` is as an area's end edge: that
    /// track's end, or the first track's start for the first line.
    fn end_edge(&self, k: usize) -> f32 {
        match k.checked_sub(1) {
            Some(t) => self.starts[t] + self.sizes[t],
            None => self.starts.first().copied().unwrap_or(0.0),
        }
    }

    /// From the first track's start to the last's end.
    fn extent(&self) -> f32 {
        match (self.starts.last(), self.sizes.last()) {
            (Some(last), Some(size)) => last + size,
            _ => 0.0,
        }
    }
}

/// The tracks of an axis placed in a content box `size` long (§11.5,
/// css-align-3 §5): laid end to end with the gutters between them, then
/// moved and spaced by the axis's content distribution, `alignment`.
/// The tracks that have not collapsed are the alignment subjects, and
/// the room they leave is shared among them alone: the gutters around a
/// collapsed track take none of it (§7.2.3.2).
fn aligned_lines(
    sizes: Vec<f32>,
    gutters: &[f32],
    collapsed: &[bool],
    alignment: Alignment,
    size: f32,
) -> Lines {
    let extent = sizes.iter().sum::<f32>() + gutters.iter().sum::<f32>();
    let subjects: Vec<usize> = (0..sizes.len()).filter(|&t| !collapsed[t]).collect();
    let (offset, between) = alignment.offsets(size - extent, subjects.len(), true);
    let mut gutters = gutters.to_vec();
    for pair in subjects.windows(2) {
        gutters[pair[0]] += between;
    }
    Lines::new(sizes, &gutters, offset)
}

/// Where an item's margin box goes in its area along one axis, from the
/// area's start (§11.2, css-align-3 §6), `free` being the room the area
/// leaves it: `auto` margins, `[start, end]`, take what is positive,
/// equally when both are, and turn the axis's self-alignment off; else
/// `alignment` places it, an overflowing item at the start when `safe`.
fn self_offset(free: f32, auto_margins: [bool; 2], alignment: Alignment) -> f32 {
    match auto_margins {
        [true, true] => (free / 2.0).max(0.0),
        [true, false] => free.max(0.0),
        [false, true] => 0.0,
        [false, false] => alignment.offsets(free, 1, true).0,
    }
}

/// A baseline-sharing group along one axis (§11.6, css-align-3 §9.1):
/// the items that align by their first baseline and whose area starts in
/// `track`, or by their last baseline and whose area ends there.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Group {
    position: BaselinePosition,
    track: usize,
}

/// The group an item takes part in along one axis, its self-alignment
/// there being `alignment` and its margins there `margins` (§11.3,
/// §11.4): a baseline value's, unless either margin is `auto` or the
/// item's size there depends on the size of an intrinsically sized track
/// its area `span` crosses, which would make the two depend on each
/// other. Here that is a `percentage` size, minimum or maximum in the
/// axis, and a track whose minimum or maximum is intrinsic, or a
/// flexible track when the container's size in the axis is not
/// `definite`: a flexible track counts by that alone, whatever its
/// minimum. Such an item takes the fallback alignment.
fn baseline_group(
    alignment: SelfAlignment,
    margins: [LengthPercentageAuto; 2],
    percentage: bool,
    tracks: &[Track],
    span: &Range<usize>,
    definite: bool,
) -> Option<Group> {
    let SelfAlignment::Baseline(position) = alignment else {
        return None;
    };
    let intrinsic = |t: &Track| match t.max {
        Sizing::Flex(_) => !definite,
        max => t.min.is_intrinsic() || max.is_intrinsic(),
    };
    if margins.iter().any(|m| m.is_auto())
        || (percentage && span.clone().any(|t| intrinsic(&tracks[t])))
    {
        return None;
    }
    let track = match position {
        BaselinePosition::First => span.start,
        BaselinePosition::Last => span.end - 1,
    };
    Some(Group { position, track })
}

/// Whether a size, a minimum or a maximum is a percentage.
fn has_percentage(size: Size, min: Size, max: MaxSize) -> bool {
    matches!(size, Size::Percent(_))
        || matches!(min, Size::Percent(_))
        || matches!(max, MaxSize::Percent(_))
}

/// The distance from the outer edge an item's group aligns it at to its
/// baseline: its outer start edge for the first baseline, its outer end
/// edge for the last; `from_start` is the distance from the outer start
/// edge to the baseline and `outer` the item's outer size.
fn distance(position: BaselinePosition, from_start: f32, outer: f32) -> f32 {
    match position {
        BaselinePosition::First => from_start,
        BaselinePosition::Last => outer - from_start,
    }
}

/// The largest distance (see [`distance`]) among each group's items, by
/// track: where the group's baseline is from the edge it aligns at.
struct Extents {
    first: Vec<f32>,
    last: Vec<f32>,
}

impl Extents {
    /// The extents of the groups of an axis with `tracks` tracks, each
    /// item in `groups` at its distance in `distances`.
    fn of(groups: &[Option<Group>], distances: &[f32], tracks: usize) -> Self {
        let mut extents = Extents {
            first: vec![f32::NEG_INFINITY; tracks],
            last: vec![f32::NEG_INFINITY; tracks],
        };
        for (group, &distance) in groups.iter().zip(distances) {
            if let Some(group) = group {
                let largest = match group.position {
                    BaselinePosition::First => &mut extents.first[group.track],
                    BaselinePosition::Last => &mut extents.last[group.track],
                };
                *largest = largest.max(distance);
            }
        }
        extents
    }

    /// How far an item at `distance` in `group` lies from the edge its
    /// group aligns at, so that its baseline meets the group's: its shim
    /// (§12.5 step 1), and its offset in its area (css-align-3 §9.3);
    /// zero for an item in no group.
    fn shim(&self, group: Option<Group>, distance: f32) -> f32 {
        group.map_or(0.0, |group| {
            let largest = match group.position {
                BaselinePosition::First => self.first[group.track],
                BaselinePosition::Last => self.last[group.track],
            };
            largest - distance
        })
    }
}

/// Where an item in a baseline-sharing group goes in its area along the
/// axis, from the area's start: `shim` from the start for the first
/// baseline, from the end for the last, `free` being the room the area
/// leaves it. The group as a whole is aligned at that edge, however it
/// overflows (css-align-3 §9.3).
fn baseline_offset(position: BaselinePosition, shim: f32, free: f32) -> f32 {
    match position {
        BaselinePosition::First => shim,
        BaselinePosition::Last => free - shim,
    }
}

/// A margin's size where the area it is a percentage of is not known
/// yet: zero for a percentage or `auto`.
fn fixed_margin(margin: LengthPercentageAuto) -> f32 {
    match margin {
        LengthPercentageAuto::Px(px) => px,
        _ => 0.0,
    }
}

/// An item's shims along the columns (§12.5 step 1): one for its
/// min-content contribution, which its minimum contribution takes too,
/// and one for its max-content contribution. They differ only in a
/// last-baseline group, whose items' distances from their right edges
/// depend on their widths.
#[derive(Debug, Clone, Copy, Default)]
struct Shims {
    min_content: f32,
    max_content: f32,
}

/// Each item's baseline-sharing group along the columns, its
/// `justify-self` a baseline value, and its shims, the container's width
/// being `definite` or not. Items have no baselines in this axis in this
/// engine's one writing mode, so each takes both from its border box's
/// left edge, as the reference browser has it (css-align-3 §9.1): the
/// group lines up their left border edges.
fn column_groups<'a>(
    items: &[GridItem<'a>],
    container: &ComputedStyle,
    columns: &[Track],
    definite: bool,
    pass: &Pass<'a>,
) -> (Vec<Option<Group>>, Vec<Shims>) {
    let groups: Vec<Option<Group>> = items
        .iter()
        .map(|item| {
            let style = &item.b.style;
            baseline_group(
                justify_self(style, container),
                [style.margin[LEFT], style.margin[RIGHT]],
                has_percentage(style.width, style.min_width, style.max_width),
                columns,
                &item.columns,
                definite,
            )
        })
        .collect();
    // Each item's distances at its min-content and its max-content
    // contribution.
    let distances: Vec<[f32; 2]> = items
        .iter()
        .zip(&groups)
        .map(|(item, group)| match group {
            None => [0.0; 2],
            Some(group) => {
                let left = fixed_margin(item.b.style.margin[LEFT]);
                let (min, max) = crate::intrinsic::contribution(item.b, pass);
                [min, max].map(|outer| distance(group.position, left, outer))
            }
        })
        .collect();
    let shims_at = |k: usize| -> Vec<f32> {
        let distances: Vec<f32> = distances.iter().map(|d| d[k]).collect();
        let extents = Extents::of(&groups, &distances, columns.len());
        groups
            .iter()
            .zip(&distances)
            .map(|(&group, &distance)| extents.shim(group, distance))
            .collect()
    };
    let shims = shims_at(0)
        .into_iter()
        .zip(shims_at(1))
        .map(|(min_content, max_content)| Shims {
            min_content,
            max_content,
        })
        .collect();
    (groups, shims)
}

/// A gap's size: a percentage of a size not known is zero (css-align-3
/// §8.1), and `normal` is zero in a grid container.
fn gap(gap: Option<LengthPercentage>, of: Option<f32>) -> f32 {
    match gap {
        None => 0.0,
        Some(LengthPercentage::Px(px)) => px,
        Some(LengthPercentage::Percent(p)) => of.map_or(0.0, |of| of * p / 100.0),
    }
}

/// Whether a content distribution value stretches `auto` tracks (§12.8):
/// `normal` and `stretch`.
fn stretches_tracks(value: ContentAlignment) -> bool {
    matches!(value, ContentAlignment::Normal | ContentAlignment::Stretch)
}

/// The `justify-self` an item with style `item` takes in a container with
/// style `container`: its own, `auto` being the container's
/// `justify-items` without its `legacy`.
fn justify_self(item: &ComputedStyle, container: &ComputedStyle) -> SelfAlignment {
    match item.grid().justify_self {
        SelfAlignment::Auto => container.grid().justify_items.value,
        own => own,
    }
}

/// The `align-self` an item takes: its own, `auto` being the
/// container's `align-items`.
fn align_self(item: &ComputedStyle, container: &ComputedStyle) -> SelfAlignment {
    match item.align_self {
        SelfAlignment::Auto => container.align_items,
        own => own,
    }
}

/// Whether an item stretches in an axis (§6.2): its self-alignment
/// there is `normal` or `stretch` (an item here is never replaced), its
/// size there is `auto`, and neither of its margins there is `auto`.
fn stretches(alignment: SelfAlignment, size: Size, margins: [LengthPercentageAuto; 2]) -> bool {
    matches!(alignment, SelfAlignment::Normal | SelfAlignment::Stretch)
        && size.is_auto()
        && !margins.iter().any(|m| m.is_auto())
}

/// Whether every track of `span` has a fixed maximum, and if so their
/// sum with the gutters between them: §6.6's limit on the automatic
/// minimum.
fn fixed_maximum(tracks: &[Track], gutters: &[f32], span: &Range<usize>) -> Option<f32> {
    let mut sum = 0.0;
    for t in span.clone() {
        match tracks[t].max {
            Sizing::Fixed(v) => sum += v,
            _ => return None,
        }
    }
    let between: f32 = gutters[span.start..span.end - 1].iter().sum();
    Some(sum + between)
}

/// Whether an item's automatic minimum size in an axis is its
/// content-based minimum size (§6.6): it is not a scroll container, it
/// spans a track whose minimum is `auto`, and when it spans more than
/// one, none of them is flexible.
fn has_automatic_minimum(style: &ComputedStyle, tracks: &[Track], span: &Range<usize>) -> bool {
    !style.overflow.clips()
        && span.clone().any(|t| tracks[t].min == Sizing::Auto)
        && (span.len() == 1
            || !span
                .clone()
                .any(|t| matches!(tracks[t].max, Sizing::Flex(_))))
}

/// A content-box size from a specified one, `box-sizing` applied.
fn inner(style: &ComputedStyle, v: f32, non_content: f32) -> f32 {
    match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    }
}

/// An item's automatic minimum width as a content-box width (§6.6),
/// zero when it does not apply: its min-content width, clamped by a
/// definite `max-width` and, when every column it spans has a fixed
/// maximum, by what those leave it after its margins and its `shim`.
fn automatic_minimum_width<'a>(
    item: &GridItem<'a>,
    columns: &[Track],
    gutters: &[f32],
    shim: f32,
    pass: &Pass<'a>,
) -> f32 {
    let style = &item.b.style;
    if !has_automatic_minimum(style, columns, &item.columns) {
        return 0.0;
    }
    let (non_content, margins) = crate::intrinsic::fixed_edges(item.b);
    let mut content = crate::intrinsic::content_widths(item.b, pass).0;
    if let MaxSize::Px(max) = style.max_width {
        content = content.min(inner(style, max, non_content));
    }
    if let Some(limit) = fixed_maximum(columns, gutters, &item.columns) {
        content = content.min((limit - margins - shim - non_content).max(0.0));
    }
    content
}

/// The item's contributions to the columns (§12.5, css-sizing-3 §5.2):
/// its min-content and max-content contributions as a block container's
/// child gives them, and its minimum contribution, the outer size its
/// minimum width, `automatic` for `auto`, would give it when its `width`
/// is `auto` or a percentage; each with its baseline shim (§12.5 step 1).
fn column_contribution<'a>(
    item: &GridItem<'a>,
    automatic: f32,
    shims: Shims,
    pass: &Pass<'a>,
) -> Contribution {
    let style = &item.b.style;
    let (min_content, max_content) = crate::intrinsic::contribution(item.b, pass);
    let minimum = match style.width {
        Size::Auto | Size::Percent(_) => {
            let (non_content, margins) = crate::intrinsic::fixed_edges(item.b);
            let widths = || crate::intrinsic::content_widths(item.b, pass);
            let minimum = match style.min_width {
                Size::Px(px) => inner(style, px, non_content),
                Size::Percent(_) => 0.0,
                Size::Auto => automatic,
                Size::MinContent | Size::FitContent => widths().0,
                Size::MaxContent => widths().1,
            };
            minimum + non_content + margins
        }
        _ => min_content,
    };
    Contribution {
        span: item.columns.clone(),
        minimum: minimum + shims.min_content,
        min_content: min_content + shims.min_content,
        max_content: max_content + shims.max_content,
    }
}

/// The margins of an item in an area `width` wide: percentages are of
/// that width on every side (§6.4, CSS2 §8.3); `auto` is zero until
/// alignment gives it the free space.
fn margins(style: &ComputedStyle, width: f32) -> [f32; 4] {
    style.margin.map(|m| match m {
        LengthPercentageAuto::Auto => 0.0,
        LengthPercentageAuto::Px(px) => px,
        LengthPercentageAuto::Percent(p) => width * p / 100.0,
    })
}

/// An item's content width in an area `width` wide (§6.2): its `width`;
/// else, stretched, the area less its margins, border and padding; else
/// `fit-content` in that room; clamped by its maximum and minimum width.
/// An `auto` minimum is `automatic`, but for an item whose `width` is
/// definite here, a percentage one included: its content-based minimum
/// is that width (§6.6's specified size suggestion), which the width
/// already meets.
#[allow(clippy::too_many_arguments)]
fn item_width<'a>(
    item: &GridItem<'a>,
    container: &ComputedStyle,
    width: f32,
    edges: &Edges,
    margin: &[f32; 4],
    automatic: f32,
    pass: &Pass<'a>,
) -> f32 {
    let style = &item.b.style;
    let horizontal = edges.horizontal();
    let room = (width - margin[LEFT] - margin[RIGHT] - horizontal).max(0.0);
    let intrinsic = Intrinsic::of(item.b, pass);
    let w = width_constraints(style, width, horizontal, &intrinsic, room);
    let stretched = stretches(
        justify_self(style, container),
        style.width,
        [style.margin[LEFT], style.margin[RIGHT]],
    );
    let content = w.width.unwrap_or_else(|| {
        if stretched {
            room
        } else {
            intrinsic.keyword(Size::FitContent, room).unwrap_or(room)
        }
    });
    let min = if style.min_width.is_auto() && w.width.is_none() {
        automatic
    } else {
        w.min
    };
    w.max.map_or(content, |max| content.min(max)).max(min)
}

/// What row sizing takes from an item laid out at its column area's
/// width.
struct Measured {
    /// Its outer height: its border-box height and its margins.
    outer: f32,
    /// Its content height as with `height: auto`.
    auto_height: f32,
    /// Its first and last baselines from its border box's top, each
    /// synthesized from the border box's bottom edge when it has none.
    baselines: [f32; 2],
}

/// An item's automatic minimum height as a content-box height (§6.6),
/// zero when it does not apply: its content height, clamped by a
/// definite `max-height` and, when every row it spans has a fixed
/// maximum, by what those leave it after its margins and its `shim`.
fn automatic_minimum_height(
    item: &GridItem<'_>,
    auto_height: f32,
    rows: &[Track],
    gutters: &[f32],
    vertical: f32,
    margin: &[f32; 4],
    shim: f32,
) -> f32 {
    let style = &item.b.style;
    if !has_automatic_minimum(style, rows, &item.rows) {
        return 0.0;
    }
    let mut content = auto_height;
    if let MaxSize::Px(max) = style.max_height {
        content = content.min(inner(style, max, vertical));
    }
    if let Some(limit) = fixed_maximum(rows, gutters, &item.rows) {
        content = content.min((limit - margin[TOP] - margin[BOTTOM] - shim - vertical).max(0.0));
    }
    content
}

/// The item's contributions to the rows (§12.5): its outer height at its
/// columns' width, both its min-content and its max-content
/// contribution; and its minimum contribution, from its minimum height
/// when its `height` is `auto` or a percentage, an `auto` one its
/// content-based minimum (§6.6); each with its baseline `shim` (§12.5
/// step 1).
fn row_contribution(
    item: &GridItem<'_>,
    measured: &Measured,
    rows: &[Track],
    gutters: &[f32],
    edges: &Edges,
    margin: &[f32; 4],
    shim: f32,
) -> Contribution {
    let style = &item.b.style;
    let vertical = edges.vertical();
    let minimum = match style.height {
        Size::Auto | Size::Percent(_) => {
            let content = match style.min_height {
                Size::Px(px) => inner(style, px, vertical),
                Size::Percent(_) => 0.0,
                Size::Auto => automatic_minimum_height(
                    item,
                    measured.auto_height,
                    rows,
                    gutters,
                    vertical,
                    margin,
                    shim,
                ),
                Size::MinContent | Size::MaxContent | Size::FitContent => measured.auto_height,
            };
            content + vertical + margin[TOP] + margin[BOTTOM]
        }
        _ => measured.outer,
    };
    Contribution {
        span: item.rows.clone(),
        minimum: minimum + shim,
        min_content: measured.outer + shim,
        max_content: measured.outer + shim,
    }
}

/// An item ready for its final layout: its border and padding, margins
/// and content width in its column area, and what measuring it gave.
struct Sized {
    edges: Edges,
    margin: [f32; 4],
    content_width: f32,
    measured: Measured,
}

/// Lays out the grid container `b`'s content at `content_width`, as
/// [`crate::block::layout_block_contents`] does a block container's, at
/// the height `imposed` on it if any: the grid layout algorithm of §12,
/// the absolutely positioned children carried up to their containing
/// block or settled here when the container is positioned. A grid
/// container is a formatting context root: its margins are its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_grid_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    pass: &Pass<'a>,
    imposed: Option<Imposed>,
) -> BlockLayout<'a> {
    let style = &b.style;
    let Edges { border, padding } = edges;
    let vertical_edges = edges.vertical();
    // The content height when it is known before the items are: the
    // height imposed, or the container's own `height` clamped by
    // `min-height` and `max-height`, as a flex container's is.
    let bounds = |content: Option<f32>| height_bounds(style, cb_height, vertical_edges, content);
    let (known_height, definite) = match imposed {
        Some(imposed) => {
            let height = (imposed.height - vertical_edges).max(0.0);
            (Some(height), imposed.definite.then_some(height))
        }
        None => {
            let height = definite_height(style, cb_height, vertical_edges)
                .map(|height| clamp_height(height, bounds(None)));
            (height, height)
        }
    };
    let fit = |content: f32| match known_height {
        Some(known) if imposed.is_some() => known,
        known => clamp_height(known.unwrap_or(content), bounds(Some(content))),
    };
    let (min_height, max_height) = bounds(None);
    let min_height = (min_height > 0.0).then_some(min_height);

    // §11.1: the gaps; a percentage row gap of a height not yet known is
    // zero while the rows are sized, and resolves once it is known.
    let column_gap = gap(style.column_gap, Some(content_width));
    let row_gap = gap(style.row_gap, definite);
    let grid = build(
        b,
        AxisInputs {
            room: Room::AtMost(content_width),
            gap: column_gap,
            size: Some(content_width),
        },
        AxisInputs {
            room: room(definite, max_height, min_height),
            gap: row_gap,
            size: definite,
        },
    );

    // The columns (§12.1 step 1), sized in the content width.
    let mut columns = grid.columns.tracks(Some(content_width));
    let column_gutters = grid.columns.gutters(column_gap);
    let (column_groups, column_shims) = column_groups(&grid.items, style, &columns, true, pass);
    let automatic_widths: Vec<f32> = grid
        .items
        .iter()
        .zip(&column_shims)
        .map(|(item, shims)| {
            automatic_minimum_width(item, &columns, &column_gutters, shims.min_content, pass)
        })
        .collect();
    let column_items: Vec<Contribution> = grid
        .items
        .iter()
        .zip(&automatic_widths)
        .zip(&column_shims)
        .map(|((item, &automatic), &shims)| column_contribution(item, automatic, shims, pass))
        .collect();
    size_tracks(
        &mut columns,
        &column_items,
        &Space {
            available: Available::Definite(content_width),
            min: None,
            max: None,
            gutters: column_gutters.clone(),
            stretch: stretches_tracks(style.justify_content),
        },
    );
    // `justify-content` places the columns now, so that an item spanning
    // a gutter the distribution widens is measured at its final width
    // (§12.1 step 2).
    let column_lines = aligned_lines(
        columns.iter().map(Track::size).collect(),
        &column_gutters,
        &grid.columns.collapsed,
        justify_content(style.justify_content, true, true),
        content_width,
    );

    // The rows (§12.1 step 2), from each item's height at its columns'
    // width. The rows its area spans may take their size from it, so a
    // percentage of the area's height in its `height`, `min-height` or
    // `max-height` is cyclic and counts as their initial value here
    // (css-sizing-3 §5.2.1c), as in the reference browser; the item's
    // final layout resolves it against its area.
    let mut rows = grid.rows.tracks(definite);
    let mut row_gutters = grid.rows.gutters(row_gap);
    let sized: Vec<Sized> = grid
        .items
        .iter()
        .zip(&automatic_widths)
        .map(|(item, &automatic)| {
            let (_, width) = column_lines.area(&item.columns);
            let edges = Edges::of_box(item.b, width);
            let margin = margins(&item.b.style, width);
            let content_width = item_width(item, style, width, &edges, &margin, automatic, pass);
            let key = ItemKey::of(item.b, content_width, None, width, None);
            let lay_out = || {
                layout_contents(
                    item.b,
                    &edges,
                    width,
                    None,
                    content_width,
                    0.0,
                    Content::Root,
                    pass,
                    None,
                    None,
                )
            };
            let measured = pass.read_item(key, lay_out, |layout| Measured {
                outer: layout.height + margin[TOP] + margin[BOTTOM],
                auto_height: layout.auto_content_height,
                baselines: [layout.first_baseline, layout.last_baseline]
                    .map(|baseline| baseline.unwrap_or(layout.height)),
            });
            Sized {
                edges,
                margin,
                content_width,
                measured,
            }
        })
        .collect();
    // §12.5 step 1: the items in baseline-sharing groups along the rows
    // and their shims, from their baselines as measured.
    let row_groups: Vec<Option<Group>> = grid
        .items
        .iter()
        .map(|item| {
            let item_style = &item.b.style;
            baseline_group(
                align_self(item_style, style),
                [item_style.margin[TOP], item_style.margin[BOTTOM]],
                has_percentage(
                    item_style.height,
                    item_style.min_height,
                    item_style.max_height,
                ),
                &rows,
                &item.rows,
                definite.is_some(),
            )
        })
        .collect();
    let row_distance = |group: Option<Group>, margin_top: f32, baselines: [f32; 2], outer: f32| {
        group.map_or(0.0, |group| {
            let baseline = match group.position {
                BaselinePosition::First => baselines[0],
                BaselinePosition::Last => baselines[1],
            };
            distance(group.position, margin_top + baseline, outer)
        })
    };
    let distances: Vec<f32> = row_groups
        .iter()
        .zip(&sized)
        .map(|(&group, s)| {
            row_distance(group, s.margin[TOP], s.measured.baselines, s.measured.outer)
        })
        .collect();
    let extents = Extents::of(&row_groups, &distances, rows.len());
    let row_items: Vec<Contribution> = grid
        .items
        .iter()
        .zip(&sized)
        .zip(row_groups.iter().zip(&distances))
        .map(|((item, s), (&group, &distance))| {
            let shim = extents.shim(group, distance);
            row_contribution(
                item,
                &s.measured,
                &rows,
                &row_gutters,
                &s.edges,
                &s.margin,
                shim,
            )
        })
        .collect();
    let row_space = |available, gutters: &[f32]| Space {
        available,
        min: min_height,
        max: max_height,
        gutters: gutters.to_vec(),
        stretch: stretches_tracks(style.align_content),
    };
    // A height imposed but not definite (a column flex container's
    // flexed item, say) is still the height the rows fill, as the
    // reference browser has it; only percentages do not resolve against
    // it.
    let available = match (definite, imposed) {
        (Some(height), _) => Available::Definite(height),
        (None, Some(_)) => Available::Definite(known_height.unwrap_or(0.0)),
        (None, None) => Available::MaxContent,
    };
    size_tracks(&mut rows, &row_items, &row_space(available, &row_gutters));
    let auto_content_height =
        rows.iter().map(Track::size).sum::<f32>() + row_gutters.iter().sum::<f32>();
    let content_height = fit(auto_content_height);
    // Percentages in the rows and in the row gap resolve against the
    // height found, and the rows are sized again in it (§7.2.1,
    // css-align-3 §8.1).
    if definite.is_none()
        && (grid.rows.has_percentages()
            || matches!(style.row_gap, Some(LengthPercentage::Percent(_))))
    {
        rows = grid.rows.tracks(Some(content_height));
        row_gutters = grid.rows.gutters(gap(style.row_gap, Some(content_height)));
        size_tracks(
            &mut rows,
            &row_items,
            &row_space(Available::Definite(content_height), &row_gutters),
        );
    }
    let row_lines = aligned_lines(
        rows.iter().map(Track::size).collect(),
        &row_gutters,
        &grid.rows.collapsed,
        align_content(style.align_content, true, false),
        content_height,
    );

    // §12 step 5: each item laid out in its area, stretched where it
    // stretches, and placed there by its `auto` margins, its
    // baseline-sharing group or its self-alignment (§11).
    let content_x = border[LEFT] + padding[LEFT];
    let content_y = border[TOP] + padding[TOP];
    let height = content_height + vertical_edges;
    let mut root = box_fragment(content_width + edges.horizontal(), height, b);
    let laid: Vec<BlockLayout<'a>> = grid
        .items
        .iter()
        .zip(&sized)
        .map(|(item, s)| {
            let item_style = &item.b.style;
            let (_, width) = column_lines.area(&item.columns);
            let (_, area_height) = row_lines.area(&item.rows);
            let vertical = s.edges.vertical();
            let margin = s.margin;
            let stretched = stretches(
                align_self(item_style, style),
                item_style.height,
                [item_style.margin[TOP], item_style.margin[BOTTOM]],
            );
            let imposed = stretched.then(|| {
                let bounds = height_bounds(
                    item_style,
                    Some(area_height),
                    vertical,
                    Some(s.measured.auto_height),
                );
                let room = (area_height - margin[TOP] - margin[BOTTOM] - vertical).max(0.0);
                let mut height = clamp_height(room, bounds);
                if item_style.min_height.is_auto() {
                    let automatic = automatic_minimum_height(
                        item,
                        s.measured.auto_height,
                        &rows,
                        &row_gutters,
                        vertical,
                        &margin,
                        0.0,
                    );
                    height = height.max(automatic);
                }
                Imposed {
                    height: height + vertical,
                    definite: true,
                }
            });
            let key = ItemKey::of(item.b, s.content_width, imposed, width, Some(area_height));
            pass.item(key, || {
                layout_contents(
                    item.b,
                    &s.edges,
                    width,
                    Some(area_height),
                    s.content_width,
                    0.0,
                    Content::Root,
                    pass,
                    None,
                    imposed,
                )
            })
        })
        .collect();
    // Each item's outer size, and its baselines from its border box's
    // top, as laid out; the groups align by these.
    let outer: Vec<(f32, f32)> = laid
        .iter()
        .zip(&sized)
        .map(|(layout, s)| {
            let width = s.content_width + s.edges.horizontal() + s.margin[LEFT] + s.margin[RIGHT];
            (width, layout.height + s.margin[TOP] + s.margin[BOTTOM])
        })
        .collect();
    let baselines: Vec<[f32; 2]> = laid
        .iter()
        .map(|layout| {
            [layout.first_baseline, layout.last_baseline]
                .map(|baseline| baseline.unwrap_or(layout.height))
        })
        .collect();
    let column_distances: Vec<f32> = column_groups
        .iter()
        .zip(&sized)
        .zip(&outer)
        .map(|((group, s), &(width, _))| {
            group.map_or(0.0, |group| distance(group.position, s.margin[LEFT], width))
        })
        .collect();
    let row_distances: Vec<f32> = row_groups
        .iter()
        .zip(&sized)
        .zip(baselines.iter().zip(&outer))
        .map(|((&group, s), (&baselines, &(_, height)))| {
            row_distance(group, s.margin[TOP], baselines, height)
        })
        .collect();
    let column_extents = Extents::of(&column_groups, &column_distances, columns.len());
    let row_extents = Extents::of(&row_groups, &row_distances, rows.len());

    let mut carried_up: Vec<Vec<Deferred<'a>>> = b.children.iter().map(|_| Vec::new()).collect();
    let mut content_bottom = 0.0f32;
    // Each item's first and last baselines from the container's border
    // box's top, for the container's own.
    let mut item_baselines = Vec::with_capacity(laid.len());
    for (i, ((item, s), layout)) in grid.items.iter().zip(&sized).zip(laid).enumerate() {
        let item_style = &item.b.style;
        let (x, width) = column_lines.area(&item.columns);
        let (y, area_height) = row_lines.area(&item.rows);
        let margin = s.margin;
        let (dx, dy) = relative_offset(item_style, width, Some(area_height));
        let auto = |side: usize| item_style.margin[side].is_auto();
        let (outer_width, outer_height) = outer[i];
        let x = x + match column_groups[i] {
            Some(group) => baseline_offset(
                group.position,
                column_extents.shim(Some(group), column_distances[i]),
                width - outer_width,
            ),
            None => self_offset(
                width - outer_width,
                [auto(LEFT), auto(RIGHT)],
                self_alignment(justify_self(item_style, style), true, true),
            ),
        };
        let y = y + match row_groups[i] {
            Some(group) => baseline_offset(
                group.position,
                row_extents.shim(Some(group), row_distances[i]),
                area_height - outer_height,
            ),
            None => self_offset(
                area_height - outer_height,
                [auto(TOP), auto(BOTTOM)],
                self_alignment(align_self(item_style, style), true, false),
            ),
        };
        let left = content_x + x + margin[LEFT];
        let top = content_y + y + margin[TOP];
        content_bottom = content_bottom.max(y + outer_height);
        item_baselines.push(baselines[i].map(|baseline| top + baseline));
        carried_up[item.child] = carried(layout.absolutes, left + dx, top + dy);
        root.children
            .push(layout.root.translated(left + dx, top + dy));
    }
    let (first_baseline, last_baseline) =
        container_baselines(&grid.items, &row_groups, &item_baselines);

    // An absolutely positioned box whose containing block the grid
    // container is takes its grid area for it (§10.1).
    let padding_box = [
        content_width + padding[LEFT] + padding[RIGHT],
        content_height + padding[TOP] + padding[BOTTOM],
    ];
    let area = |abs: &LayoutBox| {
        absolute_area(
            abs,
            &grid,
            [&column_lines, &row_lines],
            padding,
            padding_box,
        )
    };
    // An absolutely positioned child is no item (§10.2). Its
    // static-position rectangle is the content box, or its grid area when
    // the grid container is its containing block too, where it is placed
    // as the sole item would be: by its `justify-self` and `align-self`,
    // `auto` being the container's `justify-items` and `align-items`, and
    // `normal` and `stretch` being `start`; never safe.
    let contains = style.position.is_positioned();
    let mut absolutes = Vec::new();
    for (child, carried) in b.children.iter().zip(carried_up) {
        if child.kind == BoxKind::Absolute {
            let [x, y, w, h] = if contains && child.style.position == Position::Absolute {
                let [x, y, w, h] = area(child);
                [border[LEFT] + x, border[TOP] + y, w, h]
            } else {
                [content_x, content_y, content_width, content_height]
            };
            let align = [
                self_alignment(justify_self(&child.style, style), true, true),
                self_alignment(align_self(&child.style, style), true, false),
            ];
            absolutes.push(Deferred {
                b: child,
                x,
                y,
                w,
                h,
                align: align.map(|alignment| alignment.static_align(true)),
                float: None,
            });
        } else {
            absolutes.extend(carried);
        }
    }
    let absolutes = settle_absolutes_in(b, &mut root, border, absolutes, pass, &area);
    if style.overflow.clips() {
        // The scrollable overflow area (§5.3): the items' margin boxes and
        // the grid's tracks, with the end padding after them, or anything
        // reaching lower by its own border box; never less than the
        // padding box.
        let extent = root.children.iter().map(extent_bottom).fold(0.0, f32::max)
            - border[TOP]
            - padding[TOP];
        let padding_box = padding[TOP] + content_height + padding[BOTTOM];
        let grid_bottom = content_bottom.max(row_lines.extent());
        root.scroll_height = (padding[TOP] + grid_bottom + padding[BOTTOM])
            .max(padding[TOP] + extent)
            .max(padding_box);
    }

    let own_top = AdjoiningMargins::of(style.margin[TOP].resolve(cb_width, 0.0));
    let own_bottom = AdjoiningMargins::of(style.margin[BOTTOM].resolve(cb_width, 0.0));
    BlockLayout {
        root,
        height,
        margin_left,
        margin_top: own_top,
        margin_bottom: own_bottom,
        top_edge: own_top,
        bottom_edge: own_bottom,
        self_collapsing: false,
        first_baseline,
        last_baseline,
        // An inline-block takes a grid container's first baseline, as it
        // does a flex container's (M17), in the reference browser.
        inline_block_baseline: first_baseline,
        absolutes,
        auto_content_height,
        held: Vec::new(),
    }
}

/// The grid area that is the containing block of an absolutely
/// positioned box `b` whose containing block the grid container is
/// (§10.1), as `[x, y, width, height]` from the padding box's top-left:
/// its lines resolved against the explicit grid with no auto-placement
/// ([`resolve_absolute`]), an `auto` line, or one the grid as laid out
/// does not have, being the padding edge on its side. An area's edges
/// are its tracks' edges, the gutters outside it; `padding` puts the
/// tracks, which are placed from the content box, in the padding box,
/// `size` across.
fn absolute_area(
    b: &LayoutBox,
    grid: &Grid<'_>,
    [columns, rows]: [&Lines; 2],
    padding: &[f32; 4],
    size: [f32; 2],
) -> [f32; 4] {
    let style = b.style.grid();
    let axis = |start, end, explicit, first: i32, lines: &Lines, offset: f32, size: f32| {
        let count = lines.sizes.len() as i32;
        let index = |line: Option<i32>| {
            line.map(|line| line - first)
                .filter(|k| (0..=count).contains(k))
                .map(|k| k as usize)
        };
        let [start, end] = resolve_absolute(start, end, explicit).map(index);
        let from = start.map_or(0.0, |k| offset + lines.start_edge(k));
        let to = end.map_or(size, |k| offset + lines.end_edge(k));
        (from, (to - from).max(0.0))
    };
    let (x, width) = axis(
        &style.column_start,
        &style.column_end,
        &grid.column_axis,
        grid.first_column,
        columns,
        padding[LEFT],
        size[0],
    );
    let (y, height) = axis(
        &style.row_start,
        &style.row_end,
        &grid.row_axis,
        grid.first_row,
        rows,
        padding[TOP],
        size[1],
    );
    [x, y, width, height]
}

/// The grid container's first and last baselines from its border box's
/// top (§11.6), as the reference browser finds them, from `baselines`,
/// each item's first and last baselines from there, and `groups`, its
/// baseline-sharing group along the rows if any. The first is from the
/// first row an item's area starts in: the shared baseline of the items
/// there that take part in baseline alignment in that row, a
/// first-baseline group before a last; else the first baseline of the
/// first of them in grid order, which goes by the column the area starts
/// in and then by order-modified document order. The last is the same
/// from the last row an item's area ends in, first and last turned
/// round: a last-baseline group before a first, else the last baseline
/// of the last item there in grid order, which goes by the column the
/// area ends in. An item's relative offset does not move them. With no
/// item, none.
fn container_baselines(
    items: &[GridItem<'_>],
    groups: &[Option<Group>],
    baselines: &[[f32; 2]],
) -> (Option<f32>, Option<f32>) {
    let in_group = |i: usize, position: BaselinePosition, track: usize| {
        groups[i] == Some(Group { position, track })
    };
    let first = items.iter().map(|item| item.rows.start).min().map(|row| {
        let mut there: Vec<usize> = (0..items.len())
            .filter(|&i| items[i].rows.start == row)
            .collect();
        there.sort_by_key(|&i| items[i].columns.start);
        let found = |position| there.iter().copied().find(|&i| in_group(i, position, row));
        if let Some(i) = found(BaselinePosition::First) {
            baselines[i][0]
        } else if let Some(i) = found(BaselinePosition::Last) {
            baselines[i][1]
        } else {
            baselines[there[0]][0]
        }
    });
    let last = items.iter().map(|item| item.rows.end - 1).max().map(|row| {
        let mut there: Vec<usize> = (0..items.len())
            .filter(|&i| items[i].rows.end - 1 == row)
            .collect();
        there.sort_by_key(|&i| items[i].columns.end);
        let found = |position| {
            there
                .iter()
                .rev()
                .copied()
                .find(|&i| in_group(i, position, row))
        };
        if let Some(i) = found(BaselinePosition::Last) {
            baselines[i][1]
        } else if let Some(i) = found(BaselinePosition::First) {
            baselines[i][0]
        } else {
            baselines[there[there.len() - 1]][1]
        }
    });
    (first, last)
}

/// The grid container `b`'s min-content and max-content content widths
/// (§5.2): its columns and gaps sized under a min-content and a
/// max-content constraint, percentages of its width counting as `auto`
/// and percentage gaps as zero, since that width is what is being found
/// (§7.2.1, css-align-3 §8.1). An automatic repetition repeats in its
/// definite maximum width, else its minimum, else once. Its `width` is
/// what these sizes stand in for, so it does not count: one 200 pixels
/// wide with `repeat(auto-fill, 50px)` is 50 at its narrowest, as the
/// reference browser and the web-platform-tests have it
/// (flex-item-grid-container-auto-repeat-002), where §7.2.3.2's text
/// would repeat four times.
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let style = &b.style;
    let (non_content, _) = crate::intrinsic::fixed_edges(b);
    let max = match style.max_width {
        MaxSize::Px(px) => Some(inner(style, px, non_content)),
        _ => None,
    };
    let min = match style.min_width {
        Size::Px(px) => Some(inner(style, px, non_content)),
        _ => None,
    };
    let fixed_gap = |gap: Option<LengthPercentage>| match gap {
        Some(LengthPercentage::Px(px)) => px,
        _ => 0.0,
    };
    let column_gap = fixed_gap(style.column_gap);
    let grid = build(
        b,
        AxisInputs {
            room: room(None, max, min),
            gap: column_gap,
            size: None,
        },
        AxisInputs {
            room: Room::Unknown,
            gap: fixed_gap(style.row_gap),
            size: None,
        },
    );
    let columns = grid.columns.tracks(None);
    let gutters = grid.columns.gutters(column_gap);
    let (_, shims) = column_groups(&grid.items, style, &columns, false, pass);
    let items: Vec<Contribution> = grid
        .items
        .iter()
        .zip(&shims)
        .map(|(item, &shims)| {
            let automatic =
                automatic_minimum_width(item, &columns, &gutters, shims.min_content, pass);
            column_contribution(item, automatic, shims, pass)
        })
        .collect();
    let size_under = |available| {
        let mut tracks = columns.clone();
        size_tracks(
            &mut tracks,
            &items,
            &Space {
                available,
                min: None,
                max: None,
                gutters: gutters.clone(),
                stretch: false,
            },
        );
        tracks.iter().map(Track::size).sum::<f32>() + gutters.iter().sum::<f32>()
    };
    let min = size_under(Available::MinContent);
    let max = size_under(Available::MaxContent);
    (min, max.max(min))
}

#[cfg(test)]
mod tests {
    use crate::block::{Fragment, FragmentKind, layout_document};
    use crate::testing::{Laid, boxes_of};
    use text::FontCollection;

    const VIEWPORT: f32 = 800.0;

    /// The document `html` parses to, styled under `css` with the body's
    /// margin taken away, laid out against an 800px viewport.
    fn laid(html: &str, css: &str) -> Laid {
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, VIEWPORT, &fonts);
        Laid { document, layout }
    }

    fn fragment<'l>(laid: &'l Laid, id: &str) -> &'l Fragment {
        let document = &laid.document;
        let node = document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no element #{id}"));
        laid.iter()
            .find(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
            .unwrap_or_else(|| panic!("no box for #{id}"))
    }

    /// The border box of `#id`: `(x, y, w, h)`.
    fn rect(laid: &Laid, id: &str) -> (f32, f32, f32, f32) {
        let f = fragment(laid, id);
        (f.x, f.y, f.w, f.h)
    }

    /// `n` empty items `#i0`, `#i1`, … in a grid `#g`.
    fn items(n: usize) -> String {
        let items: String = (0..n).map(|i| format!("<div id=i{i}></div>")).collect();
        format!("<div id=g>{items}</div>")
    }

    /// A box `width` wide and 10 tall, as an item's content.
    fn wide(width: u32) -> String {
        format!("<span style='display: inline-block; width: {width}px; height: 10px'></span>")
    }

    #[test]
    fn items_fill_fixed_tracks_row_by_row() {
        let l = laid(
            &items(4),
            "#g { display: grid; grid-template-columns: 100px 200px; grid-template-rows: 50px 70px }",
        );
        assert_eq!(rect(&l, "i0"), (0.0, 0.0, 100.0, 50.0));
        assert_eq!(rect(&l, "i1"), (100.0, 0.0, 200.0, 50.0));
        assert_eq!(rect(&l, "i2"), (0.0, 50.0, 100.0, 70.0));
        assert_eq!(rect(&l, "i3"), (100.0, 50.0, 200.0, 70.0));
        assert_eq!(rect(&l, "g"), (0.0, 0.0, 800.0, 120.0));
    }

    #[test]
    fn flexible_columns_share_the_width_less_the_gaps() {
        let l = laid(
            &items(2),
            "#g { display: grid; grid-template-columns: 1fr 3fr; column-gap: 20px }",
        );
        assert_eq!(rect(&l, "i0"), (0.0, 0.0, 195.0, 0.0));
        assert_eq!(rect(&l, "i1"), (215.0, 0.0, 585.0, 0.0));
    }

    #[test]
    fn an_auto_row_is_as_tall_as_its_tallest_item_and_the_others_stretch() {
        let l = laid(
            "<div id=g><div id=a style='height: 30px'></div><div id=b style='height: 50px'></div><div id=c></div></div>",
            "#g { display: grid; grid-template-columns: 100px 100px 100px; row-gap: 5px }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 100.0, 30.0));
        assert_eq!(rect(&l, "b"), (100.0, 0.0, 100.0, 50.0));
        assert_eq!(rect(&l, "c"), (200.0, 0.0, 100.0, 50.0), "stretched");
        assert_eq!(rect(&l, "g").3, 50.0);
    }

    #[test]
    fn auto_columns_take_their_content_and_share_what_is_left() {
        // Grown to their max-content widths, then the `auto` tracks
        // stretch over the rest (§12.8).
        let html = format!(
            "<div id=g><div id=a>{}</div><div id=b>{}</div></div>",
            wide(40),
            wide(60)
        );
        let l = laid(
            &html,
            "#g { display: grid; grid-template-columns: auto auto }",
        );
        assert_eq!(rect(&l, "a").2, 390.0);
        assert_eq!(rect(&l, "b"), (390.0, 0.0, 410.0, rect(&l, "b").3));
        // Sized to fit, they take their content alone.
        let l = laid(
            &html,
            "#g { display: inline-grid; grid-template-columns: auto auto }",
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").2), (40.0, 60.0));
        assert_eq!(rect(&l, "g").2, 100.0);
    }

    #[test]
    fn items_span_tracks_and_the_gaps_between_them() {
        let l = laid(
            "<div id=g><div id=a style='grid-column: 1 / 3; grid-row: 2'></div><div id=b></div></div>",
            "#g { display: grid; grid-template-columns: 100px 100px; grid-auto-rows: 40px; gap: 10px }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 50.0, 210.0, 40.0));
        assert_eq!(rect(&l, "b"), (0.0, 0.0, 100.0, 40.0));
    }

    #[test]
    fn named_areas_place_their_items() {
        let l = laid(
            "<div id=g><div id=f style='grid-area: foot'></div><div id=h style='grid-area: head'></div>\
             <div id=n style='grid-area: nav'></div><div id=m style='grid-area: main'></div></div>",
            "#g { display: grid; grid-template-areas: 'head head' 'nav main' 'foot foot';
                  grid-template-columns: 100px 300px; grid-template-rows: 10px 20px 30px }",
        );
        assert_eq!(rect(&l, "h"), (0.0, 0.0, 400.0, 10.0));
        assert_eq!(rect(&l, "n"), (0.0, 10.0, 100.0, 20.0));
        assert_eq!(rect(&l, "m"), (100.0, 10.0, 300.0, 20.0));
        assert_eq!(rect(&l, "f"), (0.0, 30.0, 400.0, 30.0));
    }

    #[test]
    fn implicit_tracks_take_the_auto_sizes() {
        let l = laid(
            &items(7),
            "#g { display: grid; grid-template-columns: repeat(3, 50px); grid-auto-rows: 20px 30px }
             #i6 { grid-column: 5 }",
        );
        // `#i6`'s line 5 makes the grid five columns wide before the
        // automatic items are placed, so the cursor fills the two
        // implicit ones too. They are sized `auto` and stretched over
        // what is left: 800 - 150 = 650 for the two. The reference
        // browser agrees.
        assert_eq!(rect(&l, "i3"), (150.0, 0.0, 325.0, 20.0));
        assert_eq!(rect(&l, "i4"), (475.0, 0.0, 325.0, 20.0));
        // The implicit rows repeat the `grid-auto-rows` pattern.
        assert_eq!(rect(&l, "i5"), (0.0, 20.0, 50.0, 30.0));
        assert_eq!(rect(&l, "i6"), (475.0, 20.0, 325.0, 30.0));
        assert_eq!(rect(&l, "g").3, 50.0);
    }

    #[test]
    fn order_reorders_auto_placement() {
        let l = laid(
            &items(3),
            "#g { display: grid; grid-template-columns: 10px 10px 10px } #i2 { order: -1 }",
        );
        assert_eq!(rect(&l, "i2").0, 0.0);
        assert_eq!(rect(&l, "i0").0, 10.0);
        assert_eq!(rect(&l, "i1").0, 20.0);
    }

    #[test]
    fn percentage_rows_resolve_against_the_height_they_make() {
        // The reference browser: rows `50% 20px` in an auto-height grid
        // make it 60 tall, then the second row starts 30 down.
        let l = laid(
            "<div id=g><div id=a style='height: 40px'></div><div id=b style='height: 10px'></div></div>",
            "#g { display: grid; grid-template-rows: 50% 20px }",
        );
        assert_eq!(rect(&l, "g").3, 60.0);
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 800.0, 40.0));
        assert_eq!(rect(&l, "b").1, 30.0);
        // A percentage row gap adds nothing to the height and then
        // separates the rows: 10% of 40.
        let l = laid(
            &items(4),
            "#g { display: grid; grid-template-columns: repeat(3, 10px); row-gap: 10%; grid-auto-rows: 20px }",
        );
        assert_eq!(rect(&l, "g").3, 40.0);
        assert_eq!(rect(&l, "i3").1, 24.0);
    }

    #[test]
    fn auto_fill_repeats_as_many_tracks_as_fit_and_auto_fit_collapses_the_empty_ones() {
        let l = laid(
            &items(4),
            "#g { display: grid; width: 350px; grid-template-columns: repeat(auto-fill, 100px) }",
        );
        assert_eq!(rect(&l, "i2").0, 200.0);
        assert_eq!(
            rect(&l, "i3"),
            (0.0, 0.0, 100.0, 0.0),
            "the fourth goes below"
        );
        let fill = laid(
            &items(2),
            "#g { display: grid; width: 400px; grid-template-columns: repeat(auto-fill, minmax(50px, 1fr)) }",
        );
        assert_eq!(rect(&fill, "i1"), (50.0, 0.0, 50.0, 0.0));
        let fit = laid(
            &items(2),
            "#g { display: grid; width: 400px; gap: 10px; grid-template-columns: repeat(auto-fit, minmax(50px, 1fr)) }",
        );
        assert_eq!(
            rect(&fit, "i0").2,
            195.0,
            "the empty tracks and their gaps collapse"
        );
        assert_eq!(rect(&fit, "i1"), (205.0, 0.0, 195.0, 0.0));
    }

    #[test]
    fn a_grid_sized_to_fit_keeps_its_flexible_tracks_in_proportion() {
        // The reference browser's float with columns `1fr 2fr` and two
        // items 30 wide: 90 wide.
        let html = "<div id=w><div id=g><div id=a style='width: 30px'></div><div id=b style='width: 30px'></div></div></div>";
        for container in ["float: left", "display: inline-block"] {
            let l = laid(
                html,
                &format!(
                    "#w {{ {container} }} #g {{ display: grid; grid-template-columns: 1fr 2fr }}"
                ),
            );
            assert_eq!(rect(&l, "g").2, 90.0, "{container}");
            assert_eq!(rect(&l, "b").0, 30.0, "{container}");
        }
    }

    #[test]
    fn the_automatic_minimum_keeps_content_whole_unless_a_fixed_maximum_limits_it() {
        let wide = wide(150);
        let html = format!("<div id=g><div id=a>{wide}</div><div id=b></div></div>");
        let l = laid(
            &html,
            "#g { display: grid; width: 200px; grid-template-columns: 1fr 1fr }",
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").2), (150.0, 50.0));
        let l = laid(
            &html,
            "#g { display: grid; width: 200px; grid-template-columns: minmax(0, 1fr) 1fr }",
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").2), (100.0, 100.0));
        // The reference browser: `minmax(auto, 40px)` keeps a long word's
        // column at 40.
        let l = laid(
            &html,
            "#g { display: grid; width: 100px; grid-template-columns: minmax(auto, 40px) 1fr }",
        );
        assert_eq!(rect(&l, "a").2, 40.0);
        // Across two columns of fixed maximum, the limit counts the gap
        // between them: the reference browser's grid 70 wide keeps the
        // item at 40 + 20 + 40.
        let l = laid(
            &format!("<div id=g><div id=a>{wide}</div><div id=b></div></div>"),
            "#g { display: grid; width: 70px; column-gap: 20px;
                  grid-template-columns: minmax(auto, 40px) minmax(auto, 40px) }
             #a { grid-column: 1 / 3 } #b { grid-row: 2; grid-column: 2 }",
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (100.0, 60.0));
        // A scroll container has no automatic minimum.
        let l = laid(
            &html,
            "#g { display: grid; width: 200px; grid-template-columns: 1fr 1fr } #a { overflow: hidden }",
        );
        assert_eq!(rect(&l, "a").2, 100.0);
    }

    #[test]
    fn a_definite_width_is_its_own_automatic_minimum() {
        // The reference browser: an item 50 wide, or 50% of its area,
        // keeps that width over content 150 wide (§6.6's specified size
        // suggestion); a percentage counts as `auto` while the columns
        // are sized, so its column takes the content.
        let wide = wide(150);
        let grid = "#g { display: grid; width: 200px; grid-template-columns: 1fr 1fr }";
        let html = format!("<div id=g><div id=a>{wide}</div><div id=b></div></div>");
        let l = laid(&html, &format!("{grid} #a {{ width: 50px }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (50.0, 100.0));
        let l = laid(&html, &format!("{grid} #a {{ width: 50% }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (75.0, 150.0));
        let l = laid(
            &html,
            "#g { display: grid; width: 200px; grid-template-columns: auto 1fr } #a { width: 50px }",
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (50.0, 50.0));
    }

    #[test]
    fn a_row_takes_an_items_height_or_its_automatic_minimum_up_to_a_fixed_maximum() {
        let tall = "<div id=g><div id=a><div style='height: 50px'></div></div></div>";
        let l = laid(
            tall,
            "#g { display: grid; grid-template-columns: 100px } #a { height: 20px }",
        );
        assert_eq!((rect(&l, "g").3, rect(&l, "a").3), (20.0, 20.0));
        for rows in ["minmax(auto, 10px)", "10px"] {
            let l = laid(
                tall,
                &format!(
                    "#g {{ display: grid; grid-template-columns: 100px; grid-template-rows: {rows} }}"
                ),
            );
            assert_eq!((rect(&l, "g").3, rect(&l, "a").3), (10.0, 10.0), "{rows}");
        }
        let l = laid(tall, "#g { display: grid; grid-template-columns: 100px }");
        assert_eq!((rect(&l, "g").3, rect(&l, "a").3), (50.0, 50.0));
    }

    #[test]
    fn a_table_item_stretches_over_its_area() {
        let l = laid(
            "<div id=g><table id=t><tr><td>x</td></tr></table></div>",
            "#g { display: grid; grid-template-columns: 200px }",
        );
        assert_eq!(rect(&l, "t").2, 200.0);
    }

    #[test]
    fn an_items_minimum_and_maximum_width_shape_its_column_and_its_width() {
        // Each case as the reference browser lays it out, in a grid 200
        // wide of `1fr 1fr` with content 150 wide in the first item.
        let grid = "#g { display: grid; width: 200px; grid-template-columns: 1fr 1fr }";
        let html = format!(
            "<div id=g><div id=a>{}</div><div id=b></div></div>",
            wide(150)
        );
        // A maximum width limits the item and its automatic minimum.
        let l = laid(&html, &format!("{grid} #a {{ max-width: 60px }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (60.0, 100.0));
        // A percentage width with no minimum adds nothing to the column.
        let l = laid(&html, &format!("{grid} #a {{ width: 50%; min-width: 0 }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (50.0, 100.0));
        // A minimum width does, whatever the content.
        let l = laid(
            "<div id=g><div id=a></div><div id=b></div></div>",
            &format!("{grid} #a {{ min-width: 150px }}"),
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "b").0), (150.0, 150.0));
    }

    #[test]
    fn an_item_spanning_flexible_tracks_has_no_automatic_minimum() {
        // §6.6: `1fr 3fr` 50 wide, content 100 wide across both: the
        // columns are their shares, 12.5 and 37.5.
        let l = laid(
            &format!(
                "<div id=g><div id=a>{}</div><div id=b></div></div>",
                wide(100)
            ),
            "#g { display: grid; width: 50px; grid-template-columns: 1fr 3fr }
             #a { grid-column: 1 / 3 } #b { grid-row: 2; grid-column: 2 }",
        );
        assert_eq!(rect(&l, "a").2, 50.0);
        assert_eq!((rect(&l, "b").0, rect(&l, "b").2), (12.5, 37.5));
    }

    #[test]
    fn an_items_minimum_and_maximum_height_shape_its_row_and_its_height() {
        // A maximum height limits the item's contribution to its row.
        let l = laid(
            "<div id=g><div id=a style='max-height: 30px'><div style='height: 50px'></div></div></div>",
            "#g { display: grid }",
        );
        assert_eq!((rect(&l, "g").3, rect(&l, "a").3), (30.0, 30.0));
        // A minimum height is the minimum contribution, past a fixed
        // maximum.
        let l = laid(
            "<div id=g><div id=a style='min-height: 40px'></div><div id=b></div></div>",
            "#g { display: grid; height: 100px; grid-template-rows: minmax(auto, 10px) 1fr }",
        );
        assert_eq!(rect(&l, "b"), (0.0, 40.0, 800.0, 60.0));
        // A stretched item stops at its maximum height.
        let l = laid(
            "<div id=g><div id=a style='max-height: 20px'></div></div>",
            "#g { display: grid; grid-template-rows: 100px }",
        );
        assert_eq!(rect(&l, "a").3, 20.0);
    }

    #[test]
    fn a_percentage_height_counts_as_auto_while_the_rows_are_sized() {
        // The reference browser: two `minmax(auto, 80px)` rows in a grid
        // 60 tall share it, and the item is half of its row after.
        let l = laid(
            "<div id=g><div id=a style='height: 50%'></div><div id=b></div></div>",
            "#g { display: grid; height: 60px; grid-template-rows: minmax(auto, 80px) minmax(auto, 80px) }",
        );
        assert_eq!(rect(&l, "a").3, 15.0);
        assert_eq!(rect(&l, "b").1, 30.0);
    }

    #[test]
    fn an_item_not_stretched_takes_its_contents_size() {
        let inner = "<div id=g><div id=a><div style='width: 40px; height: 20px'></div></div></div>";
        let grid = "#g { display: grid; grid-template-columns: 100px; grid-template-rows: 100px }";
        let l = laid(
            inner,
            &format!("{grid} #g {{ justify-items: start; align-items: start }}"),
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 40.0, 20.0));
        let l = laid(
            inner,
            &format!("{grid} #a {{ justify-self: start; align-self: start }}"),
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 40.0, 20.0));
        // An `auto` margin keeps an item from stretching in its axis.
        let l = laid(
            inner,
            &format!("{grid} #a {{ margin-left: auto; margin-bottom: auto }}"),
        );
        assert_eq!((rect(&l, "a").2, rect(&l, "a").3), (40.0, 20.0));
    }

    #[test]
    fn items_sit_in_the_content_box_and_move_by_their_relative_offsets() {
        let l = laid(
            "<div id=g><div id=a></div><div id=b></div></div>",
            "#g { display: grid; margin: 10px; border: 4px solid; padding: 7px;
                  grid-template-columns: 100px 1fr; grid-template-rows: 30px }
             #a { margin: 5px 10px } #b { position: relative; top: 5px; left: 7px }",
        );
        assert_eq!(rect(&l, "a"), (31.0, 26.0, 80.0, 20.0));
        // 800 less the margins, borders and paddings leave 758: 100 and
        // 658.
        assert_eq!(rect(&l, "b"), (128.0, 26.0, 658.0, 30.0));
    }

    #[test]
    fn collapsed_tracks_inside_the_grid_leave_one_gutter() {
        // §7.2.3.2, the reference browser: four empty `auto-fit` tracks
        // between two occupied ones, the gutters around them one.
        let l = laid(
            &items(2),
            "#g { display: grid; width: 400px; column-gap: 10px; grid-auto-rows: 10px;
                  grid-template-columns: repeat(auto-fit, 50px) 100px } #i1 { grid-column: -2 }",
        );
        assert_eq!(rect(&l, "i1").0, 60.0);
    }

    #[test]
    fn a_percentage_fit_content_limit_is_of_the_grids_width() {
        let words: Vec<String> = (0..6).map(|_| wide(30)).collect();
        let l = laid(
            &format!(
                "<div id=g><div id=a>{}</div><div id=b></div></div>",
                words.join(" ")
            ),
            "#g { display: grid; width: 400px; grid-template-columns: fit-content(25%) 1fr }",
        );
        assert_eq!(rect(&l, "a").2, 100.0);
    }

    #[test]
    fn automatic_rows_repeat_in_the_maximum_or_to_the_minimum_height() {
        let grid = "display: grid; grid-template-rows: repeat(auto-fill, 30px);
                    grid-auto-flow: column; grid-auto-columns: 40px";
        // As many as fit in 100: three.
        let l = laid(&items(4), &format!("#g {{ {grid}; max-height: 100px }}"));
        assert_eq!(rect(&l, "i3"), (40.0, 0.0, 40.0, 30.0));
        // As few as reach 100: four.
        let l = laid(&items(4), &format!("#g {{ {grid}; min-height: 100px }}"));
        assert_eq!(rect(&l, "i3"), (0.0, 90.0, 40.0, 30.0));
        assert_eq!(rect(&l, "g").3, 120.0);
    }

    #[test]
    fn a_grids_intrinsic_widths_count_its_gaps_and_repetitions() {
        let l = laid(
            &items(2),
            "#g { display: inline-grid; column-gap: 10px; grid-template-columns: 50px 50px }",
        );
        assert_eq!(rect(&l, "g").2, 110.0);
        // A float's repetitions fill its maximum width.
        let l = laid(
            &items(2),
            "#g { float: left; display: grid; max-width: 200px; grid-template-columns: repeat(auto-fill, 50px) }",
        );
        assert_eq!((rect(&l, "g").2, rect(&l, "i1").0), (200.0, 50.0));
        // The limit on a spanning item's automatic minimum counts the gap:
        // the reference browser's float in no room at all is 100 wide.
        let l = laid(
            &format!(
                "<div style='width: 0'><div id=g><div id=a>{}</div></div></div>",
                wide(150)
            ),
            "#g { float: left; display: grid; column-gap: 20px;
                  grid-template-columns: minmax(auto, 40px) minmax(auto, 40px) }
             #a { grid-column: 1 / 3 }",
        );
        assert_eq!(rect(&l, "g").2, 100.0);
    }

    #[test]
    fn a_grids_own_width_does_not_repeat_its_tracks_for_its_intrinsic_sizes() {
        // The reference browser: 200 wide with `repeat(auto-fill, 50px)`,
        // its min-content and max-content widths are one repetition's.
        let grid = "display: grid; width: 200px; grid-template-columns: repeat(auto-fill, 50px)";
        for limit in ["max-width: min-content", "max-width: max-content"] {
            let l = laid(&items(1), &format!("#g {{ {grid}; {limit} }}"));
            assert_eq!(rect(&l, "g").2, 50.0, "{limit}");
        }
        // So a flex item's automatic minimum lets it shrink to that.
        let l = laid(
            "<div id=f><div id=a></div><div id=g></div></div>",
            &format!(
                "#f {{ display: flex; width: 100px }} #a {{ width: 50px; flex: none }} #g {{ {grid} }}"
            ),
        );
        assert_eq!(rect(&l, "g").2, 50.0);
    }

    #[test]
    fn rows_fill_a_height_imposed_but_not_definite() {
        // A column flex container with only a minimum height flexes the
        // grid to 100; that is not definite for percentages, but the
        // auto row still stretches to it, as in the reference browser
        // (grid-within-flexbox-indefinite).
        let l = laid(
            "<div id=f><div id=g><div id=a></div></div></div>",
            "#f { display: flex; flex-direction: column; min-height: 100px; width: 100px }
             #g { display: grid; flex: 1 }",
        );
        assert_eq!(rect(&l, "a").3, 100.0);
    }

    #[test]
    fn content_distribution_places_and_spaces_the_tracks() {
        // Each as the reference browser lays it out: two 50 by 40 tracks
        // each way in a grid 300 by 200, the first and the last item's
        // place.
        let grid = "#g { display: grid; width: 300px; height: 200px;
                         grid-template-columns: 50px 50px; grid-template-rows: 40px 40px;";
        for (value, columns, rows) in [
            ("start", (0.0, 50.0), (0.0, 40.0)),
            ("end", (200.0, 250.0), (120.0, 160.0)),
            ("center", (100.0, 150.0), (60.0, 100.0)),
            ("space-between", (0.0, 250.0), (0.0, 160.0)),
            ("space-around", (50.0, 200.0), (30.0, 130.0)),
            (
                "space-evenly",
                (200.0 / 3.0, 200.0 / 3.0 * 2.0 + 50.0),
                (40.0, 120.0),
            ),
        ] {
            let l = laid(
                &items(4),
                &format!("{grid} justify-content: {value}; align-content: {value} }}"),
            );
            let (first, last) = (rect(&l, "i0"), rect(&l, "i3"));
            assert_eq!((first.0, last.0), columns, "{value}");
            assert_eq!((first.1, last.1), rows, "{value}");
        }
    }

    #[test]
    fn a_distribution_that_cannot_apply_takes_its_fallback() {
        // Tracks 100 wide in a grid 80 wide: `center` overflows both
        // sides, `end` the start; `space-between` falls back to `start`,
        // `space-around` and `space-evenly` to `safe center`, so start.
        let grid = "#g { display: grid; width: 80px; height: 60px;
                         grid-template-columns: 50px 50px; grid-template-rows: 40px 40px;";
        for (value, at) in [
            ("center", (-10.0, -10.0)),
            ("end", (-20.0, -20.0)),
            ("space-between", (0.0, 0.0)),
            ("space-around", (0.0, 0.0)),
            ("space-evenly", (0.0, 0.0)),
            ("safe center", (0.0, 0.0)),
            ("safe end", (0.0, 0.0)),
        ] {
            let l = laid(
                &items(4),
                &format!("{grid} justify-content: {value}; align-content: {value} }}"),
            );
            assert_eq!((rect(&l, "i0").0, rect(&l, "i0").1), at, "{value}");
        }
        // `stretch` with no `auto` track to grow places them at the start;
        // with one, that one takes the room.
        let l = laid(
            &items(2),
            "#g { display: grid; width: 300px; grid-template-columns: auto 50px; justify-content: stretch }",
        );
        assert_eq!((rect(&l, "i0").2, rect(&l, "i1").0), (250.0, 250.0));
    }

    #[test]
    fn an_item_spanning_a_distributed_gap_gains_it() {
        // The design note's case, the reference browser's: rows of 20, 20
        // and 5 with `space-between` in a grid 100 tall, an item across
        // the first two 67.5 tall.
        let l = laid(
            "<div id=g><div id=a></div><div id=b></div></div>",
            "#g { display: grid; height: 100px; grid-template-columns: 50px;
                  grid-template-rows: 20px 20px 5px; align-content: space-between }
             #a { grid-row: 1 / 3 } #b { grid-row: 3 }",
        );
        assert_eq!(rect(&l, "a").3, 67.5);
        assert_eq!(rect(&l, "b").1, 95.0);
        // Collapsed tracks are no subjects and take none of the space.
        let l = laid(
            &items(2),
            "#g { display: grid; width: 400px; grid-auto-rows: 10px; justify-content: space-between;
                  grid-template-columns: repeat(auto-fit, 50px) 100px } #i1 { grid-column: -2 }",
        );
        assert_eq!((rect(&l, "i0").0, rect(&l, "i1").0), (0.0, 300.0));
        // With `space-around`, the two tracks left share the 250 pixels.
        let l = laid(
            &items(2),
            "#g { display: grid; width: 400px; grid-auto-rows: 10px; justify-content: space-around;
                  grid-template-columns: repeat(auto-fit, 50px) 100px } #i1 { grid-column: -2 }",
        );
        assert_eq!((rect(&l, "i0").0, rect(&l, "i1").0), (62.5, 237.5));
    }

    #[test]
    fn self_alignment_places_an_item_in_its_area() {
        // A 30 by 20 item in a 100 by 100 area.
        let html = "<div id=g><div id=a><div style='width: 30px; height: 20px'></div></div></div>";
        let grid = "#g { display: grid; grid-template-columns: 100px; grid-template-rows: 100px }";
        let at = |css: &str| {
            let l = laid(html, &format!("{grid} {css}"));
            (rect(&l, "a").0, rect(&l, "a").1)
        };
        for (value, x) in [
            ("start", 0.0),
            ("end", 70.0),
            ("center", 35.0),
            ("left", 0.0),
            ("right", 70.0),
            ("self-start", 0.0),
            ("self-end", 70.0),
            ("flex-start", 0.0),
            ("flex-end", 70.0),
        ] {
            let (left, _) = at(&format!(
                "#a {{ justify-self: {value}; align-self: start }}"
            ));
            assert_eq!(left, x, "{value}");
        }
        for (value, y) in [
            ("start", 0.0),
            ("end", 80.0),
            ("center", 40.0),
            ("self-end", 80.0),
        ] {
            let (_, top) = at(&format!(
                "#a {{ align-self: {value}; justify-self: start }}"
            ));
            assert_eq!(top, y, "{value}");
        }
        assert_eq!(
            at("#g { justify-items: center; align-items: end }"),
            (35.0, 80.0)
        );
        // Border and padding count in the item's outer size, and so do
        // margins.
        let l = laid(
            "<div id=g><div id=a style='width: 30px; height: 20px; padding: 5px; border: 5px solid'></div></div>",
            &format!("{grid} #a {{ justify-self: center; align-self: center }}"),
        );
        assert_eq!(rect(&l, "a"), (25.0, 30.0, 50.0, 40.0));
        assert_eq!(
            at("#a { margin: 10px 20px 5px 0; justify-self: center; align-self: center }"),
            (25.0, 42.5)
        );
    }

    #[test]
    fn an_overflowing_item_is_unsafe_unless_it_is_safe() {
        let html = "<div id=g><div id=a style='width: 150px; height: 150px'></div></div>";
        let grid = "#g { display: grid; grid-template-columns: 100px; grid-template-rows: 100px }";
        for (value, at) in [
            ("center", -25.0),
            ("end", -50.0),
            ("unsafe center", -25.0),
            ("safe center", 0.0),
            ("safe end", 0.0),
        ] {
            let l = laid(
                html,
                &format!("{grid} #a {{ justify-self: {value}; align-self: {value} }}"),
            );
            assert_eq!((rect(&l, "a").0, rect(&l, "a").1), (at, at), "{value}");
        }
    }

    #[test]
    fn auto_margins_take_the_free_space_before_alignment() {
        let html = "<div id=g><div id=a><div style='width: 30px; height: 20px'></div></div></div>";
        let grid = "#g { display: grid; grid-template-columns: 100px; grid-template-rows: 100px }";
        let at = |css: &str| {
            let l = laid(html, &format!("{grid} {css}"));
            (rect(&l, "a").0, rect(&l, "a").1)
        };
        assert_eq!(at("#a { margin: auto }"), (35.0, 40.0));
        assert_eq!(
            at("#a { margin-left: auto; margin-top: auto }"),
            (70.0, 80.0)
        );
        assert_eq!(
            at("#a { margin-right: auto; justify-self: end }").0,
            0.0,
            "the margin wins"
        );
        // An item that overflows has `auto` margins of zero.
        for margins in ["margin: auto", "margin-left: auto; margin-top: auto"] {
            let l = laid(
                &format!(
                    "<div id=g><div id=a style='width: 150px; height: 150px; {margins}'></div></div>"
                ),
                grid,
            );
            assert_eq!((rect(&l, "a").0, rect(&l, "a").1), (0.0, 0.0), "{margins}");
        }
    }

    #[test]
    fn stretch_that_cannot_stretch_falls_back_to_start() {
        let grid = "#g { display: grid; grid-template-columns: 100px; grid-template-rows: 100px }
                    #a { justify-self: stretch; align-self: stretch }";
        for item in [
            "max-width: 40px; max-height: 30px",
            "width: 40px; height: 30px",
        ] {
            let l = laid(
                &format!("<div id=g><div id=a style='{item}'></div></div>"),
                grid,
            );
            assert_eq!(rect(&l, "a"), (0.0, 0.0, 40.0, 30.0), "{item}");
        }
    }

    /// An item `id` whose first and last baseline is `baseline` from its
    /// border-box top, under [`BASELINE_CSS`]: its one line holds an
    /// inline-block as tall, with `below` of padding under it.
    fn item(id: &str, baseline: u32, below: u32, style: &str) -> String {
        format!(
            "<div id={id} style='padding-bottom: {below}px; {style}'>\
             <i style='height: {baseline}px'></i></div>"
        )
    }

    /// An item `id` with two lines, its first baseline `first` and its last
    /// `first + second`.
    fn two_lines(id: &str, first: u32, second: u32, style: &str) -> String {
        format!(
            "<div id={id} style='{style}'><div><i style='height: {first}px'></i></div>\
             <div><i style='height: {second}px'></i></div></div>"
        )
    }

    /// The `y` of each element of `ids`.
    fn ys_of(l: &Laid, ids: &[&str]) -> Vec<f32> {
        ids.iter().map(|id| rect(l, id).1).collect()
    }

    /// The style the baseline tests share: no strut, so that a line is as
    /// tall as its inline-block and its baseline at that block's bottom;
    /// a grid `.g` of three 40px columns and a 100px row.
    const BASELINE_CSS: &str = "body { font-size: 0; line-height: 0 }
         i { display: inline-block; width: 10px }
         .g { display: grid; grid-template-columns: repeat(3, 40px); grid-template-rows: 100px }";

    #[test]
    fn a_baseline_group_shares_its_baseline_in_its_row() {
        // §11.4 and css-align-3 §9.3, as the reference browser has it:
        // items whose baselines are 30, 10 and 20px down, 35, 50 and 20px
        // tall. The first group puts the item furthest from its baseline
        // at the row's top, the last group at its bottom.
        let row = format!(
            "{}{}{}",
            item("a", 30, 5, ""),
            item("b", 10, 40, ""),
            item("d", 20, 0, "")
        );
        let ys = |style: &str, ids: &[&str], body: &str| {
            let l = laid(
                &format!("<div id=g class=g style='{style}'>{body}</div>"),
                BASELINE_CSS,
            );
            (ys_of(&l, ids), rect(&l, "g").3)
        };
        let abd = ["a", "b", "d"];
        for (style, expected) in [
            ("align-items: baseline", [0.0, 20.0, 10.0]),
            ("align-items: first baseline", [0.0, 20.0, 10.0]),
            ("align-items: last baseline", [30.0, 50.0, 40.0]),
        ] {
            assert_eq!(ys(style, &abd, &row).0, expected, "{style}");
        }
        // An `auto` row holds the group: the largest distance above the
        // baseline and the largest below; both groups, when there are two.
        for style in ["align-items: baseline", "align-items: last baseline"] {
            let style = format!("{style}; grid-template-rows: auto");
            assert_eq!(ys(&style, &abd, &row), (vec![0.0, 20.0, 10.0], 70.0));
        }
        let both = format!(
            "{}{}{}",
            item("a", 30, 5, "align-self: baseline"),
            item("b", 10, 40, "align-self: last baseline"),
            item("d", 20, 0, "align-self: baseline")
        );
        assert_eq!(
            ys("grid-template-rows: auto", &abd, &both),
            (vec![0.0, 0.0, 10.0], 50.0)
        );
        // The shim is in each contribution: #b's minimum, 40 with
        // `min-height: 0`, is 60 with it, and its min-content contribution
        // 70, which the row takes.
        let low = format!(
            "{}{}{}",
            item("a", 30, 5, ""),
            item("b", 10, 40, "min-height: 0"),
            item("d", 20, 0, "")
        );
        for style in ["align-items: baseline", "align-items: last baseline"] {
            let style = format!("{style}; grid-template-rows: auto");
            assert_eq!(ys(&style, &abd, &low), (vec![0.0, 20.0, 10.0], 70.0));
        }
        // Margins count from the outer edge; an item with an `auto` margin
        // takes no part, its margin taking the space.
        let body = format!(
            "{}{}{}",
            item("a", 30, 5, ""),
            item("b", 10, 40, "margin-top: 25px"),
            item("d", 20, 0, "")
        );
        assert_eq!(
            ys("align-items: baseline", &abd, &body).0,
            [5.0, 25.0, 15.0]
        );
        let body = format!(
            "{}{}{}",
            item("a", 30, 5, ""),
            item("b", 40, 0, "margin-top: auto"),
            item("d", 20, 0, "")
        );
        assert_eq!(
            ys("align-items: baseline", &abd, &body).0,
            [0.0, 60.0, 10.0]
        );
        // An item without a baseline has one at its border box's bottom.
        let body = format!(
            "{}<div id=b style='height: 20px'></div><div id=d style='height: 50px'></div>",
            item("a", 30, 5, "")
        );
        assert_eq!(
            ys("align-items: baseline", &abd, &body).0,
            [20.0, 30.0, 0.0]
        );
        // An item's first baseline is its first line's, its last its last.
        let lines = format!("{}{}", two_lines("a", 10, 30, ""), item("b", 20, 0, ""));
        assert_eq!(
            ys("align-items: first baseline", &["a", "b"], &lines).0,
            [10.0, 0.0]
        );
        assert_eq!(
            ys("align-items: last baseline", &["a", "b"], &lines).0,
            [60.0, 80.0]
        );
        // A scroll container keeps its content's baseline.
        let body = format!(
            "{}{}",
            item("a", 30, 5, ""),
            item("b", 10, 40, "overflow: hidden")
        );
        assert_eq!(
            ys("align-items: baseline", &["a", "b"], &body).0,
            [0.0, 20.0]
        );
        // A group too large for its row stays flush with the row's edge,
        // past the other one: unsafe.
        let pair = format!("{}{}", item("a", 30, 5, ""), item("b", 10, 40, ""));
        for (style, expected) in [
            ("align-items: baseline", [0.0, 20.0]),
            ("align-items: last baseline", [-40.0, -20.0]),
        ] {
            let style = format!("{style}; grid-template-rows: 30px");
            assert_eq!(ys(&style, &["a", "b"], &pair).0, expected, "{style}");
        }
        // The others in the row align as they would without the group.
        for (style, expected) in [("", (0.0, 70.0)), ("align-self: center", (30.0, 10.0))] {
            let body = format!(
                "{}{}{}",
                item("a", 30, 5, "align-self: baseline"),
                item("b", 10, 40, "align-self: baseline"),
                item("d", 5, 5, style)
            );
            let l = laid(
                &format!("<div class=g style='grid-template-rows: auto'>{body}</div>"),
                BASELINE_CSS,
            );
            let d = rect(&l, "d");
            assert_eq!((d.1, d.3), expected, "{style}");
        }
    }

    #[test]
    fn a_spanning_item_shares_the_group_of_the_row_it_starts_or_ends_in() {
        // #a spans both rows: its first baseline joins the first row's
        // group, its last the second's (§11.4).
        let body = format!(
            "{}{}{}{}",
            item("a", 30, 5, "grid-row: 1 / 3"),
            item("b", 10, 40, ""),
            item("d", 20, 0, "grid-row: 2; grid-column: 2"),
            item("e", 5, 0, "grid-row: 2; grid-column: 3")
        );
        for (align, expected, height) in [
            ("baseline", [0.0, 20.0, 70.0, 85.0], 90.0),
            ("last baseline", [40.0, 0.0, 50.0, 65.0], 75.0),
        ] {
            let l = laid(
                &format!(
                    "<div id=g class=g style='align-items: {align}; grid-template-rows: auto auto'>\
                     {body}</div>"
                ),
                BASELINE_CSS,
            );
            assert_eq!(ys_of(&l, &["a", "b", "d", "e"]), expected, "{align}");
            assert_eq!(rect(&l, "g").3, height, "{align}");
        }
    }

    #[test]
    fn an_item_whose_size_depends_on_its_track_takes_no_part() {
        // A percentage height in a row sized by its content would depend on
        // the baseline alignment that sizes the row: the item takes the
        // fallback, `start` (§11.4), a flexible row counting as sized by
        // content only when the grid's height is not definite. The
        // reference browser aligns it all the same (design note).
        let at = |grid: &str, b: &str| {
            let l = laid(
                &format!(
                    "<div class=g style='align-items: baseline; {grid}'>{}\
                     <div id=b style='{b}'><i style='height: 10px'></i></div>{}</div>",
                    item("a", 30, 5, ""),
                    item("d", 20, 0, "")
                ),
                BASELINE_CSS,
            );
            rect(&l, "b").1
        };
        for (grid, b, y) in [
            ("grid-template-rows: auto", "height: 50%", 0.0),
            (
                "grid-template-rows: auto; height: 100px",
                "min-height: 50%",
                0.0,
            ),
            ("grid-template-rows: 1fr", "height: 50%", 0.0),
            (
                "grid-template-rows: 1fr; height: 100px",
                "height: 50%",
                20.0,
            ),
            ("grid-template-rows: 100px", "height: 50%", 20.0),
            ("grid-template-rows: auto", "height: 50px", 20.0),
            (
                "grid-template-rows: minmax(auto, 100px)",
                "height: 50%",
                0.0,
            ),
            (
                "grid-template-rows: auto; height: 100px",
                "max-height: 50%",
                0.0,
            ),
        ] {
            assert_eq!(at(grid, b), y, "{grid}, {b}");
        }
    }

    #[test]
    fn a_column_group_lines_up_its_items_left_border_edges() {
        // Items have no baselines across their lines here: the reference
        // browser synthesizes both from the left border edge, which the
        // group lines up, the first group from the column's start and the
        // last from its end (css-align-3 §9.1).
        let boxes = "<div id=a style='width: 30px'></div>\
             <div id=b style='width: 30px; margin-left: 10px; margin-right: 5px'></div>\
             <div id=d style='width: 30px; margin-left: 20px'></div>";
        let css = format!(
            "{BASELINE_CSS} .g {{ grid-template-columns: 100px; grid-template-rows: repeat(3, 20px) }}
             .g > div {{ height: 10px }}"
        );
        for (align, x) in [("baseline", 20.0), ("last baseline", 65.0)] {
            let l = laid(
                &format!("<div class=g style='justify-items: {align}'>{boxes}</div>"),
                &css,
            );
            let xs: Vec<f32> = ["a", "b", "d"].iter().map(|id| rect(&l, id).0).collect();
            assert_eq!(xs, [x, x, x], "{align}");
        }
        // The shims widen an `auto` column (§12.5 step 1): to 47, #a's 37
        // after its shim of 10, so that #d and #e line up 20 into the
        // next. Padding does not move the edge.
        let l = laid(
            "<div class=g style='justify-items: baseline; grid-template-columns: auto 1fr'>\
             <div id=a style='width: 30px; padding-left: 7px'></div>\
             <div id=b style='width: 30px; margin-left: 10px'></div>\
             <div id=d style='width: 30px; margin-left: 20px'></div>\
             <div id=e style='grid-column: 2; grid-row: 1'></div></div>",
            &css,
        );
        let xs: Vec<f32> = ["a", "b", "d", "e"]
            .iter()
            .map(|id| rect(&l, id).0)
            .collect();
        assert_eq!(xs, [10.0, 10.0, 67.0, 67.0]);
        // And the intrinsic width of a grid sized to fit: in a last-baseline
        // group the items are 30, 35 and 30 from their right edges to their
        // left border edges, so #a and #d take shims of 5, and #d is 55
        // wide with its.
        let l = laid(
            &format!(
                "<div id=g class=g style='display: inline-grid; justify-items: last baseline; \
                 grid-template-columns: auto'>{boxes}</div>"
            ),
            &css,
        );
        assert_eq!(rect(&l, "g").2, 55.0);
    }

    #[test]
    fn a_column_shim_counts_in_each_contribution() {
        // #b's margin puts the group's edge 50 in, so #a, 25 wide, has a
        // shim of 50: its minimum contribution is 75, and a fixed maximum
        // limits its content-based minimum after the shim (§6.6), as in the
        // reference browser. #r, in the next column, shows the first one's
        // width; the grid is too narrow to grow it.
        let width = |columns: &str| {
            let l = laid(
                &format!(
                    "<div class=g style='width: 0; grid-template-columns: {columns} 10px; \
                     grid-template-rows: auto auto; justify-items: baseline'>\
                     <div id=a><i style='width: 25px; height: 5px'></i></div>\
                     <div id=b style='margin-left: 50px; grid-column: 1'></div>\
                     <div id=r style='grid-column: 2; grid-row: 1'></div></div>"
                ),
                BASELINE_CSS,
            );
            rect(&l, "r").0
        };
        for (columns, expected) in [
            ("minmax(auto, 0px)", 50.0),
            ("minmax(auto, 60px)", 60.0),
            ("minmax(auto, 75px)", 75.0),
            ("minmax(auto, 100px)", 75.0),
            ("auto", 75.0),
        ] {
            assert_eq!(width(columns), expected, "{columns}");
        }
        // #a is 20 wide at its narrowest and 40 at its widest, #b nothing
        // past its 30px margin. In a first-baseline group #a's shim is 30
        // in both contributions; in a last-baseline group #b's is 20 in its
        // min-content contribution and 40 in its max-content one (§12.5
        // step 1: each contribution its own), so the grid is 50 at its
        // narrowest and 70 at its widest. The reference browser takes the
        // min-content shim for both and is 50 wide at its widest too.
        let at = |align: &str, narrow: bool| {
            let grid = format!(
                "<div id=g class=g style='display: inline-grid; grid-template-columns: auto; \
                 grid-template-rows: auto auto; justify-items: {align}'>\
                 <div id=a><i style='width: 20px; height: 5px'></i> \
                 <i style='width: 20px; height: 5px'></i></div>\
                 <div id=b style='margin-left: 30px'></div></div>"
            );
            let html = if narrow {
                format!("<div style='width: 0'>{grid}</div>")
            } else {
                grid
            };
            let l = laid(&html, BASELINE_CSS);
            (rect(&l, "g").2, rect(&l, "a").0, rect(&l, "b").0)
        };
        assert_eq!(at("baseline", false), (70.0, 30.0, 30.0));
        assert_eq!(at("baseline", true), (50.0, 30.0, 30.0));
        assert_eq!(at("last baseline", false), (70.0, 30.0, 30.0));
        assert_eq!(at("last baseline", true), (50.0, 10.0, 10.0));
    }

    #[test]
    fn shims_count_in_the_minimum_contribution() {
        // WPT grid-minimum-contribution-baseline-shim, built without its
        // vertical-align: #b's baseline is at its top, #a's at its bottom,
        // 50 down, so #b's shim is 50 and its minimum contribution 75 at
        // the least; a fixed maximum limits the content-based minimum
        // after the shim (§6.6). #r, stretched in a column of its own,
        // shows the row.
        let at = |rows: &str, align: &str| {
            let l = laid(
                &format!(
                    "<div class=g style='grid-template-rows: {rows}; align-items: {align}'>\
                     <div id=a style='padding-top: 25px'><i style='height: 25px; width: 25px'></i></div>\
                     <div id=b style='padding-bottom: 25px'><i style='height: 0; width: 25px'></i>\
                     <div style='height: 25px'></div></div>\
                     <div id=r style='grid-column: 3; grid-row: 1; min-height: 0; align-self: stretch'></div></div>"
                ),
                "body { font-size: 0; line-height: 0 } i { display: inline-block }
                 .g { display: grid; height: 0; width: 0; grid-template-columns: 50px 50px 10px }",
            );
            (rect(&l, "a").1, rect(&l, "b").1, rect(&l, "r").3)
        };
        for (rows, row) in [
            ("minmax(auto, 0px)", 75.0),
            ("minmax(auto, 75px)", 75.0),
            ("minmax(auto, 88px)", 88.0),
            ("minmax(auto, 100px)", 100.0),
            ("minmax(auto, 150px)", 100.0),
            ("minmax(auto, auto)", 100.0),
        ] {
            assert_eq!(at(rows, "baseline"), (0.0, 50.0, row), "{rows}");
        }
        // In a last-baseline group the shim is after #a, at its end.
        for (rows, expected) in [
            ("minmax(auto, 0px)", (-25.0, 25.0, 75.0)),
            ("minmax(auto, 88px)", (-12.0, 38.0, 88.0)),
            ("minmax(auto, auto)", (0.0, 50.0, 100.0)),
        ] {
            assert_eq!(at(rows, "last baseline"), expected, "{rows}");
        }
    }

    #[test]
    fn a_grid_container_has_the_baselines_of_its_first_and_last_rows() {
        // §11.6, as the reference browser has it: an inline grid sits on
        // its first baseline, which is where a 50px inline-block beside it
        // ends, so that baseline is that far below the grid's top.
        let baseline = |grid: &str, items: &str| {
            let l = laid(
                &format!(
                    "<div><i id=r style='height: 50px'></i><div id=g style='display: inline-grid; \
                     grid-template-columns: 40px 40px; {grid}'>{items}</div></div>"
                ),
                BASELINE_CSS,
            );
            rect(&l, "r").1 + 50.0 - rect(&l, "g").1
        };
        // The first row's first item in grid order, by column and then in
        // order-modified document order; relative offsets do not count.
        let cases = [
            (
                "",
                format!("{}{}", item("a", 30, 5, ""), item("b", 10, 40, "")),
                30.0,
            ),
            (
                "",
                format!(
                    "{}{}",
                    item("x", 30, 5, "grid-column: 2; grid-row: 1"),
                    item("y", 10, 5, "grid-column: 1; grid-row: 1")
                ),
                10.0,
            ),
            (
                "",
                format!(
                    "{}{}",
                    item("a", 12, 0, "grid-area: 1 / 1; order: 2"),
                    item("b", 8, 0, "grid-area: 1 / 1; order: 1")
                ),
                8.0,
            ),
            ("", item("a", 30, 5, "position: relative; top: 10px"), 30.0),
            ("", two_lines("a", 10, 30, ""), 10.0),
            // The first row an item starts in.
            (
                "grid-template-rows: 20px",
                item("x", 30, 5, "grid-row: 2"),
                50.0,
            ),
            (
                "grid-template-rows: 20px 20px",
                format!(
                    "{}{}",
                    item("x", 8, 0, "grid-row: 2; grid-column: 1"),
                    item("y", 30, 5, "grid-row: 1 / 3; grid-column: 2")
                ),
                30.0,
            ),
            // A group in that row before its first item, a first-baseline
            // group before a last; an item whose last-baseline group is in
            // another row does not count.
            (
                "grid-template-rows: 100px",
                format!(
                    "{}{}",
                    item("x", 30, 5, "align-self: start"),
                    item("y", 10, 5, "align-self: baseline; margin-top: 7px")
                ),
                17.0,
            ),
            (
                "grid-template-rows: 100px",
                format!(
                    "{}{}",
                    item("x", 30, 5, "align-self: start"),
                    item("y", 10, 5, "align-self: last baseline")
                ),
                95.0,
            ),
            (
                "grid-template-rows: 100px",
                format!(
                    "{}{}",
                    item("x", 30, 5, "align-self: last baseline"),
                    item("y", 10, 5, "align-self: baseline; margin-top: 3px")
                ),
                13.0,
            ),
            (
                "grid-template-rows: 20px 20px",
                format!(
                    "{}{}",
                    item("a", 12, 0, "grid-row: 1; grid-column: 1"),
                    item(
                        "b",
                        8,
                        0,
                        "grid-row: 1 / 3; grid-column: 2; align-self: last baseline"
                    )
                ),
                12.0,
            ),
            // An item without a baseline gives its border box's bottom;
            // a scroll container keeps its items' baseline.
            (
                "",
                "<div style='height: 20px; margin-top: 4px'></div>".to_string(),
                24.0,
            ),
            (
                "overflow: hidden; height: 100px",
                item("a", 30, 5, ""),
                30.0,
            ),
            // With no item, the inline grid's bottom margin edge.
            ("height: 40px; margin-bottom: 5px", String::new(), 45.0),
        ];
        for (grid, items, expected) in &cases {
            assert_eq!(baseline(grid, items), *expected, "{grid} {items}");
        }
    }

    #[test]
    fn a_grid_containers_last_baseline_is_its_last_rows() {
        // A flex container's last-baseline group shows it: the grid's
        // last baseline meets that of a 50px item beside it, at its end.
        let baseline = |rows: &str, items: &str| {
            let l = laid(
                &format!(
                    "<div style='display: flex; align-items: last baseline'>{}\
                     <div id=g style='display: grid; grid-template-columns: 40px 40px; \
                     grid-template-rows: {rows}'>{items}</div></div>",
                    item("r", 50, 0, "")
                ),
                BASELINE_CSS,
            );
            rect(&l, "r").1 + 50.0 - rect(&l, "g").1
        };
        for (rows, items, expected) in [
            // The last item in grid order in the last row an item ends
            // in: by the column it ends in, then in order-modified
            // document order.
            (
                "20px 20px",
                format!(
                    "{}{}",
                    item("a", 30, 5, "grid-row: 1 / 3; grid-column: 2"),
                    item("b", 8, 0, "grid-row: 2; grid-column: 1")
                ),
                30.0,
            ),
            (
                "20px 20px",
                format!(
                    "{}{}",
                    item("a", 30, 5, "grid-row: 1 / 3; grid-column: 1"),
                    item("b", 8, 0, "grid-row: 2; grid-column: 2")
                ),
                28.0,
            ),
            (
                "20px 20px",
                format!(
                    "{}{}",
                    item("a", 12, 0, "grid-area: 2 / 2; order: 2"),
                    item("b", 8, 0, "grid-area: 2 / 2; order: 1")
                ),
                32.0,
            ),
            // A last-baseline group in that row first.
            (
                "20px 60px",
                format!(
                    "{}{}{}",
                    item("a", 30, 5, "grid-row: 1"),
                    item(
                        "b",
                        10,
                        5,
                        "grid-row: 2; grid-column: 1; align-self: baseline"
                    ),
                    item(
                        "c",
                        12,
                        5,
                        "grid-row: 2; grid-column: 2; align-self: last baseline"
                    )
                ),
                75.0,
            ),
        ] {
            assert_eq!(baseline(rows, &items), expected, "{items}");
        }
    }

    #[test]
    fn around_a_grid_its_baselines_are_used() {
        // A grid whose first baseline is 30 and last 45 down: an
        // inline-block and a table cell around it take its first, as the
        // reference browser has it, a block both (css-align-3 §9.1).
        let grid = format!(
            "<div id=g style='display: grid'>{}{}</div>",
            item("a", 30, 5, ""),
            item("b", 10, 5, "")
        );
        let reference = "<i style='height: 50px'></i>";
        let l = laid(
            &format!("<div>{reference}<div id=w style='display: inline-block'>{grid}</div></div>"),
            BASELINE_CSS,
        );
        assert_eq!(rect(&l, "w").1, 20.0);
        let l = laid(
            &format!(
                "<table style='border-spacing: 0'><tr><td style='padding: 0; vertical-align: baseline'>\
                 {reference}</td><td style='padding: 0; vertical-align: baseline'>{grid}</td></tr></table>"
            ),
            BASELINE_CSS,
        );
        assert_eq!(rect(&l, "g").1, 20.0);
        for (align, y) in [("baseline", 20.0), ("last baseline", 5.0)] {
            let l = laid(
                &format!(
                    "<div style='display: flex; align-items: {align}'>{}<div id=w>{grid}</div></div>",
                    item("r", 50, 0, "")
                ),
                BASELINE_CSS,
            );
            assert_eq!(rect(&l, "w").1, y, "{align}");
        }
    }

    /// The grid the tests of absolutely positioned boxes share: its
    /// border box at the top-left, 5px border, padding 10px 20px 30px
    /// 40px, so its padding box is 5 to 465 across and 5 to 245 down and
    /// its content box 45 to 445 and 15 to 215; columns of 100 and 200 at
    /// 45 and 145, rows of 50 and 70 at 15 and 65. `.a` is an absolutely
    /// positioned box 20 by 20.
    const ABSOLUTE_CSS: &str = "
        .g { display: grid; position: relative; border: 5px solid; padding: 10px 20px 30px 40px;
             width: 400px; height: 200px; grid-template-columns: 100px 200px;
             grid-template-rows: 50px 70px }
        .a { position: absolute; width: 20px; height: 20px }
        .s { height: 10px }";

    /// `#x`'s border box in a grid `.g` with the style `grid` holding
    /// `body`.
    fn absolute_at(body: &str, grid: &str) -> (f32, f32, f32, f32) {
        let l = laid(
            &format!("<div class=g style='{grid}'>{body}</div>"),
            ABSOLUTE_CSS,
        );
        rect(&l, "x")
    }

    /// A box `#x` with the style `style` that fills its containing block.
    fn filling(style: &str) -> String {
        format!(
            "<div id=x class=a style='{style}; top: 0; right: 0; bottom: 0; left: 0; \
             width: auto; height: auto'></div>"
        )
    }

    #[test]
    fn an_absolutely_positioned_box_takes_its_grid_area_for_containing_block() {
        // §10.1, as the reference browser has it: the area its own lines
        // give, the gutters outside it, an `auto` line the padding edge,
        // percentages and insets of the area; a descendant of an item the
        // same, and a positioned item its own containing block.
        for (style, grid, expected) in [
            (
                "grid-column: 2; grid-row: 2",
                "",
                (145.0, 65.0, 320.0, 180.0),
            ),
            ("grid-column: auto / 2", "", (5.0, 5.0, 140.0, 240.0)),
            (
                "grid-area: 1 / 2 / 3 / 3",
                "gap: 10px 15px",
                (160.0, 15.0, 200.0, 130.0),
            ),
            (
                "grid-area: 1 / 1 / 2 / 2",
                "gap: 10px 15px",
                (45.0, 15.0, 100.0, 50.0),
            ),
            (
                "grid-area: 1 / 1 / 2 / 2",
                "justify-content: center; align-content: end",
                (95.0, 95.0, 100.0, 50.0),
            ),
        ] {
            assert_eq!(
                absolute_at(&filling(style), grid),
                expected,
                "{style}, {grid}"
            );
        }
        let x = absolute_at(
            "<div id=x class=a style='grid-area: 2 / 2; width: 50%; height: 50%; left: 10px'></div>",
            "",
        );
        assert_eq!(x, (155.0, 65.0, 160.0, 90.0));
        let x = absolute_at(
            &format!("<div class=s>{}</div>", filling("grid-area: 2 / 2 / 3 / 3")),
            "",
        );
        assert_eq!(x, (145.0, 65.0, 200.0, 70.0));
        let x = absolute_at(
            "<div style='position: relative; grid-area: 2 / 2; height: 30px'>\
             <div id=x class=a style='grid-column: 1; left: 0; right: 0; width: auto; \
             height: 5px'></div></div>",
            "",
        );
        assert_eq!(x, (145.0, 65.0, 200.0, 5.0));
    }

    #[test]
    fn a_line_the_grid_does_not_have_is_the_padding_edge() {
        // §10.1: no auto-placement and no implicit tracks: a span alone
        // is two `auto` lines, and a line the grid as laid out does not
        // have, named, numbered or reached by a span, is `auto` too; two
        // lines the same are a span of one. Lines the items made exist.
        let padding_box = (5.0, 5.0, 460.0, 240.0);
        for (style, grid, body, expected) in [
            ("grid-column: 5; grid-row: 4 / 7", "", "", padding_box),
            ("grid-column: span 2; grid-row: span 1", "", "", padding_box),
            ("grid-column: -2 / -1", "", "", (145.0, 5.0, 200.0, 240.0)),
            ("grid-column: 2 / 2", "", "", (145.0, 5.0, 200.0, 240.0)),
            ("grid-column: 1 / span 2", "", "", (45.0, 5.0, 300.0, 240.0)),
            (
                "grid-column: 2 / span 3",
                "",
                "",
                (145.0, 5.0, 320.0, 240.0),
            ),
            (
                "grid-column: b / c",
                "grid-template-columns: [a] 100px [b] 200px [c]",
                "",
                (145.0, 5.0, 200.0, 240.0),
            ),
            (
                "grid-column: zz / c",
                "grid-template-columns: [a] 100px [b] 200px [c]",
                "",
                (345.0, 5.0, 120.0, 240.0),
            ),
            (
                "grid-column: 3 / 4",
                "",
                "<div class=s style='grid-column: 3'></div>",
                (345.0, 5.0, 100.0, 240.0),
            ),
            (
                "grid-column: -4 / -3",
                "",
                "<div class=s style='grid-column: -4'></div>",
                (45.0, 5.0, 100.0, 240.0),
            ),
        ] {
            let body = format!("{body}{}", filling(style));
            assert_eq!(absolute_at(&body, grid), expected, "{style}");
        }
    }

    #[test]
    fn an_absolutely_positioned_child_is_placed_in_its_static_rectangle() {
        // §10.2: as the sole item of an area whose edges are the content
        // box's, or its grid area when the grid container is its
        // containing block too; by `justify-self` and `align-self`, `auto`
        // being the container's `justify-items` and `align-items`, and
        // `normal` and `stretch` being `start`, at its shrink-to-fit size.
        for (style, grid, expected) in [
            ("grid-area: 2 / 2", "", (145.0, 65.0)),
            (
                "grid-area: 2 / 2; justify-self: end; align-self: center",
                "",
                (445.0, 145.0),
            ),
            ("grid-area: 2 / 2; justify-self: center", "", (295.0, 65.0)),
            ("", "", (5.0, 5.0)),
            ("justify-self: end; align-self: end", "", (445.0, 225.0)),
            (
                "",
                "justify-items: center; align-items: end",
                (225.0, 225.0),
            ),
        ] {
            let x = absolute_at(&format!("<div id=x class=a style='{style}'></div>"), grid);
            assert_eq!((x.0, x.1), expected, "{style}, {grid}");
        }
        for style in ["justify-self: stretch; align-self: stretch", ""] {
            let x = absolute_at(
                &format!(
                    "<div id=x style='position: absolute; grid-area: 2 / 2; {style}'>\
                     <span style='display: inline-block; width: 30px; height: 10px'></span></div>"
                ),
                "",
            );
            assert_eq!((x.0, x.1, x.2), (145.0, 65.0, 30.0), "{style}");
        }
        let x = absolute_at(
            "<div id=x class=a style='grid-area: 2 / 2; margin: 5px 6px 7px 8px; \
             justify-self: end; align-self: end'></div>",
            "",
        );
        assert_eq!((x.0, x.1), (439.0, 218.0));
        // Not its containing block: the content box, whatever its own
        // placement, in a grid inside a positioned box 7 in.
        let in_wrapper = |child: &str| {
            let l = laid(
                &format!(
                    "<div style='position: relative; padding: 7px'><div class=g style='position: static'>\
                     {child}</div></div>"
                ),
                ABSOLUTE_CSS,
            );
            let x = rect(&l, "x");
            (x.0, x.1)
        };
        assert_eq!(
            in_wrapper("<div id=x class=a style='justify-self: center; align-self: end'></div>"),
            (242.0, 202.0)
        );
        assert_eq!(
            in_wrapper(
                "<div id=x class=a style='grid-area: 2 / 2; justify-self: end; align-self: end'></div>"
            ),
            (432.0, 202.0)
        );
        // A fixed child of a positioned grid container: the grid is not
        // its containing block, so the content box, as §10.2 has it; the
        // reference browser takes its grid area here too (design note).
        let x = absolute_at(
            "<div id=x style='position: fixed; width: 20px; height: 20px; grid-area: 2 / 2; \
             justify-self: end'></div>",
            "",
        );
        assert_eq!((x.0, x.1), (425.0, 15.0));
    }

    #[test]
    fn an_absolutely_positioned_child_takes_no_cell() {
        // It is no item: the items are placed as if it were not there.
        let l = laid(
            "<div class=g><div id=x class=a style='grid-row: 1; grid-column: 1'></div>\
             <div id=y class=s></div><div id=z class=s></div></div>",
            ABSOLUTE_CSS,
        );
        assert_eq!((rect(&l, "y").0, rect(&l, "z").0), (45.0, 145.0));
        assert_eq!((rect(&l, "x").0, rect(&l, "x").1), (45.0, 15.0));
    }

    #[test]
    fn a_scrolling_grid_scrolls_to_the_end_of_its_tracks() {
        let l = laid(
            "<div id=g><div id=a style='height: 10px'></div></div>",
            "#g { display: grid; height: 100px; overflow: hidden; grid-template-rows: 300px }
             #a { align-self: start }",
        );
        assert_eq!(fragment(&l, "g").scroll_height, 300.0);
        // Or to an item's margin box below them, wherever its row starts
        // (css-overflow-3 §2.2: a grid item's margins are in it, as a
        // flex item's are; the reference browser counts a grid item's
        // border box alone, 200 here).
        let l = laid(
            "<div id=g><div id=a style='height: 150px; margin-bottom: 20px; grid-row: 2'></div></div>",
            "#g { display: grid; height: 100px; overflow: hidden; grid-template-rows: 50px 50px }
             #a { align-self: start }",
        );
        assert_eq!(fragment(&l, "g").scroll_height, 220.0);
    }

    #[test]
    fn a_grid_beside_a_float_keeps_clear_of_it() {
        let l = laid(
            "<div id=f></div><div id=g><div id=a></div></div>",
            "#f { float: left; width: 100px; height: 50px } #g { display: grid; grid-template-rows: 10px }",
        );
        assert_eq!(rect(&l, "g").0, 100.0);
        assert_eq!(rect(&l, "a"), (100.0, 0.0, 700.0, 10.0));
    }

    #[test]
    fn an_items_percentages_are_of_its_area() {
        let l = laid(
            "<div id=g><div id=a><div id=in></div></div></div>",
            "#g { display: grid; grid-template-columns: 200px; grid-template-rows: 100px }
             #a { margin-left: 10%; padding-top: 5%; width: 50%; height: 50% }
             #in { height: 50% }",
        );
        assert_eq!(rect(&l, "a"), (20.0, 0.0, 100.0, 60.0));
        assert_eq!(rect(&l, "in").3, 25.0);
    }

    #[test]
    fn a_grid_inside_a_grid_item_is_laid_out_in_its_area() {
        let l = laid(
            "<div id=g><div id=a><div id=x></div><div id=y></div></div></div>",
            "#g { display: grid; grid-template-columns: 300px; grid-template-rows: 80px }
             #a { display: grid; grid-template-columns: 1fr 2fr }",
        );
        assert_eq!(rect(&l, "x"), (0.0, 0.0, 100.0, 80.0));
        assert_eq!(rect(&l, "y"), (100.0, 0.0, 200.0, 80.0));
    }

    #[test]
    fn an_empty_grid_is_as_tall_as_its_own_height_says() {
        let l = laid(
            "<div id=g></div>",
            "#g { display: grid; grid-template-columns: 1fr }",
        );
        assert_eq!(rect(&l, "g"), (0.0, 0.0, 800.0, 0.0));
        let l = laid("<div id=g></div>", "#g { display: grid; min-height: 30px }");
        assert_eq!(rect(&l, "g").3, 30.0);
    }

    #[test]
    fn a_grid_with_a_definite_height_stretches_its_auto_rows() {
        let l = laid(
            &items(2),
            "#g { display: grid; height: 100px; grid-template-rows: auto auto } #i0 { height: 10px }",
        );
        assert_eq!(rect(&l, "i0").3, 10.0);
        assert_eq!(rect(&l, "i1"), (0.0, 55.0, 800.0, 45.0));
    }
}
