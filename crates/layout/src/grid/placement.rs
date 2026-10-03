//! Grid item placement (css-grid-2 §7.1, §7.2.3, §8, M18): the explicit
//! grid's tracks and line names, each item's grid-placement properties
//! resolved to lines or spans (§8.3, §8.3.1), and the auto-placement
//! algorithm of §8.5, sparse or dense, by row or by column, which grows
//! the implicit grid. Lines are clamped to [`MIN_LINE`] and [`MAX_LINE`]
//! (§5.4).
//!
//! Positions here are line coordinates: the explicit grid's first line,
//! line 1 of the properties, is 0, and its last is the number of
//! explicit tracks; the implicit grid adds lines before and after. The
//! result shifts them so that the implicit grid starts at 0.
//!
//! Which cells are taken is kept per row (per column for `grid-auto-flow:
//! column`) as a sorted list of the ranges in use, not as a table of
//! every cell: the limit allows 20,000 lines in each axis
//! (docs/design/grid.md, "Placement").

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use style::computed::{
    GridAutoFlow, GridLine, LineNames, NamedArea, RepeatCount, TemplateAreas, TrackList,
    TrackSection, TrackSize,
};

/// The first line coordinate the grid may have (§5.4): line -10,000.
pub(crate) const MIN_LINE: i32 = -10_000;
/// The last line coordinate the grid may have (§5.4).
pub(crate) const MAX_LINE: i32 = 10_000;
/// The most tracks the explicit grid has in one axis, however many a
/// track list's repetitions ask for.
pub(crate) const MAX_EXPLICIT_TRACKS: usize = MAX_LINE as usize;

/// Which of a template's axes: the rows or the columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Rows,
    Columns,
}

impl Axis {
    /// The number of tracks the template areas make in this axis, and
    /// the lines an area's edges are at.
    fn of_areas(self, areas: &TemplateAreas) -> u32 {
        match self {
            Axis::Rows => areas.rows,
            Axis::Columns => areas.columns,
        }
    }

    fn of_area(self, area: &NamedArea) -> (u32, u32) {
        match self {
            Axis::Rows => area.rows,
            Axis::Columns => area.columns,
        }
    }
}

/// One axis of the explicit grid (§7.1): its sized tracks, how many
/// tracks it has with the ones `grid-template-areas` adds, the
/// implicit tracks' sizes, and its lines' names.
#[derive(Debug, Clone)]
pub(crate) struct ExplicitAxis {
    /// The tracks the track list sizes, its repetitions repeated.
    sized: Vec<TrackSize>,
    /// How many explicit tracks there are: the sized ones, or the
    /// template's areas' if those are more.
    len: i32,
    /// `grid-auto-rows` or `grid-auto-columns`, never empty: the sizes
    /// of the tracks the track list does not size.
    auto: Vec<TrackSize>,
    /// The tracks an `auto-fit` repetition made (§7.2.3.2), which
    /// collapse when no item occupies them.
    auto_fit: Option<Range<i32>>,
    /// Each name and the explicit lines that carry it, ascending.
    names: HashMap<Arc<str>, Vec<i32>>,
}

impl ExplicitAxis {
    /// The axis `template` (`None` for `none`) and `areas` make, the
    /// track list's automatic repetition repeated `repetitions` times;
    /// `auto` sizes the tracks it does not size. Never more than
    /// [`MAX_EXPLICIT_TRACKS`] tracks.
    pub(crate) fn new(
        template: Option<&TrackList>,
        auto: &[TrackSize],
        areas: Option<&TemplateAreas>,
        axis: Axis,
        repetitions: usize,
    ) -> Self {
        let mut sized = Vec::new();
        let mut line_names: Vec<LineNames> = vec![Vec::new()];
        let mut auto_fit = None;
        // Each track ends a line and starts the next, whose names the
        // sections after it add to.
        let add_track =
            |size: TrackSize, sized: &mut Vec<TrackSize>, names: &mut Vec<LineNames>| {
                sized.push(size);
                names.push(Vec::new());
            };
        if let Some(list) = template {
            'sections: for (section, before) in list.sections.iter().zip(&list.names) {
                line_names
                    .last_mut()
                    .expect("never empty")
                    .extend(before.iter().cloned());
                match section {
                    TrackSection::Track(size) => {
                        if sized.len() == MAX_EXPLICIT_TRACKS {
                            break 'sections;
                        }
                        add_track(*size, &mut sized, &mut line_names);
                    }
                    TrackSection::Repeat(repeat) => {
                        let count = match repeat.count {
                            RepeatCount::Count(n) => n as usize,
                            RepeatCount::AutoFill | RepeatCount::AutoFit => repetitions.max(1),
                        };
                        let first = sized.len();
                        for _ in 0..count {
                            for (size, names) in repeat.tracks.iter().zip(&repeat.names) {
                                if sized.len() == MAX_EXPLICIT_TRACKS {
                                    break 'sections;
                                }
                                line_names
                                    .last_mut()
                                    .expect("never empty")
                                    .extend(names.iter().cloned());
                                add_track(*size, &mut sized, &mut line_names);
                            }
                            let after = repeat.names.last().expect("one more than the tracks");
                            line_names
                                .last_mut()
                                .expect("never empty")
                                .extend(after.iter().cloned());
                        }
                        if repeat.count == RepeatCount::AutoFit {
                            auto_fit = Some(first as i32..sized.len() as i32);
                        }
                    }
                }
            }
            let after = list.names.last().expect("one more than the sections");
            line_names
                .last_mut()
                .expect("never empty")
                .extend(after.iter().cloned());
        }
        let mut names: HashMap<Arc<str>, Vec<i32>> = HashMap::new();
        for (line, line_names) in line_names.iter().enumerate() {
            for name in line_names {
                names.entry(name.clone()).or_default().push(line as i32);
            }
        }
        // §7.3.2: an area `foo` names its edges `foo-start` and `foo-end`.
        let mut len = sized.len() as i32;
        if let Some(areas) = areas {
            let extent = axis.of_areas(areas).min(MAX_EXPLICIT_TRACKS as u32);
            len = len.max(extent as i32);
            for area in &areas.areas {
                let (start, end) = axis.of_area(area);
                for (suffix, line) in [("-start", start), ("-end", end)] {
                    let line = line.min(MAX_EXPLICIT_TRACKS as u32) as i32;
                    let name: Arc<str> = Arc::from(format!("{}{suffix}", area.name));
                    names.entry(name).or_default().push(line);
                }
            }
            for lines in names.values_mut() {
                lines.sort_unstable();
                lines.dedup();
            }
        }
        ExplicitAxis {
            sized,
            len,
            auto: if auto.is_empty() {
                vec![TrackSize::AUTO]
            } else {
                auto.to_vec()
            },
            auto_fit,
            names,
        }
    }

    /// How many explicit tracks the axis has.
    pub(crate) fn len(&self) -> i32 {
        self.len
    }

    /// The tracks an `auto-fit` repetition made, by line coordinate.
    pub(crate) fn auto_fit(&self) -> Option<Range<i32>> {
        self.auto_fit.clone()
    }

    /// The size of the track that starts at line coordinate `start`:
    /// the track list's, or else `grid-auto-rows` or
    /// `grid-auto-columns`, a pattern whose first size goes to the first
    /// track after the sized ones and whose last to the last implicit
    /// track before the explicit grid (§7.6).
    pub(crate) fn size(&self, start: i32) -> TrackSize {
        let n = self.auto.len() as i64;
        let sized = self.sized.len() as i64;
        let start = i64::from(start);
        if start >= 0 && start < sized {
            return self.sized[start as usize];
        }
        let index = if start >= sized {
            (start - sized) % n
        } else {
            n - 1 - ((-start - 1) % n)
        };
        self.auto[index as usize]
    }

    /// The explicit lines called `name`, ascending.
    fn lines(&self, name: &str) -> &[i32] {
        self.names.get(name).map_or(&[], Vec::as_slice)
    }

    /// The line `<integer> && <custom-ident>?` gives (§8.3): the `n`th
    /// line from the start, or from the end when `n` is negative,
    /// counting only lines called `name` if there is one, and every
    /// implicit line past the explicit grid as called anything.
    fn nth(&self, n: i32, name: Option<&str>) -> i64 {
        let len = i64::from(self.len);
        let n = i64::from(n);
        let Some(name) = name else {
            return if n > 0 { n - 1 } else { len + 1 + n };
        };
        let lines = self.lines(name);
        let count = lines.len() as i64;
        if n > 0 {
            if n <= count {
                i64::from(lines[(n - 1) as usize])
            } else {
                len + (n - count)
            }
        } else if -n <= count {
            i64::from(lines[(count + n) as usize])
        } else {
            -(-n - count)
        }
    }

    /// The line a bare `<custom-ident>` gives at a start (`side` being
    /// `"-start"`) or an end (`"-end"`): the first line called
    /// `name-start` or `name-end`, else the first called `name` (§8.3).
    fn named(&self, name: &str, side: &str) -> i64 {
        match self.lines(&format!("{name}{side}")).first() {
            Some(&line) => i64::from(line),
            None => self.nth(1, Some(name)),
        }
    }

    /// The `n`th line called `name` after the line `from`, every
    /// implicit line after the explicit grid counting as one (§8.3).
    fn forward(&self, from: i64, n: u32, name: &str) -> i64 {
        let n = i64::from(n);
        let after: Vec<i64> = self
            .lines(name)
            .iter()
            .map(|&l| i64::from(l))
            .filter(|&l| l > from)
            .collect();
        let count = after.len() as i64;
        if n <= count {
            after[(n - 1) as usize]
        } else {
            from.max(i64::from(self.len)) + (n - count)
        }
    }

    /// The `n`th line called `name` before the line `from`, every
    /// implicit line before the explicit grid counting as one.
    fn backward(&self, from: i64, n: u32, name: &str) -> i64 {
        let n = i64::from(n);
        let before: Vec<i64> = self
            .lines(name)
            .iter()
            .rev()
            .map(|&l| i64::from(l))
            .filter(|&l| l < from)
            .collect();
        let count = before.len() as i64;
        if n <= count {
            before[(n - 1) as usize]
        } else {
            from.min(0) - (n - count)
        }
    }
}

/// The room an axis gives an automatic repetition (§7.2.3.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Room {
    /// The container's size or maximum size is definite: as many
    /// repetitions as fit in this content-box size.
    AtMost(f32),
    /// Only its minimum size is definite: the fewest repetitions that
    /// fill this content-box size.
    AtLeast(f32),
    /// Neither: one repetition.
    Unknown,
}

/// How many times the automatic repetition in `list` repeats in `room`,
/// with `gap` between tracks, percentages being of `percent_of` when it
/// is known (§7.2.3.2): at least once; once when a size it needs is a
/// percentage of an unknown size; and never so many that the explicit
/// grid would pass [`MAX_EXPLICIT_TRACKS`]. Each track counts as its
/// maximum sizing function when that is fixed, else its minimum, a fixed
/// minimum flooring a fixed maximum, and never as less than a pixel.
pub(crate) fn repetitions(
    list: &TrackList,
    room: Room,
    gap: f32,
    percent_of: Option<f32>,
) -> usize {
    use style::computed::{LengthPercentage, TrackBreadth};
    let Some(automatic) = list.auto_repeat() else {
        return 1;
    };
    let (at_most, room) = match room {
        Room::AtMost(room) => (true, room),
        Room::AtLeast(room) => (false, room),
        Room::Unknown => return 1,
    };
    let fixed = |breadth: TrackBreadth| match breadth {
        TrackBreadth::Fixed(LengthPercentage::Px(px)) => Some(Some(px)),
        TrackBreadth::Fixed(LengthPercentage::Percent(p)) => {
            Some(percent_of.map(|of| of * p / 100.0))
        }
        _ => None,
    };
    // A track's size for the count; `None` when a percentage has nothing
    // to resolve against.
    let size = |track: &TrackSize| -> Option<f64> {
        let (min, max) = match *track {
            TrackSize::Breadth(b) => (fixed(b), fixed(b)),
            TrackSize::MinMax(min, max) => (fixed(min), fixed(max)),
            TrackSize::FitContent(_) => (None, None),
        };
        let size = match (min, max) {
            (Some(min), Some(max)) => Some(max?.max(min?)),
            (_, Some(max)) => max,
            (Some(min), None) => min,
            (None, None) => Some(0.0),
        }?;
        Some(f64::from(size.max(1.0)))
    };
    let (mut fixed_size, mut fixed_tracks) = (0.0f64, 0usize);
    for section in &list.sections {
        match section {
            TrackSection::Track(track) => {
                let Some(track) = size(track) else { return 1 };
                fixed_size += track;
                fixed_tracks += 1;
            }
            TrackSection::Repeat(repeat) if !repeat.count.is_automatic() => {
                let RepeatCount::Count(n) = repeat.count else {
                    unreachable!()
                };
                let n = (n as usize).min(MAX_EXPLICIT_TRACKS);
                for track in &repeat.tracks {
                    let Some(track) = size(track) else { return 1 };
                    fixed_size += track * n as f64;
                    fixed_tracks = fixed_tracks.saturating_add(n);
                }
            }
            TrackSection::Repeat(_) => {}
        }
    }
    let mut repeat_size = 0.0f64;
    for track in &automatic.tracks {
        let Some(track) = size(track) else { return 1 };
        repeat_size += track;
    }
    let per = automatic.tracks.len();
    let gap = f64::from(gap.max(0.0));
    // With `n` repetitions the grid is `fixed_size + n * repeat_size`
    // wide, with a gap between each two of its `fixed_tracks + n * per`
    // tracks.
    let base = fixed_size + gap * (fixed_tracks as f64 - 1.0);
    let step = repeat_size + gap * per as f64;
    // `step` is positive, every track being at least a pixel. A sixty-
    // fourth of a pixel's tolerance keeps a count that fits exactly from
    // losing a repetition to rounding.
    let exact = (f64::from(room) - base) / step;
    let n = if at_most {
        (exact + 1.0 / 64.0 / step).floor()
    } else {
        (exact - 1.0 / 64.0 / step).ceil()
    };
    let most = MAX_EXPLICIT_TRACKS.saturating_sub(fixed_tracks) / per.max(1);
    if n.is_nan() || n < 1.0 {
        1
    } else {
        (n.min(most as f64) as usize).max(1)
    }
}

/// An item's placement in one axis once its properties are resolved: a
/// definite start and end line, or automatic with a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AxisPlacement {
    Definite(i32, i32),
    Auto(i32),
}

impl AxisPlacement {
    fn span(self) -> i32 {
        match self {
            AxisPlacement::Definite(start, end) => end - start,
            AxisPlacement::Auto(span) => span,
        }
    }

    fn definite(self) -> Option<Range<i32>> {
        match self {
            AxisPlacement::Definite(start, end) => Some(start..end),
            AxisPlacement::Auto(_) => None,
        }
    }
}

/// The most tracks a span may cross: the whole limited grid.
const MAX_SPAN: i64 = (MAX_LINE - MIN_LINE) as i64;

/// A definite area's lines clamped to the limited grid (§5.4): an area
/// reaching past the limit is cut at it, and one wholly outside moves to
/// the last track on its side with a span of one.
fn clamp_area(start: i64, end: i64) -> AxisPlacement {
    let (min, max) = (i64::from(MIN_LINE), i64::from(MAX_LINE));
    let (start, end) = if start >= max {
        (max - 1, max)
    } else if end <= min {
        (min, min + 1)
    } else {
        (start.max(min), end.min(max))
    };
    AxisPlacement::Definite(start as i32, end as i32)
}

/// An item's `*-start` and `*-end` in one axis resolved against the
/// explicit grid `axis` (§8.3, §8.3.1): two lines, swapped if reversed
/// and a span of one if equal; a line and a span, a named span searching
/// from the line; a line alone with a span of one; or no line, automatic,
/// with the start's span, the end's if the start has none, a named span
/// alone being a span of one.
pub(crate) fn resolve(start: &GridLine, end: &GridLine, axis: &ExplicitAxis) -> AxisPlacement {
    let line = |g: &GridLine, side: &str| match g {
        GridLine::Ident(name) => Some(axis.named(name, side)),
        GridLine::Line(n, name) => Some(axis.nth(*n, name.as_deref())),
        GridLine::Auto | GridLine::Span(..) => None,
    };
    let span = |n: u32| i64::from(n).min(MAX_SPAN);
    match (line(start, "-start"), line(end, "-end")) {
        (Some(s), Some(e)) if s == e => clamp_area(s, s + 1),
        (Some(s), Some(e)) => clamp_area(s.min(e), s.max(e)),
        (Some(s), None) => match end {
            GridLine::Span(n, None) => clamp_area(s, s + span(*n)),
            GridLine::Span(n, Some(name)) => clamp_area(s, axis.forward(s, *n, name)),
            _ => clamp_area(s, s + 1),
        },
        (None, Some(e)) => match start {
            GridLine::Span(n, None) => clamp_area(e - span(*n), e),
            GridLine::Span(n, Some(name)) => clamp_area(axis.backward(e, *n, name), e),
            _ => clamp_area(e - 1, e),
        },
        (None, None) => {
            let n = match (start, end) {
                (GridLine::Span(n, None), _) => span(*n),
                (GridLine::Span(_, Some(_)), _) => 1,
                (_, GridLine::Span(n, None)) => span(*n),
                _ => 1,
            };
            AxisPlacement::Auto(n as i32)
        }
    }
}

/// How many rows an area may cross and still be kept in each of them;
/// a taller one is kept once, so that a page cannot make every item cost
/// thousands of entries.
const TALL: i32 = 64;

/// The cells in use, by row of the axis auto-placement fills (the
/// columns with `grid-auto-flow: column`): for each, the ranges of the
/// other axis taken, ascending; and the areas too tall for that.
struct Occupancy {
    /// The line coordinate of `rows[0]`.
    first: i32,
    rows: Vec<Vec<Range<i32>>>,
    tall: Vec<(Range<i32>, Range<i32>)>,
}

impl Occupancy {
    /// Whether the area `major` by `minor` is free; when it is not, the
    /// end of the furthest taken range it meets, where a search along a
    /// row may go on from.
    fn fits(&self, major: &Range<i32>, minor: &Range<i32>) -> Result<(), i32> {
        let mut blocked: Option<i32> = None;
        let mut block = |end: i32| blocked = Some(blocked.map_or(end, |b| b.max(end)));
        for (rows, columns) in &self.tall {
            if rows.start < major.end
                && major.start < rows.end
                && columns.start < minor.end
                && minor.start < columns.end
            {
                block(columns.end);
            }
        }
        for row in major.clone() {
            let Some(taken) = usize::try_from(row - self.first)
                .ok()
                .and_then(|i| self.rows.get(i))
            else {
                continue;
            };
            for range in taken {
                if range.start >= minor.end {
                    break;
                }
                if range.end > minor.start {
                    block(range.end);
                }
            }
        }
        blocked.map_or(Ok(()), Err)
    }

    /// Marks the area `major` by `minor` taken.
    fn take(&mut self, major: &Range<i32>, minor: &Range<i32>) {
        if major.len() > TALL as usize {
            self.tall.push((major.clone(), minor.clone()));
            return;
        }
        for row in major.clone() {
            let index = (row - self.first) as usize;
            if self.rows.len() <= index {
                self.rows.resize_with(index + 1, Vec::new);
            }
            let taken = &mut self.rows[index];
            let at = taken.partition_point(|r| r.start < minor.start);
            taken.insert(at, minor.clone());
        }
    }
}

/// Where every item went, and the implicit grid's size (§8.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    /// How many rows and columns the implicit grid has.
    pub(crate) rows: i32,
    pub(crate) columns: i32,
    /// The line coordinate of the implicit grid's first row and column
    /// lines: zero or negative, the explicit grid's first line being 0.
    pub(crate) first_row: i32,
    pub(crate) first_column: i32,
    /// Each item's rows and columns, from 0 at the implicit grid's first
    /// line, in the order the items were given.
    pub(crate) areas: Vec<(Range<i32>, Range<i32>)>,
}

/// Places the items, in order-modified document order, each with its
/// rows' and columns' placement as [`resolve`] gives them, in a grid
/// whose explicit part has `explicit_rows` rows and `explicit_columns`
/// columns, by §8.5's algorithm in the direction and with the packing
/// `flow` says. The implicit grid grows to hold them, never past the
/// limit; an item that does not fit inside it is put at its far edge,
/// over whatever is there.
pub(crate) fn place(
    items: &[(AxisPlacement, AxisPlacement)],
    explicit_rows: i32,
    explicit_columns: i32,
    flow: GridAutoFlow,
) -> Placement {
    // The algorithm runs in the flow's axes: it fills the minor axis
    // along each line of the major one, adding major tracks as it needs.
    let (explicit_major, explicit_minor) = if flow.column {
        (explicit_columns, explicit_rows)
    } else {
        (explicit_rows, explicit_columns)
    };
    let axes: Vec<(AxisPlacement, AxisPlacement)> = items
        .iter()
        .map(|&(rows, columns)| {
            if flow.column {
                (columns, rows)
            } else {
                (rows, columns)
            }
        })
        .collect();

    // The implicit grid's extent before auto-placement: the explicit
    // grid and every definite position.
    let mut major_min = 0;
    let mut major_max = explicit_major;
    let mut minor_min = 0;
    let mut minor_max = explicit_minor;
    for (major, minor) in &axes {
        if let Some(r) = major.definite() {
            major_min = major_min.min(r.start);
            major_max = major_max.max(r.end);
        }
        if let Some(r) = minor.definite() {
            minor_min = minor_min.min(r.start);
            minor_max = minor_max.max(r.end);
        }
    }
    let mut taken = Occupancy {
        first: major_min,
        rows: Vec::new(),
        tall: Vec::new(),
    };
    let mut areas: Vec<Option<(Range<i32>, Range<i32>)>> = vec![None; axes.len()];

    // 1. Items with a definite position in both axes.
    for (i, (major, minor)) in axes.iter().enumerate() {
        if let (Some(major), Some(minor)) = (major.definite(), minor.definite()) {
            taken.take(&major, &minor);
            areas[i] = Some((major, minor));
        }
    }

    // 2. Items locked to a major line: the first minor line, from the
    // explicit grid's first, where they overlap nothing; when sparse,
    // past the items this step placed on the same line before them.
    let mut cursors: HashMap<i32, i32> = HashMap::new();
    for (i, (major, minor)) in axes.iter().enumerate() {
        let (Some(major), AxisPlacement::Auto(span)) = (major.definite(), *minor) else {
            continue;
        };
        let mut at = if flow.dense {
            0
        } else {
            cursors.get(&major.start).copied().unwrap_or(0)
        };
        loop {
            if at + span > MAX_LINE {
                at = (MAX_LINE - span).max(0);
                break;
            }
            match taken.fits(&major, &(at..at + span)) {
                Ok(()) => break,
                Err(end) => at = end.max(at + 1),
            }
        }
        let minor = at..at + span;
        taken.take(&major, &minor);
        cursors.insert(major.start, minor.end);
        minor_max = minor_max.max(minor.end);
        areas[i] = Some((major, minor));
    }

    // 3. The minor axis is now as large as it will be: wide enough for
    // the widest automatic span left too.
    let widest = axes
        .iter()
        .zip(&areas)
        .filter(|(_, area)| area.is_none())
        .map(|((_, minor), _)| minor.span())
        .max()
        .unwrap_or(0);
    minor_max = minor_max.max(minor_min + widest);

    // 4. The rest, by the auto-placement cursor.
    let (mut cursor_major, mut cursor_minor) = (major_min, minor_min);
    for (i, (major, minor)) in axes.iter().enumerate() {
        if areas[i].is_some() {
            continue;
        }
        let major_span = major.span();
        let fits_below_limit = |row: i32| row + major_span <= MAX_LINE;
        let (row, minor) = match minor.definite() {
            Some(minor) => {
                if flow.dense {
                    cursor_major = major_min;
                } else if minor.start < cursor_minor {
                    cursor_major += 1;
                }
                cursor_minor = minor.start;
                while fits_below_limit(cursor_major)
                    && taken
                        .fits(&(cursor_major..cursor_major + major_span), &minor)
                        .is_err()
                {
                    cursor_major += 1;
                }
                (
                    cursor_major.min(MAX_LINE - major_span).max(major_min),
                    minor,
                )
            }
            None => {
                let span = minor.span();
                if flow.dense {
                    (cursor_major, cursor_minor) = (major_min, minor_min);
                }
                let found = loop {
                    if !fits_below_limit(cursor_major) {
                        break (
                            (MAX_LINE - major_span).max(major_min),
                            minor_min..minor_min + span,
                        );
                    }
                    let rows = cursor_major..cursor_major + major_span;
                    let mut at = cursor_minor;
                    let mut found = None;
                    while at + span <= minor_max {
                        match taken.fits(&rows, &(at..at + span)) {
                            Ok(()) => {
                                found = Some(at);
                                break;
                            }
                            Err(end) => at = end.max(at + 1),
                        }
                    }
                    match found {
                        Some(at) => break (cursor_major, at..at + span),
                        None => {
                            cursor_major += 1;
                            cursor_minor = minor_min;
                        }
                    }
                };
                cursor_minor = found.1.start;
                found
            }
        };
        let major = row..row + major_span;
        taken.take(&major, &minor);
        major_max = major_max.max(major.end);
        areas[i] = Some((major, minor));
    }

    let shift = |r: Range<i32>, by: i32| r.start - by..r.end - by;
    let areas = areas
        .into_iter()
        .map(|area| {
            let (major, minor) = area.expect("every item placed");
            let (major, minor) = (shift(major, major_min), shift(minor, minor_min));
            if flow.column {
                (minor, major)
            } else {
                (major, minor)
            }
        })
        .collect();
    let (major_len, minor_len) = (major_max - major_min, minor_max - minor_min);
    if flow.column {
        Placement {
            rows: minor_len,
            columns: major_len,
            first_row: minor_min,
            first_column: major_min,
            areas,
        }
    } else {
        Placement {
            rows: major_len,
            columns: minor_len,
            first_row: major_min,
            first_column: minor_min,
            areas,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use style::computed::{LengthPercentage, TrackBreadth, TrackRepeat};

    fn px(v: f32) -> TrackSize {
        TrackSize::Breadth(TrackBreadth::Fixed(LengthPercentage::Px(v)))
    }

    fn names(names: &[&str]) -> LineNames {
        names.iter().map(|&n| Arc::from(n)).collect()
    }

    /// A track list of `n` one-pixel tracks, the lines named as `lines`
    /// says, one entry a line.
    fn list(lines: &[&[&str]]) -> TrackList {
        TrackList {
            names: lines.iter().map(|l| names(l)).collect(),
            sections: (1..lines.len())
                .map(|_| TrackSection::Track(px(1.0)))
                .collect(),
        }
    }

    fn axis(list: &TrackList) -> ExplicitAxis {
        ExplicitAxis::new(Some(list), &[TrackSize::AUTO], None, Axis::Columns, 1)
    }

    fn ident(name: &str) -> GridLine {
        GridLine::Ident(Arc::from(name))
    }

    fn line(n: i32, name: Option<&str>) -> GridLine {
        GridLine::Line(n, name.map(Arc::from))
    }

    fn span(n: u32, name: Option<&str>) -> GridLine {
        GridLine::Span(n, name.map(Arc::from))
    }

    #[test]
    fn a_repetitions_line_names_merge_with_their_neighbours() {
        // §7.2.3: `repeat(2, [b] 1fr [c])` is `[b] 1fr [c b] 1fr [c]`.
        let list = TrackList {
            names: vec![names(&["a"]), names(&["d"]), vec![]],
            sections: vec![
                TrackSection::Repeat(TrackRepeat {
                    count: RepeatCount::Count(2),
                    names: vec![names(&["b"]), names(&["c"])],
                    tracks: vec![px(1.0)],
                }),
                TrackSection::Track(px(2.0)),
            ],
        };
        let axis = axis(&list);
        assert_eq!(axis.len(), 3);
        assert_eq!(axis.lines("a"), &[0]);
        assert_eq!(axis.lines("b"), &[0, 1]);
        assert_eq!(axis.lines("c"), &[1, 2]);
        assert_eq!(axis.lines("d"), &[2]);
        assert_eq!(axis.size(2), px(2.0));
    }

    #[test]
    fn an_automatic_repetition_repeats_as_told_and_auto_fit_marks_its_tracks() {
        let list = TrackList {
            names: vec![vec![], vec![], vec![]],
            sections: vec![
                TrackSection::Track(px(5.0)),
                TrackSection::Repeat(TrackRepeat {
                    count: RepeatCount::AutoFit,
                    names: vec![names(&["x"]), vec![]],
                    tracks: vec![px(1.0)],
                }),
            ],
        };
        let axis = ExplicitAxis::new(Some(&list), &[TrackSize::AUTO], None, Axis::Rows, 3);
        assert_eq!(axis.len(), 4);
        assert_eq!(axis.auto_fit(), Some(1..4));
        assert_eq!(axis.lines("x"), &[1, 2, 3]);
        // Never fewer than one repetition.
        let axis = ExplicitAxis::new(Some(&list), &[TrackSize::AUTO], None, Axis::Rows, 0);
        assert_eq!(axis.len(), 2);
    }

    #[test]
    fn a_huge_repetition_stops_at_the_limit() {
        let list = TrackList {
            names: vec![vec![], vec![]],
            sections: vec![TrackSection::Repeat(TrackRepeat {
                count: RepeatCount::Count(u32::MAX),
                names: vec![vec![], vec![]],
                tracks: vec![px(1.0), px(2.0)],
            })],
        };
        assert_eq!(axis(&list).len(), MAX_EXPLICIT_TRACKS as i32);
    }

    /// A track list of `before`, then `repeat(auto-fill, tracks)`, then
    /// `after`.
    fn auto_fill(before: &[TrackSize], tracks: &[TrackSize], after: &[TrackSize]) -> TrackList {
        let mut sections: Vec<TrackSection> =
            before.iter().map(|&t| TrackSection::Track(t)).collect();
        sections.push(TrackSection::Repeat(TrackRepeat {
            count: RepeatCount::AutoFill,
            names: vec![vec![]; tracks.len() + 1],
            tracks: tracks.to_vec(),
        }));
        sections.extend(after.iter().map(|&t| TrackSection::Track(t)));
        TrackList {
            names: vec![vec![]; sections.len() + 1],
            sections,
        }
    }

    #[test]
    fn auto_fill_repeats_as_often_as_the_room_allows() {
        use style::computed::TrackBreadth as B;
        let fr = B::Flex(1.0);
        let pct = |p| B::Fixed(LengthPercentage::Percent(p));
        let px_b = |v| B::Fixed(LengthPercentage::Px(v));
        let hundred = auto_fill(&[], &[px(100.0)], &[]);
        assert_eq!(repetitions(&hundred, Room::AtMost(350.0), 0.0, None), 3);
        assert_eq!(
            repetitions(&hundred, Room::AtMost(300.0), 0.0, None),
            3,
            "exactly"
        );
        assert_eq!(repetitions(&hundred, Room::AtMost(320.0), 10.0, None), 3);
        assert_eq!(repetitions(&hundred, Room::AtMost(319.0), 10.0, None), 2);
        assert_eq!(
            repetitions(&hundred, Room::AtMost(50.0), 0.0, None),
            1,
            "at least one"
        );
        assert_eq!(repetitions(&hundred, Room::AtLeast(250.0), 0.0, None), 3);
        assert_eq!(repetitions(&hundred, Room::AtLeast(200.0), 0.0, None), 2);
        assert_eq!(repetitions(&hundred, Room::Unknown, 0.0, None), 1);
        // The other tracks take their room first.
        let framed = auto_fill(&[px(50.0)], &[px(100.0)], &[px(50.0)]);
        assert_eq!(repetitions(&framed, Room::AtMost(400.0), 0.0, None), 3);
        // A flexible maximum counts as the fixed minimum: the reference
        // browser's 30px columns three to a 100px grid with 5px gaps.
        let flexible = auto_fill(&[], &[TrackSize::MinMax(px_b(30.0), fr)], &[]);
        assert_eq!(repetitions(&flexible, Room::AtMost(100.0), 5.0, None), 3);
        // A fixed minimum floors a fixed maximum.
        let floored = auto_fill(&[], &[TrackSize::MinMax(px_b(50.0), px_b(20.0))], &[]);
        assert_eq!(repetitions(&floored, Room::AtMost(100.0), 0.0, None), 2);
        // Percentages need something to be of.
        let quarter = auto_fill(&[], &[TrackSize::Breadth(pct(25.0))], &[]);
        assert_eq!(
            repetitions(&quarter, Room::AtMost(400.0), 0.0, Some(400.0)),
            4
        );
        assert_eq!(repetitions(&quarter, Room::AtMost(400.0), 0.0, None), 1);
        // A track is never less than a pixel, and the grid never more than
        // the limit.
        let nothing = auto_fill(&[], &[px(0.0)], &[]);
        assert_eq!(repetitions(&nothing, Room::AtMost(5.0), 0.0, None), 5);
        assert_eq!(
            repetitions(&nothing, Room::AtMost(1e9), 0.0, None),
            MAX_EXPLICIT_TRACKS
        );
        let two = auto_fill(&[px(1.0)], &[px(0.0), px(0.0)], &[]);
        assert_eq!(
            repetitions(&two, Room::AtMost(1e9), 0.0, None),
            (MAX_EXPLICIT_TRACKS - 1) / 2
        );
    }

    #[test]
    fn the_implicit_tracks_take_the_auto_pattern_forwards_and_backwards() {
        // §7.6: the first after the sized tracks takes the first size, the
        // last before the explicit grid the last.
        let list = list(&[&[], &[]]);
        let axis = ExplicitAxis::new(Some(&list), &[px(1.0), px(2.0)], None, Axis::Rows, 1);
        let sizes: Vec<TrackSize> = (-3..4).map(|t| axis.size(t)).collect();
        assert_eq!(
            sizes,
            vec![
                px(2.0),
                px(1.0),
                px(2.0),
                px(1.0),
                px(1.0),
                px(2.0),
                px(1.0)
            ]
        );
    }

    #[test]
    fn template_areas_add_tracks_and_name_their_edges() {
        let areas = TemplateAreas {
            rows: 2,
            columns: 3,
            areas: vec![
                NamedArea {
                    name: Arc::from("a"),
                    rows: (0, 1),
                    columns: (0, 3),
                },
                NamedArea {
                    name: Arc::from("b"),
                    rows: (1, 2),
                    columns: (1, 2),
                },
            ],
        };
        let template = list(&[&[], &["x"], &[]]);
        let columns =
            ExplicitAxis::new(Some(&template), &[px(9.0)], Some(&areas), Axis::Columns, 1);
        assert_eq!(columns.len(), 3, "the areas' third column");
        assert_eq!(
            columns.size(2),
            px(9.0),
            "an area's track takes the auto size"
        );
        assert_eq!(columns.lines("a-start"), &[0]);
        assert_eq!(columns.lines("a-end"), &[3]);
        assert_eq!(columns.lines("b-start"), &[1]);
        let rows = ExplicitAxis::new(None, &[TrackSize::AUTO], Some(&areas), Axis::Rows, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.lines("b-end"), &[2]);
        // A bare name is the area's edge in both axes.
        assert_eq!(
            resolve(&ident("b"), &ident("b"), &rows),
            AxisPlacement::Definite(1, 2)
        );
        assert_eq!(
            resolve(&ident("a"), &ident("a"), &columns),
            AxisPlacement::Definite(0, 3)
        );
    }

    #[test]
    fn lines_resolve_as_section_8_3_says() {
        // §8.3's example: one row of eight columns, the nine lines named
        // A B C A B C A B C. Line n is coordinate n - 1.
        let abc: [&[&str]; 9] = [
            &["A"],
            &["B"],
            &["C"],
            &["A"],
            &["B"],
            &["C"],
            &["A"],
            &["B"],
            &["C"],
        ];
        let axis = axis(&list(&abc));
        let d = AxisPlacement::Definite;
        for (start, end, expected) in [
            (line(4, None), GridLine::Auto, d(3, 4)),
            (GridLine::Auto, line(6, None), d(4, 5)),
            (ident("C"), line(-1, Some("C")), d(2, 8)),
            (ident("C"), span(1, Some("C")), d(2, 5)),
            (span(1, Some("C")), line(-1, Some("C")), d(5, 8)),
            (
                span(1, Some("C")),
                span(1, Some("C")),
                AxisPlacement::Auto(1),
            ),
            (line(5, None), line(-1, Some("C")), d(4, 8)),
            (line(5, None), span(1, Some("C")), d(4, 5)),
            (line(8, None), line(8, None), d(7, 8)),
            (line(2, Some("B")), span(1, None), d(4, 5)),
        ] {
            assert_eq!(
                resolve(&start, &end, &axis),
                expected,
                "{start:?} / {end:?}"
            );
        }
    }

    #[test]
    fn missing_names_and_lines_are_implicit_ones() {
        // §8.3: `grid-column: span foo / 4` with one explicit column: no
        // line is called `foo`, and the search goes startward, so the
        // start is the implicit line before the explicit grid.
        let one = axis(&list(&[&[], &[]]));
        assert_eq!(
            resolve(&span(1, Some("foo")), &line(4, None), &one),
            AxisPlacement::Definite(-1, 3)
        );
        // A name no line has: past the explicit grid.
        assert_eq!(
            resolve(&ident("nope"), &GridLine::Auto, &one),
            AxisPlacement::Definite(2, 3)
        );
        assert_eq!(
            resolve(&line(3, Some("nope")), &GridLine::Auto, &one),
            AxisPlacement::Definite(4, 5)
        );
        assert_eq!(
            resolve(&line(-2, Some("nope")), &GridLine::Auto, &one),
            AxisPlacement::Definite(-2, -1)
        );
        // Counting from the end, past the explicit grid's start.
        assert_eq!(
            resolve(&line(-4, None), &GridLine::Auto, &one),
            AxisPlacement::Definite(-2, -1)
        );
        // §8.3.1: reversed lines swap, equal ones span one, the end's
        // span goes when both are spans, and a named span alone is one.
        assert_eq!(
            resolve(&line(3, None), &line(1, None), &one),
            AxisPlacement::Definite(0, 2)
        );
        assert_eq!(
            resolve(&GridLine::Auto, &span(3, None), &one),
            AxisPlacement::Auto(3)
        );
        assert_eq!(
            resolve(&span(2, None), &span(5, None), &one),
            AxisPlacement::Auto(2)
        );
        assert_eq!(
            resolve(&GridLine::Auto, &span(4, Some("x")), &one),
            AxisPlacement::Auto(1)
        );
        assert_eq!(
            resolve(&span(3, None), &line(2, None), &one),
            AxisPlacement::Definite(-2, 1)
        );
        // A named span searching endward from inside the explicit grid
        // skips its unnamed lines and counts the implicit ones after it.
        let two = axis(&list(&[&[], &[], &[]]));
        assert_eq!(
            resolve(&line(1, None), &span(1, Some("foo")), &two),
            AxisPlacement::Definite(0, 3)
        );
        assert_eq!(
            resolve(&span(2, Some("foo")), &line(3, None), &two),
            AxisPlacement::Definite(-2, 2)
        );
    }

    #[test]
    fn placements_are_clamped_to_the_limited_grid() {
        // §5.4: an area reaching past the limit is cut there; one wholly
        // outside moves to the last track on its side.
        let one = axis(&list(&[&[], &[]]));
        let d = AxisPlacement::Definite;
        assert_eq!(
            resolve(&line(20_000, None), &GridLine::Auto, &one),
            d(MAX_LINE - 1, MAX_LINE)
        );
        assert_eq!(
            resolve(&line(-20_000, None), &GridLine::Auto, &one),
            d(MIN_LINE, MIN_LINE + 1)
        );
        assert_eq!(
            resolve(&line(1, None), &span(u32::MAX, None), &one),
            d(0, MAX_LINE)
        );
        assert_eq!(
            resolve(&line(i32::MAX, None), &line(i32::MIN + 1, None), &one),
            d(MIN_LINE, MAX_LINE)
        );
        assert_eq!(
            resolve(&span(u32::MAX, None), &GridLine::Auto, &one),
            AxisPlacement::Auto(MAX_SPAN as i32)
        );
    }

    fn auto(span: i32) -> AxisPlacement {
        AxisPlacement::Auto(span)
    }

    fn at(start: i32, end: i32) -> AxisPlacement {
        AxisPlacement::Definite(start, end)
    }

    const ROW: GridAutoFlow = GridAutoFlow {
        column: false,
        dense: false,
    };

    #[test]
    fn auto_placement_fills_each_row_in_turn() {
        let items = vec![(auto(1), auto(1)); 5];
        let p = place(&items, 0, 3, ROW);
        assert_eq!(
            (p.rows, p.columns, p.first_row, p.first_column),
            (2, 3, 0, 0)
        );
        assert_eq!(
            p.areas,
            vec![
                (0..1, 0..1),
                (0..1, 1..2),
                (0..1, 2..3),
                (1..2, 0..1),
                (1..2, 1..2)
            ]
        );
    }

    #[test]
    fn sparse_packing_leaves_holes_that_dense_packing_fills() {
        let items = [(auto(1), auto(2)), (auto(1), auto(2)), (auto(1), auto(1))];
        let sparse = place(&items, 0, 3, ROW);
        assert_eq!(sparse.areas, vec![(0..1, 0..2), (1..2, 0..2), (1..2, 2..3)]);
        let dense = place(
            &items,
            0,
            3,
            GridAutoFlow {
                column: false,
                dense: true,
            },
        );
        assert_eq!(dense.areas, vec![(0..1, 0..2), (1..2, 0..2), (0..1, 2..3)]);
    }

    #[test]
    fn definite_items_go_first_and_the_others_flow_around_them() {
        let items = [
            (auto(1), auto(1)),
            (at(0, 1), at(1, 2)),
            (auto(1), auto(1)),
            (auto(1), auto(1)),
        ];
        let p = place(&items, 0, 2, ROW);
        assert_eq!(
            p.areas,
            vec![(0..1, 0..1), (0..1, 1..2), (1..2, 0..1), (1..2, 1..2)]
        );
    }

    #[test]
    fn items_locked_to_a_row_take_its_next_free_columns() {
        // §8.5 step 2: past the items placed in the same row before them,
        // growing the implicit grid's columns if need be.
        let items = vec![(at(0, 1), auto(1)); 4];
        let p = place(&items, 0, 3, ROW);
        assert_eq!(p.columns, 4);
        let columns: Vec<_> = p.areas.iter().map(|a| a.1.clone()).collect();
        assert_eq!(columns, vec![0..1, 1..2, 2..3, 3..4]);
        // Sparse: a later item locked to the row does not go back to a
        // hole an earlier one left; dense, it does.
        let items = [
            (at(0, 1), at(1, 2)),
            (at(0, 1), auto(2)),
            (at(0, 1), auto(1)),
        ];
        let p = place(&items, 0, 4, ROW);
        assert_eq!(p.areas[1].1, 2..4);
        assert_eq!(p.areas[2].1, 4..5);
        let dense = GridAutoFlow {
            column: false,
            dense: true,
        };
        let p = place(&items, 0, 4, dense);
        assert_eq!(p.areas[2].1, 0..1);
    }

    #[test]
    fn an_item_with_a_definite_column_moves_the_cursor_down_when_it_goes_back() {
        // §8.5 step 4, sparse: a column before the cursor's means the
        // next row.
        let items = [(auto(1), at(1, 2)), (auto(1), at(0, 1)), (auto(1), auto(1))];
        let p = place(&items, 0, 2, ROW);
        assert_eq!(p.areas, vec![(0..1, 1..2), (1..2, 0..1), (1..2, 1..2)]);
        let dense = GridAutoFlow {
            column: false,
            dense: true,
        };
        let p = place(&items, 0, 2, dense);
        assert_eq!(p.areas, vec![(0..1, 1..2), (0..1, 0..1), (1..2, 0..1)]);
    }

    #[test]
    fn column_flow_places_before_the_explicit_grid_in_both_axes() {
        let flow = GridAutoFlow {
            column: true,
            dense: false,
        };
        let p = place(&[(at(-3, -2), at(-1, 0)), (auto(1), auto(1))], 1, 1, flow);
        assert_eq!((p.first_row, p.first_column), (-3, -1));
        assert_eq!((p.rows, p.columns), (4, 2));
        assert_eq!(p.areas[0], (0..1, 0..1));
    }

    #[test]
    fn column_flow_fills_each_column_in_turn() {
        let items = vec![(auto(1), auto(1)); 3];
        let flow = GridAutoFlow {
            column: true,
            dense: false,
        };
        let p = place(&items, 2, 0, flow);
        assert_eq!((p.rows, p.columns), (2, 2));
        assert_eq!(p.areas, vec![(0..1, 0..1), (1..2, 0..1), (0..1, 1..2)]);
    }

    #[test]
    fn positions_before_the_explicit_grid_add_tracks_there() {
        let items = [(auto(1), at(-2, -1)), (at(-1, 0), auto(1))];
        let p = place(&items, 1, 2, ROW);
        assert_eq!((p.first_row, p.first_column), (-1, -2));
        assert_eq!((p.rows, p.columns), (2, 4));
        assert_eq!(p.areas[0].1, 0..1);
        assert_eq!(p.areas[1].0, 0..1);
        // The auto-placement cursor starts at the implicit grid's start.
        let p = place(&[(at(-1, 0), at(-1, 0)), (auto(1), auto(1))], 0, 2, ROW);
        assert_eq!(p.areas[1], (0..1, 1..2));
    }

    #[test]
    fn the_widest_automatic_span_widens_the_grid() {
        let p = place(&[(auto(1), auto(4)), (auto(1), auto(1))], 0, 2, ROW);
        assert_eq!(p.columns, 4);
        assert_eq!(p.areas, vec![(0..1, 0..4), (1..2, 0..1)]);
    }

    #[test]
    fn tall_items_are_avoided_like_any_other() {
        let items = [
            (at(0, 200), at(0, 1)),
            (auto(1), auto(1)),
            (auto(150), auto(1)),
            (auto(1), at(0, 1)),
        ];
        let p = place(&items, 0, 2, ROW);
        assert_eq!(p.areas[1], (0..1, 1..2));
        assert_eq!(p.areas[2], (1..151, 1..2));
        // Below the tall item, its column is free again.
        assert_eq!(p.areas[3], (200..201, 0..1));
        assert_eq!(p.rows, 201);
    }

    #[test]
    fn placement_never_leaves_the_limited_grid_and_ends_quickly() {
        // Two thousand items each spanning ten thousand rows, auto-placed
        // in one column: the grid stops growing at the limit, and every
        // area stays inside it.
        let items = vec![(auto(10_000), auto(1)); 2_000];
        let started = std::time::Instant::now();
        let p = place(&items, 0, 1, ROW);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        for (rows, columns) in &p.areas {
            assert!(rows.start >= 0 && rows.end <= p.rows, "{rows:?}");
            assert!(
                columns.start >= 0 && columns.end <= p.columns,
                "{columns:?}"
            );
        }
        assert!(p.rows <= MAX_LINE - MIN_LINE);
        // A span longer than the limit leaves room for still starts at the
        // grid's first line, not before it.
        let items = vec![(auto(15_000), auto(1)); 3];
        let p = place(&items, 0, 1, ROW);
        for (rows, _) in &p.areas {
            assert!(rows.start >= 0 && rows.end <= p.rows, "{rows:?}");
        }
        // Locked to a row past the limit's columns.
        let items = vec![(at(0, 1), auto(MAX_SPAN as i32)); 3];
        let p = place(&items, 0, 1, ROW);
        for (_, columns) in &p.areas {
            assert!(columns.start >= 0 && columns.end <= p.columns);
        }
    }

    #[test]
    fn auto_placed_items_never_overlap_what_was_there() {
        // A property over many generated grids: every area lies inside
        // the implicit grid, and an auto-placed item overlaps nothing
        // placed before it.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as i32
        };
        for _ in 0..300 {
            let (explicit_rows, explicit_columns) = (next(4), next(5));
            let count = next(12) as usize + 1;
            let mut items = Vec::new();
            for _ in 0..count {
                let mut one = |len: i32| {
                    if next(3) == 0 {
                        let start = next(len + 3) - 1;
                        at(start, start + next(3) + 1)
                    } else {
                        auto(next(3) + 1)
                    }
                };
                items.push((one(explicit_rows), one(explicit_columns)));
            }
            let flow = GridAutoFlow {
                column: next(2) == 0,
                dense: next(2) == 0,
            };
            let p = place(&items, explicit_rows, explicit_columns, flow);
            let overlap = |a: &(Range<i32>, Range<i32>), b: &(Range<i32>, Range<i32>)| {
                a.0.start < b.0.end
                    && b.0.start < a.0.end
                    && a.1.start < b.1.end
                    && b.1.start < a.1.end
            };
            // The step of §8.5 an item is placed in: both axes definite,
            // locked to a line of the flow's major axis, or the rest.
            let step = |(rows, columns): (AxisPlacement, AxisPlacement)| {
                let (major, minor) = if flow.column {
                    (columns, rows)
                } else {
                    (rows, columns)
                };
                match (major.definite(), minor.definite()) {
                    (Some(_), Some(_)) => 0,
                    (Some(_), None) => 1,
                    _ => 2,
                }
            };
            for (i, area) in p.areas.iter().enumerate() {
                assert!(
                    area.0.start >= 0 && area.0.end <= p.rows,
                    "{items:?} {flow:?}"
                );
                assert!(
                    area.1.start >= 0 && area.1.end <= p.columns,
                    "{items:?} {flow:?}"
                );
                if step(items[i]) == 0 {
                    continue;
                }
                for (j, other) in p.areas.iter().enumerate() {
                    let earlier = (step(items[j]), j) < (step(items[i]), i);
                    if earlier {
                        assert!(!overlap(area, other), "{items:?} {flow:?}: {i} and {j}");
                    }
                }
            }
        }
    }
}
