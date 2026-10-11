//! Colours as a canvas's styles take and give them: parsed as CSS
//! writes them, kept as 8-bit sRGB not premultiplied, and serialized as
//! the HTML Standard says (4.12.5.1.10), `#rrggbb` when opaque and
//! `rgba(r, g, b, a)` otherwise.

/// A colour, straight (not premultiplied) 8-bit sRGB with its alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const TRANSPARENT: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// The colour with its alpha multiplied by `alpha`, 0 to 1.
    pub(crate) fn faded(self, alpha: f64) -> Rgba {
        Rgba {
            a: (f64::from(self.a) * alpha).round().clamp(0.0, 255.0) as u8,
            ..self
        }
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }
}

fn channel(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// `text` as a canvas's colour: a CSS colour of those the engine knows,
/// or `currentcolor`, which is `current`; `None` for any other text.
pub fn parse_color(text: &str, current: Rgba) -> Option<Rgba> {
    let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if text.eq_ignore_ascii_case("currentcolor") {
        return Some(current);
    }
    let color = style::parse_color_text(text)?;
    Some(Rgba {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: channel(color.a),
    })
}

/// A colour as `fillStyle` reads it back.
pub fn serialize_color(color: Rgba) -> String {
    if color.a == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "rgba({}, {}, {}, {})",
            color.r,
            color.g,
            color.b,
            alpha_text(color.a)
        )
    }
}

/// An 8-bit alpha in the fewest decimal places that come back to it, as
/// CSS Color 4 serializes one.
fn alpha_text(alpha: u8) -> String {
    let exact = f64::from(alpha) / 255.0;
    for places in 1..=3 {
        let scale = 10f64.powi(places);
        let value = (exact * scale).round() / scale;
        if (value * 255.0).round() as u8 == alpha {
            return format!("{value}");
        }
    }
    format!("{exact}")
}
