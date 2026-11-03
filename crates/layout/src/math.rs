//! MathML layout (MathML Core sections 3.1 to 3.4, gap batch H, D77): a
//! box of `display: math` or `block math` whose element is one of the
//! layouts [`MathElement`] names lays its children out by that layout's
//! rules, here, each child an atomic piece with a baseline, laid out as
//! an inline-block is ([`layout_inline_block`]); a token element (`mi`,
//! `mn`, `mo`, `mtext`, `ms`) is a block container of its text, its
//! baseline its line's, and a table (`mtable`) is a CSS table.
//!
//! - A row (`math`, `mrow` and the elements laid out as one) puts its
//!   children side by side on its baseline, each operator (`mo`) with
//!   the space the operator dictionary of MathML Core's appendix B puts
//!   before and after it, by its form, which its place in the row gives;
//!   a fence or a bar is stretched to cover the row's other children
//!   about the math axis, its glyph scaled. A block formula is placed in
//!   its line by its `text-align`, centred by the UA sheet.
//! - A fraction (`mfrac`) has its numerator over its denominator, both
//!   centred, with a rule between them on the math axis, as section
//!   3.3.2 shifts them, by its `math-style`'s constants.
//! - A radical (`msqrt`, `mroot`) has the radical sign before its
//!   content and an overbar above it (section 3.3.3); `mroot`'s index
//!   is raised before the sign.
//! - Scripts (`msub`, `msup`, `msubsup`) are shifted from their base's
//!   baseline as section 3.4.1 shifts them, and limits (`munder`,
//!   `mover`, `munderover`) are stacked centred under and over their
//!   base (section 3.4.2).
//! - `mspace` is a box of its `width`, `height` above its baseline and
//!   `depth` below it.
//!
//! The constants are the font's MATH table's ([`MathConstant`]) where it
//! has one; where it has none, the fallbacks are MathML Core's where it
//! gives them, and the rest are those of TeX's Computer Modern, scaled
//! by the font size, so that a formula set in a text font still looks
//! set. A layout given children other than it takes (a fraction of one
//! child, say) lays them out as a row, as an `merror` would show them.

use std::sync::Arc;

use style::ComputedStyle;
use style::computed::{LEFT, MathElement, MathStyle, TOP, TextAlign};
use text::MathConstant;

use crate::block::{
    AtomicLayout, BlockLayout, Deferred, Edges, box_fragment, carried, layout_inline_block,
    settle_absolutes,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::pass::Pass;
use crate::{Fragment, FragmentKind};

/// Whether `b` is laid out by this module: a box of `display: math` of
/// one of the layouts past a token's.
pub(crate) fn lays_out(b: &LayoutBox) -> bool {
    b.style.display.is_math()
        && b.replaced.is_none()
        && !matches!(
            b.style.text().math,
            MathElement::None | MathElement::Token | MathElement::Operator
        )
}

/// The box's `(min-content, max-content)` content widths: both the width
/// its layout gives its content, which does not wrap.
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let width = arrange(b, pass).width;
    (width, width)
}

/// Lays out the box `b` (see the module doc), its content box
/// `content_width` wide, as [`crate::block::layout_contents`] does a
/// block container's.
pub(crate) fn layout_math_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    content_width: f32,
    margin_left: f32,
    pass: &Pass<'a>,
) -> BlockLayout<'a> {
    let arranged = arrange(b, pass);
    let width = content_width.max(arranged.width);
    let shift = match b.style.text_align {
        TextAlign::Center => (width - arranged.width) / 2.0,
        TextAlign::Right | TextAlign::End => width - arranged.width,
        _ => 0.0,
    };
    let content_x = edges.border[LEFT] + edges.padding[LEFT];
    let content_y = edges.border[TOP] + edges.padding[TOP];
    let content_height = arranged.ascent + arranged.descent;
    let height = content_height + edges.vertical();
    let mut root = box_fragment(width + edges.horizontal(), height, b);
    let (dx, dy) = (content_x + shift, content_y + arranged.ascent);
    let mut absolutes = Vec::new();
    for (fragment, list) in arranged.pieces {
        absolutes.extend(carried(list, dx, dy));
        root.children.push(fragment.translated(dx, dy));
    }
    absolutes.extend(carried(arranged.absolutes, dx, dy));
    let absolutes = settle_absolutes(b, &mut root, &edges.border, absolutes, pass);
    crate::ruby::finished(
        b,
        root,
        height,
        cb_width,
        margin_left,
        Some(content_y + arranged.ascent),
        content_height,
        absolutes,
    )
}

/// A box's content arranged: its width, its ascent above its baseline
/// and descent below it, and its fragments with the absolutely
/// positioned boxes met in each, relative to the content box's left
/// edge and the baseline.
struct Arranged<'a> {
    width: f32,
    ascent: f32,
    descent: f32,
    pieces: Vec<(Fragment, Vec<Deferred<'a>>)>,
    /// The box's own absolutely positioned children, at the content's
    /// end, its top.
    absolutes: Vec<Deferred<'a>>,
}

impl<'a> Arranged<'a> {
    fn new() -> Self {
        Arranged {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            pieces: Vec::new(),
            absolutes: Vec::new(),
        }
    }

    /// Puts `piece` with its margin box's left edge at `x` and its
    /// baseline `shift` below the baseline (above it when negative), the
    /// extent grown to hold it.
    fn put(&mut self, piece: Piece<'a>, x: f32, shift: f32) {
        self.width = self.width.max(x + piece.width);
        self.ascent = self.ascent.max(piece.ascent - shift);
        self.descent = self.descent.max(piece.descent + shift);
        let fx = x + piece.margin_left + piece.relative.0;
        let fy = shift - piece.ascent + piece.margin_top + piece.relative.1;
        let list = carried(
            piece.absolutes,
            x + piece.margin_left,
            fy - piece.relative.1,
        );
        self.pieces.push((piece.root.translated(fx, fy), list));
    }

    /// Puts a fragment drawn here (a rule or a sign), which reaches
    /// `ascent` above the baseline and `descent` below it, at `x`.
    fn draw(&mut self, fragment: Fragment, ascent: f32, descent: f32) {
        self.width = self.width.max(fragment.x + fragment.w);
        self.ascent = self.ascent.max(ascent);
        self.descent = self.descent.max(descent);
        self.pieces.push((fragment, Vec::new()));
    }
}

/// A child laid out, as an inline-block is.
struct Piece<'a> {
    root: Fragment,
    /// Its margin box's width.
    width: f32,
    /// Above and below its baseline, its margin box's.
    ascent: f32,
    descent: f32,
    margin_left: f32,
    margin_top: f32,
    relative: (f32, f32),
    absolutes: Vec<Deferred<'a>>,
}

impl<'a> Piece<'a> {
    fn of(layout: AtomicLayout<'a>) -> Self {
        Piece {
            width: layout.width,
            ascent: layout.ascent,
            descent: layout.height - layout.ascent,
            margin_left: layout.margin_left,
            margin_top: layout.margin_top,
            relative: layout.relative,
            absolutes: layout.absolutes,
            root: layout.root,
        }
    }
}

/// `child` laid out at its max-content width.
fn piece<'a>(child: &'a LayoutBox, pass: &Pass<'a>) -> Piece<'a> {
    let width = crate::intrinsic::contribution(child, pass).1;
    Piece::of(layout_inline_block(child, width, None, pass))
}

/// What a layout reads of its box's font: the constants, scaled.
struct Metrics {
    em: f32,
    x_height: f32,
    rule: f32,
    axis: f32,
    display: bool,
    style: Arc<ComputedStyle>,
    face: Option<Box<[Option<f32>]>>,
}

impl Metrics {
    fn of(b: &LayoutBox, pass: &Pass<'_>) -> Self {
        let style = &b.style;
        let em = style.font_size_px;
        let font = crate::font::font_of(pass.fonts, style);
        let face = pass.fonts.face(font.face);
        let x_height = face.x_height(em);
        // The default rule thickness: the font's underline's, else an
        // eighteenth of an em (MathML Core section 3.1.1's fallback).
        let rule = face
            .underline(em)
            .map_or(em / 18.0, |(_, thickness)| thickness)
            .max(1.0);
        let constants: Option<Box<[Option<f32>]>> = face
            .math_constant(MathConstant::AxisHeight, em)
            .map(|_| ALL.iter().map(|c| face.math_constant(*c, em)).collect());
        let axis = constants
            .as_ref()
            .and_then(|c| c[MathConstant::AxisHeight as usize])
            .unwrap_or(x_height / 2.0);
        Metrics {
            em,
            x_height,
            rule,
            axis,
            display: style.text().math_style == MathStyle::Normal,
            style: b.style.clone(),
            face: constants,
        }
    }

    /// The font's `constant`, else `fallback`.
    fn get(&self, constant: MathConstant, fallback: f32) -> f32 {
        self.face
            .as_ref()
            .and_then(|c| c[constant as usize])
            .unwrap_or(fallback)
    }
}

/// The constants [`Metrics`] reads, in [`MathConstant`]'s order.
const ALL: [MathConstant; 32] = [
    MathConstant::AxisHeight,
    MathConstant::FractionNumeratorShiftUp,
    MathConstant::FractionNumeratorDisplayStyleShiftUp,
    MathConstant::FractionDenominatorShiftDown,
    MathConstant::FractionDenominatorDisplayStyleShiftDown,
    MathConstant::FractionNumeratorGapMin,
    MathConstant::FractionNumDisplayStyleGapMin,
    MathConstant::FractionDenominatorGapMin,
    MathConstant::FractionDenomDisplayStyleGapMin,
    MathConstant::FractionRuleThickness,
    MathConstant::SubscriptShiftDown,
    MathConstant::SubscriptTopMax,
    MathConstant::SubscriptBaselineDropMin,
    MathConstant::SuperscriptShiftUp,
    MathConstant::SuperscriptBottomMin,
    MathConstant::SuperscriptBaselineDropMax,
    MathConstant::SubSuperscriptGapMin,
    MathConstant::SuperscriptBottomMaxWithSubscript,
    MathConstant::SpaceAfterScript,
    MathConstant::UpperLimitGapMin,
    MathConstant::UpperLimitBaselineRiseMin,
    MathConstant::LowerLimitGapMin,
    MathConstant::LowerLimitBaselineDropMin,
    MathConstant::OverbarVerticalGap,
    MathConstant::UnderbarVerticalGap,
    MathConstant::RadicalVerticalGap,
    MathConstant::RadicalDisplayStyleVerticalGap,
    MathConstant::RadicalRuleThickness,
    MathConstant::RadicalExtraAscender,
    MathConstant::RadicalKernBeforeDegree,
    MathConstant::RadicalKernAfterDegree,
    MathConstant::RadicalDegreeBottomRaisePercent,
];

/// `b`'s in-flow children: the absolutely positioned ones are left where
/// the box's content starts.
fn in_flow(b: &LayoutBox) -> impl Iterator<Item = &LayoutBox> {
    b.children.iter().filter(|c| c.kind != BoxKind::Absolute)
}

/// Lays `b`'s children out by its layout.
fn arrange<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> Arranged<'a> {
    let children: Vec<&'a LayoutBox> = in_flow(b).collect();
    let mut arranged = match (b.style.text().math, children.as_slice()) {
        (MathElement::Space { height, depth }, _) => {
            let mut arranged = Arranged::new();
            arranged.ascent = height;
            arranged.descent = depth;
            arranged
        }
        (MathElement::Fraction { thickness }, [numerator, denominator]) => {
            fraction(b, numerator, denominator, thickness, pass)
        }
        (MathElement::Sqrt, _) => radical(b, row(b, &children, pass), None, pass),
        (MathElement::Root, [base, index]) => radical(b, row(b, &[base], pass), Some(index), pass),
        (MathElement::Sub, [base, sub]) => scripts(b, base, Some(sub), None, pass),
        (MathElement::Sup, [base, sup]) => scripts(b, base, None, Some(sup), pass),
        (MathElement::SubSup, [base, sub, sup]) => scripts(b, base, Some(sub), Some(sup), pass),
        (MathElement::Under, [base, under]) => limits(b, base, Some(under), None, pass),
        (MathElement::Over, [base, over]) => limits(b, base, None, Some(over), pass),
        (MathElement::UnderOver, [base, under, over]) => {
            limits(b, base, Some(under), Some(over), pass)
        }
        _ => row(b, &children, pass),
    };
    for child in b.children.iter().filter(|c| c.kind == BoxKind::Absolute) {
        let at = Deferred::at(child, arranged.width, -arranged.ascent);
        arranged.absolutes.push(at);
    }
    arranged
}

/// The form an operator takes by its place in a row of `count` children
/// (MathML Core section 3.2.4.1): `prefix` first, `postfix` last, else
/// `infix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Prefix,
    Infix,
    Postfix,
}

/// The text of the operator `b`, its text boxes' in order.
fn operator_text(b: &LayoutBox) -> String {
    let mut out = String::new();
    fn walk(b: &LayoutBox, out: &mut String) {
        if let BoxKind::Text(text) = &b.kind {
            out.push_str(text);
        }
        for child in &b.children {
            walk(child, out);
        }
    }
    walk(b, &mut out);
    out.trim().to_owned()
}

/// The space before and after an operator of `text` in `form`, in
/// eighteenths of an em, by MathML Core's operator dictionary (appendix
/// B) for the operators met most: what is not in it takes the space of
/// a relation, the dictionary's default.
fn spacing(text: &str, form: Form) -> (f32, f32) {
    match (text, form) {
        ("(" | "[" | "{" | "|" | "\u{2016}" | "\u{27e8}" | "\u{2308}" | "\u{230a}", _)
        | (")" | "]" | "}" | "\u{27e9}" | "\u{2309}" | "\u{230b}", _) => (0.0, 0.0),
        ("+" | "-" | "\u{2212}" | "\u{b1}" | "\u{2213}", Form::Prefix) => (0.0, 1.0),
        ("+" | "-" | "\u{2212}" | "\u{b1}" | "\u{2213}", _) => (4.0, 4.0),
        ("\u{d7}" | "\u{b7}" | "\u{22c5}" | "\u{2217}" | "\u{2218}" | "*" | "/", _) => (4.0, 4.0),
        ("," | ";", _) => (0.0, 3.0),
        ("!" | "'" | "\u{2032}", _) => (0.0, 0.0),
        ("\u{2211}" | "\u{220f}" | "\u{2210}" | "\u{22c3}" | "\u{22c2}", _) => (3.0, 3.0),
        ("\u{222b}" | "\u{222c}" | "\u{222d}" | "\u{222e}", _) => (3.0, 3.0),
        ("\u{2061}" | "\u{2062}" | "\u{2063}" | "\u{2064}", _) => (0.0, 0.0),
        (".", _) => (0.0, 0.0),
        _ => (5.0, 5.0),
    }
}

/// Whether an operator of `text` stretches to the row around it by
/// default (MathML Core's dictionary: the fences and the bars).
fn stretchy(text: &str) -> bool {
    matches!(
        text,
        "(" | ")"
            | "["
            | "]"
            | "{"
            | "}"
            | "|"
            | "\u{2016}"
            | "\u{27e8}"
            | "\u{27e9}"
            | "\u{2308}"
            | "\u{2309}"
            | "\u{230a}"
            | "\u{230b}"
    )
}

/// An operator of a row: its text, and the space before and after it.
type Spaced = (String, f32, f32);

/// `children` side by side as a row of `b` (section 3.1).
fn row<'a>(b: &'a LayoutBox, children: &[&'a LayoutBox], pass: &Pass<'a>) -> Arranged<'a> {
    let metrics = Metrics::of(b, pass);
    let count = children.len();
    let mut pieces: Vec<(Piece<'a>, Option<Spaced>)> = children
        .iter()
        .enumerate()
        .map(|(i, child)| {
            let operator = (child.style.text().math == MathElement::Operator).then(|| {
                let text = operator_text(child);
                let form = match i {
                    _ if count == 1 => Form::Infix,
                    0 => Form::Prefix,
                    i if i + 1 == count => Form::Postfix,
                    _ => Form::Infix,
                };
                let (before, after) = spacing(&text, form);
                let em = child.style.font_size_px / 18.0;
                (text, before * em, after * em)
            });
            (piece(child, pass), operator)
        })
        .collect();
    // The stretchy operators cover the others about the math axis.
    let (ascent, descent) = pieces
        .iter()
        .filter(|(_, op)| op.as_ref().is_none_or(|(text, ..)| !stretchy(text)))
        .fold((0.0f32, 0.0f32), |(a, d), (p, _)| {
            (a.max(p.ascent), d.max(p.descent))
        });
    let half = (ascent - metrics.axis).max(descent + metrics.axis);
    for (i, (p, op)) in pieces.iter_mut().enumerate() {
        if let Some((text, ..)) = op
            && stretchy(text)
            && 2.0 * half > p.ascent + p.descent
        {
            *p = stretched(children[i], text, 2.0 * half, metrics.axis, pass);
        }
    }
    let mut arranged = Arranged::new();
    let mut x = 0.0;
    for (p, op) in pieces {
        let (before, after) = op.map_or((0.0, 0.0), |(_, before, after)| (before, after));
        x += before;
        let width = p.width;
        arranged.put(p, x, 0.0);
        x += width + after;
    }
    arranged.width = arranged.width.max(x);
    arranged
}

/// The operator `b` of `text`, its glyph scaled to `height` centred on
/// the math axis, `axis` above the baseline.
fn stretched<'a>(
    b: &'a LayoutBox,
    text: &str,
    height: f32,
    axis: f32,
    pass: &Pass<'a>,
) -> Piece<'a> {
    let natural = crate::font::line(pass.fonts, text, &b.style);
    let tall = (natural.ascent + natural.descent).max(1.0);
    let mut style = (*b.style).clone();
    style.font_size_px *= height / tall;
    let style = Arc::new(style);
    let metrics = crate::font::line(pass.fonts, text, &style);
    let width = crate::font::measure(pass.fonts, text, &style);
    // The glyph's line box centred on the axis.
    let baseline = (metrics.ascent - metrics.descent) / 2.0 - axis;
    let ascent = metrics.ascent - baseline;
    let descent = metrics.descent + baseline;
    let mut root = box_fragment(width, ascent + descent, b);
    root.children.push(glyph(b, style, text, 0.0, ascent, pass));
    Piece {
        root,
        width,
        ascent,
        descent,
        margin_left: 0.0,
        margin_top: 0.0,
        relative: (0.0, 0.0),
        absolutes: Vec::new(),
    }
}

/// A text fragment of `text` in `style`, of the box `b`, its baseline at
/// `(x, y)`.
fn glyph(
    b: &LayoutBox,
    style: Arc<ComputedStyle>,
    text: &str,
    x: f32,
    y: f32,
    pass: &Pass<'_>,
) -> Fragment {
    let metrics = crate::font::line(pass.fonts, text, &style);
    let font = crate::font::font_for(pass.fonts, &style, text);
    Fragment {
        x,
        y,
        w: crate::font::measure(pass.fonts, text, &style),
        h: metrics.ascent + metrics.descent,
        ascent: metrics.ascent,
        kind: FragmentKind::Text(text.to_owned()),
        font,
        children: Default::default(),
        tree_order: 0,
        parent: Some(b.node),
        style,
        ..box_fragment(0.0, 0.0, b)
    }
}

/// A rule of `b`'s colour, `w` by `h`, its top-left at `(x, y)`.
fn rule(b: &LayoutBox, x: f32, y: f32, w: f32, h: f32) -> Fragment {
    let mut style = ComputedStyle::inherited_from(&b.style);
    style.background_color = b.style.color.used();
    let mut fragment = box_fragment(w, h, b);
    fragment.style = Arc::new(style);
    fragment.translated(x, y)
}

/// A fraction of `numerator` over `denominator` (section 3.3.2), its
/// rule `thickness` thick or the font's.
fn fraction<'a>(
    b: &'a LayoutBox,
    numerator: &'a LayoutBox,
    denominator: &'a LayoutBox,
    thickness: Option<f32>,
    pass: &Pass<'a>,
) -> Arranged<'a> {
    let m = Metrics::of(b, pass);
    let t = thickness.unwrap_or_else(|| m.get(MathConstant::FractionRuleThickness, m.rule));
    let (num, den) = (piece(numerator, pass), piece(denominator, pass));
    let (shift_up, gap_num, shift_down, gap_den) = if m.display {
        (
            m.get(MathConstant::FractionNumeratorDisplayStyleShiftUp, 0.0),
            m.get(MathConstant::FractionNumDisplayStyleGapMin, 3.0 * m.rule),
            m.get(MathConstant::FractionDenominatorDisplayStyleShiftDown, 0.0),
            m.get(MathConstant::FractionDenomDisplayStyleGapMin, 3.0 * m.rule),
        )
    } else {
        (
            m.get(MathConstant::FractionNumeratorShiftUp, 0.0),
            m.get(MathConstant::FractionNumeratorGapMin, m.rule),
            m.get(MathConstant::FractionDenominatorShiftDown, 0.0),
            m.get(MathConstant::FractionDenominatorGapMin, m.rule),
        )
    };
    let up = shift_up.max(m.axis + t / 2.0 + gap_num + num.descent);
    let down = shift_down.max(gap_den + den.ascent + t / 2.0 - m.axis);
    let width = num.width.max(den.width);
    let mut arranged = Arranged::new();
    let (num_x, den_x) = ((width - num.width) / 2.0, (width - den.width) / 2.0);
    arranged.put(num, num_x, -up);
    arranged.put(den, den_x, down);
    if t > 0.0 {
        let top = -(m.axis + t / 2.0);
        arranged.draw(rule(b, 0.0, top, width, t), -top, top + t);
    }
    arranged.width = width;
    arranged
}

/// A radical over `radicand` (section 3.3.3), with `index` raised before
/// its sign for `mroot`.
fn radical<'a>(
    b: &'a LayoutBox,
    radicand: Arranged<'a>,
    index: Option<&'a LayoutBox>,
    pass: &Pass<'a>,
) -> Arranged<'a> {
    let m = Metrics::of(b, pass);
    let t = m.get(MathConstant::RadicalRuleThickness, m.rule);
    let gap = if m.display {
        m.get(
            MathConstant::RadicalDisplayStyleVerticalGap,
            t + m.x_height / 4.0,
        )
    } else {
        m.get(MathConstant::RadicalVerticalGap, t + t / 4.0)
    };
    let extra = m.get(MathConstant::RadicalExtraAscender, t);
    // The sign covers the radicand with the gap and the rule over it.
    let top = radicand.ascent + gap + t;
    let height = top + radicand.descent;
    let sign = "\u{221a}";
    let natural = crate::font::line(pass.fonts, sign, &m.style);
    let mut style = (*m.style).clone();
    style.font_size_px *= (height / (natural.ascent + natural.descent).max(1.0)).max(1.0);
    let style = Arc::new(style);
    let sign_metrics = crate::font::line(pass.fonts, sign, &style);
    let sign_width = crate::font::measure(pass.fonts, sign, &style);
    let mut arranged = Arranged::new();
    // The index, raised: its bottom the percentage of the sign's height
    // up from the sign's bottom, with the kerns around it.
    let mut x = 0.0;
    if let Some(index) = index {
        let index = piece(index, pass);
        let before = m.get(MathConstant::RadicalKernBeforeDegree, 5.0 * m.em / 18.0);
        let after = m.get(MathConstant::RadicalKernAfterDegree, -10.0 * m.em / 18.0);
        let raise = m.get(MathConstant::RadicalDegreeBottomRaisePercent, 60.0) / 100.0;
        let bottom = radicand.descent - raise * height;
        let (width, shift) = (index.width, bottom - index.descent);
        arranged.put(index, before.max(0.0), shift);
        x = (before + width + after).max(0.0);
    }
    let sign_baseline = -top + sign_metrics.ascent;
    arranged.draw(
        glyph(b, style, sign, x, sign_baseline, pass),
        top,
        sign_baseline + sign_metrics.descent,
    );
    x += sign_width;
    let width = radicand.width;
    arranged.draw(rule(b, x, -top, width, t), top + extra, -top + t);
    for (fragment, list) in radicand.pieces {
        arranged
            .pieces
            .push((fragment.translated(x, 0.0), carried(list, x, 0.0)));
    }
    arranged
        .absolutes
        .extend(carried(radicand.absolutes, x, 0.0));
    arranged.ascent = arranged.ascent.max(radicand.ascent);
    arranged.descent = arranged.descent.max(radicand.descent);
    arranged.width = arranged.width.max(x + width);
    arranged
}

/// A base with a subscript, a superscript or both (section 3.4.1).
fn scripts<'a>(
    b: &'a LayoutBox,
    base: &'a LayoutBox,
    sub: Option<&'a LayoutBox>,
    sup: Option<&'a LayoutBox>,
    pass: &Pass<'a>,
) -> Arranged<'a> {
    let m = Metrics::of(b, pass);
    let base = piece(base, pass);
    let (base_ascent, base_descent, base_width) = (base.ascent, base.descent, base.width);
    let sub = sub.map(|s| piece(s, pass));
    let sup = sup.map(|s| piece(s, pass));
    let mut down = sub.as_ref().map_or(0.0, |s| {
        m.get(MathConstant::SubscriptShiftDown, 0.15 * m.em)
            .max(base_descent + m.get(MathConstant::SubscriptBaselineDropMin, 0.05 * m.em))
            .max(s.ascent - m.get(MathConstant::SubscriptTopMax, 0.8 * m.x_height))
    });
    let mut up = sup.as_ref().map_or(0.0, |s| {
        m.get(MathConstant::SuperscriptShiftUp, 0.413 * m.em)
            .max(base_ascent - m.get(MathConstant::SuperscriptBaselineDropMax, 0.386 * m.em))
            .max(m.get(MathConstant::SuperscriptBottomMin, m.x_height / 4.0) + s.descent)
    });
    // Both: kept apart by the least gap, the superscript raised as far
    // as it may first, then the subscript lowered.
    if let (Some(sb), Some(sp)) = (&sub, &sup) {
        let gap_min = m.get(MathConstant::SubSuperscriptGapMin, 4.0 * m.rule);
        let gap = (up - sp.descent) - (sb.ascent - down);
        if gap < gap_min {
            let max_bottom = m.get(
                MathConstant::SuperscriptBottomMaxWithSubscript,
                0.8 * m.x_height,
            );
            let raise = (max_bottom - (up - sp.descent)).clamp(0.0, gap_min - gap);
            up += raise;
            down += gap_min - gap - raise;
        }
    }
    let after = m.get(MathConstant::SpaceAfterScript, m.em / 24.0);
    let mut arranged = Arranged::new();
    arranged.put(base, 0.0, 0.0);
    let mut width = base_width;
    if let Some(sb) = sub {
        width = width.max(base_width + sb.width);
        arranged.put(sb, base_width, down);
    }
    if let Some(sp) = sup {
        width = width.max(base_width + sp.width);
        arranged.put(sp, base_width, -up);
    }
    arranged.width = width + after;
    arranged
}

/// A base with a limit under it, over it or both (section 3.4.2), each
/// centred on the widest.
fn limits<'a>(
    b: &'a LayoutBox,
    base: &'a LayoutBox,
    under: Option<&'a LayoutBox>,
    over: Option<&'a LayoutBox>,
    pass: &Pass<'a>,
) -> Arranged<'a> {
    let m = Metrics::of(b, pass);
    let base = piece(base, pass);
    let under = under.map(|u| piece(u, pass));
    let over = over.map(|o| piece(o, pass));
    let width = [Some(&base), under.as_ref(), over.as_ref()]
        .into_iter()
        .flatten()
        .map(|p| p.width)
        .fold(0.0, f32::max);
    let (base_ascent, base_descent) = (base.ascent, base.descent);
    let mut arranged = Arranged::new();
    let centred = |p: &Piece<'_>| (width - p.width) / 2.0;
    let x = centred(&base);
    arranged.put(base, x, 0.0);
    if let Some(u) = under {
        let gap = m.get(
            MathConstant::LowerLimitGapMin,
            m.get(MathConstant::UnderbarVerticalGap, 3.0 * m.rule),
        );
        let drop = m.get(MathConstant::LowerLimitBaselineDropMin, 0.0);
        let down = (base_descent + gap + u.ascent).max(drop);
        let x = centred(&u);
        arranged.put(u, x, down);
    }
    if let Some(o) = over {
        let gap = m.get(
            MathConstant::UpperLimitGapMin,
            m.get(MathConstant::OverbarVerticalGap, 3.0 * m.rule),
        );
        let rise = m.get(MathConstant::UpperLimitBaselineRiseMin, 0.0);
        let up = (base_ascent + gap + o.descent).max(rise);
        let x = centred(&o);
        arranged.put(o, x, -up);
    }
    arranged.width = width;
    arranged
}

#[cfg(test)]
mod tests {
    use crate::block::{Fragment, FragmentKind};
    use crate::ruby::tests::{element, laid, text};

    /// The rules drawn: the box fragments with a background.
    fn rules(l: &crate::testing::Laid) -> Vec<&Fragment> {
        l.iter()
            .filter(|f| matches!(f.kind, FragmentKind::Box) && f.style.background_color.a > 0.0)
            .collect()
    }

    #[test]
    fn a_fraction_sets_its_numerator_over_its_denominator_about_a_rule() {
        let l = laid(
            "<p><math><mfrac><mi>a</mi><mn>22</mn></mfrac></math></p>",
            "p { margin: 0 }",
        );
        let (a, two) = (text(&l, "a"), text(&l, "22"));
        assert!(a.top() + a.h <= two.top(), "over it");
        assert!(
            (a.x + a.w / 2.0 - (two.x + two.w / 2.0)).abs() < 0.5,
            "centred"
        );
        let drawn = rules(&l);
        assert_eq!(drawn.len(), 1);
        let rule = drawn[0];
        assert!(rule.y >= a.y && rule.y + rule.h <= two.top() + two.h);
        assert!((rule.w - two.w).abs() < 0.01, "as wide as the wider part");
        // In a formula set inline, its parts are a size smaller.
        assert!((a.style.font_size_px - 16.0 * 0.71).abs() < 0.01);
        let l = laid(
            "<math display=block><mfrac linethickness=0><mi>a</mi><mn>2</mn></mfrac></math>",
            "",
        );
        assert!(rules(&l).is_empty(), "a linethickness of 0 draws none");
        assert_eq!(text(&l, "a").style.font_size_px, 16.0, "display style");
    }

    #[test]
    fn scripts_are_shifted_up_and_down_after_their_base() {
        let l = laid(
            "<math><msup><mi>x</mi><mn>2</mn></msup><msub><mi>y</mi><mn>1</mn></msub>\
             <msubsup><mi>z</mi><mn>3</mn><mn>4</mn></msubsup></math>",
            "",
        );
        let (x, two) = (text(&l, "x"), text(&l, "2"));
        assert!(two.y < x.y && two.x >= x.x + x.w - 0.01);
        let (y, one) = (text(&l, "y"), text(&l, "1"));
        assert!(one.y > y.y && one.x >= y.x + y.w - 0.01);
        let (sub, sup) = (text(&l, "3"), text(&l, "4"));
        assert!(sup.top() + sup.h <= sub.top() + 0.01 || sup.y < sub.y - sub.h / 2.0);
        assert!((two.style.font_size_px - 16.0 * 0.71).abs() < 0.01);
        assert_eq!(x.style.font_size_px, 16.0, "the base keeps its size");
    }

    #[test]
    fn a_root_has_its_sign_and_a_bar_over_its_content() {
        let l = laid("<math><msqrt><mi>x</mi></msqrt></math>", "");
        let (sign, x) = (text(&l, "\u{221a}"), text(&l, "x"));
        assert!(sign.x + sign.w <= x.x + 0.01);
        let drawn = rules(&l);
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].y + drawn[0].h <= x.top() + 0.01, "over it");
        let l = laid("<math><mroot><mi>x</mi><mn>3</mn></mroot></math>", "");
        let (index, sign) = (text(&l, "3"), text(&l, "\u{221a}"));
        // Raised, and tucked into the sign's crook by the kern after it.
        assert!(index.y < text(&l, "x").y && index.x < sign.x + sign.w / 2.0);
    }

    #[test]
    fn limits_are_centred_under_and_over_their_base() {
        let l = laid(
            "<math><munderover><mo>\u{2211}</mo><mi>i</mi><mi>n</mi></munderover></math>",
            "",
        );
        let (sum, under, over) = (text(&l, "\u{2211}"), text(&l, "i"), text(&l, "n"));
        assert!(under.top() >= sum.top() + sum.h - 0.01);
        assert!(over.top() + over.h <= sum.top() + 0.01);
        let centre = |f: &Fragment| f.x + f.w / 2.0;
        assert!((centre(under) - centre(sum)).abs() < 0.5);
        assert!((centre(over) - centre(sum)).abs() < 0.5);
    }

    #[test]
    fn operators_take_their_dictionarys_space_and_fences_stretch() {
        let l = laid("<math><mi>a</mi><mo>+</mo><mi>b</mi></math>", "");
        let (a, plus, b) = (text(&l, "a"), text(&l, "+"), text(&l, "b"));
        let em = 16.0 / 18.0;
        assert!((plus.x - (a.x + a.w) - 4.0 * em).abs() < 0.5);
        assert!((b.x - (plus.x + plus.w) - 4.0 * em).abs() < 0.5);
        let l = laid(
            "<math display=block><mo>(</mo><mfrac><mi>a</mi><mi>b</mi></mfrac><mo>)</mo></math>",
            "",
        );
        let paren = text(&l, "(");
        assert!(paren.style.font_size_px > 16.0, "stretched");
        assert!(paren.h >= text(&l, "b").top() + text(&l, "b").h - text(&l, "a").top() - 0.5);
    }

    #[test]
    fn a_block_formula_is_centred_and_mspace_has_its_size() {
        let l = laid("<math display=block><mi>x</mi></math>", "");
        let x = text(&l, "x");
        assert!((x.x + x.w / 2.0 - 400.0).abs() < 0.5);
        let l = laid(
            "<p><math><mspace width=30px height=20px depth=10px></mspace></math></p>",
            "p { margin: 0 }",
        );
        let space = element(&l, "mspace");
        assert_eq!((space.w, space.h), (30.0, 30.0));
    }

    #[test]
    fn a_font_without_a_math_table_sets_a_formula_by_the_fallbacks() {
        // The repository's monospace face has no MATH table.
        let mono = include_bytes!("../../text/fonts/DejaVuSansMono.ttf");
        let face = text::FontStore::from_bytes(mono.to_vec(), 0).expect("parses");
        assert_eq!(
            face.math_constant(text::MathConstant::AxisHeight, 16.0),
            None
        );
        let l = crate::ruby::tests::laid_in(
            mono,
            "<math><mfrac><mi>a</mi><mi>b</mi></mfrac><msup><mi>x</mi><mn>2</mn></msup></math>",
            "",
        );
        let (a, b) = (text(&l, "a"), text(&l, "b"));
        let drawn = rules(&l);
        assert_eq!(drawn.len(), 1);
        let rule = drawn[0];
        // The rule on the axis, half the x-height up, the parts a rule's
        // thickness away from it at least.
        assert!(a.top() + a.h <= rule.y + 0.01 && rule.y + rule.h <= b.top() + 0.01);
        let (x, two) = (text(&l, "x"), text(&l, "2"));
        assert!(x.y > two.y);
        // Over a base reaching nowhere, raised by TeX's 0.413em.
        let l = crate::ruby::tests::laid_in(
            mono,
            "<math><mn>1</mn><msup><mspace width=4px></mspace><mn>2</mn></msup></math>",
            "",
        );
        let (one, two) = (text(&l, "1"), text(&l, "2"));
        assert!(
            (one.y - two.y - 0.413 * 16.0).abs() < 0.5,
            "{} {}",
            one.y,
            two.y
        );
    }

    #[test]
    fn children_a_layout_does_not_take_are_set_in_a_row() {
        let l = laid("<math><mfrac><mi>a</mi></mfrac><mi>b</mi></math>", "");
        let (a, b) = (text(&l, "a"), text(&l, "b"));
        assert_eq!(a.y, b.y);
        assert!(rules(&l).is_empty());
    }
}
