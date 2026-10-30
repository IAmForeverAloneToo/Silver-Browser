//! The font a box's text is in and the height of its lines (gap batch
//! D, D53): the face its font properties choose, and `line-height`'s
//! leading (CSS 2 section 10.8.1).

use style::computed::{ComputedStyle, FontKerning, FontProps, FontStyle, LineHeight};
use text::{Font, FontCollection, FontMetrics, FontRequest, Slant};

/// What `props` asks a collection for; paint's display list carries the
/// properties and render asks the same of its fonts.
pub fn font_request(props: &FontProps) -> FontRequest<'_> {
    FontRequest {
        families: &props.family,
        weight: props.weight.round().clamp(1.0, 1000.0) as u16,
        slant: match props.style {
            FontStyle::Normal => Slant::Normal,
            FontStyle::Italic => Slant::Italic,
            FontStyle::DefaultOblique => Slant::Oblique(14.0),
            FontStyle::Oblique(angle) => Slant::Oblique(angle),
        },
        stretch: props.stretch,
        kerning: props.kerning != FontKerning::None,
        features: &props.features,
        language: "",
    }
}

/// The font of `props`, the initial font when none: the primary font.
pub fn font_of_props(fonts: &FontCollection, props: Option<&FontProps>) -> Font {
    props.map_or(Font::default(), |props| fonts.font(&font_request(props)))
}

/// The font `style`'s text is in.
pub(crate) fn font_of(fonts: &FontCollection, style: &ComputedStyle) -> Font {
    font_of_props(fonts, style.font.as_deref())
}

/// The metrics of `text` in `style`'s font, unleaded: its content area.
pub(crate) fn content(fonts: &FontCollection, text: &str, style: &ComputedStyle) -> FontMetrics {
    fonts.metrics_in(font_of(fonts, style), text, style.font_size_px)
}

/// The metrics of `text` in `style`'s font, with the leading of its
/// `line-height`: what a line box is made of.
pub(crate) fn line(fonts: &FontCollection, text: &str, style: &ComputedStyle) -> FontMetrics {
    leaded(content(fonts, text, style), style)
}

/// `m` with `style`'s `line-height`: half the leading above, as the
/// reference browser rounds it, down, and the rest below; `normal`
/// the font's own metrics.
pub(crate) fn leaded(m: FontMetrics, style: &ComputedStyle) -> FontMetrics {
    let height = match style.font().line_height {
        LineHeight::Normal => return m,
        LineHeight::Number(n) => n * style.font_size_px,
        LineHeight::Px(px) => px,
    };
    let half = ((height - (m.ascent + m.descent)) / 2.0).floor();
    let ascent = m.ascent + half;
    FontMetrics {
        ascent,
        descent: height - ascent,
        line_gap: 0.0,
    }
}

/// `text`'s width in `style`'s font.
pub(crate) fn measure(fonts: &FontCollection, text: &str, style: &ComputedStyle) -> f32 {
    fonts.measure_in(font_of(fonts, style), text, style.font_size_px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_height_adds_half_the_leading_above_and_the_rest_below() {
        let m = FontMetrics {
            ascent: 15.0,
            descent: 4.0,
            line_gap: 0.0,
        };
        let with = |lh: LineHeight| {
            let mut style = ComputedStyle {
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            style.font_mut().line_height = lh;
            let l = leaded(m, &style);
            (l.ascent, l.descent, l.line_height())
        };
        assert_eq!(with(LineHeight::Normal), (15.0, 4.0, 19.0));
        assert_eq!(with(LineHeight::Px(30.0)), (20.0, 10.0, 30.0));
        // Half of an odd leading, down, above.
        assert_eq!(with(LineHeight::Px(24.0)), (17.0, 7.0, 24.0));
        assert_eq!(with(LineHeight::Number(2.0)), (21.0, 11.0, 32.0));
        // Less than the content: the text overflows its line.
        assert_eq!(with(LineHeight::Px(9.0)), (10.0, -1.0, 9.0));
    }

    #[test]
    fn the_initial_font_is_the_primary_font() {
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("parses");
        let style = ComputedStyle::default();
        assert_eq!(font_of(&fonts, &style), Font::default());
        let mut bold = style.clone();
        bold.font_mut().weight = 700.0;
        assert!(font_of(&fonts, &bold).bold, "the test font has no bold");
        assert_eq!(measure(&fonts, "ab", &bold), fonts.measure("ab", 16.0));
    }
}
