//! Focusgroups (gap batch H): the `focusgroup` attribute of Open UI's
//! scoped focusgroup explainer, which the reference browser ships, as
//! its tests have it. An owner's items are the focusable elements of
//! its flat subtree whose nearest focusgroup is it (`dom`'s
//! [`Document::focus_group_owner`]): the arrow keys move the focus among
//! them along the axes its tokens give, wrapping where they say, in the
//! flat tree's order as reading flows order it, and Home and End to the
//! first and the last; a `grid` owner that is a table moves by its rows
//! and columns. An element that takes the arrow keys itself (a text
//! field, a select, a radio button, an editing host, a media element's
//! controls, a scroller along its axes) keeps them.
//!
//! For Tab, the items are split into segments by what else Tab stops at
//! in the owner's subtree (opted-out content, a nested focusgroup's
//! items, a popover shown there) and by the element focused when it is
//! such content: each segment is one stop, its entry, at the place of
//! its first item in the sequential order. The entry is the item the
//! owner remembers as last focused, unless it has `nomemory`; else the
//! first with `focusgroupstart`; else the first Tab could stop at. Tab
//! from an item leaves its segment, but from an element that keeps the
//! arrow keys, which goes on to the item beside it.

use std::collections::{HashMap, HashSet};

use dom::{Document, FocusGroup, FocusGroupAttribute, FocusGroupBehavior, NodeId, forms};
use style::Styles;
use style::computed::{Direction, Display};

use crate::focus::{self, Laid};

/// What the focusgroup code reads: the document, its styles and layout.
#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    pub document: &'a Document,
    pub styles: Option<&'a Styles>,
    pub laid: Option<&'a Laid>,
}

/// The way a key moves the focus in a focusgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    /// Along the inline axis (`true`) or the block axis, forward or not.
    Step {
        inline: bool,
        forward: bool,
    },
    /// Home and End.
    First,
    Last,
}

/// The most elements a focusgroup walk visits, past which the rest of a
/// subtree is not an owner's: the DOM's own bound on nodes is far above
/// what a group of controls holds.
const MAX_WALK: usize = 1 << 16;

impl Context<'_> {
    /// Whether `element` is one the arrow keys could move the focus to.
    fn target(&self, element: NodeId) -> bool {
        focus::keyboard_target(self.document, self.styles, self.laid, element)
    }

    /// Whether `element` takes the focus at all, as an item may have it.
    fn focusable(&self, element: NodeId) -> bool {
        focus::focusable_in(self.document, self.styles, self.laid, element)
    }

    /// `element`'s computed style, if it has one.
    fn style(&self, element: NodeId) -> Option<&style::ComputedStyle> {
        self.styles?.get(element)
    }

    /// The elements of `owner`'s flat subtree as a focusgroup walks
    /// them, `owner` aside: in the flat tree's order, reading flows'
    /// items in their order. Each with whether it is one of `owner`'s
    /// items: not under an element that opts out, a nested owner (which
    /// is an item itself) or an element in the top layer.
    fn walk(&self, owner: NodeId) -> Vec<(NodeId, bool)> {
        let document = self.document;
        let mut out = Vec::new();
        // Each entry: an element and whether it is still `owner`'s.
        let mut stack: Vec<(NodeId, bool)> =
            focus::reading_children(document, self.styles, self.laid, owner)
                .into_iter()
                .rev()
                .map(|child| (child, true))
                .collect();
        while let Some((node, mine)) = stack.pop() {
            if out.len() >= MAX_WALK {
                break;
            }
            if self.style(node).is_some_and(|s| s.display == Display::None) {
                continue;
            }
            let attribute = document.focus_group(node);
            let item = mine
                && !document.in_top_layer(node)
                && !matches!(attribute, Some(FocusGroupAttribute::OptOut));
            out.push((node, item));
            let inner = item && attribute.is_none();
            let children = focus::reading_children(document, self.styles, self.laid, node);
            stack.extend(children.into_iter().rev().map(|child| (child, inner)));
        }
        out
    }

    /// The direction `key` moves the focus in from `focused`, if it is
    /// one a focusgroup takes: Left and Right along the inline axis, by
    /// `focused`'s own direction; Up and Down along the block axis.
    fn movement(&self, focused: NodeId, key: &str) -> Option<Move> {
        let rtl = self
            .style(focused)
            .is_some_and(|s| s.direction == Direction::Rtl);
        Some(match key {
            "ArrowRight" => Move::Step {
                inline: true,
                forward: !rtl,
            },
            "ArrowLeft" => Move::Step {
                inline: true,
                forward: rtl,
            },
            "ArrowDown" => Move::Step {
                inline: false,
                forward: true,
            },
            "ArrowUp" => Move::Step {
                inline: false,
                forward: false,
            },
            "Home" => Move::First,
            "End" => Move::Last,
            _ => return None,
        })
    }

    /// Whether `element` takes the arrow keys of the inline axis, or of
    /// the block axis, itself, as its own controls: a text field, a
    /// textarea, a select, a radio button, an editing host, a media
    /// element with controls along both; a scroller along those it
    /// scrolls in. Home and End for any.
    fn handles(&self, element: NodeId, inline: Option<bool>) -> bool {
        let document = self.document;
        let own = match document.tag(element) {
            Some("textarea" | "select") => true,
            Some("input") => {
                forms::is_text_control(document, element)
                    || forms::input_type(document, element) == "radio"
            }
            Some("video" | "audio") => document.attr(element, "controls").is_some(),
            _ => focus::editable(document, element),
        };
        if own {
            return true;
        }
        let Some((across, down)) = self.laid.and_then(|laid| laid.scroll_axes(element)) else {
            return false;
        };
        match inline {
            Some(true) => across,
            Some(false) => down,
            None => across || down,
        }
    }

    /// The element an arrow key, Home or End moves the focus to from
    /// `focused` in its focusgroup: `None` when the key is not one the
    /// focusgroup takes there, `Some(None)` when it is but the focus
    /// stays.
    pub(crate) fn directional_target(&self, focused: NodeId, key: &str) -> Option<Option<NodeId>> {
        let document = self.document;
        let (owner, group) = document.focus_group_owner(focused)?;
        let step = self.movement(focused, key)?;
        let inline = match step {
            Move::Step { inline, .. } => Some(inline),
            Move::First | Move::Last => None,
        };
        if self.handles(focused, inline) {
            return None;
        }
        if group.behavior == FocusGroupBehavior::Grid {
            return self.grid_target(owner, &group, focused, step);
        }
        if let Move::Step { inline, .. } = step
            && !(if inline { group.inline } else { group.block })
        {
            return None;
        }
        let items: Vec<NodeId> = self
            .walk(owner)
            .into_iter()
            .filter(|&(node, item)| item && (node == focused || self.target(node)))
            .map(|(node, _)| node)
            .collect();
        let at = items.iter().position(|&node| node == focused)?;
        let target = |node: &&NodeId| **node != focused && self.target(**node);
        // Home and End go to the first and last, which may be where the
        // focus is.
        let end = |node: &&NodeId| self.target(**node);
        let found = match step {
            Move::First => items.iter().find(end),
            Move::Last => items.iter().rev().find(end),
            Move::Step { forward: true, .. } => items[at + 1..].iter().find(target).or_else(|| {
                group
                    .wrap
                    .then(|| items[..at].iter().find(target))
                    .flatten()
            }),
            Move::Step { forward: false, .. } => {
                items[..at].iter().rev().find(target).or_else(|| {
                    group
                        .wrap
                        .then(|| items[at + 1..].iter().rev().find(target))
                        .flatten()
                })
            }
        };
        Some(found.copied())
    }

    /// The cell a key moves the focus to in a grid focusgroup, `owner` a
    /// table by its style; `None` for another owner, which is no
    /// focusgroup at all, as the reference browser has it.
    fn grid_target(
        &self,
        owner: NodeId,
        group: &FocusGroup,
        focused: NodeId,
        step: Move,
    ) -> Option<Option<NodeId>> {
        let grid = Grid::of(self, owner)?;
        let cell = std::iter::once(focused)
            .chain(self.document.flat_ancestors(focused))
            .take_while(|&node| node != owner)
            .find_map(|node| grid.cells.iter().position(|c| c.node == node))?;
        Some(grid.step(self, group, cell, step))
    }
}

/// A table's cells as a grid focusgroup moves among them: each cell's
/// first row and column and how many it spans, and the slots of the
/// grid by row and column.
struct Grid {
    cells: Vec<Cell>,
    /// `slots[row][column]`: the cell there, if any.
    slots: Vec<Vec<Option<usize>>>,
    columns: usize,
}

/// One cell of a [`Grid`].
#[derive(Debug, Clone, Copy)]
struct Cell {
    node: NodeId,
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
}

/// The most rows and columns a grid focusgroup reads of a table, as
/// HTML caps `rowspan` and `colspan`.
const MAX_GRID: usize = 1 << 12;

impl Grid {
    /// The grid of `table` by its boxes' display (an HTML table's and a
    /// CSS one's alike): its header groups' rows first, then its rows and
    /// body groups' in order, then its footer groups'; each row's cells
    /// placed as HTML's table forming algorithm places them, by their
    /// `rowspan` and `colspan`. `None` when `table` is no table.
    fn of(context: &Context, table: NodeId) -> Option<Grid> {
        let document = context.document;
        let display = |node: NodeId| context.style(node).map(|s| s.display);
        if !matches!(display(table)?, Display::Table | Display::InlineTable) {
            return None;
        }
        let children = |node: NodeId| -> Vec<NodeId> {
            document
                .flat_children(node)
                .filter(|&child| document.is_element(child))
                .collect()
        };
        let (mut head, mut body, mut foot) = (Vec::new(), Vec::new(), Vec::new());
        for child in children(table) {
            match display(child) {
                Some(Display::TableRow) => body.push(child),
                Some(Display::TableRowGroup) => body.extend(children(child)),
                Some(Display::TableHeaderGroup) => head.extend(children(child)),
                Some(Display::TableFooterGroup) => foot.extend(children(child)),
                _ => {}
            }
        }
        let rows: Vec<NodeId> = head
            .into_iter()
            .chain(body)
            .chain(foot)
            .filter(|&row| display(row) == Some(Display::TableRow))
            .take(MAX_GRID)
            .collect();
        let span = |cell: NodeId, name: &str, zero: usize| match document
            .attr(cell, name)
            .and_then(dom::parse_non_negative_integer)
        {
            Some(0) => zero,
            Some(n) => usize::try_from(n).unwrap_or(MAX_GRID).clamp(1, MAX_GRID),
            None => 1,
        };
        let mut grid = Grid {
            cells: Vec::new(),
            slots: vec![Vec::new(); rows.len()],
            columns: 0,
        };
        for (r, &row) in rows.iter().enumerate() {
            let mut column = 0;
            for cell in children(row) {
                if display(cell) != Some(Display::TableCell) {
                    continue;
                }
                while grid.slots[r].get(column).copied().flatten().is_some() {
                    column += 1;
                }
                let html = matches!(document.tag(cell), Some("td" | "th"));
                let (down, across) = match html {
                    true => (
                        span(cell, "rowspan", rows.len() - r),
                        span(cell, "colspan", 1),
                    ),
                    false => (1, 1),
                };
                let down = down.min(rows.len() - r);
                let across = across.min(MAX_GRID.saturating_sub(column)).max(1);
                if column + across > MAX_GRID {
                    break;
                }
                let index = grid.cells.len();
                grid.cells.push(Cell {
                    node: cell,
                    row: r,
                    column,
                    rows: down,
                    columns: across,
                });
                for slot_row in &mut grid.slots[r..r + down] {
                    if slot_row.len() < column + across {
                        slot_row.resize(column + across, None);
                    }
                    for slot in &mut slot_row[column..column + across] {
                        *slot = Some(index);
                    }
                }
                column += across;
                grid.columns = grid.columns.max(column);
            }
        }
        Some(grid)
    }

    /// The cell at `row`, `column`, if one covers it.
    fn at(&self, row: usize, column: usize) -> Option<usize> {
        self.slots.get(row)?.get(column).copied().flatten()
    }

    /// The node of the first cell of `slots` (row and column pairs) that
    /// is not `from`, when given, and that the arrow keys could focus.
    fn first(
        &self,
        context: &Context,
        from: Option<usize>,
        slots: impl Iterator<Item = (usize, usize)>,
    ) -> Option<NodeId> {
        slots
            .filter_map(|(row, column)| self.at(row, column))
            .filter(|&cell| Some(cell) != from)
            .map(|cell| self.cells[cell].node)
            .find(|&node| context.target(node))
    }

    /// Where `step` moves the focus from cell `from`: along its first row
    /// or column, past the edge by the group's wrapping or flowing in
    /// that direction, a flow going on to the next row or column round
    /// to the first.
    fn step(
        &self,
        context: &Context,
        group: &FocusGroup,
        from: usize,
        step: Move,
    ) -> Option<NodeId> {
        let cell = self.cells[from];
        let (rows, columns) = (self.slots.len(), self.columns);
        let in_row = |row: usize, cols: Vec<usize>| cols.into_iter().map(move |c| (row, c));
        let in_column = |column: usize, rs: Vec<usize>| rs.into_iter().map(move |r| (r, column));
        match step {
            Move::First => self.first(
                context,
                None,
                (0..rows).flat_map(|r| (0..columns).map(move |c| (r, c))),
            ),
            Move::Last => self.first(
                context,
                None,
                (0..rows)
                    .rev()
                    .flat_map(|r| (0..columns).rev().map(move |c| (r, c))),
            ),
            Move::Step {
                inline: true,
                forward,
            } => {
                let r = cell.row;
                let ahead: Vec<usize> = match forward {
                    true => (cell.column + cell.columns..columns).collect(),
                    false => (0..cell.column).rev().collect(),
                };
                if let Some(found) = self.first(context, Some(from), in_row(r, ahead)) {
                    return Some(found);
                }
                if group.row_wrap {
                    let behind: Vec<usize> = match forward {
                        true => (0..cell.column).collect(),
                        false => (cell.column + cell.columns..columns).rev().collect(),
                    };
                    return self.first(context, Some(from), in_row(r, behind));
                }
                if group.row_flow {
                    // The rows after, round to this one, each from its
                    // start (its end backward).
                    let order: Vec<usize> = match forward {
                        true => (1..=rows).map(|n| (r + n) % rows).collect(),
                        false => (1..=rows).map(|n| (r + rows - n) % rows).collect(),
                    };
                    let slots = order.into_iter().flat_map(|row| {
                        let cols: Vec<usize> = match forward {
                            true => (0..columns).collect(),
                            false => (0..columns).rev().collect(),
                        };
                        in_row(row, cols)
                    });
                    return self.first(context, Some(from), slots);
                }
                None
            }
            Move::Step {
                inline: false,
                forward,
            } => {
                let c = cell.column;
                let ahead: Vec<usize> = match forward {
                    true => (cell.row + cell.rows..rows).collect(),
                    false => (0..cell.row).rev().collect(),
                };
                if let Some(found) = self.first(context, Some(from), in_column(c, ahead)) {
                    return Some(found);
                }
                if group.col_wrap {
                    let behind: Vec<usize> = match forward {
                        true => (0..cell.row).collect(),
                        false => (cell.row + cell.rows..rows).rev().collect(),
                    };
                    return self.first(context, Some(from), in_column(c, behind));
                }
                if group.col_flow && columns > 0 {
                    let order: Vec<usize> = match forward {
                        true => (1..=columns).map(|n| (c + n) % columns).collect(),
                        false => (1..=columns).map(|n| (c + columns - n) % columns).collect(),
                    };
                    let slots = order.into_iter().flat_map(|column| {
                        let rs: Vec<usize> = match forward {
                            true => (0..rows).collect(),
                            false => (0..rows).rev().collect(),
                        };
                        in_column(column, rs)
                    });
                    return self.first(context, Some(from), slots);
                }
                None
            }
        }
    }
}

/// A focusgroup's segment: its owner and its items, in the walk's order.
struct Segment {
    owner: NodeId,
    group: FocusGroup,
    items: Vec<NodeId>,
}

/// The sequential order as focusgroups make it: each segment one stop.
pub(crate) struct TabStops {
    /// The stops in order, each with the index in the sequential order
    /// of the place it takes.
    stops: Vec<(NodeId, usize)>,
    /// The segment each item is in.
    segment_of: HashMap<NodeId, usize>,
    /// Each segment's entry, if it has one.
    entries: Vec<Option<NodeId>>,
}

/// Where Tab starts in the sequential order.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Origin {
    /// From the element at this index of the order.
    At(usize),
    /// From a node not in it, before the element at this index going
    /// forward, after it going backward.
    Beside(usize),
    /// From nothing: the first or the last.
    Nothing,
    /// To this area, which the order does not have.
    Direct(NodeId),
    /// From past the order's end: nowhere.
    Past,
}

impl TabStops {
    /// The stops of `order`, the sequential order, with `focused` the
    /// element focused; `None` when no focusgroup owner is in the
    /// document, and the order stands as it is.
    pub(crate) fn new(
        context: &Context,
        order: &[NodeId],
        focused: Option<NodeId>,
    ) -> Option<TabStops> {
        let document = context.document;
        let root = document.root();
        let nodes = match document.has_shadow_roots() {
            true => document.shadow_including_subtree(root),
            false => document.subtree(root).collect(),
        };
        let owners: Vec<(NodeId, FocusGroup)> = nodes
            .into_iter()
            .filter(|&node| document.is_element(node) && document.in_flat_tree(node))
            .filter_map(|node| match document.focus_group(node) {
                Some(FocusGroupAttribute::Owner(group)) => Some((node, group)),
                _ => None,
            })
            .collect();
        if owners.is_empty() {
            return None;
        }
        let in_order: HashSet<NodeId> = order.iter().copied().collect();
        let mut segments: Vec<Segment> = Vec::new();
        for (owner, group) in owners {
            let mut items = Vec::new();
            for (node, item) in context.walk(owner) {
                if item {
                    if context.focusable(node) {
                        items.push(node);
                    }
                } else if in_order.contains(&node) || Some(node) == focused {
                    // Something else Tab stops at, or the element focused
                    // that is not an item, ends the segment.
                    if !items.is_empty() {
                        segments.push(Segment {
                            owner,
                            group,
                            items: std::mem::take(&mut items),
                        });
                    }
                }
            }
            if !items.is_empty() {
                segments.push(Segment {
                    owner,
                    group,
                    items,
                });
            }
        }
        let mut segment_of = HashMap::new();
        for (at, segment) in segments.iter().enumerate() {
            for &item in &segment.items {
                segment_of.entry(item).or_insert(at);
            }
        }
        let entries: Vec<Option<NodeId>> = segments
            .iter()
            .map(|segment| entry(context, segment, &in_order))
            .collect();
        let mut stops = Vec::new();
        let mut placed = vec![false; segments.len()];
        for (at, &node) in order.iter().enumerate() {
            match segment_of.get(&node) {
                Some(&segment) => {
                    if !placed[segment] {
                        placed[segment] = true;
                        if let Some(entry) = entries[segment] {
                            stops.push((entry, at));
                        }
                    }
                }
                None => stops.push((node, at)),
            }
        }
        Some(TabStops {
            stops,
            segment_of,
            entries,
        })
    }

    /// The stop Tab moves the focus to from `current`, at `origin` in
    /// `order`, the next or, `backward`, the previous: from an item, the
    /// next stop out of its segment; from an item that keeps the arrow
    /// keys, the element beside it in the order when that is of its
    /// segment.
    pub(crate) fn next(
        &self,
        context: &Context,
        order: &[NodeId],
        current: Option<NodeId>,
        origin: Origin,
        backward: bool,
    ) -> Option<NodeId> {
        let segment = current.and_then(|c| self.segment_of.get(&c).copied());
        if let (Some(current), Some(segment)) = (current, segment)
            && context.handles(current, None)
        {
            let beside = match (origin, backward) {
                (Origin::At(i), false) => order.get(i + 1),
                (Origin::At(i), true) => i.checked_sub(1).and_then(|i| order.get(i)),
                (Origin::Beside(j), _) => order.get(j),
                (Origin::Direct(node), _) => return Some(node),
                (Origin::Nothing | Origin::Past, _) => None,
            };
            if let Some(&beside) = beside
                && self.segment_of.get(&beside) == Some(&segment)
            {
                return Some(beside);
            }
        }
        let skip = |node: NodeId| segment.is_some_and(|s| self.entries[s] == Some(node));
        let found = match (origin, backward) {
            (Origin::At(i), false) => self.stops.iter().find(|&&(n, at)| at > i && !skip(n)),
            (Origin::At(i), true) => self.stops.iter().rev().find(|&&(n, at)| at < i && !skip(n)),
            (Origin::Beside(j), false) => self.stops.iter().find(|&&(n, at)| at >= j && !skip(n)),
            (Origin::Beside(j), true) => self
                .stops
                .iter()
                .rev()
                .find(|&&(n, at)| at <= j && !skip(n)),
            (Origin::Nothing, false) => self.stops.first(),
            (Origin::Nothing, true) => self.stops.last(),
            (Origin::Direct(node), _) => return Some(node),
            (Origin::Past, _) => None,
        };
        found.map(|&(node, _)| node)
    }
}

/// A segment's entry: the item its owner remembers as last focused when
/// it has memory and that is in it, else its first item with
/// `focusgroupstart`, else its first; each one Tab could stop at, in the
/// order or kept out of it only by a scope's `tabindex`.
fn entry(context: &Context, segment: &Segment, in_order: &HashSet<NodeId>) -> Option<NodeId> {
    let document = context.document;
    let eligible = |node: NodeId| in_order.contains(&node) || context.target(node);
    if segment.group.memory
        && let Some(memory) = document.focus_group_memory(segment.owner)
        && segment.items.contains(&memory)
        && eligible(memory)
    {
        return Some(memory);
    }
    segment
        .items
        .iter()
        .copied()
        .find(|&item| document.attr(item, "focusgroupstart").is_some() && eligible(item))
        .or_else(|| segment.items.iter().copied().find(|&item| eligible(item)))
}
