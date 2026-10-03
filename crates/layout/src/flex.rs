//! Flex layout (M17): a flex container's items laid out by the flexible
//! box layout algorithm of css-flexbox-1 §9, in the abstract main and
//! cross axes and mapped to physical coordinates at the end. A row
//! container's main axis is horizontal and a column container's
//! vertical; `*-reverse` swaps main-start and main-end, and
//! `wrap-reverse` cross-start and cross-end. Everything runs left to
//! right and top to bottom, as the rest of layout does.
//!
//! Each item is laid out as a formatting context root (§4) through the
//! pass's kept item layouts ([`Pass::item`]): at the content width the
//! algorithm gives it, and with the height the algorithm fixes for it,
//! if any, imposed ([`Imposed`]). Sizes in the main axis are content-box
//! sizes while the algorithm runs, as the draft's inner flex base size
//! is; an item's outer size adds its border, padding and margins.
//!
//! Every line is packed at main-start, and every item is stretched or
//! placed at its line's cross-start: `justify-content`, the other
//! values of `align-items` and `align-self`, `align-content` but for
//! `stretch`, and `auto` margins, which count as zero, are not applied
//! yet. An absolutely positioned child's static position is the content
//! box's top-left corner.

use std::cell::OnceCell;
use std::ops::Range;

use style::computed::{
    BOTTOM, BoxSizing, ComputedStyle, ContentAlignment, FlexBasis, FlexWrap, LEFT,
    LengthPercentage, MaxSize, RIGHT, SelfAlignment, Size, TOP,
};
use text::FontCollection;

use crate::block::{
    AdjoiningMargins, BlockLayout, Content, Deferred, Edges, Imposed, Intrinsic, box_fragment,
    carried, clamp_height, definite_height, extent_bottom, height_bounds, layout_contents,
    relative_offset, settle_absolutes, used, width_constraints,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::pass::{ItemKey, Pass};

/// What line breaking compares sums with (design note, step 5): a
/// sixty-fourth of a pixel, the reference browser's layout unit, so that
/// a fraction lost to rounding does not push an item to the next line.
const TOLERANCE: f32 = 1.0 / 64.0;

/// The flex container as its items see it.
struct Container {
    /// The main axis is vertical: a column container.
    column: bool,
    /// Main-start is at the right or the bottom.
    reverse: bool,
    /// Cross-start is at the bottom or the right.
    wrap_reverse: bool,
    /// The content box's width, the items' containing block's.
    width: f32,
    /// The content box's height when it is definite (§9.8): what the
    /// items' percentages resolve against.
    definite_height: Option<f32>,
    /// The gap between neighbouring items on a line, and between lines.
    main_gap: f32,
    cross_gap: f32,
    /// The container's `align-items`, which `align-self: auto` takes.
    align_items: SelfAlignment,
}

impl Container {
    /// The content box's size in the main axis when it is definite.
    fn definite_main(&self) -> Option<f32> {
        if self.column {
            self.definite_height
        } else {
            Some(self.width)
        }
    }

    /// The physical sides at main-start and main-end.
    fn main_sides(&self) -> (usize, usize) {
        match (self.column, self.reverse) {
            (false, false) => (LEFT, RIGHT),
            (false, true) => (RIGHT, LEFT),
            (true, false) => (TOP, BOTTOM),
            (true, true) => (BOTTOM, TOP),
        }
    }

    /// The physical side at cross-start.
    fn cross_start(&self) -> usize {
        match (self.column, self.wrap_reverse) {
            (false, false) => TOP,
            (false, true) => BOTTOM,
            (true, false) => LEFT,
            (true, true) => RIGHT,
        }
    }
}

/// A flex item as the algorithm sizes it.
struct Item<'a> {
    b: &'a LayoutBox,
    /// The item's content widths, measured if anything asks.
    intrinsic: Intrinsic<'a>,
    edges: Edges,
    /// The margins, top, right, bottom and left, an `auto` one zero.
    margin: [f32; 4],
    /// Border and padding in the main axis, and with the margins
    /// there what the outer size adds to the content-box size.
    main_edges: f32,
    main_extra: f32,
    /// The margins in the cross axis.
    cross_margins: f32,
    /// The flex base size (§9.2 step 3), and whether it came from a
    /// definite `flex-basis` rather than the content.
    base: f32,
    definite_basis: bool,
    /// The used minimum and maximum main sizes, `INFINITY` for none.
    min: f32,
    max: f32,
    /// The hypothetical main size: `base` clamped by `min` and `max`.
    hypothetical: f32,
    /// The target main size (§9.7), and at the end the used one.
    target: f32,
    /// Whether the item is stretched in the cross axis (step 11).
    stretches: bool,
    /// The border-box cross size: hypothetical, then used.
    cross: f32,
    /// A row item's content height as with `height: auto`, from the
    /// layout that gave its hypothetical cross size: what a keyword in
    /// its `min-height` or `max-height` is when it is stretched.
    auto_height: f32,
    /// A column item's content width, the cross size's content box.
    width: f32,
    /// A row item stretched: its border-box height is imposed.
    imposed_cross: Option<f32>,
    /// The border box's position in the container's content box.
    x: f32,
    y: f32,
}

impl<'a> Item<'a> {
    /// The item `b`, its flex base size and hypothetical main size
    /// found (§9.2 step 3) and its minimum and maximum main sizes
    /// resolved, the automatic minimum (§4.5) among them.
    fn new(b: &'a LayoutBox, c: &Container, single_line: bool, pass: &Pass<'a>) -> Self {
        let style = &b.style;
        let edges = Edges::of_box(b, c.width);
        // Margins are percentages of the containing block's width on
        // every side (CSS2 §8.3), the container's content box here.
        let margin: [f32; 4] =
            std::array::from_fn(|side| used(style.margin[side], Some(c.width)).unwrap_or(0.0));
        let (main_edges, main_margins, cross_margins) = if c.column {
            (
                edges.vertical(),
                margin[TOP] + margin[BOTTOM],
                margin[LEFT] + margin[RIGHT],
            )
        } else {
            (
                edges.horizontal(),
                margin[LEFT] + margin[RIGHT],
                margin[TOP] + margin[BOTTOM],
            )
        };
        let auto_cross_margin = if c.column {
            style.margin[LEFT].is_auto() || style.margin[RIGHT].is_auto()
        } else {
            style.margin[TOP].is_auto() || style.margin[BOTTOM].is_auto()
        };
        let cross_property = if c.column { style.width } else { style.height };
        let stretches = matches!(
            align_self(style, c.align_items),
            SelfAlignment::Stretch | SelfAlignment::Normal
        ) && cross_property.is_auto()
            && !auto_cross_margin;
        let mut item = Item {
            b,
            intrinsic: Intrinsic::of(b, pass.fonts),
            edges,
            margin,
            main_edges,
            main_extra: main_edges + main_margins,
            cross_margins,
            base: 0.0,
            definite_basis: false,
            min: 0.0,
            max: f32::INFINITY,
            hypothetical: 0.0,
            target: 0.0,
            stretches,
            cross: 0.0,
            auto_height: 0.0,
            width: 0.0,
            imposed_cross: None,
            x: 0.0,
            y: 0.0,
        };

        let inner = |v: f32| match style.box_sizing {
            BoxSizing::ContentBox => v.max(0.0),
            BoxSizing::BorderBox => (v - main_edges).max(0.0),
        };
        // The room `fit-content` fills in the inline axis, a row item's
        // main axis: the container's width less the item's margins,
        // border and padding.
        let room = (c.width - item.edges.horizontal() - margin[LEFT] - margin[RIGHT]).max(0.0);
        // A main size as a definite content-box size: a length, a
        // percentage of a definite main size, and in a row container an
        // intrinsic keyword; `None` for `auto`, a percentage of an
        // indefinite size, and a keyword in the block axis, which is
        // `auto` there (css-sizing-3 §3.2).
        let definite = |size: Size| match size {
            Size::Px(px) => Some(inner(px)),
            Size::Percent(p) => c.definite_main().map(|of| inner(of * p / 100.0)),
            Size::Auto => None,
            keyword if !c.column => item.intrinsic.keyword(keyword, room),
            _ => None,
        };
        let main_property = if c.column { style.height } else { style.width };
        // §7.2.3: `flex-basis: auto` is the main size property; either
        // being `auto` is `content`.
        let basis = match style.flex_basis {
            FlexBasis::Content => None,
            FlexBasis::Size(Size::Auto) => definite(main_property),
            FlexBasis::Size(size) => definite(size),
        };
        // A column item's height at its cross size (§9.2 step 3, case
        // E): its definite width, or stretched in a single-line container
        // the container's width less its margins, else `fit-content`;
        // laid out once, if the basis, a limit or the automatic minimum
        // asks. Its content's height does not depend on its own height,
        // whose percentage is indefinite here as in the reference
        // browsers: the content inside it is measured as with `auto`.
        let measured = OnceCell::new();
        let content_height = || {
            *measured.get_or_init(|| {
                let room = (c.width - cross_margins - item.edges.horizontal()).max(0.0);
                let stretched = (single_line && item.stretches).then_some(room);
                let width = item.column_width(c, stretched);
                let layout = item.lay_out_in(c, width, None, None, pass);
                layout.auto_content_height
            })
        };
        // The content size in the main axis, measured if anything asks:
        // under a max-content constraint, what `content` means (case E);
        // the min-content size, for the automatic minimum. A column
        // item's are both its height.
        let min_content = || {
            if c.column {
                content_height()
            } else {
                item.intrinsic.widths().0
            }
        };
        let max_content = || {
            if c.column {
                content_height()
            } else {
                item.intrinsic.widths().1
            }
        };
        // A keyword in `min-height` or `max-height` is the column item's
        // content height, its automatic size (css-sizing-3 §3.2).
        let max = match if c.column {
            style.max_height
        } else {
            style.max_width
        } {
            MaxSize::None => None,
            MaxSize::Px(px) => Some(inner(px)),
            MaxSize::Percent(p) => c.definite_main().map(|of| inner(of * p / 100.0)),
            MaxSize::MinContent => Some(min_content()),
            MaxSize::MaxContent => Some(max_content()),
            MaxSize::FitContent if c.column => Some(content_height()),
            MaxSize::FitContent => item.intrinsic.keyword(Size::FitContent, room),
        };
        let max = max.unwrap_or(f32::INFINITY);
        // §4.5: the content-based minimum size, for an item that is not
        // a scroll container: the content size suggestion, its min-content
        // size, or the specified size suggestion, its main size, when that
        // is definite and smaller; never more than the maximum.
        let automatic = || {
            if style.overflow.clips() {
                return 0.0;
            }
            let content = min_content();
            definite(main_property)
                .map_or(content, |specified| specified.min(content))
                .min(max)
        };
        let min = match if c.column {
            style.min_height
        } else {
            style.min_width
        } {
            Size::Px(px) => inner(px),
            Size::Percent(p) => c.definite_main().map_or(0.0, |of| inner(of * p / 100.0)),
            Size::Auto => automatic(),
            Size::MinContent => min_content(),
            Size::MaxContent => max_content(),
            Size::FitContent if c.column => content_height(),
            Size::FitContent => item
                .intrinsic
                .keyword(Size::FitContent, room)
                .unwrap_or(0.0),
        };
        item.base = basis.unwrap_or_else(max_content);
        item.definite_basis = basis.is_some();
        item.min = min;
        item.max = max;
        item.hypothetical = item.base.min(max).max(min);
        item
    }

    /// The item's layout at the content width `width` and the height
    /// `imposed`, kept for the pass: its containing block is the
    /// container's content box.
    fn lay_out(
        &self,
        c: &Container,
        width: f32,
        imposed: Option<Imposed>,
        pass: &Pass<'a>,
    ) -> BlockLayout<'a> {
        self.lay_out_in(c, width, imposed, c.definite_height, pass)
    }

    /// As [`Item::lay_out`], against the containing-block height
    /// `cb_height` in place of the container's.
    fn lay_out_in(
        &self,
        c: &Container,
        width: f32,
        imposed: Option<Imposed>,
        cb_height: Option<f32>,
        pass: &Pass<'a>,
    ) -> BlockLayout<'a> {
        let key = ItemKey::of(self.b, width, imposed, c.width, cb_height);
        pass.item(key, || {
            layout_contents(
                self.b,
                &self.edges,
                c.width,
                cb_height,
                width,
                0.0,
                Content::Root,
                pass,
                None,
                imposed,
            )
        })
    }

    /// A column item's content width (§9.4 step 7): its own `width`;
    /// else `stretched`, the room its line leaves it, when it is
    /// stretched; else `fit-content` in the container's width less its
    /// margins, border and padding; clamped by `min-width` and
    /// `max-width`.
    fn column_width(&self, c: &Container, stretched: Option<f32>) -> f32 {
        let horizontal = self.edges.horizontal();
        let room = (c.width - self.cross_margins - horizontal).max(0.0);
        let w = width_constraints(&self.b.style, c.width, horizontal, &self.intrinsic, room);
        let width = w.width.or(stretched).unwrap_or_else(|| {
            self.intrinsic
                .keyword(Size::FitContent, room)
                .unwrap_or(room)
        });
        w.max.map_or(width, |max| width.min(max)).max(w.min)
    }
}

/// The alignment an item with style `item` takes in the cross axis: its
/// `align-self`, `auto` being the container's `align-items`.
fn align_self(item: &ComputedStyle, align_items: SelfAlignment) -> SelfAlignment {
    match item.align_self {
        SelfAlignment::Auto => align_items,
        own => own,
    }
}

/// Lays out the flex container `b`'s content at `content_width`, as
/// [`crate::block::layout_block_contents`] does a block container's,
/// at the height `imposed` on it if any: the items by css-flexbox-1 §9,
/// the absolutely positioned children carried up to their containing
/// block or settled here when the container is positioned. A flex
/// container is a formatting context root: its margins are its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_flex_contents<'a>(
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
    // height imposed, which is already clamped, or the container's own
    // `height` clamped by `min-height` and `max-height` — by a keyword
    // in them only once the content's height is known, as a block's is.
    // Percentages inside resolve against it only when it is definite
    // (§9.8).
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
    // A content height the container takes in an axis, given its
    // content's, `content`, the height `auto` would give it: the height
    // imposed; else its known height or `content`, clamped, a keyword
    // limit being `content`, its automatic height (css-sizing-3 §3.2).
    let fit = |content: f32| match known_height {
        Some(known) if imposed.is_some() => known,
        known => clamp_height(known.unwrap_or(content), bounds(Some(content))),
    };
    // The gaps (CSS Gaps 1 §2): `normal` is zero in a flex container,
    // and so is a percentage of an indefinite height, as in the
    // reference browsers.
    let gap = |gap: Option<LengthPercentage>, of: Option<f32>| match gap {
        None => 0.0,
        Some(LengthPercentage::Px(px)) => px,
        Some(LengthPercentage::Percent(p)) => of.map_or(0.0, |of| of * p / 100.0),
    };
    let column = style.flex_direction.is_column();
    let column_gap = gap(style.column_gap, Some(content_width));
    let row_gap = gap(style.row_gap, definite);
    let c = Container {
        column,
        reverse: style.flex_direction.is_reverse(),
        wrap_reverse: style.flex_wrap == FlexWrap::WrapReverse,
        width: content_width,
        definite_height: definite,
        main_gap: if column { row_gap } else { column_gap },
        cross_gap: if column { column_gap } else { row_gap },
        align_items: style.align_items,
    };
    let single_line = style.flex_wrap == FlexWrap::NoWrap;

    // §9.1: the items, in order-modified document order (§5.4), each
    // with its place among the children kept for the document order
    // the boxes they carry go up in.
    let mut children: Vec<(usize, &'a LayoutBox)> = b
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.kind != BoxKind::Absolute)
        .collect();
    children.sort_by_key(|(_, child)| child.style.order);
    let (indices, mut items): (Vec<usize>, Vec<Item<'a>>) = children
        .into_iter()
        .map(|(index, child)| (index, Item::new(child, &c, single_line, pass)))
        .unzip();

    // §9.2 step 4 and §9.3 step 5: the main size and the lines. A row
    // container's main size is its content width. A column container's
    // is its known height; else its items' outer hypothetical main sizes
    // on one line, which nothing breaks with infinite room, clamped by
    // `min-height` and `max-height` — and when a `max-height` made it
    // wrap, its longest line, clamped again, as the reference browsers
    // have it: its content's height laid out in that room. Its automatic
    // height, which a keyword limit is, is the same: the outer sizes on
    // one line, or the longest line.
    let gaps = |n: usize, gap: f32| gap * n.saturating_sub(1) as f32;
    let outer_sum = |items: &[Item<'_>]| {
        items
            .iter()
            .map(|item| item.hypothetical + item.main_extra)
            .sum::<f32>()
            + gaps(items.len(), c.main_gap)
    };
    let room = if column {
        fit(outer_sum(&items))
    } else {
        content_width
    };
    let lines = collect_lines(&items, room, c.main_gap, single_line);
    let longest = lines
        .iter()
        .map(|line| outer_sum(&items[line.clone()]))
        .fold(0.0, f32::max);
    let main_size = if column { fit(longest) } else { content_width };
    // §9.7: the items' main sizes on each line.
    for line in &lines {
        resolve_flexible_lengths(&mut items[line.clone()], main_size, c.main_gap);
    }

    // §9.4 step 7: each item's hypothetical cross size, a row item's
    // its height laid out at its main size with `height: auto` (its own
    // `height`, `min-height` and `max-height` applied), a column item's
    // its width.
    for item in &mut items {
        if column {
            item.width = item.column_width(&c, None);
            item.cross = item.width + item.edges.horizontal();
        } else {
            let layout = item.lay_out(&c, item.target, None, pass);
            item.cross = layout.height;
            item.auto_height = layout.auto_content_height;
        }
    }

    // §9.4 steps 8, 9 and 15: each line's cross size, the largest outer
    // hypothetical cross size on it, and the container's cross size: a
    // column container's its width, a row container's its known height,
    // else its lines' and the gaps between them clamped as a block's
    // height is. Its automatic height, which a keyword limit is, is the
    // lines' — a single line's being the known height, as the reference
    // browsers have it (step 8). A single-line container's line is as
    // large as the container; in a multi-line one, `align-content:
    // normal` and `stretch` share the free space among the lines (step
    // 16, done here so that the items stretch to their lines).
    let mut line_cross: Vec<f32> = lines
        .iter()
        .map(|line| {
            items[line.clone()]
                .iter()
                .map(|item| item.cross + item.cross_margins)
                .fold(0.0, f32::max)
        })
        .collect();
    let lines_sum = line_cross.iter().sum::<f32>() + gaps(lines.len(), c.cross_gap);
    let cross_size = if column {
        content_width
    } else {
        fit(known_height.filter(|_| single_line).unwrap_or(lines_sum))
    };
    if single_line {
        line_cross.fill(cross_size);
    } else if matches!(
        style.align_content,
        ContentAlignment::Normal | ContentAlignment::Stretch
    ) && cross_size > lines_sum
        && !line_cross.is_empty()
    {
        let share = (cross_size - lines_sum) / line_cross.len() as f32;
        for size in &mut line_cross {
            *size += share;
        }
    }

    // §9.4 step 11: a stretched item takes its line's cross size less
    // its margins, clamped by its own minimum and maximum: a row item's
    // height, imposed and definite, a keyword limit being its content's
    // height; a column item's width.
    for (line, &size) in lines.iter().zip(&line_cross) {
        for item in items[line.clone()].iter_mut().filter(|item| item.stretches) {
            let outer = (size - item.cross_margins).max(0.0);
            if column {
                let room = (outer - item.edges.horizontal()).max(0.0);
                item.width = item.column_width(&c, Some(room));
                item.cross = item.width + item.edges.horizontal();
            } else {
                let vertical = item.edges.vertical();
                let bounds = height_bounds(
                    &item.b.style,
                    c.definite_height,
                    vertical,
                    Some(item.auto_height),
                );
                item.cross = clamp_height((outer - vertical).max(0.0), bounds) + vertical;
                item.imposed_cross = Some(item.cross);
            }
        }
    }

    // §9.5 and §9.6, in their simplest form: the lines from
    // cross-start, the items on each from main-start, `*-reverse` and
    // `wrap-reverse` turned around at the end.
    let (main_start, main_end) = c.main_sides();
    let cross_start = c.cross_start();
    let mut line_position = 0.0f32;
    for (line, &size) in lines.iter().zip(&line_cross) {
        let mut position = 0.0f32;
        for item in &mut items[line.clone()] {
            position += item.margin[main_start];
            let border_main = item.target + item.main_edges;
            let main = if c.reverse {
                main_size - position - border_main
            } else {
                position
            };
            let flow_cross = line_position + item.margin[cross_start];
            let cross = if c.wrap_reverse {
                cross_size - flow_cross - item.cross
            } else {
                flow_cross
            };
            (item.x, item.y) = if column { (cross, main) } else { (main, cross) };
            position += border_main + item.margin[main_end] + c.main_gap;
        }
        line_position += size + c.cross_gap;
    }

    // Every item laid out at its used size, the kept layout when there
    // is one: a row item at its used width, with its stretched height
    // imposed; a column item at its used width with its used height
    // imposed, definite when the container's height or the item's basis
    // is (§9.8).
    let layouts: Vec<BlockLayout<'a>> = items
        .iter()
        .map(|item| {
            if column {
                let imposed = Imposed {
                    height: item.target + item.edges.vertical(),
                    definite: c.definite_height.is_some() || item.definite_basis,
                };
                item.lay_out(&c, item.width, Some(imposed), pass)
            } else {
                let imposed = item.imposed_cross.map(|height| Imposed {
                    height,
                    definite: true,
                });
                item.lay_out(&c, item.target, imposed, pass)
            }
        })
        .collect();

    let content_x = border[LEFT] + padding[LEFT];
    let content_y = border[TOP] + padding[TOP];
    let (first_baseline, last_baseline) = baselines(&items, &layouts, &lines, column);
    let content_height = if column { main_size } else { cross_size };
    let height = content_height + vertical_edges;
    let mut root = box_fragment(content_width + edges.horizontal(), height, b);

    // The items' fragments in order-modified document order, which
    // painting and hit testing follow; `position: relative` moves one
    // from where the algorithm put it, its containing block the
    // container's content box. The absolutely positioned boxes met in
    // them go up with them, and with the container's own, in document
    // order.
    let mut carried_up: Vec<Vec<Deferred<'a>>> = b.children.iter().map(|_| Vec::new()).collect();
    let mut content_bottom = 0.0f32;
    for ((item, layout), &index) in items.iter().zip(layouts).zip(&indices) {
        let (dx, dy) = relative_offset(&item.b.style, content_width, c.definite_height);
        let x = content_x + item.x + dx;
        let y = content_y + item.y + dy;
        content_bottom = content_bottom.max(item.y + layout.height + item.margin[BOTTOM]);
        carried_up[index] = carried(layout.absolutes, x, y);
        root.children.push(layout.root.translated(x, y));
    }
    let mut absolutes = Vec::new();
    for (child, carried) in b.children.iter().zip(carried_up) {
        if child.kind == BoxKind::Absolute {
            absolutes.push(Deferred {
                b: child,
                x: content_x,
                y: content_y,
            });
        } else {
            absolutes.extend(carried);
        }
    }
    let absolutes = settle_absolutes(b, &mut root, border, absolutes, pass);
    if style.overflow.clips() {
        // The scrollable overflow area (M10.5): the items' margin boxes
        // and anything inside them reaching lower, from the padding box's
        // top, then the bottom padding; never less than the padding box.
        let extent = root.children.iter().map(extent_bottom).fold(0.0, f32::max)
            - border[TOP]
            - padding[TOP];
        let padding_box = padding[TOP] + content_height + padding[BOTTOM];
        root.scroll_height =
            (padding[TOP] + content_bottom.max(extent) + padding[BOTTOM]).max(padding_box);
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
        first_baseline: first_baseline.map(|y| content_y + y),
        last_baseline: last_baseline.map(|y| content_y + y),
        absolutes,
        // The content as if `height` were `auto`: a row container's
        // lines, a column container's longest line.
        auto_content_height: if column { longest } else { lines_sum },
        held: Vec::new(),
    }
}

/// §9.3 step 5: the items in lines. A single-line container has one;
/// otherwise items are collected until the next one's outer hypothetical
/// main size, after a gap, would not fit in `main_size`, the first item
/// of a line going on it whatever its size.
fn collect_lines(
    items: &[Item<'_>],
    main_size: f32,
    gap: f32,
    single_line: bool,
) -> Vec<Range<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    if single_line {
        return std::iter::once(0..items.len()).collect();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0.0f32;
    for (i, item) in items.iter().enumerate() {
        let outer = item.hypothetical + item.main_extra;
        if i == start {
            used = outer;
        } else if used + gap + outer > main_size + TOLERANCE {
            lines.push(start..i);
            start = i;
            used = outer;
        } else {
            used += gap + outer;
        }
    }
    lines.push(start..items.len());
    lines
}

/// §9.7: the flexible lengths of one line's items, in a container whose
/// inner main size is `main_size`: each item's target main size, then
/// its used one. The free space, the factors and the sizes are summed in
/// `f64`, so that sums of large factors and sizes stay finite.
fn resolve_flexible_lengths(items: &mut [Item<'_>], main_size: f32, gap: f32) {
    let available = f64::from(main_size) - f64::from(gap) * items.len().saturating_sub(1) as f64;
    let outer = |item: &Item<'_>, size: f64| size + f64::from(item.main_extra);
    // 1. The flex factor: grow when the items' outer hypothetical main
    // sizes leave room, shrink otherwise.
    let used: f64 = items
        .iter()
        .map(|item| outer(item, f64::from(item.hypothetical)))
        .sum();
    let grow = used < available;
    let factor = |item: &Item<'_>| {
        f64::from(if grow {
            item.b.style.flex_grow
        } else {
            item.b.style.flex_shrink
        })
    };
    // 2. The inflexible items, frozen at their hypothetical main size:
    // a zero factor, or a base size already past it in the direction
    // the line flexes.
    let mut target: Vec<f64> = items
        .iter()
        .map(|item| f64::from(item.hypothetical))
        .collect();
    let mut frozen: Vec<bool> = items
        .iter()
        .map(|item| {
            factor(item) == 0.0
                || (grow && item.base > item.hypothetical)
                || (!grow && item.base < item.hypothetical)
        })
        .collect();
    // 3. The free space: the frozen items at their targets, the others
    // at their base sizes.
    let free = |target: &[f64], frozen: &[bool]| {
        available
            - items
                .iter()
                .zip(target)
                .zip(frozen)
                .map(|((item, &target), &frozen)| {
                    outer(item, if frozen { target } else { f64::from(item.base) })
                })
                .sum::<f64>()
    };
    let initial = free(&target, &frozen);
    // 4. The loop, which freezes at least one item each time round.
    while frozen.iter().any(|&frozen| !frozen) {
        // b. The remaining free space, no more than the factors' share
        // of the initial free space when they sum to less than one.
        let mut remaining = free(&target, &frozen);
        let unfrozen = || {
            items
                .iter()
                .zip(&frozen)
                .filter(|(_, frozen)| !**frozen)
                .map(|(item, _)| item)
        };
        let factors: f64 = unfrozen().map(factor).sum();
        if factors < 1.0 {
            let share = initial * factors;
            if share.abs() < remaining.abs() {
                remaining = share;
            }
        }
        // c. The remaining free space distributed: in proportion to the
        // grow factors, or taken away in proportion to the shrink
        // factors scaled by the inner base sizes.
        let scaled: f64 = unfrozen()
            .map(|item| factor(item) * f64::from(item.base))
            .sum();
        for (i, item) in items.iter().enumerate() {
            if frozen[i] {
                continue;
            }
            let base = f64::from(item.base);
            target[i] = if remaining == 0.0 {
                base
            } else if grow {
                base + remaining * factor(item) / factors
            } else if scaled > 0.0 {
                base - remaining.abs() * factor(item) * base / scaled
            } else {
                base
            };
        }
        // d. The min and max violations: each target clamped, its
        // content box never negative, and the clamps summed.
        let mut total = 0.0f64;
        let mut violation = vec![0.0f64; items.len()];
        for (i, item) in items.iter().enumerate() {
            if frozen[i] {
                continue;
            }
            let clamped = target[i]
                .min(f64::from(item.max))
                .max(f64::from(item.min))
                .max(0.0);
            violation[i] = clamped - target[i];
            total += violation[i];
            target[i] = clamped;
        }
        // e. Freeze: every item when the clamps cancel out, else the
        // items clamped in the direction of the total. A total that is
        // not a number, from sizes past the largest finite one, freezes
        // them all too: the loop must end.
        let cancel = total == 0.0 || total.is_nan();
        for i in 0..items.len() {
            if frozen[i] {
                continue;
            }
            frozen[i] = cancel
                || (total > 0.0 && violation[i] > 0.0)
                || (total < 0.0 && violation[i] < 0.0);
        }
    }
    // 5. The used main sizes, finite.
    for (item, target) in items.iter_mut().zip(target) {
        item.target = if target.is_nan() {
            0.0
        } else {
            target.clamp(0.0, f64::from(f32::MAX)) as f32
        };
    }
}

/// The container's first and last baselines from its content box's
/// top (§8.5): a row container's the first item on its first line, in
/// order-modified document order, that has a baseline, its last
/// baseline from its last line the same way from the end, each else
/// synthesized from the border box's bottom edge of the startmost or
/// endmost item on the line (css-align-3 §9.1); a column container's
/// its startmost item's and its endmost item's, synthesized the same
/// way. With no item, none.
fn baselines(
    items: &[Item<'_>],
    layouts: &[BlockLayout<'_>],
    lines: &[Range<usize>],
    column: bool,
) -> (Option<f32>, Option<f32>) {
    let bottom = |i: usize| items[i].y + layouts[i].height;
    let first = |i: usize| layouts[i].first_baseline.map(|b| items[i].y + b);
    let last = |i: usize| layouts[i].last_baseline.map(|b| items[i].y + b);
    if column {
        let n = items.len();
        return (
            (n > 0).then(|| first(0).unwrap_or_else(|| bottom(0))),
            (n > 0).then(|| last(n - 1).unwrap_or_else(|| bottom(n - 1))),
        );
    }
    let first_baseline = lines.first().map(|line| {
        line.clone()
            .find_map(first)
            .unwrap_or_else(|| bottom(line.start))
    });
    let last_baseline = lines.last().map(|line| {
        line.clone()
            .rev()
            .find_map(last)
            .unwrap_or_else(|| bottom(line.end - 1))
    });
    (first_baseline, last_baseline)
}

/// A flex container's min-content and max-content content widths
/// (§9.9.1, §9.9.2), by the Web-compatible algorithm: a row container's
/// max-content width is its items' max-content contributions and the
/// gaps between them; its min-content width the min-content
/// contributions so summed when it is single-line, the largest of them
/// when it wraps. A column container's are its items' largest
/// contributions, as a block container's are (a wrapping column
/// container's as a single-line one's, design note section 2). A
/// percentage gap is of the width being found, zero here (css-sizing-3
/// §5.2.1).
pub(crate) fn content_widths(b: &LayoutBox, fonts: &FontCollection) -> (f32, f32) {
    let style = &b.style;
    let items = b
        .children
        .iter()
        .filter(|child| child.kind != BoxKind::Absolute);
    if style.flex_direction.is_column() {
        return items
            .map(|item| crate::intrinsic::contribution(item, fonts))
            .fold((0.0, 0.0), |(min, max), (item_min, item_max)| {
                (min.max(item_min), max.max(item_max))
            });
    }
    let gap = match style.column_gap {
        Some(LengthPercentage::Px(px)) => px,
        _ => 0.0,
    };
    let (mut count, mut min_sum, mut max_sum, mut widest_min) = (0usize, 0.0f32, 0.0f32, 0.0f32);
    for item in items {
        let (min, max) = main_contribution(item, fonts);
        count += 1;
        min_sum += min;
        max_sum += max;
        widest_min = widest_min.max(min);
    }
    let gaps = gap * count.saturating_sub(1) as f32;
    let min = if style.flex_wrap == FlexWrap::NoWrap {
        min_sum + gaps
    } else {
        widest_min
    };
    (min.max(0.0), (max_sum + gaps).max(0.0))
}

/// A row container's item's outer min-content and max-content
/// contributions in the main axis (§9.9.3): the larger of its content
/// size and its preferred size when that is not `auto`, capped by its
/// flex base size when it cannot grow and floored by it when it cannot
/// shrink, then clamped by its minimum and maximum main sizes, the
/// automatic minimum among them, plus its border, padding and margins.
/// The base size is under the constraint being found: a definite basis,
/// else the content size. A percentage is of the width being found, so
/// `auto` for a size and zero for an edge, and `auto` margins are zero;
/// a table is never narrower than its minimum, as in a block.
fn main_contribution(item: &LayoutBox, fonts: &FontCollection) -> (f32, f32) {
    let style = &item.style;
    let (non_content, margins) = crate::intrinsic::fixed_edges(item);
    let inner = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    };
    let own = OnceCell::new();
    let own = || *own.get_or_init(|| crate::intrinsic::content_widths(item, fonts));
    // A size under the constraint: `max_content` or min-content.
    let size = |v: Size, max_content: bool| match v {
        Size::Px(px) => Some(inner(px)),
        Size::MinContent => Some(own().0),
        Size::MaxContent => Some(own().1),
        Size::FitContent if max_content => Some(own().1),
        Size::FitContent => Some(own().0),
        Size::Auto | Size::Percent(_) => None,
    };
    let limit = |v: MaxSize, max_content: bool| match v {
        MaxSize::Px(px) => Some(inner(px)),
        MaxSize::MinContent => size(Size::MinContent, max_content),
        MaxSize::MaxContent => size(Size::MaxContent, max_content),
        MaxSize::FitContent => size(Size::FitContent, max_content),
        MaxSize::None | MaxSize::Percent(_) => None,
    };
    let contribution = |max_content: bool| {
        let content = if max_content { own().1 } else { own().0 };
        let preferred = size(style.width, max_content);
        let base = match style.flex_basis {
            FlexBasis::Content => content,
            FlexBasis::Size(Size::Auto) => preferred.unwrap_or(content),
            FlexBasis::Size(basis) => size(basis, max_content).unwrap_or(content),
        };
        let mut v = preferred.map_or(content, |preferred| preferred.max(content));
        if style.flex_grow == 0.0 {
            v = v.min(base);
        }
        if style.flex_shrink == 0.0 {
            v = v.max(base);
        }
        let max = limit(style.max_width, max_content);
        let min = match style.min_width {
            Size::Auto if style.overflow.clips() => 0.0,
            Size::Auto => size(style.width, max_content)
                .map_or(own().0, |specified| specified.min(own().0))
                .min(max.unwrap_or(f32::INFINITY)),
            floor => size(floor, max_content).unwrap_or(0.0),
        };
        if let Some(max) = max {
            v = v.min(max);
        }
        v = v.max(min);
        if item.is_table() {
            v = v.max(own().0);
        }
        (v + non_content + margins).max(0.0)
    };
    (contribution(false), contribution(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Fragment, FragmentKind, layout_document};
    use crate::testing::{Laid, boxes_of};

    const VIEWPORT: f32 = 800.0;

    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The document `html` parses to, styled under `css` with the body's
    /// margin taken away, laid out against an 800px viewport.
    fn laid(html: &str, css: &str) -> Laid {
        let fonts = test_fonts();
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, VIEWPORT, &fonts);
        Laid { document, layout }
    }

    /// The element whose `id` is `id`.
    fn node(laid: &Laid, id: &str) -> dom::NodeId {
        let document = &laid.document;
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    /// The box fragment of the element whose `id` is `id`.
    fn fragment<'l>(laid: &'l Laid, id: &str) -> &'l Fragment {
        let node = node(laid, id);
        laid.iter()
            .find(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
            .unwrap_or_else(|| panic!("no box for #{id}"))
    }

    /// Its border box: `(x, y, w, h)`.
    fn rect(laid: &Laid, id: &str) -> (f32, f32, f32, f32) {
        let f = fragment(laid, id);
        (f.x, f.y, f.w, f.h)
    }

    #[test]
    fn free_space_is_shared_by_the_grow_factors() {
        // §9.7: 300px of free space, one part to a and three to b; b
        // stretches to the line, as tall as a.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; width: 400px }
             #a { width: 50px; height: 40px; flex-grow: 1 } #b { width: 50px; flex-grow: 3 }",
        );
        assert_eq!(rect(&l, "c"), (0.0, 0.0, 400.0, 40.0));
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 125.0, 40.0));
        assert_eq!(rect(&l, "b"), (125.0, 0.0, 275.0, 40.0));
        // Factors summing to less than one take that share of the free
        // space only.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; width: 400px } #a, #b { flex: 0.25 0 50px }",
        );
        assert_eq!(rect(&l, "a").2, 50.0 + 300.0 * 0.25);
        assert_eq!(rect(&l, "b").2, 50.0 + 300.0 * 0.25);
        // An item whose base size is past its maximum is frozen from the
        // start, so the initial free space, which such factors share,
        // counts it at its maximum: b takes a quarter of 300, not of 250.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; width: 400px } #c div { height: 10px }
             #a { flex: 0.5 0 100px; max-width: 50px } #b { flex: 0.25 0 50px }",
        );
        assert_eq!(rect(&l, "a").2, 50.0);
        assert_eq!(rect(&l, "b"), (50.0, 0.0, 125.0, 10.0));
    }

    #[test]
    fn negative_free_space_is_taken_by_the_scaled_shrink_factors() {
        // 650px of bases in 350px: d does not shrink; a and b give up the
        // 300px in proportion to their bases, 200 and 400.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div><div id=d></div></div>",
            "#c { display: flex; width: 350px; height: 10px }
             #a { flex: 0 1 200px } #b { flex: 0 1 400px } #d { flex: 0 0 50px }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 100.0, 10.0));
        assert_eq!(rect(&l, "b"), (100.0, 0.0, 200.0, 10.0));
        assert_eq!(rect(&l, "d"), (300.0, 0.0, 50.0, 10.0));
        // An item whose base size is below its minimum is frozen from the
        // start, so the initial free space counts it at its minimum: with
        // shrink factors summing to a half, b gives up half of 80, not of
        // 50.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; width: 100px } #c div { height: 10px }
             #a { flex: 0 0.5 50px; min-width: 80px } #b { flex: 0 0.5 100px }",
        );
        assert_eq!(rect(&l, "a").2, 80.0);
        assert_eq!(rect(&l, "b"), (80.0, 0.0, 60.0, 10.0));
    }

    #[test]
    fn a_clamped_item_is_frozen_and_the_rest_flex_again() {
        // Three items from nothing in 300px: b's maximum freezes it at
        // 50 and the other two share the 250 left; a minimum the same way
        // the other way.
        let html = "<div id=c><div id=a></div><div id=b></div><div id=d></div></div>";
        let l = laid(
            html,
            "#c { display: flex; width: 300px } #c div { flex: 1 1 0; height: 10px }
             #b { max-width: 50px }",
        );
        assert_eq!(rect(&l, "a").2, 125.0);
        assert_eq!((rect(&l, "b").0, rect(&l, "b").2), (125.0, 50.0));
        assert_eq!((rect(&l, "d").0, rect(&l, "d").2), (175.0, 125.0));
        let l = laid(
            html,
            "#c { display: flex; width: 300px } #c div { flex: 1 1 0; height: 10px }
             #b { min-width: 200px }",
        );
        assert_eq!(rect(&l, "a").2, 50.0);
        assert_eq!(rect(&l, "b").2, 200.0);
        assert_eq!(rect(&l, "d").2, 50.0);
    }

    #[test]
    fn the_automatic_minimum_keeps_an_items_content_whole() {
        // §4.5: a 120px piece of content in a 100px container. The item
        // is no narrower than its content, unless it is a scroll
        // container or its minimum is given.
        let html = "<div id=c><div id=a><span></span></div></div>";
        let css = "#c { display: flex; width: 100px }
                   span { display: inline-block; width: 120px; height: 10px }";
        let width = |extra: &str| rect(&laid(html, &format!("{css} {extra}")), "a").2;
        assert_eq!(width("#a { flex: 1 1 0 }"), 120.0);
        assert_eq!(width("#a { flex: 1 1 0; overflow: hidden }"), 100.0);
        assert_eq!(width("#a { flex: 1 1 0; min-width: 0 }"), 100.0);
        // A definite main size smaller than the content is the minimum
        // instead, and a maximum caps it.
        let css = "#c { display: flex; width: 50px }
                   span { display: inline-block; width: 120px; height: 10px }";
        let width = |extra: &str| rect(&laid(html, &format!("{css} {extra}")), "a").2;
        assert_eq!(width("#a { width: 80px }"), 80.0);
        assert_eq!(width("#a { width: 80px; max-width: 60px }"), 60.0);
        // A column item's minimum is its content's height.
        let l = laid(
            "<div id=c><div id=a><p></p></div><div id=b></div></div>",
            "#c { display: flex; flex-direction: column; height: 50px }
             p { margin: 0; height: 40px } #b { height: 40px }",
        );
        assert_eq!(rect(&l, "a").3, 40.0);
        assert_eq!(rect(&l, "b"), (0.0, 40.0, 800.0, 10.0));
    }

    #[test]
    fn a_column_items_content_is_measured_as_if_its_height_were_auto() {
        // The content size suggestion of an item whose own `height` is a
        // percentage of the container's: the percentage inside it is of
        // an indefinite height while it is measured, so the content is
        // empty and the item flexes to 90 (css-flexbox WPT
        // flex-minimum-height-flex-items-012).
        let l = laid(
            "<div id=c><div id=a></div><div id=b><div id=i></div></div></div>",
            "#c { display: flex; flex-direction: column; height: 100px; width: 100px }
             #a { height: 10px; flex: 0.1 }
             #b { height: 100%; display: flex; flex: 0.9 } #i { height: 100% }",
        );
        assert_eq!(rect(&l, "a").3, 10.0);
        assert_eq!(rect(&l, "b"), (0.0, 10.0, 100.0, 90.0));
        assert_eq!(rect(&l, "i").3, 90.0, "laid out at its flexed height");
    }

    #[test]
    fn a_keyword_height_limit_is_the_content_height() {
        // css-sizing-3 §3.2, as the reference browsers have it. A column
        // item's keyword limit is its content's height, 100 here: a
        // ceiling on its basis, a floor above its own `height`, which
        // `min-height: auto` would take.
        let html = "<div id=c><div id=a><p></p></div></div>";
        let css = "#c { display: flex; flex-direction: column; height: 300px }
                   p { margin: 0; height: 100px }";
        let height = |extra: &str| rect(&laid(html, &format!("{css} {extra}")), "a").3;
        assert_eq!(
            height("#a { flex: 0 0 200px; max-height: min-content }"),
            100.0
        );
        assert_eq!(height("#a { flex: 1; max-height: fit-content }"), 100.0);
        assert_eq!(
            height("#a { flex: 0 0 20px; height: 50px; min-height: max-content }"),
            100.0
        );
        assert_eq!(height("#a { flex: 0 0 20px; height: 50px }"), 50.0);
        // A stretched row item's is its content's height too.
        let css = "#c { display: flex; height: 300px } p { margin: 0; height: 40px }";
        let height = |extra: &str| rect(&laid(html, &format!("{css} {extra}")), "a").3;
        assert_eq!(height("#a { max-height: min-content }"), 40.0);
        assert_eq!(height(""), 300.0);
        assert_eq!(
            height("#c { align-items: start } #a { height: 10px; min-height: min-content }"),
            40.0
        );
        // A container's: a column one's is its items' hypothetical sizes,
        // into which they flex; a row one's its lines', a single line's
        // being the height it is given.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; flex-direction: column; height: 300px;
                  max-height: min-content }
             #a { flex: 1 0 50px } #b { height: 20px }",
        );
        assert_eq!(rect(&l, "c").3, 70.0);
        assert_eq!(rect(&l, "a").3, 50.0);
        let html = "<div id=c><div id=a><p></p></div><div id=b></div></div>";
        let css = "#c { display: flex; height: 300px; max-height: min-content; width: 100px }
                   #a, #b { width: 60px } p { margin: 0; height: 40px } #b { height: 30px }";
        assert_eq!(rect(&laid(html, css), "c").3, 300.0);
        let l = laid(html, &format!("{css} #c {{ flex-wrap: wrap }}"));
        assert_eq!(rect(&l, "c").3, 70.0);
        assert_eq!(rect(&l, "a").3, 40.0, "nothing left to stretch into");
    }

    #[test]
    fn a_tables_own_height_is_no_second_minimum_once_flexed() {
        // The flex layout applies the table's `height` once, as its
        // basis; the flexed size then stands, but for the rows, which
        // the automatic minimum keeps whole anyway (css-flexbox WPT
        // table-as-item-specified-height).
        let html = "<div id=c><div id=t><div id=r><div id=d></div></div></div></div>";
        let css = "#c { display: flex; flex-direction: column; width: 100px }
                   #t { display: table; width: 100px; height: 500px; flex: 0 0 100px }
                   #r { display: table-row } #d { display: table-cell; padding: 0 }";
        assert_eq!(rect(&laid(html, css), "t").3, 100.0);
        let l = laid(
            html,
            &format!("{css} #t {{ flex-basis: 10px }} #d {{ height: 40px }}"),
        );
        assert_eq!(rect(&l, "t").3, 40.0);
    }

    #[test]
    fn flex_one_starts_from_nothing_and_flex_auto_from_the_content() {
        // `flex: 1` is a basis of 0%; `flex: auto` the content's width.
        let html = "<div id=c><div id=a><span class=w></span></div><div id=b><span class=n></span></div></div>";
        let css = "#c { display: flex; width: 400px }
                   span { display: inline-block; height: 10px } .w { width: 100px } .n { width: 20px }";
        let l = laid(html, &format!("{css} #a, #b {{ flex: 1 }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").2), (200.0, 200.0));
        let l = laid(html, &format!("{css} #a, #b {{ flex: auto }}"));
        assert_eq!((rect(&l, "a").2, rect(&l, "b").2), (240.0, 160.0));
        // A percentage of an indefinite main size is the content: a
        // column container with no height.
        let l = laid(
            "<div id=c><div id=a><p></p></div></div>",
            "#c { display: flex; flex-direction: column } #a { flex-basis: 50% }
             p { margin: 0; height: 30px }",
        );
        assert_eq!(rect(&l, "a").3, 30.0);
        let l = laid(
            "<div id=c><div id=a><p></p></div></div>",
            "#c { display: flex; flex-direction: column; height: 200px } #a { flex-basis: 50% }
             p { margin: 0; height: 30px }",
        );
        assert_eq!(rect(&l, "a").3, 100.0);
    }

    #[test]
    fn a_column_container_stacks_its_items_and_is_as_tall_as_they_are() {
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; flex-direction: column; width: 200px }
             #a { height: 20px; margin: 5px 0 } #b { height: 30px; width: 50px }",
        );
        // a stretches across, b keeps its width; margins do not collapse.
        assert_eq!(rect(&l, "a"), (0.0, 5.0, 200.0, 20.0));
        assert_eq!(rect(&l, "b"), (0.0, 30.0, 50.0, 30.0));
        assert_eq!(rect(&l, "c"), (0.0, 0.0, 200.0, 60.0));
    }

    #[test]
    fn a_column_item_is_measured_at_the_width_it_is_laid_out_at() {
        // Stretched in a single-line container, the item is the
        // container's width, 400, and its half-width child holds both
        // pieces on one line; measured at its `fit-content` width it
        // would have taken two.
        let l = laid(
            "<div id=c><div id=a><div id=k><span></span><span></span></div></div></div>",
            "#c { display: flex; flex-direction: column; width: 400px } #k { width: 50% }
             span { display: inline-block; width: 90px; height: 10px; vertical-align: top }",
        );
        assert_eq!(rect(&l, "k").2, 200.0);
        assert_eq!(rect(&l, "a").3, rect(&l, "k").3);
        let one_line = rect(&l, "k").3;
        let l = laid(
            "<div id=c><div id=a><div id=k><span></span><span></span></div></div></div>",
            "#c { display: flex; flex-direction: column; width: 100px } #k { width: 50% }
             span { display: inline-block; width: 90px; height: 10px; vertical-align: top }",
        );
        assert!(rect(&l, "a").3 > one_line, "two lines in a 50px child");
        // In a multi-line container an item is measured at its
        // `fit-content` width and stretched to its line after: three
        // lines of 100, 30 and 30, 240px over, 80 more each.
        let l = laid(
            "<div id=c><div id=a><span class=w></span></div><div id=b><span class=n></span></div><div id=d><span class=n></span></div></div>",
            "#c { display: flex; flex-flow: column wrap; height: 30px; width: 400px }
             #c div { height: 20px } span { display: block; height: 20px }
             .w { width: 100px } .n { width: 30px }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 180.0, 20.0));
        assert_eq!(rect(&l, "b"), (180.0, 0.0, 110.0, 20.0));
        assert_eq!(rect(&l, "d"), (290.0, 0.0, 110.0, 20.0));
    }

    #[test]
    fn a_column_containers_height_is_flexed_and_definite_inside() {
        // §9.8: with a definite height the flexed heights are definite,
        // so a percentage inside resolves against them.
        let l = laid(
            "<div id=c><div id=a></div><div id=b><div id=p></div></div></div>",
            "#c { display: flex; flex-direction: column; height: 300px; width: 100px }
             #a { flex: 1 } #b { flex: 2 } #p { height: 50% }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 100.0, 100.0));
        assert_eq!(rect(&l, "b"), (0.0, 100.0, 100.0, 200.0));
        assert_eq!(rect(&l, "p").3, 100.0);
        // Without one, a flexed height is definite only from a definite
        // basis.
        let html = "<div id=c><div id=b><div id=p></div><div id=q></div></div></div>";
        let css = "#c { display: flex; flex-direction: column } #p { height: 50% }
                   #q { height: 40px }";
        let l = laid(html, &format!("{css} #b {{ flex: 1 0 auto }}"));
        assert_eq!((rect(&l, "b").3, rect(&l, "p").3), (40.0, 0.0));
        let l = laid(html, &format!("{css} #b {{ flex: 1 0 60px }}"));
        assert_eq!((rect(&l, "b").3, rect(&l, "p").3), (60.0, 30.0));
    }

    #[test]
    fn a_wrapping_column_container_wraps_at_its_maximum_height() {
        // Three 40px items in at most 100px: two lines, which share the
        // container's width between them (`align-content: normal`), and
        // the container as tall as the longer.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div><div id=d></div></div>",
            "#c { display: flex; flex-flow: column wrap; max-height: 100px; width: 300px }
             #c div { height: 40px; width: 50px }",
        );
        assert_eq!(rect(&l, "c").3, 80.0, "its longest line");
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 50.0, 40.0));
        assert_eq!(rect(&l, "b"), (0.0, 40.0, 50.0, 40.0));
        assert_eq!(rect(&l, "d"), (150.0, 0.0, 50.0, 40.0));
    }

    #[test]
    fn lines_share_the_free_cross_space_and_their_items_stretch_to_them() {
        let html = "<div id=c><div id=a></div><div id=b></div><div id=d></div></div>";
        let css = "#c { display: flex; flex-wrap: wrap; width: 200px; height: 100px }
                   #c div { width: 80px } #a { height: 20px }";
        // Lines of 20 and 0, and 80px over: 40 more each.
        let l = laid(html, css);
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 80.0, 20.0));
        assert_eq!(rect(&l, "b"), (80.0, 0.0, 80.0, 60.0));
        assert_eq!(rect(&l, "d"), (0.0, 60.0, 80.0, 40.0));
        // Packed at the start, the lines keep their own sizes.
        let l = laid(html, &format!("{css} #c {{ align-content: flex-start }}"));
        assert_eq!(rect(&l, "b").3, 20.0);
        assert_eq!(rect(&l, "d"), (0.0, 20.0, 80.0, 0.0));
        // An item aligned other than by stretching keeps its height.
        let l = laid(html, &format!("{css} #b {{ align-self: flex-start }}"));
        assert_eq!(rect(&l, "b").3, 0.0);
    }

    #[test]
    fn order_reorders_the_items_and_their_fragments() {
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div><div id=d></div></div>",
            "#c { display: flex } #c div { width: 10px; height: 10px }
             #a { order: 2 } #d { order: -1 }",
        );
        assert_eq!(rect(&l, "d").0, 0.0);
        assert_eq!(rect(&l, "b").0, 10.0);
        assert_eq!(rect(&l, "a").0, 20.0);
        let ids: Vec<_> = fragment(&l, "c")
            .children
            .iter()
            .map(|f| {
                l.document
                    .attr(f.node, "id")
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(ids, ["d", "b", "a"]);
    }

    #[test]
    fn the_reverse_directions_turn_the_axes_around() {
        let html = "<div id=c><div id=a></div><div id=b></div></div>";
        let l = laid(
            html,
            "#c { display: flex; flex-direction: row-reverse; width: 300px }
             #c div { width: 50px; height: 10px }",
        );
        assert_eq!(rect(&l, "a").0, 250.0);
        assert_eq!(rect(&l, "b").0, 200.0);
        let l = laid(
            html,
            "#c { display: flex; flex-direction: column-reverse; height: 100px }
             #c div { height: 30px }",
        );
        assert_eq!(rect(&l, "a").1, 70.0);
        assert_eq!(rect(&l, "b").1, 40.0);
        // `wrap-reverse`: the first line at the bottom.
        let l = laid(
            html,
            "#c { display: flex; flex-wrap: wrap-reverse; width: 100px }
             #c div { width: 60px } #a { height: 10px } #b { height: 20px }",
        );
        assert_eq!(rect(&l, "c").3, 30.0);
        assert_eq!(rect(&l, "a").1, 20.0);
        assert_eq!(rect(&l, "b").1, 0.0);
    }

    #[test]
    fn gaps_separate_the_items_and_the_lines() {
        let html =
            "<div id=c><div id=a></div><div id=b></div><div id=d></div><div id=e></div></div>";
        let l = laid(
            html,
            "#c { display: flex; flex-wrap: wrap; width: 100px; gap: 10px 5px }
             #c div { width: 30px; height: 20px }",
        );
        assert_eq!(rect(&l, "b").0, 35.0);
        assert_eq!(rect(&l, "d").0, 70.0);
        assert_eq!((rect(&l, "e").0, rect(&l, "e").1), (0.0, 30.0));
        assert_eq!(rect(&l, "c").3, 50.0);
        // A percentage of the width; of an indefinite height, nothing.
        let l = laid(
            html,
            "#c { display: flex; flex-wrap: wrap; width: 100px; gap: 10% 10% }
             #c div { width: 25px; height: 20px }",
        );
        // Three fit in 25 * 3 + 10 * 2 = 95; the fourth goes on a line
        // right below, the row gap being nothing.
        assert_eq!(rect(&l, "b").0, 35.0);
        assert_eq!((rect(&l, "e").0, rect(&l, "e").1), (0.0, 20.0));
    }

    #[test]
    fn items_margins_are_their_own_and_relative_offsets_move_them() {
        let l = laid(
            "<div id=x></div><div id=c><div id=a></div><div id=b></div></div>",
            "#x { margin-bottom: 15px } #c { display: flex; margin-top: 10px }
             #a { margin: 20px 10px 5px; width: 30px; height: 10px; position: relative; left: 4px; top: 3px }
             #b { width: 30px }",
        );
        // The container's margin collapses with its sibling's, not with
        // its items'.
        assert_eq!(rect(&l, "c").1, 15.0);
        assert_eq!(rect(&l, "a"), (14.0, 15.0 + 20.0 + 3.0, 30.0, 10.0));
        assert_eq!(rect(&l, "b"), (50.0, 15.0, 30.0, 35.0));
        assert_eq!(rect(&l, "c").3, 35.0);
    }

    #[test]
    fn an_empty_flex_container_is_its_own_height() {
        let l = laid("<div id=c></div>", "#c { display: flex }");
        assert_eq!(rect(&l, "c"), (0.0, 0.0, 800.0, 0.0));
        let l = laid(
            "<div id=c> </div>",
            "#c { display: flex; min-height: 20px }",
        );
        assert_eq!(rect(&l, "c"), (0.0, 0.0, 800.0, 20.0));
    }

    #[test]
    fn an_items_floats_stay_inside_it() {
        // An item is a formatting context root (§4).
        let l = laid(
            "<div id=c><div id=a><div id=f></div></div></div><p id=p></p>",
            "#c { display: flex } #f { float: left; width: 20px; height: 40px }
             #p { margin: 0; height: 5px }",
        );
        assert_eq!(rect(&l, "a"), (0.0, 0.0, 20.0, 40.0));
        assert_eq!(rect(&l, "p").1, 40.0);
    }

    #[test]
    fn an_item_stretches_to_a_single_lines_container_unless_it_cannot() {
        // A single-line container's line is as tall as the container,
        // its own height or its minimum; an item stretches to it within
        // its own maximum, and not at all with an `auto` cross margin or
        // a height of its own.
        let html =
            "<div id=c><div id=a></div><div id=b></div><div id=d></div><div id=e></div></div>";
        let css = "#c { display: flex } #c div { width: 10px }
                   #b { max-height: 30px } #d { margin-top: auto } #e { height: 5px }";
        for container in ["#c { height: 100px }", "#c { min-height: 100px }"] {
            let l = laid(html, &format!("{css} {container}"));
            assert_eq!(rect(&l, "c").3, 100.0, "{container}");
            assert_eq!(rect(&l, "a").3, 100.0, "{container}");
            assert_eq!(rect(&l, "b").3, 30.0, "{container}");
            assert_eq!(rect(&l, "d").3, 0.0, "{container}");
            assert_eq!(rect(&l, "e").3, 5.0, "{container}");
        }
    }

    #[test]
    fn a_stretched_items_height_is_definite_for_the_percentages_inside() {
        let l = laid(
            "<div id=c><div id=a><div id=p></div></div><div id=b></div></div>",
            "#c { display: flex } #b { height: 80px } #p { height: 50% }",
        );
        assert_eq!(rect(&l, "a").3, 80.0);
        assert_eq!(rect(&l, "p").3, 40.0);
        // A table stretched grows its rows to the height.
        let l = laid(
            "<div id=c><table id=t><tr><td id=d></td></tr></table><div id=b></div></div>",
            "#c { display: flex } #b { height: 100px } td { padding: 0 }
             table { border-spacing: 0 }",
        );
        assert_eq!(rect(&l, "t").3, 100.0);
        assert_eq!(rect(&l, "d").3, 100.0);
    }

    #[test]
    fn an_absolutely_positioned_child_is_no_item() {
        // It takes no room among the items; its static position is the
        // content box's corner, and a positioned container is its
        // containing block.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; position: relative; padding: 10px; width: 200px }
             #a { position: absolute; width: 10px; height: 10px }
             #b { width: 30px; height: 30px }",
        );
        assert_eq!(rect(&l, "a"), (10.0, 10.0, 10.0, 10.0));
        assert_eq!(rect(&l, "b"), (10.0, 10.0, 30.0, 30.0));
        assert_eq!(rect(&l, "c").3, 50.0);
        let c = fragment(&l, "c");
        assert_eq!(c.children.len(), 2);
        assert_eq!(c.children[1].node, node(&l, "a"), "after the items");
    }

    #[test]
    fn a_flex_scroll_container_records_its_overflow() {
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; flex-direction: column; height: 50px; overflow: hidden;
                  padding: 5px }
             #c div { height: 40px; flex-shrink: 0 }",
        );
        let c = fragment(&l, "c");
        assert_eq!(c.h, 60.0);
        assert_eq!(c.scroll_height, 5.0 + 80.0 + 5.0);
        // An item's bottom margin is in the area too.
        let l = laid(
            "<div id=c><div id=a></div></div>",
            "#c { display: flex; height: 50px; overflow: hidden }
             #a { height: 80px; margin-bottom: 10px; width: 10px }",
        );
        assert_eq!(fragment(&l, "c").scroll_height, 90.0);
    }

    #[test]
    fn a_floated_or_inline_flex_container_shrinks_to_its_items() {
        // The max-content width of a row is its items' and its gaps.
        let l = laid(
            "<div id=c><div id=a></div><div id=b></div></div>",
            "#c { display: flex; float: left; column-gap: 10px }
             #a { width: 30px; height: 5px } #b { width: 50px }",
        );
        assert_eq!(rect(&l, "c"), (0.0, 0.0, 90.0, 5.0));
        // An inline flex container sits on its line by its first item's
        // baseline.
        let l = laid(
            "<p id=p>x<span id=s><i id=i>y</i></span></p>",
            "p { margin: 0 } #s { display: inline-flex } #i { width: 50px; padding-top: 7px }",
        );
        assert_eq!(rect(&l, "s").2, 50.0);
        let text = |l: &Laid, t: &str| {
            l.iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(s) if s == t))
                .unwrap_or_else(|| panic!("text {t}"))
                .y
        };
        assert_eq!(text(&l, "x"), text(&l, "y"));
        // A column one by its first item's first baseline, not its last
        // item's.
        let l = laid(
            "<p id=p>x<span id=s><i>y</i><i style='padding-top: 20px'>z</i></span></p>",
            "p { margin: 0 } #s { display: inline-flex; flex-direction: column }",
        );
        assert_eq!(text(&l, "x"), text(&l, "y"));
        assert_eq!(rect(&l, "s").1, 0.0);
    }

    #[test]
    fn a_flex_container_beside_a_float_keeps_clear_of_it() {
        // A formatting context root (CSS2 §9.5, M11).
        let l = laid(
            "<div id=f></div><div id=c><div id=a></div></div>",
            "#f { float: left; width: 100px; height: 50px } #c { display: flex }
             #a { height: 10px }",
        );
        assert_eq!(rect(&l, "c"), (100.0, 0.0, 700.0, 10.0));
    }

    #[test]
    fn a_flex_containers_intrinsic_widths_are_its_items() {
        // §9.9: a row's widths sum its items' contributions and its
        // gaps, a wrapping row's minimum is the widest item's, a
        // column's are the widest.
        let fonts = test_fonts();
        let widths = |css: &str| {
            let doc = html::parse_document(
                "<div><p><span class=a></span></p><p><span class=b></span></p></div>",
            )
            .expect("html parsing never fails");
            let sheet = css::parse_stylesheet(&format!(
                "div {{ display: flex; gap: 10px }} p {{ margin: 0 }}
                 span {{ display: inline-block }} .a {{ width: 30px }} .b {{ width: 50px }} {css}"
            ))
            .expect("test css is valid");
            let (_, boxes) = boxes_of(&doc, &[sheet]);
            // html > body > div
            content_widths(&boxes[0].children[0].children[0], &fonts)
        };
        assert_eq!(widths(""), (90.0, 90.0));
        assert_eq!(widths("div { flex-wrap: wrap }"), (50.0, 90.0));
        assert_eq!(widths("div { flex-direction: column }"), (50.0, 50.0));
        // An item that cannot grow is capped by a definite basis, one
        // that cannot shrink floored by it; the automatic minimum still
        // holds the content whole.
        assert_eq!(widths("p { flex: 0 0 100px }"), (210.0, 210.0));
        assert_eq!(widths("p { flex: 0 1 10px }"), (90.0, 90.0));
        assert_eq!(widths("p { flex: 0 1 10px; min-width: 0 }"), (30.0, 30.0));
        assert_eq!(widths("p { flex: 1 1 100px }"), (90.0, 90.0));
    }

    #[test]
    fn the_flexing_loop_ends_whatever_sizes_it_is_given() {
        // The cascade keeps lengths far inside f32's range, but the loop
        // must end on anything: an infinite base size makes the targets
        // not a number, which freezes every item, and the sizes stay
        // finite.
        let fonts = test_fonts();
        let doc =
            html::parse_document("<div><p></p><p></p></div>").expect("html parsing never fails");
        let sheet = css::parse_stylesheet("div { display: flex }").expect("test css is valid");
        let (_, boxes) = boxes_of(&doc, &[sheet]);
        let flex = &boxes[0].children[0].children[0];
        let pass = Pass::new(&fonts);
        let c = Container {
            column: false,
            reverse: false,
            wrap_reverse: false,
            width: 100.0,
            definite_height: None,
            main_gap: 0.0,
            cross_gap: 0.0,
            align_items: SelfAlignment::Normal,
        };
        let mut items: Vec<Item<'_>> = flex
            .children
            .iter()
            .map(|b| Item::new(b, &c, true, &pass))
            .collect();
        items[0].base = f32::INFINITY;
        items[0].hypothetical = f32::INFINITY;
        resolve_flexible_lengths(&mut items, 100.0, 0.0);
        assert!(items.iter().all(|item| item.target.is_finite()));
    }

    #[test]
    fn a_chain_of_nested_flex_containers_lays_each_item_out_a_few_times() {
        // The pass keeps every item's layouts (M17): the chain costs a
        // constant number of layouts per level, not two to the power of
        // its depth.
        let fonts = test_fonts();
        let computed = |depth: usize| {
            let mut html = String::new();
            for i in 0..depth {
                let direction = if i % 2 == 0 { "row" } else { "column" };
                html.push_str(&format!(
                    "<div style='display: flex; flex-direction: {direction}'>"
                ));
            }
            html.push('x');
            html.push_str(&"</div>".repeat(depth));
            let doc = html::parse_document(&html).expect("html parsing never fails");
            let (_, boxes) = boxes_of(&doc, &[]);
            let root = &boxes[0].children[0].children[0];
            let pass = Pass::new(&fonts);
            let edges = Edges::of_box(root, VIEWPORT);
            let layout = layout_contents(
                root,
                &edges,
                VIEWPORT,
                None,
                VIEWPORT,
                0.0,
                Content::Root,
                &pass,
                None,
                None,
            );
            assert!(layout.height > 0.0);
            pass.computed()
        };
        // Two or three layouts a level: measured or laid out at its
        // hypothetical size, then at its used size.
        let (shallow, deep) = (computed(10), computed(40));
        assert!(deep <= 3 * 40, "{deep} layouts for 40 levels");
        assert!(deep - shallow <= 3 * 30, "{shallow} for 10, {deep} for 40");
    }

    #[test]
    fn a_maximum_on_either_container_makes_a_definite_height_inside() {
        // css-flexbox WPT flexbox-definite-sizes-003 and -004, which the
        // subset cannot check for want of the body's overflow carried to
        // the viewport: a nested container stretched to a line that the
        // outer container's `max-height` clamps, or clamped by its own,
        // is a definite height for the percentage in its item.
        for (outer, inner) in [("max-height: 100px", ""), ("", "max-height: 100px")] {
            let l = laid(
                "<div id=o><div id=i><div id=b><div id=c></div></div></div></div>",
                &format!(
                    "#o {{ display: flex; width: 100px; {outer} }}
                     #i {{ display: flex; width: 100px; {inner} }}
                     #b {{ width: 100px; max-height: 100% }} #c {{ height: 9999px }}"
                ),
            );
            assert_eq!(rect(&l, "o").3, 100.0, "{outer}{inner}");
            assert_eq!(rect(&l, "i").3, 100.0, "{outer}{inner}");
            assert_eq!(rect(&l, "b").3, 100.0, "{outer}{inner}");
        }
    }
}
