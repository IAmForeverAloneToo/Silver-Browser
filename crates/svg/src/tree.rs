//! The render tree (SVG 2 sections 9 to 13): what an `<svg>` draws,
//! built from its document and computed styles and holding no node of
//! the document but each shape's id. Groups carry a transform and an
//! opacity; shapes a path in their group's user units, a fill and a
//! stroke resolved to what the canvas paints with.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use canvas::{FillRule, LineCap, LineJoin, Matrix, Path, Rgba};
use dom::{Document, NodeId};
use style::Styles;
use style::computed::{
    self, ComputedStyle, Display, LengthPercentage, LengthPercentageAuto, PaintLayer,
    ShapeRendering, Size, StrokeLinecap, StrokeLinejoin, SvgPaint, TransformBox, VectorEffect,
};
use style::transform::{ReferenceBox, flattened_2d};

use crate::MAX_SVG_ELEMENTS;
use crate::syntax::{
    Axis, Length, Viewport, parse_length, parse_points, parse_preserve_aspect_ratio,
    parse_view_box, view_box_matrix,
};

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

/// A group: its children drawn under its transform, composited at its
/// opacity.
#[derive(Debug, Clone)]
pub struct Group {
    /// The element, when the group is one's.
    pub node: Option<NodeId>,
    /// From its user units to its parent's.
    pub transform: Matrix,
    /// Its opacity, in `[0, 1]`.
    pub opacity: f64,
    /// What it draws, in order.
    pub children: Vec<Item>,
}

/// A group or a shape.
#[derive(Debug, Clone)]
pub enum Item {
    /// A group.
    Group(Group),
    /// A shape.
    Shape(Shape),
}

/// A shape: its path, filled and stroked.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Its element.
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

/// The render tree of the `<svg>` element `svg` of `document`, styled by
/// `styles`, for a viewport `width` by `height` CSS pixels: its
/// `viewBox` mapped onto the viewport by its `preserveAspectRatio`, a
/// `viewBox` of no area drawing nothing. At most [`MAX_SVG_ELEMENTS`]
/// elements are built; past them, what was built is drawn.
pub fn build(document: &Document, styles: &Styles, svg: NodeId, width: f64, height: f64) -> Tree {
    build_within(document, styles, svg, (width, height), MAX_SVG_ELEMENTS)
}

/// [`build`], at most `limit` elements built.
pub(crate) fn build_within(
    document: &Document,
    styles: &Styles,
    svg: NodeId,
    (width, height): (f64, f64),
    limit: usize,
) -> Tree {
    let root_font_size = document
        .element_children(document.root())
        .next()
        .and_then(|root| styles.get(root))
        .map_or(16.0, |s| f64::from(s.font_size_px));
    let font_size = styles.get(svg).map_or(16.0, |s| f64::from(s.font_size_px));
    let mut builder = Builder {
        document,
        styles,
        elements: 1,
        limit,
    };
    let empty = Group {
        node: Some(svg),
        transform: Matrix::IDENTITY,
        opacity: 1.0,
        children: Vec::new(),
    };
    let mut viewport = Viewport {
        width,
        height,
        font_size,
        root_font_size,
    };
    let mut root = empty;
    match document.attr(svg, "viewBox").and_then(parse_view_box) {
        Some(view_box) if !view_box.has_area() => {}
        Some(view_box) => {
            let ratio = document
                .attr(svg, "preserveAspectRatio")
                .map(parse_preserve_aspect_ratio)
                .unwrap_or_default();
            let [a, b, c, d, e, f] = view_box_matrix(&view_box, &ratio, width, height);
            root.transform = Matrix::new(a, b, c, d, e, f);
            viewport.width = view_box.width;
            viewport.height = view_box.height;
            root.children = builder.children(svg, &viewport);
        }
        None => root.children = builder.children(svg, &viewport),
    }
    Tree {
        width,
        height,
        root,
        elements: builder.elements,
    }
}

struct Builder<'a> {
    document: &'a Document,
    styles: &'a Styles,
    elements: usize,
    /// The most elements built.
    limit: usize,
}

impl Builder<'_> {
    fn children(&mut self, parent: NodeId, viewport: &Viewport) -> Vec<Item> {
        let mut out = Vec::new();
        for child in self.document.element_children(parent) {
            if self.elements >= self.limit {
                break;
            }
            if let Some(item) = self.item(child, viewport) {
                out.push(item);
            }
        }
        out
    }

    /// What the element `node` draws: nothing for `display: none`, an
    /// element of another namespace, or one SVG does not draw.
    fn item(&mut self, node: NodeId, viewport: &Viewport) -> Option<Item> {
        let tag = self.document.svg_tag(node)?;
        let style = self.styles.get(node)?;
        if style.display == Display::None {
            return None;
        }
        self.elements += 1;
        let viewport = &Viewport {
            font_size: f64::from(style.font_size_px),
            ..*viewport
        };
        match tag {
            "g" | "a" => {
                let children = self.children(node, viewport);
                if children.is_empty() {
                    return None;
                }
                Some(Item::Group(Group {
                    node: Some(node),
                    transform: transform_of(style, viewport, None).unwrap_or(Matrix::IDENTITY),
                    opacity: f64::from(style.effects().opacity),
                    children,
                }))
            }
            "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "path" => {
                self.shape(node, tag, style, viewport)
            }
            _ => None,
        }
    }

    fn shape(
        &mut self,
        node: NodeId,
        tag: &str,
        style: &ComputedStyle,
        viewport: &Viewport,
    ) -> Option<Item> {
        let (path, bounds) = geometry(self.document, node, tag, style, viewport)?;
        let svg = style.svg();
        // A line has no inside to fill.
        let fill = paint(&svg.fill, style.color)
            .filter(|_| tag != "line")
            .map(|paint| Fill {
                paint,
                opacity: f64::from(svg.fill_opacity),
                rule: fill_rule(svg.fill_rule),
            });
        let stroke = stroke(style, viewport);
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let layers = svg.paint_order.layers();
        let at = |layer| layers.iter().position(|l| *l == layer);
        let opacity = f64::from(style.effects().opacity);
        let mut shape = Shape {
            node,
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
        };
        let transform = transform_of(style, viewport, Some(bounds));
        let overlap = opacity < 1.0 && shape.fill.is_some() && shape.stroke.is_some();
        if transform.is_none() && !overlap {
            shape.opacity = opacity;
            return Some(Item::Shape(shape));
        }
        Some(Item::Group(Group {
            node: None,
            transform: transform.unwrap_or(Matrix::IDENTITY),
            opacity,
            children: vec![Item::Shape(shape)],
        }))
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
    let m = Matrix::new(
        f64::from(a),
        f64::from(b),
        f64::from(c),
        f64::from(d),
        f64::from(e),
        f64::from(f),
    );
    m.is_finite().then_some(m)
}

/// A colour of the cascade's as the canvas has one.
pub(crate) fn rgba(c: computed::Color) -> Rgba {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba::new(byte(c.r), byte(c.g), byte(c.b), byte(c.a))
}

/// What a paint paints with: `None` for `none`, and for what draws as
/// it (`context-fill`, `context-stroke`, a reference to nothing with no
/// fallback).
fn paint(paint_value: &SvgPaint, color: computed::Color) -> Option<Paint> {
    match paint_value {
        SvgPaint::None | SvgPaint::ContextFill | SvgPaint::ContextStroke => None,
        SvgPaint::Color(c) => Some(Paint::Color(rgba(*c))),
        SvgPaint::CurrentColor => Some(Paint::Color(rgba(color))),
        SvgPaint::Url(_, fallback) => fallback.as_deref().and_then(|f| paint(f, color)),
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

/// A shape's stroke: `None` for no paint or a width not above 0.
fn stroke(style: &ComputedStyle, viewport: &Viewport) -> Option<Stroke> {
    let svg = style.svg();
    let paint = paint(&svg.stroke, style.color)?;
    let width = resolve(&svg.stroke_width, Axis::Diagonal, viewport);
    if !(width > 0.0 && width.is_finite()) {
        return None;
    }
    let mut dashes: Vec<f64> = svg
        .stroke_dasharray
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|d| resolve(d, Axis::Diagonal, viewport))
        .collect();
    // Dashes that sum to nothing, or one below zero (a percentage of a
    // calculation), draw a solid line.
    if dashes.iter().any(|d| d.is_nan() || *d < 0.0) || dashes.iter().sum::<f64>() <= 0.0 {
        dashes.clear();
    }
    Some(Stroke {
        paint,
        opacity: f64::from(svg.stroke_opacity),
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

/// A `width` or `height` of a shape: `auto` is 0 (SVG 2 section 10.2).
fn size(value: &Size, axis: Axis, viewport: &Viewport) -> f64 {
    value
        .length_percentage()
        .map_or(0.0, |v| resolve(&v, axis, viewport))
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
/// error or of no geometry (a radius or a size not above 0, no points,
/// no path data), which draws nothing.
fn geometry(
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
