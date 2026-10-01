//! The paint order (M11.6, step 2 of `docs/design/floats-positioning.md`):
//! one walk over the fragment tree that says what is painted, where, in
//! which order and under which clip. [`paint_fragments_in`](crate::
//! paint_fragments_in) turns the steps into display items, and the
//! engine's hit test reads the same steps — the last one under the
//! pointer is the element hit — so paint order and hit order have one
//! definition. Until stacking contexts land the order is tree order,
//! which is CSS2 Appendix E's for content in normal flow.
//!
//! Every rectangle a step carries is in viewport coordinates: the
//! viewport's scroll offset and, inside a scroll container, its offset
//! (M10.5) are already applied, and so is the shift of a `position:
//! sticky` box ([`sticky_shift`], M11), which paint and hit testing
//! thereby agree on. A clip's rectangle is the effective one,
//! intersected with the clips around it, so a consumer needs only the
//! innermost. Content the viewport cannot show is left out: a box or a
//! line it does not meet has no step, and a clipping box it does not
//! meet is skipped with its whole subtree, since nothing under a clip
//! can show where the clip does not.

use layout::{Fragment, FragmentKind};
use style::computed::{BOTTOM, ComputedStyle, LEFT, LengthPercentageAuto, Position, RIGHT, TOP};

use crate::{ScrollOffsets, Viewport};

/// An axis-aligned rectangle in viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Whether the point is inside: the left and top edges are, the
    /// right and bottom ones are not, as `Fragment::contains` has it.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Whether the two rectangles share any area.
    pub fn meets(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    /// The common part of the two, empty (never negative) when they do
    /// not meet.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        Rect {
            x,
            y,
            w: (right - x).max(0.0),
            h: (bottom - y).max(0.0),
        }
    }
}

/// One step of the paint order.
#[derive(Debug, Clone, Copy)]
pub enum Step<'a> {
    /// A box's own background and border, over its border box `rect`.
    Box { fragment: &'a Fragment, rect: Rect },
    /// A text run: `rect` is its line box, `baseline` where its glyphs
    /// sit.
    Text {
        fragment: &'a Fragment,
        rect: Rect,
        baseline: f32,
    },
    /// Everything up to the matching [`Step::PopClip`] shows only inside
    /// `rect`, the padding box of the clipping box `node` intersected
    /// with the clips around it (CSS2 §11.1.1). The clipping box's
    /// content is scrolled by its offset; the box, its clip and its
    /// scrollbar are not.
    PushClip { node: usize, rect: Rect },
    /// The end of the innermost [`Step::PushClip`].
    PopClip,
    /// A scroll container's place for its overlay scrollbar (M10.5):
    /// over its padding box `rect`, scrolled to `offset`. An indicator,
    /// not a control: hit testing passes over it.
    Scrollbar {
        fragment: &'a Fragment,
        rect: Rect,
        offset: f32,
    },
}

/// The paint order of `roots` for `viewport`, with each scroll
/// container scrolled as `offsets` says.
pub fn paint_order<'a>(
    roots: &'a [Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
) -> Vec<Step<'a>> {
    let mut walk = Walk {
        steps: Vec::new(),
        viewport: Rect {
            x: 0.0,
            y: 0.0,
            w: viewport.width,
            h: viewport.height,
        },
        dx: 0.0,
        dy: -viewport.scroll_y,
        clip: None,
        offsets,
    };
    // The roots' containing block is the initial one: as wide as the
    // viewport, from the document's top down without end.
    let initial = Rect {
        x: 0.0,
        y: -viewport.scroll_y,
        w: viewport.width,
        h: f32::INFINITY,
    };
    walk.all(roots, initial);
    walk.steps
}

/// The walk's state: where the document is relative to the viewport,
/// and the clip in force.
struct Walk<'a, 'o> {
    steps: Vec<Step<'a>>,
    viewport: Rect,
    /// From a document `x` to the viewport's: the shifts of the sticky
    /// boxes being walked into (M11).
    dx: f32,
    /// From a document `y` to the viewport's: the viewport's scroll
    /// offset and the offsets of the scroll containers being walked
    /// into, negated, and the shifts of the sticky boxes.
    dy: f32,
    /// The effective clip in force, if any.
    clip: Option<Rect>,
    offsets: &'o dyn ScrollOffsets,
}

impl<'a> Walk<'a, '_> {
    /// Walks `fragments`, children of a box whose padding box is
    /// `containing` in viewport coordinates: what a sticky one among
    /// them is kept inside.
    fn all(&mut self, fragments: &'a [Fragment], containing: Rect) {
        for f in fragments {
            self.one(f, containing);
        }
    }

    /// A fragment's box in viewport coordinates.
    fn rect_of(&self, f: &Fragment) -> Rect {
        Rect {
            x: f.x + self.dx,
            y: f.top() + self.dy,
            w: f.w,
            h: f.h,
        }
    }

    fn one(&mut self, f: &'a Fragment, containing: Rect) {
        // A `position: fixed` box (M11), a root of the document's
        // fragments by layout, is placed against the viewport and does
        // not scroll with the document: its subtree is walked with the
        // viewport's offset left out.
        if f.style.position == Position::Fixed {
            let scrolled = self.dy;
            self.dy = 0.0;
            self.one_in_place(f, containing);
            self.dy = scrolled;
        } else {
            self.one_in_place(f, containing);
        }
    }

    fn one_in_place(&mut self, f: &'a Fragment, containing: Rect) {
        match &f.kind {
            FragmentKind::Box => {
                // A `position: sticky` box (M11) is shifted, its subtree
                // with it, to keep its insets from the edges of the
                // visible part of its scrollport — the innermost clip,
                // else the viewport — within its containing block.
                let (sx, sy) = if f.style.position == Position::Sticky {
                    let scrollport = self.clip.unwrap_or(self.viewport);
                    sticky_shift(&f.style, self.rect_of(f), containing, scrollport)
                } else {
                    (0.0, 0.0)
                };
                self.dx += sx;
                self.dy += sy;
                self.box_in_place(f);
                self.dx -= sx;
                self.dy -= sy;
            }
            FragmentKind::Text(_) => {
                let rect = self.rect_of(f);
                if rect.meets(&self.viewport) {
                    self.steps.push(Step::Text {
                        fragment: f,
                        rect,
                        baseline: f.y + self.dy,
                    });
                }
            }
        }
    }

    /// A box's steps, with the shifts in force.
    fn box_in_place(&mut self, f: &'a Fragment) {
        let rect = self.rect_of(f);
        let clips = f.style.overflow.clips();
        let in_view = rect.meets(&self.viewport);
        if clips && !in_view {
            return;
        }
        if in_view {
            self.steps.push(Step::Box { fragment: f, rect });
        }
        let padding_box = padding_box(f, rect);
        if !clips {
            self.all(&f.children, padding_box);
            return;
        }
        let effective = match self.clip {
            Some(outer) => outer.intersect(&padding_box),
            None => padding_box,
        };
        self.steps.push(Step::PushClip {
            node: f.node,
            rect: effective,
        });
        let outer_clip = self.clip.replace(effective);
        let offset = self.offsets.offset(f.node);
        self.dy -= offset;
        self.all(&f.children, padding_box);
        self.dy += offset;
        self.clip = outer_clip;
        self.steps.push(Step::Scrollbar {
            fragment: f,
            rect: padding_box,
            offset,
        });
        self.steps.push(Step::PopClip);
    }
}

/// The shift a `position: sticky` box gets (css-position-3 §3.4, M11):
/// as little as keeps each of its insets that is not `auto` — `top`,
/// `right`, `bottom`, `left`, a percentage of the scrollport's size —
/// between its border box `rect` and the matching edge of
/// `scrollport`, the visible part of its nearest scroll container (the
/// viewport, without one), and never past `containing`, its containing
/// block, so that the box scrolls away with it. A box pushed one way
/// by its start inset is not pulled back by its end inset: the start
/// edge wins. All rectangles are in viewport coordinates. The
/// containing rectangle taken is the parent's padding box, and it is
/// the border box, not the margin box, that is kept inside it: stated
/// approximations.
pub fn sticky_shift(
    style: &ComputedStyle,
    rect: Rect,
    containing: Rect,
    scrollport: Rect,
) -> (f32, f32) {
    let inset = |side: usize, of: f32| match style.inset[side] {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(px),
        LengthPercentageAuto::Percent(p) => Some(of * p / 100.0),
    };
    // Along one axis: the box from `pos` for `size`, the scrollport
    // from `port` for `port_size`, the containing block from `cb` for
    // `cb_size`.
    let along = |insets: (Option<f32>, Option<f32>),
                 (pos, size): (f32, f32),
                 (port, port_size): (f32, f32),
                 (cb, cb_size): (f32, f32)| {
        if let Some(start) = insets.0 {
            let push = (port + start - pos).max(0.0);
            if push > 0.0 {
                let room = (cb + cb_size - (pos + size)).max(0.0);
                return push.min(room);
            }
        }
        if let Some(end) = insets.1 {
            let pull = ((port + port_size - end) - (pos + size)).min(0.0);
            let room = (cb - pos).min(0.0);
            return pull.max(room);
        }
        0.0
    };
    let dx = along(
        (inset(LEFT, scrollport.w), inset(RIGHT, scrollport.w)),
        (rect.x, rect.w),
        (scrollport.x, scrollport.w),
        (containing.x, containing.w),
    );
    let dy = along(
        (inset(TOP, scrollport.h), inset(BOTTOM, scrollport.h)),
        (rect.y, rect.h),
        (scrollport.y, scrollport.h),
        (containing.y, containing.h),
    );
    (dx, dy)
}

/// The padding box of a box fragment whose border box is `rect`: the
/// border box inset by the border, zero-width on a side whose style
/// paints nothing (CSS2 §8.5.3). What `overflow` clips to (CSS2
/// §11.1.1).
pub fn padding_box(f: &Fragment, rect: Rect) -> Rect {
    let [top, right, bottom, left] = f.style.used_border_width();
    Rect {
        x: rect.x + left,
        y: rect.y + top,
        w: (rect.w - left - right).max(0.0),
        h: (rect.h - top - bottom).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    fn test_fonts() -> text::FontCollection {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    }

    /// The fragment tree of `root` under `css`, laid out at 800px.
    fn layout_of(root: Element, css: &str) -> layout::DocumentLayout {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = layout::build_box_tree(&styled);
        layout::layout_document(&boxes, 800.0, &test_fonts())
    }

    /// The steps as short names, with the node for a box and the clip
    /// rectangle for a push.
    fn names(steps: &[Step<'_>]) -> Vec<String> {
        steps
            .iter()
            .map(|s| match s {
                Step::Box { fragment, .. } => format!("box {}", fragment.node),
                Step::Text { fragment, .. } => match &fragment.kind {
                    FragmentKind::Text(t) => format!("text {t}"),
                    FragmentKind::Box => unreachable!(),
                },
                Step::PushClip { node, rect } => {
                    format!("clip {node} {} {} {} {}", rect.x, rect.y, rect.w, rect.h)
                }
                Step::PopClip => "pop".to_string(),
                Step::Scrollbar { offset, .. } => format!("scrollbar {offset}"),
            })
            .collect()
    }

    #[test]
    fn the_order_is_tree_order_with_clips_around_a_clipping_box_content() {
        // <div><p>a</p><div class=c><p>b</p></div></div>, the clipping
        // box 50px tall with a 5px border: nodes div 1, p 2, div.c 3,
        // p 4. The text is a child of the p's fragment; the clip wraps
        // the clipping box's content, after its own background step.
        let layout = layout_of(
            Element::new("div").push(Element::new("p").text("a")).push(
                Element::new("div")
                    .attr("class", "c")
                    .push(Element::new("p").text("b")),
            ),
            "div, p { margin: 0; } .c { overflow: hidden; height: 50px; border: 5px solid #000; width: 100px; }",
        );
        let m = test_fonts().primary().metrics(16.0);
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let line = m.line_height();
        assert_eq!(
            names(&steps),
            vec![
                "box 1".to_string(),
                "box 2".to_string(),
                "text a".to_string(),
                "box 3".to_string(),
                format!("clip 3 5 {} 100 50", line + 5.0),
                "box 4".to_string(),
                "text b".to_string(),
                "scrollbar 0".to_string(),
                "pop".to_string(),
            ]
        );
    }

    #[test]
    fn rectangles_are_in_viewport_coordinates_and_offsets_apply_inside_the_clip() {
        // The page scrolled by 20, the 100px scroll container (node 2)
        // scrolled by 30: its own box is at document 0, viewport -20;
        // its child (node 3, 200px tall) at viewport -20 - 30. The clip
        // rectangle is the container's padding box, not moved by the
        // container's own offset; with no clip around it, the effective
        // clip is that padding box itself.
        let layout = layout_of(
            Element::new("div").push(
                Element::new("div")
                    .attr("class", "s")
                    .push(Element::new("p").attr("class", "t")),
            ),
            "div, p { margin: 0; } .s { overflow: auto; height: 100px; } .t { height: 200px; }",
        );
        let offsets: std::collections::HashMap<usize, f32> = [(2, 30.0)].into_iter().collect();
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
            scroll_y: 20.0,
        };
        let steps = paint_order(&layout.roots, viewport, &offsets);
        let rect = |node: usize| {
            steps
                .iter()
                .find_map(|s| match s {
                    Step::Box { fragment, rect } if fragment.node == node => Some(*rect),
                    _ => None,
                })
                .expect("the box has a step")
        };
        assert_eq!(rect(2).y, -20.0);
        assert_eq!(rect(3).y, -50.0);
        assert!(matches!(
            steps[2],
            Step::PushClip {
                node: 2,
                rect: Rect {
                    y: -20.0,
                    h: 100.0,
                    ..
                }
            }
        ));
        assert!(matches!(steps[4], Step::Scrollbar { offset: 30.0, .. }));
    }

    #[test]
    fn nested_clips_intersect_and_out_of_view_content_is_left_out() {
        // An outer clipping box 50px tall holding an inner one 200px
        // tall at the same place: the inner's effective clip is the
        // outer's 50px. A text 1000px down the page is not in the
        // 600px viewport, and a clipping box there is skipped whole.
        let layout = layout_of(
            Element::new("div")
                .push(
                    Element::new("div")
                        .attr("class", "o")
                        .push(Element::new("div").attr("class", "i").text("in")),
                )
                .push(Element::new("div").attr("class", "far").text("far"))
                .push(
                    Element::new("div")
                        .attr("class", "farclip")
                        .text("clipped far"),
                ),
            "div { margin: 0; } .o { overflow: hidden; height: 50px; }
             .i { overflow: hidden; height: 200px; }
             .far { margin-top: 1000px; } .farclip { overflow: hidden; height: 10px; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let clips: Vec<(usize, f32)> = steps
            .iter()
            .filter_map(|s| match s {
                Step::PushClip { node, rect } => Some((*node, rect.h)),
                _ => None,
            })
            .collect();
        assert_eq!(
            clips,
            vec![(2, 50.0), (3, 50.0)],
            "the inner clip is cut to the outer"
        );
        assert!(
            !names(&steps).iter().any(|n| n.contains("far")),
            "{:?}",
            names(&steps)
        );
        assert_eq!(names(&steps).iter().filter(|n| *n == "pop").count(), 2);
    }

    #[test]
    fn a_sticky_box_is_shifted_to_keep_its_insets_within_its_containing_block() {
        let sticky = |inset: [LengthPercentageAuto; 4]| ComputedStyle {
            position: Position::Sticky,
            inset,
            ..ComputedStyle::default()
        };
        let auto = LengthPercentageAuto::Auto;
        let px = LengthPercentageAuto::Px;
        let rect = |x, y, w, h| Rect { x, y, w, h };
        let port = rect(0.0, 0.0, 800.0, 600.0);
        let top = sticky([px(10.0), auto, auto, auto]);
        // Already 10px or more below the scrollport's top: no shift.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, 100.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 0.0)
        );
        // Scrolled 50px past the top: pushed down to 10, as the
        // containing block, ending far below, allows.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // The containing block ends at 0: the box is held with its
        // bottom there, pushed by 30 only.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 200.0),
                port
            ),
            (0.0, 30.0)
        );
        // `bottom: 0` pulls a box sticking out below the scrollport up,
        // no further than its containing block's top.
        let bottom = sticky([auto, auto, px(0.0), auto]);
        assert_eq!(
            sticky_shift(
                &bottom,
                rect(0.0, 590.0, 50.0, 20.0),
                rect(0.0, 500.0, 800.0, 400.0),
                port
            ),
            (0.0, -10.0)
        );
        assert_eq!(
            sticky_shift(
                &bottom,
                rect(0.0, 590.0, 50.0, 20.0),
                rect(0.0, 585.0, 800.0, 400.0),
                port
            ),
            (0.0, -5.0)
        );
        // Both given: the start edge wins once it pushes.
        let both = sticky([px(10.0), auto, px(0.0), auto]);
        assert_eq!(
            sticky_shift(
                &both,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // Horizontal the same way; a percentage is of the scrollport.
        let left = sticky([auto, auto, auto, px(5.0)]);
        assert_eq!(
            sticky_shift(
                &left,
                rect(-20.0, 0.0, 10.0, 10.0),
                rect(-100.0, 0.0, 1000.0, 100.0),
                port
            ),
            (25.0, 0.0)
        );
        let percent = sticky([LengthPercentageAuto::Percent(10.0), auto, auto, auto]);
        assert_eq!(
            sticky_shift(
                &percent,
                rect(0.0, 0.0, 50.0, 20.0),
                rect(0.0, 0.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // Every inset `auto`: nothing holds the box, however far out.
        let none = sticky([auto; 4]);
        assert_eq!(
            sticky_shift(
                &none,
                rect(-500.0, -500.0, 50.0, 20.0),
                rect(-600.0, -600.0, 2000.0, 2000.0),
                port
            ),
            (0.0, 0.0)
        );
    }

    #[test]
    fn the_walk_shifts_a_sticky_box_and_its_content_together() {
        // A 300px container holding a 20px sticky box with `top: 0` and
        // a text, the page scrolled by 100: the box and its text are
        // held at the viewport's top; scrolled by 290, the container's
        // bottom at 10 holds the box 10px up. Nodes: div 1, s 2.
        let layout = layout_of(
            Element::new("div").push(Element::new("div").attr("class", "s").text("x")),
            "div { margin: 0; height: 300px; } .s { position: sticky; top: 0; height: 20px; }",
        );
        let rects = |scroll_y: f32| {
            let viewport = Viewport {
                width: 800.0,
                height: 600.0,
                scroll_y,
            };
            let steps = paint_order(&layout.roots, viewport, &());
            let s = steps
                .iter()
                .find_map(|st| match st {
                    Step::Box { fragment, rect } if fragment.node == 2 => Some(rect.y),
                    _ => None,
                })
                .expect("the sticky box");
            let text = steps
                .iter()
                .find_map(|st| match st {
                    Step::Text { rect, .. } => Some(rect.y),
                    _ => None,
                })
                .expect("its text");
            (s, text)
        };
        assert_eq!(rects(0.0), (0.0, 0.0));
        assert_eq!(rects(100.0), (0.0, 0.0), "held at the top, text with it");
        assert_eq!(
            rects(290.0),
            (-10.0, -10.0),
            "held by the container's bottom"
        );
    }

    #[test]
    fn rect_arithmetic() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        let b = Rect {
            x: 5.0,
            y: 5.0,
            w: 10.0,
            h: 10.0,
        };
        assert!(a.contains(0.0, 0.0) && !a.contains(10.0, 0.0) && !a.contains(-1.0, 5.0));
        assert!(a.meets(&b));
        assert_eq!(
            a.intersect(&b),
            Rect {
                x: 5.0,
                y: 5.0,
                w: 5.0,
                h: 5.0
            }
        );
        let c = Rect {
            x: 20.0,
            y: 0.0,
            w: 5.0,
            h: 5.0,
        };
        assert!(!a.meets(&c));
        assert_eq!(a.intersect(&c).w, 0.0);
    }
}
