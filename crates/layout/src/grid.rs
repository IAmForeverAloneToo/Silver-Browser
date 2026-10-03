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
    BOTTOM, BoxSizing, ComputedStyle, ContentAlignment, LEFT, LengthPercentage,
    LengthPercentageAuto, MaxSize, RIGHT, SelfAlignment, Size, TOP, TrackBreadth, TrackList,
    TrackSize,
};

use crate::block::{
    AdjoiningMargins, BlockLayout, Content, Deferred, Edges, Imposed, Intrinsic, box_fragment,
    carried, clamp_height, definite_height, extent_bottom, height_bounds, layout_contents,
    relative_offset, settle_absolutes, width_constraints,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::pass::{ItemKey, Pass};
use placement::{Axis, ExplicitAxis, Room, place, repetitions, resolve};
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

/// The grid: its items, placed, and its tracks in each axis.
struct Grid<'a> {
    items: Vec<GridItem<'a>>,
    rows: AxisTracks,
    columns: AxisTracks,
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

    /// From the first track's start to the last's end.
    fn extent(&self) -> f32 {
        match (self.starts.last(), self.sizes.last()) {
            (Some(last), Some(size)) => last + size,
            _ => 0.0,
        }
    }
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
/// maximum, by what those leave it.
fn automatic_minimum_width<'a>(
    item: &GridItem<'a>,
    columns: &[Track],
    gutters: &[f32],
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
        content = content.min((limit - margins - non_content).max(0.0));
    }
    content
}

/// The item's contributions to the columns (§12.5, css-sizing-3 §5.2):
/// its min-content and max-content contributions as a block container's
/// child gives them, and its minimum contribution, the outer size its
/// minimum width, `automatic` for `auto`, would give it when its `width`
/// is `auto` or a percentage.
fn column_contribution<'a>(item: &GridItem<'a>, automatic: f32, pass: &Pass<'a>) -> Contribution {
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
        minimum,
        min_content,
        max_content,
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
}

/// An item's automatic minimum height as a content-box height (§6.6),
/// zero when it does not apply: its content height, clamped by a
/// definite `max-height` and, when every row it spans has a fixed
/// maximum, by what those leave it.
fn automatic_minimum_height(
    item: &GridItem<'_>,
    auto_height: f32,
    rows: &[Track],
    gutters: &[f32],
    vertical: f32,
    margin: &[f32; 4],
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
        content = content.min((limit - margin[TOP] - margin[BOTTOM] - vertical).max(0.0));
    }
    content
}

/// The item's contributions to the rows (§12.5): its outer height at its
/// columns' width, both its min-content and its max-content
/// contribution; and its minimum contribution, from its minimum height
/// when its `height` is `auto` or a percentage, an `auto` one its
/// content-based minimum (§6.6).
fn row_contribution(
    item: &GridItem<'_>,
    measured: &Measured,
    rows: &[Track],
    gutters: &[f32],
    edges: &Edges,
    margin: &[f32; 4],
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
                ),
                Size::MinContent | Size::MaxContent | Size::FitContent => measured.auto_height,
            };
            content + vertical + margin[TOP] + margin[BOTTOM]
        }
        _ => measured.outer,
    };
    Contribution {
        span: item.rows.clone(),
        minimum,
        min_content: measured.outer,
        max_content: measured.outer,
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
    let automatic_widths: Vec<f32> = grid
        .items
        .iter()
        .map(|item| automatic_minimum_width(item, &columns, &column_gutters, pass))
        .collect();
    let column_items: Vec<Contribution> = grid
        .items
        .iter()
        .zip(&automatic_widths)
        .map(|(item, &automatic)| column_contribution(item, automatic, pass))
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
    let column_lines = Lines::new(
        columns.iter().map(Track::size).collect(),
        &column_gutters,
        0.0,
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
            });
            Sized {
                edges,
                margin,
                content_width,
                measured,
            }
        })
        .collect();
    let row_items: Vec<Contribution> = grid
        .items
        .iter()
        .zip(&sized)
        .map(|(item, s)| {
            row_contribution(item, &s.measured, &rows, &row_gutters, &s.edges, &s.margin)
        })
        .collect();
    let row_space = |available, gutters: &[f32]| Space {
        available,
        min: min_height,
        max: max_height,
        gutters: gutters.to_vec(),
        stretch: stretches_tracks(style.align_content),
    };
    let available = definite.map_or(Available::MaxContent, Available::Definite);
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
    let row_lines = Lines::new(rows.iter().map(Track::size).collect(), &row_gutters, 0.0);

    // §12 step 5: each item laid out in its area, stretched where it
    // stretches, at the start otherwise.
    let content_x = border[LEFT] + padding[LEFT];
    let content_y = border[TOP] + padding[TOP];
    let height = content_height + vertical_edges;
    let mut root = box_fragment(content_width + edges.horizontal(), height, b);
    let mut carried_up: Vec<Vec<Deferred<'a>>> = b.children.iter().map(|_| Vec::new()).collect();
    let mut content_bottom = 0.0f32;
    for (item, s) in grid.items.iter().zip(sized) {
        let item_style = &item.b.style;
        let (x, width) = column_lines.area(&item.columns);
        let (y, area_height) = row_lines.area(&item.rows);
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
                );
                height = height.max(automatic);
            }
            Imposed {
                height: height + vertical,
                definite: true,
            }
        });
        let key = ItemKey::of(item.b, s.content_width, imposed, width, Some(area_height));
        let layout = pass.item(key, || {
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
        });
        let (dx, dy) = relative_offset(item_style, width, Some(area_height));
        let left = content_x + x + margin[LEFT];
        let top = content_y + y + margin[TOP];
        content_bottom = content_bottom.max(y + margin[TOP] + layout.height + margin[BOTTOM]);
        carried_up[item.child] = carried(layout.absolutes, left + dx, top + dy);
        root.children
            .push(layout.root.translated(left + dx, top + dy));
    }

    // An absolutely positioned child is no item; until the grid aligns
    // it (M18 step 7), its static position is the content box's start.
    let mut absolutes = Vec::new();
    for (child, carried) in b.children.iter().zip(carried_up) {
        if child.kind == BoxKind::Absolute {
            absolutes.push(Deferred::at(child, content_x, content_y));
        } else {
            absolutes.extend(carried);
        }
    }
    let absolutes = settle_absolutes(b, &mut root, border, absolutes, pass);
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
        first_baseline: None,
        last_baseline: None,
        inline_block_baseline: None,
        absolutes,
        auto_content_height,
        held: Vec::new(),
    }
}

/// The grid container `b`'s min-content and max-content content widths
/// (§5.2): its columns and gaps sized under a min-content and a
/// max-content constraint, percentages of its width counting as `auto`
/// and percentage gaps as zero, since that width is what is being found
/// (§7.2.1, css-align-3 §8.1). An automatic repetition repeats in its
/// own definite width, else its maximum, else its minimum, else once.
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let style = &b.style;
    let (non_content, _) = crate::intrinsic::fixed_edges(b);
    let px = |size: Size| match size {
        Size::Px(px) => Some(inner(style, px, non_content)),
        _ => None,
    };
    let max = match style.max_width {
        MaxSize::Px(px) => Some(inner(style, px, non_content)),
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
            room: room(px(style.width), max, px(style.min_width)),
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
    let items: Vec<Contribution> = grid
        .items
        .iter()
        .map(|item| {
            let automatic = automatic_minimum_width(item, &columns, &gutters, pass);
            column_contribution(item, automatic, pass)
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
    fn a_scrolling_grid_scrolls_to_the_end_of_its_tracks() {
        let l = laid(
            "<div id=g><div id=a style='height: 10px'></div></div>",
            "#g { display: grid; height: 100px; overflow: hidden; grid-template-rows: 300px }
             #a { align-self: start }",
        );
        assert_eq!(fragment(&l, "g").scroll_height, 300.0);
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
