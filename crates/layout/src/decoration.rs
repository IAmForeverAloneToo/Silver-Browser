//! Text decoration (Text Decoration 3 and 4, M21.7): the decorations in
//! force over a run of text, and the lines they are drawn as.
//!
//! A decoration is propagated, not inherited (Text Decoration 3 §2):
//! from the element that names it to the text in its inline formatting
//! context and in its in-flow block descendants, but not into a float,
//! an absolutely positioned box, an atomic inline or a replaced element,
//! whose content takes the decorations it names itself alone. Each keeps
//! its element's colour, style, thickness, underline offset and position
//! and font size, whatever the text it decorates has: the reference
//! browser draws one underline of the same thickness under a paragraph's
//! 16-pixel text and a 32-pixel span in it. The box tree gives each text
//! box the decorations in force over it, outermost element's first
//! ([`crate::LayoutBox::decorations`]).
//!
//! A line is drawn from the line box's baseline `B` with its element's
//! font, as the reference browser draws it, measured at 12 to 40
//! pixels: with `A` the font's ascent rounded, `D` its descent rounded
//! and `T` the thickness — a tenth of the font size for `auto`, the
//! font's underline thickness for `from-font`, the value given otherwise
//! — the line is `max(1, ⌊T⌋)` pixels thick for `auto` and `T` rounded,
//! at least one, otherwise. An underline's top is `B + max(1, ⌈T/2⌉)`,
//! or `B` plus the rounded `text-underline-offset` when one is given; `B
//! + D` plus the offset for `under`, and `B` plus the font's underline
//! position for `from-font`. An overline's top is `B − A − t`, a
//! line-through's `B − A/3 − T/2` rounded; a `double` line's second line
//! is the first's top plus `T + 1`, rounded.

use std::sync::Arc;

use style::ComputedStyle;
use style::computed::{
    Color, DecorationLine, DecorationStyle, DecorationThickness, Float, LengthPercentage,
    UnderlinePosition,
};
use text::FontCollection;

/// A decoration in force over text: an element's `text-decoration` as
/// the text it decorates takes it, with the element's values.
#[derive(Debug, Clone, PartialEq)]
pub struct Decorating {
    pub line: DecorationLine,
    pub style: DecorationStyle,
    /// `text-decoration-color`, `currentcolor` the element's `color`.
    pub color: Color,
    pub thickness: DecorationThickness,
    pub underline_offset: Option<LengthPercentage>,
    pub underline_position: UnderlinePosition,
    /// The element's font size, which its lines are drawn with.
    pub font_size: f32,
}

impl Decorating {
    /// The decoration the element styled `style` names, if it draws a
    /// line.
    pub(crate) fn of(style: &ComputedStyle) -> Option<Decorating> {
        let decoration = style.decoration();
        decoration.line.draws().then(|| Decorating {
            line: decoration.line,
            style: decoration.style,
            color: decoration.color.unwrap_or(style.color),
            thickness: decoration.thickness.clone(),
            underline_offset: decoration.underline_offset.clone(),
            underline_position: decoration.underline_position,
            font_size: style.font_size_px,
        })
    }
}

/// Whether the box an element styled `style` generates stops the
/// decorations of the elements around it from reaching its content: a
/// float, an absolutely positioned box, or an atomic inline.
pub(crate) fn stops_propagation(style: &ComputedStyle) -> bool {
    style.float != Float::None
        || style.position.is_out_of_flow()
        || style.display.is_atomic_inline()
}

/// The decorations `parent`'s give with `own` added inside them:
/// `parent`'s own list when `own` adds none, so that runs of text the
/// same decorations are over share it.
pub(crate) fn within(
    parent: Option<&Arc<[Decorating]>>,
    own: Option<Decorating>,
) -> Option<Arc<[Decorating]>> {
    match (parent, own) {
        (parent, None) => parent.cloned(),
        (None, Some(own)) => Some(Arc::from([own])),
        (Some(parent), Some(own)) => Some(parent.iter().cloned().chain([own]).collect()),
    }
}

/// Which line a [`DecorationLine`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Underline,
    Overline,
    LineThrough,
}

/// One line a decoration draws over a run of text, placed from the
/// line box's baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnLine {
    pub kind: LineKind,
    pub style: DecorationStyle,
    pub color: Color,
    /// Its top, below the baseline (above it when negative).
    pub top: f32,
    /// How thick it is, in whole pixels.
    pub thickness: f32,
    /// A `double` line's second line's top, the same way.
    pub second: Option<f32>,
}

/// The lines `decorations` draw, outermost element's first, each
/// element's underline, overline and line-through in that order.
pub(crate) fn lines(decorations: &[Decorating], fonts: &FontCollection) -> Vec<DrawnLine> {
    let mut out = Vec::new();
    for d in decorations {
        let metrics = fonts.primary().metrics(d.font_size);
        let (ascent, descent) = (metrics.ascent, metrics.descent);
        let underline = fonts.underline(d.font_size);
        let thickness = match &d.thickness {
            DecorationThickness::Auto => d.font_size / 10.0,
            DecorationThickness::FromFont => underline.map_or(d.font_size / 10.0, |(_, t)| t),
            DecorationThickness::Length(length) => length.resolve(d.font_size),
        };
        let drawn = match d.thickness {
            DecorationThickness::Auto => thickness.floor().max(1.0),
            _ => thickness.round().max(1.0),
        };
        let offset = d
            .underline_offset
            .as_ref()
            .map(|offset| offset.resolve(d.font_size).round());
        let line = |kind: LineKind, top: f32| DrawnLine {
            kind,
            style: d.style,
            color: d.color,
            top,
            thickness: drawn,
            second: (d.style == DecorationStyle::Double).then(|| top + (thickness + 1.0).round()),
        };
        if d.line.underline {
            let top = match (d.underline_position, offset) {
                (UnderlinePosition::Under, offset) => descent + offset.unwrap_or(0.0),
                (UnderlinePosition::FromFont, offset) => {
                    let position =
                        underline.map_or((thickness / 2.0).ceil().max(1.0), |u| u.0.round());
                    position + offset.unwrap_or(0.0)
                }
                (UnderlinePosition::Auto, Some(offset)) => offset,
                (UnderlinePosition::Auto, None) => (thickness / 2.0).ceil().max(1.0),
            };
            out.push(line(LineKind::Underline, top));
        }
        if d.line.overline {
            out.push(line(LineKind::Overline, -ascent - drawn));
        }
        if d.line.line_through {
            out.push(line(
                LineKind::LineThrough,
                (-ascent / 3.0 - thickness / 2.0).round(),
            ));
        }
    }
    out
}

/// The text decoration lines a text fragment is drawn with
/// ([`crate::Fragment::decorated`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Decorated {
    /// The lines, outermost element's first.
    pub lines: Arc<[DrawnLine]>,
    /// How far the lines reach past the fragment's left edge and its
    /// right: across the space before and after it that the decorating
    /// elements hold, and the letter spacing after a character.
    pub left: f32,
    pub right: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{FragmentKind, layout_document};
    use crate::box_tree::{BoxKind, LayoutBox};
    use crate::testing::boxes_of;

    fn fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    fn sheet(css_src: &str) -> css::Stylesheet {
        css::parse_stylesheet(&format!("body {{ margin: 0 }} {css_src}"))
            .expect("test css is valid")
    }

    /// Each text box of `html` under `css_src`, with its decorations.
    fn texts(html: &str, css_src: &str) -> Vec<(String, Option<Arc<[Decorating]>>)> {
        let doc = html::parse_document(html).expect("html parsing never fails");
        let (_, boxes) = boxes_of(&doc, &[sheet(css_src)]);
        let mut out = Vec::new();
        let mut stack: Vec<&LayoutBox> = boxes.iter().rev().collect();
        while let Some(b) = stack.pop() {
            if let BoxKind::Text(text) = &b.kind {
                out.push((text.clone(), b.decorations.clone()));
            }
            stack.extend(b.children.iter().rev());
        }
        out
    }

    #[test]
    fn a_decoration_is_propagated_to_in_flow_text_and_no_further() {
        let texts = texts(
            "<p>a <span class=o>b</span> <i>e</i> <span class=ib>c</span> \
             <span class=f>d</span></p>",
            "p { text-decoration: underline #f00 }
             .o { text-decoration: overline; color: #00f }
             .ib { display: inline-block } .f { float: left }",
        );
        let of = |text: &str| {
            texts
                .iter()
                .find(|(t, _)| t == text)
                .unwrap_or_else(|| panic!("no text {text:?}"))
                .1
                .clone()
        };
        let p = of("a ").expect("the paragraph's");
        assert_eq!(p.len(), 1);
        assert!(p[0].line.underline);
        assert_eq!(p[0].color, Color::rgb8(255, 0, 0));
        // Inside the span, the paragraph's and then its own, in its
        // colour.
        let b = of("b").expect("both");
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], p[0]);
        assert!(b[1].line.overline);
        assert_eq!(b[1].color, Color::rgb8(0, 0, 255));
        // An element that adds none shares the list.
        assert!(Arc::ptr_eq(&of("e").expect("the paragraph's"), &p));
        // Not into an atomic inline nor a float.
        assert_eq!(of("c"), None);
        assert_eq!(of("d"), None);
    }

    #[test]
    fn the_lines_are_where_the_reference_browser_draws_them() {
        let fonts = fonts();
        let a = fonts.primary().metrics(16.0).ascent;
        let d = fonts.primary().metrics(16.0).descent;
        let style = |css_src: &str| {
            let doc = html::parse_document("<p>x</p>").expect("never fails");
            let (_, boxes) = boxes_of(&doc, &[sheet(css_src)]);
            let mut stack: Vec<&LayoutBox> = boxes.iter().collect();
            while let Some(b) = stack.pop() {
                if let Some(decorations) = &b.decorations {
                    return lines(decorations, &fonts);
                }
                stack.extend(b.children.iter());
            }
            panic!("no decorated text");
        };
        let one = |css_src: &str| {
            let lines = style(&format!("p {{ {css_src} }}"));
            assert_eq!(lines.len(), 1, "{css_src}");
            (lines[0].top, lines[0].thickness, lines[0].second)
        };
        // 16 pixels: a tenth of the size, a pixel thick, a pixel down.
        assert_eq!(one("text-decoration: underline"), (1.0, 1.0, None));
        assert_eq!(one("text-decoration: overline"), (-a - 1.0, 1.0, None));
        assert_eq!(
            one("text-decoration: line-through"),
            ((-a / 3.0 - 0.8).round(), 1.0, None)
        );
        assert_eq!(
            one("text-decoration: underline double"),
            (1.0, 1.0, Some(4.0))
        );
        // 32 pixels: three thick, two down.
        assert_eq!(
            one("text-decoration: underline; font-size: 32px"),
            (2.0, 3.0, None)
        );
        // A thickness and an offset given are rounded.
        assert_eq!(one("text-decoration: underline 2.6px"), (2.0, 3.0, None));
        assert_eq!(
            one("text-decoration: underline; text-underline-offset: 4.4px"),
            (4.0, 1.0, None)
        );
        assert_eq!(
            one("text-decoration: underline; text-underline-position: under"),
            (d, 1.0, None)
        );
        let (top, thickness) = fonts.underline(16.0).expect("the bundled font has one");
        assert_eq!(
            one("text-decoration: underline from-font; text-underline-position: from-font"),
            (top.round(), thickness.round().max(1.0), None)
        );
        // Each element's underline, overline and line-through, outermost
        // first.
        let kinds: Vec<LineKind> = style("p { text-decoration: line-through overline underline }")
            .iter()
            .map(|line| line.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                LineKind::Underline,
                LineKind::Overline,
                LineKind::LineThrough
            ]
        );
    }

    #[test]
    fn a_line_reaches_over_the_spaces_its_element_holds() {
        let fonts = fonts();
        let space = fonts.measure(" ", 16.0);
        let reach = |html: &str| -> Vec<(String, f32, f32)> {
            let doc = html::parse_document(html).expect("never fails");
            let (_, boxes) = boxes_of(&doc, &[sheet("u { color: #000 }")]);
            let layout = layout_document(&boxes, 800.0, &fonts);
            layout
                .iter()
                .filter_map(|f| match (&f.kind, &f.decorated) {
                    (FragmentKind::Text(t), Some(d)) => Some((t.clone(), d.left, d.right)),
                    _ => None,
                })
                .collect()
        };
        let s = |t: &str, left: f32, right: f32| (t.to_string(), left, right);
        // Between two words of the element, the space after the first.
        assert_eq!(
            reach("<u>ab cd</u>"),
            [s("ab", 0.0, 0.0), s("cd", space, 0.0)]
        );
        // A space inside the element, before its end: on from its word.
        assert_eq!(reach("<u>ab </u>cd"), [s("ab", 0.0, space)]);
        // A space outside it: not at all.
        assert_eq!(reach("<u>ab</u> cd"), [s("ab", 0.0, 0.0)]);
        // Before an inline-block in it, from the word before it.
        assert_eq!(
            reach("<u>ab <b style='display: inline-block'>x</b></u>"),
            [s("ab", 0.0, space)]
        );
    }
}
