//! The render tree (SVG 2 sections 5, 9 to 14): what an `<svg>` draws,
//! built from its document and computed styles and holding no node of
//! the document but each shape's id. Groups carry a transform, an
//! opacity and a clip; shapes a path in their group's user units, a fill
//! and a stroke resolved to what the canvas paints with: a colour or a
//! gradient.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::Arc;

use canvas::{FillRule, Gradient, GradientKind, LineCap, LineJoin, Matrix, Path, Rgba, Spread};
use dom::{Document, MAX_DEPTH, NodeId};
use style::computed::{
    self, ComputedStyle, Display, LengthPercentage, LengthPercentageAuto, PaintLayer,
    ShapeRendering, Size, StrokeLinecap, StrokeLinejoin, SvgPaint, TransformBox, VectorEffect,
};
use style::svg_transform::parse_transform_list;
use style::transform::{ReferenceBox, flattened_2d};
use style::{Recascade, Styles};
use text::FontCollection;

use crate::syntax::{
    Axis, Length, Unit, Viewport, parse_length, parse_points, parse_preserve_aspect_ratio,
    parse_view_box, view_box_matrix,
};
use crate::{MAX_GRADIENT_STOPS, MAX_SVG_ELEMENTS, MAX_USE_DEPTH};

/// What an `<svg>` draws, in the user units of a viewport `width` by
/// `height`.
#[derive(Debug, Clone)]
pub struct Tree {
    /// The viewport's width, in CSS pixels.
    pub width: f64,
    /// Its height.
    pub height: f64,
    /// The `<svg>`'s content, under its `viewBox`'s matrix.
    pub root: Group,
    /// The elements it was built from.
    pub elements: usize,
}

/// A group: its children drawn under its transform and clip,
/// composited at its opacity.
#[derive(Debug, Clone)]
pub struct Group {
    /// The element, when the group is one's.
    pub node: Option<NodeId>,
    /// From its user units to its parent's.
    pub transform: Matrix,
    /// Its opacity, in `[0, 1]`.
    pub opacity: f64,
    /// What it is clipped to, in its user units.
    pub clip: Option<Clip>,
    /// What it draws, in order.
    pub children: Vec<Item>,
}

/// A clip: a viewport's rectangle (`overflow`), or a `clipPath`'s
/// paths, their union.
#[derive(Debug, Clone)]
pub enum Clip {
    /// `[x, y, width, height]`.
    Rect([f64; 4]),
    /// Each path by its rule; none clips everything away.
    Paths(Vec<(Path, FillRule)>),
}

/// A group or a shape.
#[derive(Debug, Clone)]
pub enum Item {
    /// A group.
    Group(Group),
    /// A shape, boxed: it is the larger.
    Shape(Box<Shape>),
}

/// A shape: its path, filled and stroked.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Its element, or the outermost `<use>` it is an instance under:
    /// what a point on it hits.
    pub node: NodeId,
    /// Its geometry, in its group's user units.
    pub path: Path,
    /// The geometry's bounds, `[x, y, width, height]`: its fill box.
    pub bounds: [f64; 4],
    /// Its fill, when it has one.
    pub fill: Option<Fill>,
    /// Its stroke, when it has one.
    pub stroke: Option<Stroke>,
    /// Whether the stroke is painted before the fill (`paint-order`).
    pub stroke_first: bool,
    /// Its opacity, its fill's and its stroke's alike: a shape with
    /// both and an opacity below 1 is a group's, for the two overlap.
    pub opacity: f64,
    /// Whether its edges are smoothed (`shape-rendering` other than
    /// `crispEdges` and `optimizeSpeed`).
    pub anti_alias: bool,
}

/// What a fill or a stroke paints with.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// A colour, not premultiplied.
    Color(Rgba),
    /// A gradient, its transform from its geometry's coordinates to the
    /// shape's user units.
    Gradient(Gradient),
}

/// A fill.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    /// Its paint.
    pub paint: Paint,
    /// `fill-opacity`.
    pub opacity: f64,
    /// `fill-rule`.
    pub rule: FillRule,
}

/// A stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    /// Its paint.
    pub paint: Paint,
    /// `stroke-opacity`.
    pub opacity: f64,
    /// Its width, in user units, above 0.
    pub width: f64,
    /// `stroke-linecap`.
    pub cap: LineCap,
    /// `stroke-linejoin`.
    pub join: LineJoin,
    /// `stroke-miterlimit`.
    pub miter_limit: f64,
    /// The dashes, in user units: empty for a solid line.
    pub dashes: Vec<f64>,
    /// `stroke-dashoffset`.
    pub offset: f64,
    /// `vector-effect: non-scaling-stroke`: traced in the coordinates
    /// the drawing is placed in, its width unscaled by the transforms
    /// within.
    pub non_scaling: bool,
}

/// What a render tree is built from: a document, its computed styles,
/// what styles a `<use>`'s instance (its rules matched as its
/// original's, inheriting from the `<use>`, SVG 2 section 5.5; without
/// it, as its original is), and the fonts text is shaped in (without
/// them, no text is drawn).
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// The document.
    pub document: &'a Document,
    /// Its computed styles.
    pub styles: &'a Styles,
    /// What styles an instance.
    pub recascade: Option<&'a dyn Recascade>,
    /// The engine's fonts.
    pub fonts: Option<&'a FontCollection>,
}

/// The render tree of the `<svg>` element `svg` of `context`'s document
/// for a viewport `width` by `height` CSS pixels: its `viewBox` mapped
/// onto the viewport by its `preserveAspectRatio`, a `viewBox` of no
/// area drawing nothing. At most [`MAX_SVG_ELEMENTS`] elements are
/// built; past them, what was built is drawn.
pub fn build(context: Context<'_>, svg: NodeId, width: f64, height: f64) -> Tree {
    build_within(context, svg, (width, height), (MAX_SVG_ELEMENTS, MAX_DEPTH))
}

/// [`build`] for scripts' questions: a shape that paints nothing is
/// kept, its fill and stroke transparent, for its geometry.
pub(crate) fn build_queries(context: Context<'_>, svg: NodeId, width: f64, height: f64) -> Tree {
    build_with(
        context,
        svg,
        (width, height),
        (MAX_SVG_ELEMENTS, MAX_DEPTH),
        true,
    )
}

/// [`build`], at most `limit` elements built, at most `max_depth` open
/// at once.
pub(crate) fn build_within(
    context: Context<'_>,
    svg: NodeId,
    size: (f64, f64),
    bounds: (usize, usize),
) -> Tree {
    build_with(context, svg, size, bounds, false)
}

fn build_with(
    context: Context<'_>,
    svg: NodeId,
    (width, height): (f64, f64),
    (limit, max_depth): (usize, usize),
    queries: bool,
) -> Tree {
    let Context {
        document,
        styles,
        recascade,
        fonts,
    } = context;
    let root_font_size = document
        .element_children(document.root())
        .next()
        .and_then(|root| styles.get(root))
        .map_or(16.0, |s| f64::from(s.font_size_px));
    let font_size = styles.get(svg).map_or(16.0, |s| f64::from(s.font_size_px));
    let mut builder = Builder {
        document,
        styles,
        recascade,
        fonts,
        elements: 1,
        limit,
        max_depth,
        queries,
        ids: OnceCell::new(),
        using: Vec::new(),
        host: None,
        depth: 0,
        stops: HashMap::new(),
        glyphs: HashMap::new(),
        outlines: HashMap::new(),
    };
    let mut root = Group {
        node: Some(svg),
        transform: Matrix::IDENTITY,
        opacity: 1.0,
        clip: None,
        children: Vec::new(),
    };
    let mut viewport = Viewport {
        width,
        height,
        font_size,
        root_font_size,
    };
    match document.attr(svg, "viewBox").and_then(parse_view_box) {
        Some(view_box) if !view_box.has_area() => {}
        Some(view_box) => {
            let ratio = document
                .attr(svg, "preserveAspectRatio")
                .map(parse_preserve_aspect_ratio)
                .unwrap_or_default();
            root.transform = matrix(view_box_matrix(&view_box, &ratio, width, height));
            viewport.width = view_box.width;
            viewport.height = view_box.height;
            root.children = builder.children(svg, &viewport, None);
        }
        None => root.children = builder.children(svg, &viewport, None),
    }
    Tree {
        width,
        height,
        root,
        elements: builder.elements,
    }
}

fn matrix([a, b, c, d, e, f]: [f64; 6]) -> Matrix {
    Matrix::new(a, b, c, d, e, f)
}

fn translate(x: f64, y: f64) -> Matrix {
    Matrix::new(1.0, 0.0, 0.0, 1.0, x, y)
}

pub(crate) struct Builder<'a> {
    pub(crate) document: &'a Document,
    styles: &'a Styles,
    recascade: Option<&'a dyn Recascade>,
    pub(crate) fonts: Option<&'a FontCollection>,
    elements: usize,
    /// The most elements built.
    limit: usize,
    /// The most elements open at once ([`MAX_DEPTH`], the document's
    /// own bound, which instances could pass).
    max_depth: usize,
    /// Whether shapes that paint nothing are kept, for scripts'
    /// questions.
    queries: bool,
    /// The document's elements by `id`, the first of each, made when a
    /// reference first asks.
    ids: OnceCell<HashMap<&'a str, NodeId>>,
    /// The elements `<use>`s are instantiating, outermost first.
    using: Vec<NodeId>,
    /// The outermost `<use>` whose instance is being built.
    host: Option<NodeId>,
    /// The elements open, instances' included, against `max_depth`.
    depth: usize,
    /// Each gradient's stops, once read.
    stops: HashMap<NodeId, Arc<[(f64, Rgba)]>>,
    /// Each run's glyphs by its text and size, and each glyph's outline
    /// by its font, id and size, once made: a text whose characters are
    /// set apart (spaced, placed) is shaped once a character it has,
    /// not once a character it shows.
    pub(crate) glyphs: HashMap<(String, u32), Glyphs>,
    pub(crate) outlines: HashMap<(text::Script, u16, u32), Outline>,
}

/// A run's glyphs as shaped, each with the script whose font made it.
pub(crate) type Glyphs = Arc<[(text::Script, text::ShapedGlyph)]>;

/// A glyph's outline; `None` for a glyph with none.
pub(crate) type Outline = Option<Arc<[text::Curve]>>;

impl<'a> Builder<'a> {
    /// The element a fragment reference (`#id`) names in the document.
    fn referenced(&self, reference: &str) -> Option<NodeId> {
        let id = reference.trim().strip_prefix('#')?;
        let document = self.document;
        self.ids
            .get_or_init(|| {
                let mut ids = HashMap::new();
                for node in document.subtree(document.root()) {
                    if let Some(id) = document.attr(node, "id") {
                        ids.entry(id).or_insert(node);
                    }
                }
                ids
            })
            .get(id)
            .copied()
    }

    /// `node`'s style: inheriting from `parent` in an instance, else the
    /// cascade's.
    pub(crate) fn style_of(
        &self,
        node: NodeId,
        parent: Option<&ComputedStyle>,
    ) -> Option<Cow<'a, ComputedStyle>> {
        match (parent, self.recascade) {
            (Some(parent), Some(recascade)) => Some(Cow::Owned(recascade.element(node, parent))),
            _ => self.styles.get(node).map(Cow::Borrowed),
        }
    }

    /// Whether `node` passes conditional processing (SVG 2 section
    /// 5.8): no `requiredExtensions`, which no extension satisfies, and
    /// a `systemLanguage`, if it has one, of a language the user reads
    /// (English).
    pub(crate) fn passes(&self, node: NodeId) -> bool {
        if self.document.attr(node, "requiredExtensions").is_some() {
            return false;
        }
        let Some(languages) = self.document.attr(node, "systemLanguage") else {
            return true;
        };
        languages.split(',').any(|tag| {
            let tag = tag.trim();
            tag.eq_ignore_ascii_case("en")
                || tag.eq_ignore_ascii_case("en-US")
                || tag.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("en-"))
        })
    }

    fn children(
        &mut self,
        parent: NodeId,
        viewport: &Viewport,
        inherit: Option<&ComputedStyle>,
    ) -> Vec<Item> {
        let mut out = Vec::new();
        for child in self.document.element_children(parent) {
            if self.elements >= self.limit {
                break;
            }
            if let Some(item) = self.item(child, viewport, inherit) {
                out.push(item);
            }
        }
        out
    }

    /// What the element `node` draws: nothing for `display: none`, an
    /// element of another namespace or that SVG does not draw, or one
    /// that conditional processing leaves out. `inherit` is the style it
    /// inherits from in an instance.
    fn item(
        &mut self,
        node: NodeId,
        viewport: &Viewport,
        inherit: Option<&ComputedStyle>,
    ) -> Option<Item> {
        if self.elements >= self.limit || self.depth >= self.max_depth {
            return None;
        }
        let tag = self.document.svg_tag(node)?;
        if !self.passes(node) {
            return None;
        }
        let style = self.style_of(node, inherit)?;
        if style.display == Display::None {
            return None;
        }
        self.elements += 1;
        self.depth += 1;
        let viewport = &Viewport {
            font_size: f64::from(style.font_size_px),
            ..*viewport
        };
        let inherit = inherit.is_some().then_some(&*style);
        let item = match tag {
            "g" | "a" => {
                let children = self.children(node, viewport, inherit);
                (!children.is_empty()).then(|| Item::Group(group(node, &style, viewport, children)))
            }
            "switch" => {
                let document = self.document;
                let chosen = document
                    .element_children(node)
                    .filter(|&c| document.svg_tag(c).is_some())
                    .find(|&c| self.passes(c));
                chosen
                    .and_then(|c| self.item(c, viewport, inherit))
                    .map(|child| Item::Group(group(node, &style, viewport, vec![child])))
            }
            "svg" => {
                let own = style.svg_own();
                let x = resolve(&own.x, Axis::X, viewport);
                let y = resolve(&own.y, Axis::Y, viewport);
                let w = size_or_all(&style.width, Axis::X, viewport);
                let h = size_or_all(&style.height, Axis::Y, viewport);
                self.viewport_element(node, &style, viewport, inherit, [x, y, w, h])
            }
            "use" => self.instance(node, &style, viewport),
            "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "path" => {
                self.shape(node, tag, &style, viewport)
            }
            "text" => self.text(node, style.clone(), viewport, inherit.is_some()),
            _ => None,
        };
        self.depth -= 1;
        let item = item?;
        Some(self.clipped(item, &style, viewport))
    }

    /// A nested `<svg>`'s or an instanced `<symbol>`'s viewport `rect`:
    /// its content under its `viewBox`, clipped to it when its
    /// `overflow` clips (SVG 2 section 8.1); a viewport or a `viewBox`
    /// of no area draws nothing.
    fn viewport_element(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        viewport: &Viewport,
        inherit: Option<&ComputedStyle>,
        [x, y, w, h]: [f64; 4],
    ) -> Option<Item> {
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let mut inner = translate(x, y);
        let mut content = Viewport {
            width: w,
            height: h,
            ..*viewport
        };
        if let Some(view_box) = self.document.attr(node, "viewBox").and_then(parse_view_box) {
            if !view_box.has_area() {
                return None;
            }
            let ratio = self
                .document
                .attr(node, "preserveAspectRatio")
                .map(parse_preserve_aspect_ratio)
                .unwrap_or_default();
            inner = inner.then(&matrix(view_box_matrix(&view_box, &ratio, w, h)));
            content.width = view_box.width;
            content.height = view_box.height;
        }
        let children = self.children(node, &content, inherit);
        let mut outer = group(
            node,
            style,
            viewport,
            vec![Item::Group(Group {
                node: None,
                transform: inner,
                opacity: 1.0,
                clip: None,
                children,
            })],
        );
        outer.clip = style.overflow.clips().then_some(Clip::Rect([x, y, w, h]));
        Some(Item::Group(outer))
    }

    /// A `<use>`'s instance (SVG 2 section 5.6): the element it
    /// references in this document, under the `<use>`'s transform and
    /// moved by its `x` and `y`; a `<symbol>` or an `<svg>` a viewport
    /// of the `<use>`'s `width` and `height`. Nothing for a reference
    /// to another document or to nothing, one that reaches its own
    /// ancestor or an element being instantiated, or one past
    /// [`MAX_USE_DEPTH`] within others.
    fn instance(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        viewport: &Viewport,
    ) -> Option<Item> {
        let document = self.document;
        let href = document
            .attr(node, "href")
            .or_else(|| document.attr(node, "xlink:href"))?;
        let target = self.referenced(href)?;
        let tag = document.svg_tag(target)?;
        let mut at = Some(node);
        while let Some(n) = at {
            if n == target {
                return None;
            }
            at = document.parent(n);
        }
        if self.using.len() >= MAX_USE_DEPTH || self.using.contains(&target) {
            return None;
        }
        let own = style.svg_own();
        let (x, y) = (
            resolve(&own.x, Axis::X, viewport),
            resolve(&own.y, Axis::Y, viewport),
        );
        self.using.push(target);
        let host = self.host;
        self.host = Some(host.unwrap_or(node));
        let instance = match tag {
            "symbol" | "svg" => self.viewport_instance(target, tag, style, viewport),
            _ => self.item(target, viewport, Some(style)),
        };
        self.host = host;
        self.using.pop();
        // Its `x` and `y` a group of their own, so that the use's own
        // user units are its transform's alone.
        let moved = Group {
            node: None,
            transform: translate(x, y),
            opacity: 1.0,
            clip: None,
            children: vec![instance?],
        };
        Some(Item::Group(group(
            node,
            style,
            viewport,
            vec![Item::Group(moved)],
        )))
    }

    /// An instanced `<symbol>` or `<svg>`: the `<use>`'s `width` and
    /// `height` where they are not `auto`, else the `<svg>`'s, else all
    /// of the viewport.
    fn viewport_instance(
        &mut self,
        target: NodeId,
        tag: &str,
        user: &ComputedStyle,
        viewport: &Viewport,
    ) -> Option<Item> {
        if self.elements >= self.limit || self.depth >= self.max_depth || !self.passes(target) {
            return None;
        }
        let style = self.style_of(target, Some(user))?;
        if style.display == Display::None {
            return None;
        }
        self.elements += 1;
        self.depth += 1;
        let pick = |own: &Size, theirs: &Size, axis| {
            if own.is_auto() {
                size_or_all(theirs, axis, viewport)
            } else {
                size_or_all(own, axis, viewport)
            }
        };
        let w = pick(&user.width, &style.width, Axis::X);
        let h = pick(&user.height, &style.height, Axis::Y);
        let (x, y) = if tag == "svg" {
            let own = style.svg_own();
            (
                resolve(&own.x, Axis::X, viewport),
                resolve(&own.y, Axis::Y, viewport),
            )
        } else {
            (0.0, 0.0)
        };
        let item = self.viewport_element(target, &style, viewport, Some(&style), [x, y, w, h]);
        self.depth -= 1;
        Some(self.clipped(item?, &style, viewport))
    }

    fn shape(
        &mut self,
        node: NodeId,
        tag: &str,
        style: &ComputedStyle,
        viewport: &Viewport,
    ) -> Option<Item> {
        let (path, bounds) = geometry(self.document, node, tag, style, viewport)?;
        // A line has no inside to fill.
        let mut shape = self.painted(node, path, bounds, style, viewport, tag != "line")?;
        let opacity = f64::from(style.effects().opacity);
        let transform = transform_of(style, viewport, Some(bounds));
        let overlap = opacity < 1.0 && shape.fill.is_some() && shape.stroke.is_some();
        if transform.is_none() && !overlap {
            shape.opacity = opacity;
            return Some(Item::Shape(Box::new(shape)));
        }
        Some(Item::Group(Group {
            node: None,
            transform: transform.unwrap_or(Matrix::IDENTITY),
            opacity,
            clip: None,
            children: vec![Item::Shape(Box::new(shape))],
        }))
    }

    /// `path`, of `bounds`, painted as `style` says: `None` when it has
    /// neither a fill (or may not have one) nor a stroke. Its opacity and
    /// transform are its caller's.
    fn painted(
        &mut self,
        node: NodeId,
        path: Path,
        bounds: [f64; 4],
        style: &ComputedStyle,
        viewport: &Viewport,
        fillable: bool,
    ) -> Option<Shape> {
        let svg = style.svg();
        let fill = if fillable {
            match self.paint(&svg.fill, style.color, bounds, viewport) {
                Some(paint) => Some(Fill {
                    paint,
                    opacity: f64::from(svg.fill_opacity),
                    rule: fill_rule(svg.fill_rule),
                }),
                None if self.queries => Some(Fill {
                    paint: Paint::Color(Rgba::TRANSPARENT),
                    opacity: 0.0,
                    rule: fill_rule(svg.fill_rule),
                }),
                None => None,
            }
        } else {
            None
        };
        let stroke = self.stroke(style, bounds, viewport);
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let layers = svg.paint_order.layers();
        let at = |layer| layers.iter().position(|l| *l == layer);
        Some(Shape {
            node: self.host.unwrap_or(node),
            path,
            bounds,
            stroke_first: at(PaintLayer::Stroke) < at(PaintLayer::Fill),
            opacity: 1.0,
            anti_alias: !matches!(
                svg.shape_rendering,
                ShapeRendering::CrispEdges | ShapeRendering::OptimizeSpeed
            ),
            fill,
            stroke,
        })
    }

    /// A `<text>`: its runs as shapes in a group of its transform and
    /// opacity (a `tspan`'s own opacity is not taken, recorded).
    fn text(
        &mut self,
        node: NodeId,
        style: Cow<'a, ComputedStyle>,
        viewport: &Viewport,
        instance: bool,
    ) -> Option<Item> {
        let runs = self.text_runs(node, style.clone(), viewport, instance)?;
        let mut children = Vec::new();
        let mut extent: Option<[f64; 4]> = None;
        for run in runs {
            let [x, y, w, h] = run.bounds;
            extent = Some(match extent {
                None => [x, y, x + w, y + h],
                Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x + w), b.max(y + h)],
            });
            if let Some(shape) =
                self.painted(run.node, run.path, run.bounds, &run.style, viewport, true)
            {
                children.push(Item::Shape(Box::new(shape)));
            }
        }
        if children.is_empty() {
            return None;
        }
        let bounds = extent.map(|[l, t, r, b]| [l, t, r - l, b - t]);
        let mut group = group(node, &style, viewport, children);
        group.transform = transform_of(&style, viewport, bounds).unwrap_or(Matrix::IDENTITY);
        Some(Item::Group(group))
    }

    /// A shape's stroke: `None` for no paint or a width not above 0.
    fn stroke(
        &mut self,
        style: &ComputedStyle,
        bounds: [f64; 4],
        viewport: &Viewport,
    ) -> Option<Stroke> {
        let svg = style.svg();
        let width = resolve(&svg.stroke_width, Axis::Diagonal, viewport);
        if !(width > 0.0 && width.is_finite()) {
            return None;
        }
        let (paint, opacity) = match self.paint(&svg.stroke, style.color, bounds, viewport) {
            Some(paint) => (paint, f64::from(svg.stroke_opacity)),
            // A script's question is of the stroke's shape, whatever
            // paints it.
            None if self.queries => (Paint::Color(Rgba::TRANSPARENT), 0.0),
            None => return None,
        };
        let mut dashes: Vec<f64> = svg
            .stroke_dasharray
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|d| resolve(d, Axis::Diagonal, viewport))
            .collect();
        // Dashes that sum to nothing, or one below zero (a percentage of
        // a calculation), draw a solid line.
        if dashes.iter().any(|d| d.is_nan() || *d < 0.0) || dashes.iter().sum::<f64>() <= 0.0 {
            dashes.clear();
        }
        Some(Stroke {
            paint,
            opacity,
            width,
            cap: match svg.stroke_linecap {
                StrokeLinecap::Butt => LineCap::Butt,
                StrokeLinecap::Round => LineCap::Round,
                StrokeLinecap::Square => LineCap::Square,
            },
            join: match svg.stroke_linejoin {
                StrokeLinejoin::Miter => LineJoin::Miter,
                StrokeLinejoin::Round => LineJoin::Round,
                StrokeLinejoin::Bevel => LineJoin::Bevel,
            },
            miter_limit: f64::from(svg.stroke_miterlimit),
            dashes,
            offset: resolve(&svg.stroke_dashoffset, Axis::Diagonal, viewport),
            non_scaling: style.svg_own().vector_effect == VectorEffect::NonScalingStroke,
        })
    }

    /// What a paint paints a shape of `bounds` with: `None` for `none`
    /// and what draws as it (`context-fill`, `context-stroke`, a
    /// reference to no paint server and no fallback, a gradient with no
    /// stops or of a bounding box of no area).
    fn paint(
        &mut self,
        value: &SvgPaint,
        color: computed::Color,
        bounds: [f64; 4],
        viewport: &Viewport,
    ) -> Option<Paint> {
        let SvgPaint::Url(reference, fallback) = value else {
            return plain_paint(value, color);
        };
        let server = self.referenced(reference).filter(|&n| {
            matches!(
                self.document.svg_tag(n),
                Some("linearGradient" | "radialGradient")
            )
        });
        match server {
            Some(gradient) => self.gradient(gradient, bounds, viewport),
            None => fallback.as_deref().and_then(|f| plain_paint(f, color)),
        }
    }

    /// A gradient (SVG 2 section 14.2) for a shape of `bounds`: its
    /// attributes and stops its own or, where it has none, its
    /// templates' through `href`; one stop paints its colour (the
    /// canvas's gradients do), a gradient of no length or radius its
    /// last stop's.
    fn gradient(&mut self, node: NodeId, bounds: [f64; 4], viewport: &Viewport) -> Option<Paint> {
        let document = self.document;
        let tag = document.svg_tag(node)?;
        let mut chain = vec![node];
        while chain.len() < MAX_USE_DEPTH {
            let last = chain[chain.len() - 1];
            let Some(next) = document
                .attr(last, "href")
                .or_else(|| document.attr(last, "xlink:href"))
                .and_then(|href| self.referenced(href))
            else {
                break;
            };
            if chain.contains(&next)
                || !matches!(
                    document.svg_tag(next),
                    Some("linearGradient" | "radialGradient")
                )
            {
                break;
            }
            chain.push(next);
        }
        // The two kinds' geometries share no attribute's name, so each
        // takes its own from a template of its kind alone.
        let attr = |name: &str| chain.iter().find_map(|&n| document.attr(n, name));
        let with_stops = chain.iter().copied().find(|&n| {
            document
                .element_children(n)
                .any(|c| document.svg_tag(c) == Some("stop"))
        })?;
        let stops = self.stops_of(with_stops);
        let last = stops.last()?.1;
        let bounding = attr("gradientUnits") != Some("userSpaceOnUse");
        let own = attr("gradientTransform")
            .and_then(parse_transform_list)
            .map_or(Matrix::IDENTITY, matrix);
        let (space, transform) = if bounding {
            let [x, y, w, h] = bounds;
            if !(w > 0.0 && h > 0.0) {
                return None;
            }
            let unit = Viewport {
                width: 1.0,
                height: 1.0,
                ..*viewport
            };
            (unit, Matrix::new(w, 0.0, 0.0, h, x, y).then(&own))
        } else {
            (*viewport, own)
        };
        let length = |name: &str, default: Length, axis| {
            attr(name)
                .and_then(parse_length)
                .unwrap_or(default)
                .resolve(axis, &space)
        };
        let percent = |value| Length {
            value,
            unit: Unit::Percent,
        };
        let kind = if tag == "linearGradient" {
            let (x1, y1) = (
                length("x1", percent(0.0), Axis::X),
                length("y1", percent(0.0), Axis::Y),
            );
            let (x2, y2) = (
                length("x2", percent(100.0), Axis::X),
                length("y2", percent(0.0), Axis::Y),
            );
            if x1 == x2 && y1 == y2 {
                return Some(Paint::Color(last));
            }
            GradientKind::Linear {
                x0: x1,
                y0: y1,
                x1: x2,
                y1: y2,
            }
        } else {
            let (cx, cy) = (
                length("cx", percent(50.0), Axis::X),
                length("cy", percent(50.0), Axis::Y),
            );
            let r = length("r", percent(50.0), Axis::Diagonal);
            let fx = attr("fx")
                .and_then(parse_length)
                .map_or(cx, |l| l.resolve(Axis::X, &space));
            let fy = attr("fy")
                .and_then(parse_length)
                .map_or(cy, |l| l.resolve(Axis::Y, &space));
            let fr = length("fr", percent(0.0), Axis::Diagonal);
            if r < 0.0 || fr < 0.0 {
                return None;
            }
            if r == 0.0 {
                return Some(Paint::Color(last));
            }
            GradientKind::Radial {
                x0: fx,
                y0: fy,
                r0: fr,
                x1: cx,
                y1: cy,
                r1: r,
            }
        };
        let spread = match attr("spreadMethod") {
            Some("reflect") => Spread::Reflect,
            Some("repeat") => Spread::Repeat,
            _ => Spread::Pad,
        };
        Some(Paint::Gradient(Gradient {
            kind,
            stops: stops.to_vec(),
            spread,
            transform,
        }))
    }

    /// A gradient element's stops: each `stop` child's offset (a number
    /// or a percentage, in `[0, 1]`, none below the one before) and its
    /// `stop-color` at its `stop-opacity`; at most
    /// [`MAX_GRADIENT_STOPS`].
    fn stops_of(&mut self, node: NodeId) -> Arc<[(f64, Rgba)]> {
        if let Some(stops) = self.stops.get(&node) {
            return Arc::clone(stops);
        }
        let document = self.document;
        let mut out = Vec::new();
        let mut floor = 0.0f64;
        for stop in document
            .element_children(node)
            .filter(|&c| document.svg_tag(c) == Some("stop"))
            .take(MAX_GRADIENT_STOPS)
        {
            let offset = match document.attr(stop, "offset").and_then(parse_length) {
                Some(Length {
                    value,
                    unit: Unit::None,
                }) => value,
                Some(Length {
                    value,
                    unit: Unit::Percent,
                }) => value / 100.0,
                _ => 0.0,
            };
            floor = offset.clamp(0.0, 1.0).max(floor);
            let Some(style) = self.styles.get(stop) else {
                continue;
            };
            let own = style.svg_own();
            let color = match &own.stop_color {
                SvgPaint::Color(c) => *c,
                _ => style.color,
            };
            let mut rgba = rgba(color);
            rgba.a = (f32::from(rgba.a) * own.stop_opacity).round() as u8;
            out.push((floor, rgba));
        }
        let stops: Arc<[(f64, Rgba)]> = Arc::from(out);
        self.stops.insert(node, Arc::clone(&stops));
        stops
    }

    /// `item`, the drawing of an element of `style`, clipped by its
    /// `clip-path` (CSS Masking 1 section 6): a reference to a
    /// `clipPath` clips it to the union of its children; one to anything
    /// else clips nothing.
    fn clipped(&mut self, item: Item, style: &ComputedStyle, viewport: &Viewport) -> Item {
        let Some(reference) = style.svg_own().clip_path.clone() else {
            return item;
        };
        let bounds = item_bounds(&item);
        let Some(clip) = self.clip_path(&reference, bounds, viewport) else {
            return item;
        };
        match item {
            // The element's own group: clipped in its user units.
            Item::Group(mut group) if group.clip.is_none() => {
                group.clip = Some(clip);
                Item::Group(group)
            }
            Item::Group(mut group) => {
                let children = std::mem::take(&mut group.children);
                group.children = vec![Item::Group(Group {
                    node: None,
                    transform: Matrix::IDENTITY,
                    opacity: 1.0,
                    clip: Some(clip),
                    children,
                })];
                Item::Group(group)
            }
            shape @ Item::Shape(_) => Item::Group(Group {
                node: None,
                transform: Matrix::IDENTITY,
                opacity: 1.0,
                clip: Some(clip),
                children: vec![shape],
            }),
        }
    }

    /// A `clipPath`'s paths (CSS Masking 1 section 6.2) for an element
    /// of `bounds`: its shape children and `<use>`s of shapes, each
    /// under its transforms and the `clipPath`'s, in the element's user
    /// units or its bounding box's (`clipPathUnits`), by its
    /// `clip-rule`.
    fn clip_path(
        &mut self,
        reference: &str,
        bounds: Option<[f64; 4]>,
        viewport: &Viewport,
    ) -> Option<Clip> {
        let document = self.document;
        let node = self.referenced(reference)?;
        if document.svg_tag(node) != Some("clipPath") {
            return None;
        }
        let style = self.styles.get(node)?;
        let mut base = transform_of(style, viewport, None).unwrap_or(Matrix::IDENTITY);
        let mut space = *viewport;
        if document.attr(node, "clipPathUnits") == Some("objectBoundingBox") {
            let Some([x, y, w, h]) = bounds.filter(|b| b[2] > 0.0 && b[3] > 0.0) else {
                return Some(Clip::Paths(Vec::new()));
            };
            base = base.then(&Matrix::new(w, 0.0, 0.0, h, x, y));
            space.width = 1.0;
            space.height = 1.0;
        }
        let mut paths = Vec::new();
        for child in document.element_children(node) {
            if self.elements >= self.limit {
                break;
            }
            let Some(mut tag) = document.svg_tag(child) else {
                continue;
            };
            let (mut shape, mut at) = (child, base);
            if tag == "use" {
                let Some(style) = self.styles.get(child) else {
                    continue;
                };
                let own = style.svg_own();
                let target = document
                    .attr(child, "href")
                    .or_else(|| document.attr(child, "xlink:href"))
                    .and_then(|href| self.referenced(href));
                let Some(target) = target else {
                    continue;
                };
                at = at
                    .then(&transform_of(style, &space, None).unwrap_or(Matrix::IDENTITY))
                    .then(&translate(
                        resolve(&own.x, Axis::X, &space),
                        resolve(&own.y, Axis::Y, &space),
                    ));
                shape = target;
                tag = match document.svg_tag(target) {
                    Some(tag) => tag,
                    None => continue,
                };
            }
            if !matches!(
                tag,
                "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "path"
            ) || !self.passes(child)
            {
                continue;
            }
            let Some(style) = self.styles.get(shape) else {
                continue;
            };
            if style.display == Display::None {
                continue;
            }
            self.elements += 1;
            let space = Viewport {
                font_size: f64::from(style.font_size_px),
                ..space
            };
            let Some((path, bounds)) = geometry(document, shape, tag, style, &space) else {
                continue;
            };
            let own = transform_of(style, &space, Some(bounds)).unwrap_or(Matrix::IDENTITY);
            paths.push((
                path.transformed(&at.then(&own)),
                fill_rule(style.svg().clip_rule),
            ));
        }
        Some(Clip::Paths(paths))
    }
}

/// An element's group: its transform and opacity over `children`.
fn group(node: NodeId, style: &ComputedStyle, viewport: &Viewport, children: Vec<Item>) -> Group {
    Group {
        node: Some(node),
        transform: transform_of(style, viewport, None).unwrap_or(Matrix::IDENTITY),
        opacity: f64::from(style.effects().opacity),
        clip: None,
        children,
    }
}

/// What an item covers in its parent's user units, `[x, y, width,
/// height]`, its transforms taken: the bounding box `objectBoundingBox`
/// units are of.
pub(crate) fn item_bounds(item: &Item) -> Option<[f64; 4]> {
    match item {
        Item::Shape(shape) => Some(shape.bounds),
        Item::Group(group) => {
            let mut out: Option<[f64; 4]> = None;
            for child in &group.children {
                let Some([x, y, w, h]) = item_bounds(child) else {
                    continue;
                };
                for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                    let (px, py) = group.transform.apply(px, py);
                    out = Some(match out {
                        None => [px, py, px, py],
                        Some([l, t, r, b]) => [l.min(px), t.min(py), r.max(px), b.max(py)],
                    });
                }
            }
            out.map(|[l, t, r, b]| [l, t, r - l, b - t])
        }
    }
}

/// An element's transform (CSS Transforms 1, the `transform` attribute
/// among it): `None` for none. Its reference box is the nearest
/// viewport at the user origin (`view-box`, the initial value), or the
/// shape's geometry for the other boxes (a group's is its viewport's
/// too, recorded).
fn transform_of(
    style: &ComputedStyle,
    viewport: &Viewport,
    bounds: Option<[f64; 4]>,
) -> Option<Matrix> {
    let effects = style.effects();
    if !effects.has_transform() {
        return None;
    }
    let reference = match (effects.transform_box, bounds) {
        (TransformBox::ViewBox, _) | (_, None) => ReferenceBox {
            x: 0.0,
            y: 0.0,
            width: viewport.width as f32,
            height: viewport.height as f32,
        },
        (_, Some([x, y, w, h])) => ReferenceBox {
            x: x as f32,
            y: y as f32,
            width: w as f32,
            height: h as f32,
        },
    };
    let m = effects.matrix(&reference)?;
    let [a, b, c, d, e, f] = flattened_2d(&m).to_2d()?;
    let m = matrix([a, b, c, d, e, f].map(f64::from));
    m.is_finite().then_some(m)
}

/// A colour of the cascade's as the canvas has one.
pub(crate) fn rgba(c: computed::Color) -> Rgba {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba::new(byte(c.r), byte(c.g), byte(c.b), byte(c.a))
}

/// A paint that is no reference.
fn plain_paint(value: &SvgPaint, color: computed::Color) -> Option<Paint> {
    match value {
        SvgPaint::Color(c) => Some(Paint::Color(rgba(*c))),
        SvgPaint::CurrentColor => Some(Paint::Color(rgba(color))),
        _ => None,
    }
}

fn fill_rule(rule: computed::FillRule) -> FillRule {
    match rule {
        computed::FillRule::NonZero => FillRule::Winding,
        computed::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

/// A length-percentage in user units along `axis`.
fn resolve(value: &LengthPercentage, axis: Axis, viewport: &Viewport) -> f64 {
    f64::from(value.resolve(viewport.along(axis) as f32))
}

/// A `width` or `height` of a shape: `auto` is 0 (SVG 2 section 10.2).
fn size(value: &Size, axis: Axis, viewport: &Viewport) -> f64 {
    value
        .length_percentage()
        .map_or(0.0, |v| resolve(&v, axis, viewport))
}

/// A `width` or `height` of a viewport: `auto` is all of the one it is
/// in (100%).
fn size_or_all(value: &Size, axis: Axis, viewport: &Viewport) -> f64 {
    value
        .length_percentage()
        .map_or(viewport.along(axis), |v| resolve(&v, axis, viewport))
}

/// An attribute's length (`x1` and the rest), 0 when it is missing or
/// does not parse.
fn attr_length(document: &Document, node: NodeId, name: &str) -> Length {
    document
        .attr(node, name)
        .and_then(parse_length)
        .unwrap_or(Length::ZERO)
}

/// A shape's path and bounds, in its user units: `None` for a shape in
/// error or of no geometry (a radius or a size not above 0, fewer than
/// two points, no path data), which draws nothing.
pub(crate) fn geometry(
    document: &Document,
    node: NodeId,
    tag: &str,
    style: &ComputedStyle,
    viewport: &Viewport,
) -> Option<(Path, [f64; 4])> {
    let own = style.svg_own();
    let at = |v: &LengthPercentage, axis| resolve(v, axis, viewport);
    let id = &Matrix::IDENTITY;
    let mut path = Path::new();
    let bounds = match tag {
        "rect" => {
            let (x, y) = (at(&own.x, Axis::X), at(&own.y, Axis::Y));
            let w = size(&style.width, Axis::X, viewport);
            let h = size(&style.height, Axis::Y, viewport);
            if !(w > 0.0 && h > 0.0) {
                return None;
            }
            let (rx, ry) = radii(&own.rx, &own.ry, viewport);
            let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
            if rx > 0.0 && ry > 0.0 {
                path.move_to(x + rx, y, id);
                let corners = [
                    (x + w - rx, y + ry, -FRAC_PI_2),
                    (x + w - rx, y + h - ry, 0.0),
                    (x + rx, y + h - ry, FRAC_PI_2),
                    (x + rx, y + ry, PI),
                ];
                for (cx, cy, start) in corners {
                    path.ellipse(cx, cy, rx, ry, 0.0, start, start + FRAC_PI_2, false, id)
                        .ok()?;
                }
                path.close();
            } else {
                path.rect(x, y, w, h, id);
            }
            [x, y, w, h]
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (at(&own.cx, Axis::X), at(&own.cy, Axis::Y));
            let (rx, ry) = if tag == "circle" {
                let r = at(&own.r, Axis::Diagonal);
                (r, r)
            } else {
                if own.rx.is_auto() && own.ry.is_auto() {
                    return None;
                }
                radii(&own.rx, &own.ry, viewport)
            };
            if !(rx > 0.0 && ry > 0.0) {
                return None;
            }
            path.ellipse(cx, cy, rx, ry, 0.0, 0.0, TAU, false, id)
                .ok()?;
            path.close();
            [cx - rx, cy - ry, 2.0 * rx, 2.0 * ry]
        }
        "line" => {
            let at = |name, axis| attr_length(document, node, name).resolve(axis, viewport);
            let (x1, y1) = (at("x1", Axis::X), at("y1", Axis::Y));
            let (x2, y2) = (at("x2", Axis::X), at("y2", Axis::Y));
            path.move_to(x1, y1, id);
            path.line_to(x2, y2, id);
            [x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs()]
        }
        "polyline" | "polygon" => {
            let points = parse_points(document.attr(node, "points").unwrap_or(""));
            // Fewer than two points are in error.
            let (&(x0, y0), rest) = points.split_first()?;
            if rest.is_empty() {
                return None;
            }
            path.move_to(x0, y0, id);
            for &(x, y) in rest {
                path.line_to(x, y, id);
            }
            if tag == "polygon" {
                path.close();
            }
            let mut b = [x0, y0, x0, y0];
            for &(x, y) in rest {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            [b[0], b[1], b[2] - b[0], b[3] - b[1]]
        }
        _ => {
            path = Path::from_svg(document.attr(node, "d")?);
            if path.is_empty() {
                return None;
            }
            path.bounds().unwrap_or([0.0; 4])
        }
    };
    Some((path, bounds))
}

/// A rect's or an ellipse's `rx` and `ry`, `auto` taking the other's
/// value, both `auto` none (SVG 2 sections 10.2 and 10.4).
fn radii(rx: &LengthPercentageAuto, ry: &LengthPercentageAuto, viewport: &Viewport) -> (f64, f64) {
    let at = |v: &LengthPercentageAuto, axis| {
        (!v.is_auto()).then(|| f64::from(v.resolve(viewport.along(axis) as f32, 0.0)))
    };
    match (at(rx, Axis::X), at(ry, Axis::Y)) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, x),
        (None, Some(y)) => (y, y),
        (None, None) => (0.0, 0.0),
    }
}
