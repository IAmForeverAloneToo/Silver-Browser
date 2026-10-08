//! Animations and transitions (M22) from arbitrary bytes. The input is a
//! style sheet up to its first zero byte, and after it the times the
//! checks are made at, eight bytes each, read as an `f64` and as a
//! number of milliseconds. The sheet styles a small fixed document,
//! whose elements' animations, keyframes and transitions it declares.
//! Nothing may panic, and every computation must end within its bounds:
//!
//! - every easing function the document's styles compute, an
//!   animation's or a transition's, gives at each time and at the edges
//!   of its domain, with the before flag and without, an output that is
//!   finite and no further from zero than [`MAX_PROGRESS`];
//! - every animation's timing the styles compute gives at each finite
//!   time an active time, an iteration and a progress that are finite
//!   where it is in effect, the iteration whole and not below zero;
//! - every animatable longhand blended from each element's style to the
//!   next one's, at each time's fraction and at each easing's output
//!   there, gives a style whose lengths resolve in range against every
//!   basis, whose colours are within their channels, whose opacity is
//!   within [0, 1], whose font size is in range, and whose transform's
//!   matrix is finite;
//! - the document drawn by one engine at each time in turn is what a
//!   fresh engine draws at the first time and then at that one: every
//!   element's computed style, and every pixel.

#![no_main]

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Duration;

use dom::NodeId;
use engine::{Engine, FrameTime};
use libfuzzer_sys::fuzz_target;
use style::ComputedStyle;
use style::animate::{Longhand, blend};
use style::animation::Timing;
use style::cascade::MAX_LENGTH_PX;
use style::computed::{
    AnimationDirection, AnimationFillMode, Color, LengthPercentage, MaxSize, Size,
};
use style::easing::{Easing, MAX_PROGRESS};
use style::transform::ReferenceBox;

/// The document the sheet styles: blocks, inlines, a list item and
/// generated boxes for the sheet to animate.
const PAGE: &str = "<div id=a class=b><p class=c>one <span>two</span></p>\
                    <ul><li class=d>item</li></ul></div><div id=e class=f>three</div>";

/// The most times an input is checked at: enough for any sequence of
/// frames the sampling could get wrong, few enough to keep it quick.
const MAX_TIMES: usize = 8;
/// The latest time a frame is drawn at, in milliseconds.
const MAX_MS: u64 = 10_000_000;
/// The frame: small, so the time goes into the engine.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

/// The bases a percentage is resolved against: none, a box's, and the
/// largest a length is.
const BASES: [f32; 3] = [0.0, 800.0, MAX_LENGTH_PX];

/// Times past every easing's domain, at its edges and inside it.
const EDGES: [f64; 9] = [
    f64::NEG_INFINITY,
    -1e300,
    -1.0,
    0.0,
    0.5,
    1.0,
    2.0,
    1e300,
    f64::INFINITY,
];

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

thread_local! {
    /// The engine under test and the fresh one it is compared with,
    /// made once: loading a document starts either over.
    static ENGINES: RefCell<(Engine, Engine)> =
        RefCell::new((Engine::with_fonts(fonts().clone()), Engine::with_fonts(fonts().clone())));
}

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

/// Whether `v` is a length layout can take.
fn in_range(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_LENGTH_PX
}

fn check_colour(name: &str, colour: &Color) {
    for channel in [colour.r, colour.g, colour.b, colour.a] {
        assert!(
            channel.is_finite() && (0.0..=1.0).contains(&channel),
            "{name}: {colour:?}"
        );
    }
}

fn check_length_percentage(name: &str, value: &LengthPercentage) {
    for basis in BASES {
        let v = value.resolve(basis);
        assert!(in_range(v), "{name}: {value:?} resolves to {v} of {basis}");
    }
}

fn check_size(name: &str, size: &Size) {
    if let Some(value) = size.length_percentage() {
        check_length_percentage(name, &value);
    }
}

fn check_max_size(name: &str, size: &MaxSize) {
    if let Some(value) = size.length_percentage() {
        check_length_percentage(name, &value);
    }
}

/// What a blended style must be: within each property's range and its
/// bounds.
fn check_style(style: &ComputedStyle, what: &str) {
    check_colour(what, &style.color);
    check_colour(what, &style.background_color);
    for colour in &style.border_color {
        check_colour(what, colour);
    }
    assert!(
        in_range(style.font_size_px) && style.font_size_px >= 0.0,
        "{what}: font size {}",
        style.font_size_px
    );
    for v in [style.letter_spacing, style.word_spacing] {
        assert!(in_range(v), "{what}: spacing {v}");
    }
    for v in style.border_width {
        assert!(in_range(v) && v >= 0.0, "{what}: border width {v}");
    }
    for v in [style.flex_grow, style.flex_shrink] {
        assert!(v.is_finite() && v >= 0.0, "{what}: flex factor {v}");
    }
    for size in [
        &style.width,
        &style.height,
        &style.min_width,
        &style.min_height,
    ] {
        check_size(what, size);
    }
    for size in [&style.max_width, &style.max_height] {
        check_max_size(what, size);
    }
    for edge in style.margin.iter().chain(&style.inset) {
        for basis in BASES {
            let v = edge.resolve(basis, 0.0);
            assert!(in_range(v), "{what}: {edge:?} resolves to {v} of {basis}");
        }
    }
    for edge in &style.padding {
        check_length_percentage(what, edge);
    }
    let effects = style.effects();
    assert!(
        effects.opacity.is_finite() && (0.0..=1.0).contains(&effects.opacity),
        "{what}: opacity {}",
        effects.opacity
    );
    if let Some(matrix) = effects.matrix(&ReferenceBox::border(100.0, 50.0)) {
        assert!(matrix.is_finite(), "{what}: {matrix:?}");
    }
}

/// The checks of easings and timings at `times`.
fn check_timing(style: &ComputedStyle, times: &[f64]) {
    let motion = style.motion();
    let easings = motion
        .animation_timing_function
        .iter()
        .chain(&motion.transition_timing_function);
    for easing in easings {
        for &t in times.iter().chain(&EDGES) {
            for before in [false, true] {
                let out = easing.output(t, before);
                assert!(
                    out.is_finite() && out.abs() <= MAX_PROGRESS,
                    "{easing:?} at {t} ({before}): {out}"
                );
            }
        }
    }
    let at = |list: &[f64], i: usize, default: f64| {
        if list.is_empty() {
            default
        } else {
            list[i % list.len()]
        }
    };
    for i in 0..motion.animation_name.len() {
        let direction = &motion.animation_direction;
        let fill = &motion.animation_fill_mode;
        let timing = Timing {
            delay: at(&motion.animation_delay, i, 0.0),
            duration: at(&motion.animation_duration, i, 0.0),
            iterations: at(&motion.animation_iteration_count, i, 1.0),
            direction: direction
                .get(i % direction.len().max(1))
                .copied()
                .unwrap_or(AnimationDirection::Normal),
            fill: fill
                .get(i % fill.len().max(1))
                .copied()
                .unwrap_or(AnimationFillMode::None),
            easing: Easing::Linear,
        };
        for &t in times.iter().filter(|t| t.is_finite()) {
            let progress = timing.progress(t);
            if let Some(active) = progress.active_time {
                assert!(active.is_finite(), "{timing:?} at {t}: {progress:?}");
            }
            if let Some(iteration) = progress.iteration {
                assert!(
                    iteration.is_finite() && iteration >= 0.0 && iteration.fract() == 0.0,
                    "{timing:?} at {t}: {progress:?}"
                );
            }
            if let Some(p) = progress.progress {
                assert!(p.is_finite(), "{timing:?} at {t}: {progress:?}");
            }
        }
    }
}

/// The elements of `engine`'s document, in tree order.
fn elements(engine: &Engine) -> Vec<NodeId> {
    let document = engine.document();
    document
        .subtree(document.root())
        .filter(|&n| document.is_element(n))
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let split = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let Ok(sheet) = std::str::from_utf8(&data[..split]) else {
        return;
    };
    let rest = data.get(split + 1..).unwrap_or(&[]);
    let raw: Vec<f64> = rest
        .chunks_exact(8)
        .take(MAX_TIMES)
        .map(|c| f64::from_le_bytes(c.try_into().expect("eight bytes")))
        .collect();
    // The frames' times, in order, as a clock gives them.
    let mut ms: Vec<u64> = rest
        .chunks_exact(8)
        .take(MAX_TIMES)
        .map(|c| u64::from_le_bytes(c.try_into().expect("eight bytes")) % MAX_MS)
        .collect();
    ms.sort_unstable();
    let source = format!("<style>{sheet}</style>{PAGE}");
    ENGINES.with(|engines| {
        let (engine, fresh) = &mut *engines.borrow_mut();
        engine.load_document(&source);
        let first = FrameTime::at(Duration::from_millis(ms.first().copied().unwrap_or(0)));
        engine.frame_at(WIDTH, HEIGHT, first);
        // The styles the sheet computes, their easings and timings, and
        // every animatable property blended between them.
        let nodes = elements(engine);
        let styles: Vec<ComputedStyle> = {
            let computed = engine.styles();
            nodes
                .iter()
                .filter_map(|&n| computed.get(n).cloned())
                .collect()
        };
        for style in &styles {
            check_timing(style, &raw);
        }
        let fractions: Vec<f64> = raw
            .iter()
            .filter(|t| t.is_finite())
            .map(|t| t.clamp(-2.0, 3.0))
            .chain([0.0, 0.5, 1.0])
            .collect();
        for pair in styles.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let easings = a.motion().animation_timing_function.clone();
            for &p in &fractions {
                let eased = easings.iter().map(|e| e.output(p, false));
                for p in std::iter::once(p).chain(eased) {
                    let mut to = a.clone();
                    for &longhand in Longhand::ALL {
                        blend(longhand, a, b, p, &mut to);
                    }
                    check_style(&to, &format!("blended at {p}"));
                }
            }
        }
        // The frames, one after another, against a fresh engine at each.
        for &t in ms.iter().skip(1) {
            let at = FrameTime::at(Duration::from_millis(t));
            let pixels = engine.frame_at(WIDTH, HEIGHT, at).to_vec();
            fresh.load_document(&source);
            fresh.frame_at(WIDTH, HEIGHT, first);
            let fresh_pixels = fresh.frame_at(WIDTH, HEIGHT, at).to_vec();
            let (ours, theirs) = (elements(engine), elements(fresh));
            assert_eq!(ours.len(), theirs.len());
            for (i, (&n, &m)) in ours.iter().zip(&theirs).enumerate() {
                let mine = engine.styles().get(n).cloned();
                let fresh_style = fresh.styles().get(m).cloned();
                assert!(
                    mine == fresh_style,
                    "element {i} at {t}ms: {mine:?}, a fresh engine computes {fresh_style:?}"
                );
            }
            assert!(pixels == fresh_pixels, "the frame at {t}ms differs");
        }
    });
});
