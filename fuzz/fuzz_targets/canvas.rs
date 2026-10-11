//! The 2D context alone (M30, `docs/design/canvas.md`): sequences of
//! its operations with arguments from the input, numbers not finite,
//! huge and negative among them, degenerate paths, every composite
//! operation, style and repetition, text in fonts the input writes, SVG
//! path data, pixels in and out, images drawn from the input's pixels,
//! on a small bitmap or none. Nothing panics; the bitmap stays the size
//! it was made, its bytes four a pixel; no path holds more points, nor
//! the stack more states, than their bounds; and the PNG it exports
//! decodes to its size.

#![no_main]

use std::sync::{Arc, Mutex, OnceLock};

use canvas::{
    Canvas, Composite, FillRule, FontBase, Gradient, GradientKind, LineCap, LineJoin, Matrix,
    Paint, Path, Pattern, Quality, Repetition, Rgba, Source, Spacing, TextAlign, TextBaseline,
    parse_color, parse_font,
};
use libfuzzer_sys::fuzz_target;

/// The most operations one input makes.
const MAX_OPERATIONS: usize = 64;

/// Numbers an argument often is: the edges of what a context takes.
const NUMBERS: [f64; 16] = [
    0.0,
    1.0,
    -1.0,
    0.5,
    2.5,
    7.0,
    16.0,
    -16.0,
    100.0,
    1e9,
    -1e9,
    1e300,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::MIN_POSITIVE,
];

const COLORS: [&str; 8] = [
    "red",
    "#0f08",
    "rgba(0, 0, 255, 0.5)",
    "hsl(120 50% 50%)",
    "transparent",
    "currentcolor",
    "bogus",
    "hwb(30 10% 10% / 0.3)",
];

const FONTS: [&str; 6] = [
    "10px sans-serif",
    "bold 30px serif",
    "italic small-caps 300 16px/2 Georgia, serif",
    "1e9px x",
    "0px a",
    "2rem b",
];

const COMPOSITES: [&str; 26] = [
    "source-over",
    "source-in",
    "source-out",
    "source-atop",
    "destination-over",
    "destination-in",
    "destination-out",
    "destination-atop",
    "lighter",
    "copy",
    "xor",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    /// A number: one of the table's, or a small one of the input's.
    fn number(&mut self) -> f64 {
        let pick = self.byte();
        if pick < 128 {
            return NUMBERS[usize::from(pick) % NUMBERS.len()];
        }
        f64::from(self.byte()) - 64.0 + f64::from(pick & 3) / 4.0
    }

    fn text(&mut self) -> String {
        let length = usize::from(self.byte() % 24).min(self.0.len());
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        String::from_utf8_lossy(taken).into_owned()
    }

    fn pick<'t>(&mut self, table: &[&'t str]) -> &'t str {
        table[usize::from(self.byte()) % table.len()]
    }
}

fn fonts() -> &'static text::FontCollection {
    static FONTS: OnceLock<text::FontCollection> = OnceLock::new();
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the test font")
    })
}

/// A small image from the input: up to 8 by 8, premultiplied.
fn image(input: &mut Input) -> Source {
    let (w, h) = (
        u32::from(input.byte() % 8 + 1),
        u32::from(input.byte() % 8 + 1),
    );
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        let a = input.byte();
        let c = |v: u8| ((u32::from(v) * u32::from(a)) / 255) as u8;
        pixels.extend_from_slice(&[c(input.byte()), c(input.byte()), c(input.byte()), a]);
    }
    Source {
        pixels: Arc::from(pixels),
        width: w,
        height: h,
        origin_clean: input.byte() % 4 != 0,
    }
}

fn paint(input: &mut Input) -> Paint {
    match input.byte() % 3 {
        0 => Paint::Color(parse_color(input.pick(&COLORS), Rgba::BLACK).unwrap_or(Rgba::BLACK)),
        1 => {
            let kind = match input.byte() % 3 {
                0 => GradientKind::Linear {
                    x0: input.number(),
                    y0: input.number(),
                    x1: input.number(),
                    y1: input.number(),
                },
                1 => GradientKind::Radial {
                    x0: input.number(),
                    y0: input.number(),
                    r0: input.number().abs(),
                    x1: input.number(),
                    y1: input.number(),
                    r1: input.number().abs(),
                },
                _ => GradientKind::Conic {
                    angle: input.number(),
                    x: input.number(),
                    y: input.number(),
                },
            };
            let mut gradient = Gradient::new(kind);
            for _ in 0..input.byte() % 5 {
                let offset = f64::from(input.byte()) / 200.0;
                let color = parse_color(input.pick(&COLORS), Rgba::BLACK).unwrap_or(Rgba::BLACK);
                let _ = gradient.add_stop(offset, color);
            }
            Paint::Gradient(Arc::new(Mutex::new(gradient)))
        }
        _ => {
            let source = image(input);
            let repetition = [
                Repetition::Repeat,
                Repetition::RepeatX,
                Repetition::RepeatY,
                Repetition::NoRepeat,
            ][usize::from(input.byte()) % 4];
            let transform = Matrix::new(
                input.number(),
                0.0,
                0.0,
                input.number(),
                input.number(),
                input.number(),
            );
            Paint::Pattern(Arc::new(Mutex::new(Pattern {
                width: source.width,
                height: source.height,
                pixels: source.pixels,
                repetition,
                transform,
                origin_clean: source.origin_clean,
            })))
        }
    }
}

fn operate(canvas: &mut Canvas, path2d: &mut Path, input: &mut Input) {
    let rule = if input.byte() % 2 == 0 {
        FillRule::Winding
    } else {
        FillRule::EvenOdd
    };
    match input.byte() % 32 {
        0 => canvas.fill_rect(
            input.number(),
            input.number(),
            input.number(),
            input.number(),
        ),
        1 => canvas.stroke_rect(
            input.number(),
            input.number(),
            input.number(),
            input.number(),
        ),
        2 => canvas.clear_rect(
            input.number(),
            input.number(),
            input.number(),
            input.number(),
        ),
        3 => canvas.begin_path(),
        4 => canvas.with_path(|path, m| path.move_to(input.number(), input.number(), m)),
        5 => canvas.with_path(|path, m| path.line_to(input.number(), input.number(), m)),
        6 => canvas.with_path(|path, m| {
            path.cubic_to(
                input.number(),
                input.number(),
                input.number(),
                input.number(),
                input.number(),
                input.number(),
                m,
            )
        }),
        7 => {
            let _ = canvas.with_path(|path, m| {
                path.arc(
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                    input.byte() % 2 == 0,
                    m,
                )
            });
        }
        8 => {
            let _ = canvas.with_path(|path, m| {
                path.arc_to(
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                    m,
                )
            });
        }
        9 => {
            let radii: Vec<(f64, f64)> = (0..input.byte() % 6)
                .map(|_| (input.number(), input.number()))
                .collect();
            let _ = canvas.with_path(|path, m| {
                path.round_rect(
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                    &radii,
                    m,
                )
            });
        }
        10 => canvas.with_path(|path, _| path.close()),
        11 => canvas.fill(None, rule),
        12 => canvas.stroke(None),
        13 => canvas.clip(None, rule),
        14 => canvas.fill(Some(path2d), rule),
        15 => {
            let text = input.text();
            *path2d = Path::from_svg(&text);
        }
        16 => {
            let _ = canvas.is_point_in_path(Some(path2d), input.number(), input.number(), rule);
            let _ = canvas.is_point_in_stroke(None, input.number(), input.number());
        }
        17 => canvas.save(),
        18 => canvas.restore(),
        19 => canvas.transform(&Matrix::new(
            input.number(),
            input.number(),
            input.number(),
            input.number(),
            input.number(),
            input.number(),
        )),
        20 => canvas.rotate(input.number()),
        21 => canvas.state_mut().fill = paint(input),
        22 => canvas.state_mut().stroke = paint(input),
        23 => {
            let state = canvas.state_mut();
            state.composite =
                Composite::from_name(input.pick(&COMPOSITES)).unwrap_or(state.composite);
            state.set_global_alpha(input.number());
        }
        24 => {
            let state = canvas.state_mut();
            state.shadow_color =
                parse_color(input.pick(&COLORS), Rgba::BLACK).unwrap_or(Rgba::BLACK);
            state.set_shadow_blur(input.number());
            state.set_shadow_offset(Some(input.number()), Some(input.number()));
        }
        25 => {
            let state = canvas.state_mut();
            state.set_line_width(input.number());
            state.line_cap =
                [LineCap::Butt, LineCap::Round, LineCap::Square][usize::from(input.byte()) % 3];
            state.line_join =
                [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][usize::from(input.byte()) % 3];
            state.set_miter_limit(input.number());
            let dashes: Vec<f64> = (0..input.byte() % 5).map(|_| input.number()).collect();
            state.dash.set(&dashes);
            state.set_dash_offset(input.number());
        }
        26 => {
            let (x, y, w, h) = (
                i64::from(input.byte()) - 32,
                i64::from(input.byte()) - 32,
                i64::from(input.byte()) - 64,
                i64::from(input.byte()) - 64,
            );
            if let Ok(data) = canvas.get_image_data(x, y, w, h) {
                assert_eq!(data.len() as i64, w.abs() * h.abs() * 4);
            }
        }
        27 => {
            let source = image(input);
            let mut data = source.pixels.to_vec();
            data.truncate(usize::from(input.byte()) * 4 + data.len() / 2);
            let dirty = (input.byte() % 2 == 0).then(|| {
                (
                    i64::from(input.byte()) - 8,
                    i64::from(input.byte()) - 8,
                    i64::from(input.byte()) - 16,
                    i64::from(input.byte()) - 16,
                )
            });
            canvas.put_image_data(
                &data,
                source.width,
                source.height,
                i64::from(input.byte()) - 16,
                i64::from(input.byte()) - 16,
                dirty,
            );
        }
        28 => {
            let source = image(input);
            let src = (input.byte() % 2 == 0).then(|| {
                [
                    input.number(),
                    input.number(),
                    input.number(),
                    input.number(),
                ]
            });
            let dst = [
                input.number(),
                input.number(),
                input.number(),
                input.number(),
            ];
            let state = canvas.state_mut();
            state.image_smoothing = input.byte() % 2 == 0;
            state.image_smoothing_quality = Quality::High;
            canvas.draw_image(&source, src, dst);
        }
        29 => {
            let base = FontBase::default();
            let font = input.text();
            let state = canvas.state_mut();
            if let Some(font) = parse_font(&font, &base).or_else(|| parse_font(FONTS[0], &base)) {
                state.set_font(font);
            }
            if let Some(font) = parse_font(input.pick(&FONTS), &base) {
                state.set_font(font);
            }
            state.text_align = [
                TextAlign::Start,
                TextAlign::End,
                TextAlign::Left,
                TextAlign::Right,
                TextAlign::Center,
            ][usize::from(input.byte()) % 5];
            state.text_baseline = [
                TextBaseline::Top,
                TextBaseline::Hanging,
                TextBaseline::Middle,
                TextBaseline::Alphabetic,
                TextBaseline::Ideographic,
                TextBaseline::Bottom,
            ][usize::from(input.byte()) % 6];
            if let Some(spacing) = Spacing::parse(&input.text(), &base) {
                state.letter_spacing = spacing;
            }
        }
        30 => {
            let text = input.text();
            let max = (input.byte() % 2 == 0).then(|| input.number());
            let stroke = input.byte() % 2 == 0;
            canvas.draw_text(
                fonts(),
                &text,
                input.number(),
                input.number(),
                max,
                stroke,
                input.byte() % 2 == 0,
            );
            let metrics = canvas.measure_text(fonts(), &text, false);
            assert!(!metrics.width.is_nan());
        }
        _ => {
            if let Some(png) = canvas.to_png() {
                let decoded = images::decode(&png).expect("our own PNG decodes");
                assert_eq!(
                    (decoded.width, decoded.height),
                    (canvas.width(), canvas.height())
                );
            }
            canvas.reset();
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let (width, height) = match input.byte() % 8 {
        0 => (0, 4),
        1 => (1, 1),
        2 => (canvas::MAX_CANVAS_SIDE + 1, 1),
        _ => (
            u32::from(input.byte() % 48 + 1),
            u32::from(input.byte() % 48 + 1),
        ),
    };
    let mut canvas = Canvas::new(width, height, input.byte() % 4 == 0);
    let mut path2d = Path::new();
    for _ in 0..MAX_OPERATIONS {
        if input.0.is_empty() {
            break;
        }
        operate(&mut canvas, &mut path2d, &mut input);
        assert_eq!((canvas.width(), canvas.height()), (width, height));
        if let Some(bytes) = canvas.bytes() {
            assert_eq!(bytes.len() as u64, u64::from(width) * u64::from(height) * 4);
        }
        assert!(canvas.path().len() <= canvas::MAX_PATH_POINTS);
        assert!(path2d.len() <= canvas::MAX_PATH_POINTS);
    }
});
