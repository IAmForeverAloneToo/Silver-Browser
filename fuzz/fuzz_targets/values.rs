//! Values (M21) from arbitrary bytes: math functions, custom properties
//! with their substitutions, cycles and fallbacks, typed `attr()`, and
//! counters written in their styles. The input is a style sheet up to
//! its first zero byte, and after it the values of a small fixed
//! document's attributes, each up to the next: `data-a`, `data-b` and
//! `data-c` on every element, the lists' `start` and the items' `value`.
//! Nothing may panic, and every computation must end within its bounds:
//!
//! - each math function among the sheet's tokens, parsed for every
//!   place and computed with a percentage's basis and without, is never
//!   `NaN`, and what layout resolves of one with a percentage in it is a
//!   length in range whatever the basis;
//! - in the document's styles under the sheet, each custom property of
//!   every element and pseudo-element weighs no more than
//!   [`style::custom::MAX_SUBSTITUTION`], at least the size of its
//!   tokens, and every length resolves in range;
//! - in its box tree, which numbers its counters and writes its
//!   generated content, no text a pseudo-element generates is longer
//!   than [`layout::MAX_GENERATED_TEXT`];
//! - the integers among the sheet's tokens, written in every predefined
//!   counter style, and the extremes of the range written in each name
//!   among its tokens, are no longer than
//!   [`style::counter_style::MAX_REPRESENTATION`] code points, their
//!   markers no more than their styles' prefixes and suffixes longer.

#![no_main]
#![forbid(unsafe_code)]

use std::sync::OnceLock;

use css::Token;
use libfuzzer_sys::fuzz_target;
use style::calc::{self, Computed, LengthUnit, Lengths, Want};
use style::cascade::MAX_LENGTH_PX;
use style::computed::{Calc, LengthPercentage, LengthPercentageAuto};
use style::counter_style::{self, MAX_REPRESENTATION};
use style::custom::MAX_SUBSTITUTION;
use style::selector_matching::PseudoElement;
use style::{CascadeContext, ComputedStyle, Media, Stylesheets};

/// The document the sheet styles: lists nested for counters, a list
/// item that is not an `li`, quotes, and elements of each kind for
/// `attr()` to read.
const PAGE: &str = "<div id=a class=b><ol><li><p>x</p><ol><li>y<li>z</ol><li>w</ol>\
                    <ul><li>u<ul><li>v</ul></ul><p class=c><span>s</span><q>q</q></p>\
                    <div class=d style='display: list-item'>t</div></div>";

/// The attributes the input's values go to, in order: on every element,
/// then on the lists and on their items.
const ATTRIBUTES: [(&str, Option<&str>); 5] = [
    ("data-a", None),
    ("data-b", None),
    ("data-c", None),
    ("start", Some("ol")),
    ("value", Some("li")),
];

/// Every place a math function is parsed for.
const WANTS: [Want; 8] = [
    Want::Number,
    Want::NumberOrPercentage,
    Want::Length,
    Want::LengthPercentage,
    Want::Angle,
    Want::Time,
    Want::Resolution,
    Want::Percentage,
];

/// The bases a percentage is resolved against: none, the largest a
/// length is, and one past what any box can be.
const BASES: [f32; 5] = [0.0, 1.0, 800.0, MAX_LENGTH_PX, f32::MAX];

/// The extremes of the counters' range, and the edges of the predefined
/// styles' ranges.
const EXTREMES: [i32; 8] = [i32::MIN, i32::MIN + 1, -1, 0, 1, 3999, 4000, i32::MAX];

static PREDEFINED: OnceLock<Vec<String>> = OnceLock::new();

/// The predefined counter styles' names, read from their rules.
fn predefined() -> &'static [String] {
    PREDEFINED.get_or_init(|| {
        include_str!("../../crates/style/assets/counter-styles.css")
            .split("@counter-style")
            .skip(1)
            .filter_map(|rule| rule.split_whitespace().next())
            .map(str::to_string)
            .collect()
    })
}

/// Relative lengths, of a 16-pixel font and an 800-pixel viewport alike.
struct Bases;

impl Lengths for Bases {
    fn px(&self, value: f64, _unit: LengthUnit) -> f64 {
        value * 16.0
    }
}

/// Whether `v` is a length layout can take: finite, and no larger than
/// the largest a declaration sets.
fn in_range(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_LENGTH_PX
}

/// The tokens of the math function that starts at `start`: to the
/// parenthesis that closes it, or to the end.
fn function_at(tokens: &[Token], start: usize) -> &[Token] {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        match token {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &tokens[start..=i];
                }
            }
            _ => {}
        }
    }
    &tokens[start..]
}

fn math_functions(tokens: &[Token]) {
    let starts = (0..tokens.len())
        .filter(|&i| calc::is_math_function(&tokens[i..i + 1]))
        .take(64);
    for start in starts {
        let function = function_at(tokens, start);
        for want in WANTS {
            let Some(parsed) = calc::parse(function, want) else {
                continue;
            };
            for basis in [None, Some(0.0), Some(800.0), Some(1e30)] {
                match parsed.compute(&Bases, basis) {
                    Computed::Value(v) | Computed::Percent(v) => {
                        assert!(!v.is_nan(), "{want:?} at {basis:?}: NaN")
                    }
                    Computed::Expression(expression) => {
                        for non_negative in [false, true] {
                            let calc = Calc::new(expression.clone(), non_negative);
                            for basis in BASES {
                                let v = calc.resolve(basis);
                                assert!(in_range(v), "{expression:?} of {basis}: {v}");
                                assert!(!non_negative || v >= 0.0, "{expression:?}: {v}");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Holds `style`'s custom properties and lengths to their bounds.
fn check(style: &ComputedStyle) {
    if let Some(custom) = &style.custom {
        for (name, tokens) in custom.iter() {
            let weight = tokens.len() * std::mem::size_of::<Token>();
            assert!(weight <= MAX_SUBSTITUTION, "{name} weighs {weight}");
        }
    }
    assert!(
        in_range(style.font_size_px) && style.font_size_px >= 0.0,
        "font-size {}",
        style.font_size_px
    );
    for v in [style.letter_spacing, style.word_spacing]
        .into_iter()
        .chain(style.border_width)
    {
        assert!(in_range(v), "{v}");
    }
    let lengths: Vec<LengthPercentage> = [
        style.width.length_percentage(),
        style.height.length_percentage(),
        style.min_width.length_percentage(),
        style.min_height.length_percentage(),
        style.max_width.length_percentage(),
        style.max_height.length_percentage(),
        style.row_gap.clone(),
        style.column_gap.clone(),
    ]
    .into_iter()
    .flatten()
    .chain(style.padding.iter().cloned())
    .collect();
    let edges: Vec<&LengthPercentageAuto> = style.margin.iter().chain(&style.inset).collect();
    for basis in BASES {
        for length in &lengths {
            let v = length.resolve(basis);
            assert!(in_range(v), "{length:?} of {basis}: {v}");
        }
        for edge in &edges {
            let v = edge.resolve(basis, 0.0);
            assert!(in_range(v), "{edge:?} of {basis}: {v}");
        }
    }
}

/// Holds every text a pseudo-element generates in `boxes` to its bound.
fn check_generated(boxes: &[layout::LayoutBox]) {
    for b in boxes {
        if let (Some(_), layout::BoxKind::Text(text)) = (b.generated, &b.kind) {
            assert!(
                text.len() <= layout::MAX_GENERATED_TEXT,
                "a generated text of {} bytes",
                text.len()
            );
        }
        check_generated(&b.children);
    }
}

fn counter_styles(tokens: &[Token]) {
    let written = |name: &str, value: i32| {
        let representation = counter_style::represent(name, value);
        assert!(
            representation.chars().count() <= MAX_REPRESENTATION,
            "{value} in {name}: {representation}"
        );
        // The predefined styles' prefixes and suffixes are a character
        // or two.
        if let Some(marker) = counter_style::marker(name, value) {
            assert!(
                marker.chars().count() <= representation.chars().count() + 4,
                "{value}'s marker in {name}: {marker}"
            );
        }
    };
    let values = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Number {
                value,
                is_integer: true,
            } => Some(value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32),
            _ => None,
        })
        .take(8);
    for value in values {
        for name in predefined() {
            written(name, value);
        }
    }
    let names = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Ident(name) => Some(name.as_str()),
            _ => None,
        })
        .take(8);
    for name in names {
        for value in EXTREMES {
            written(name, value);
            written(&counter_style::canonical_name(name), value);
        }
    }
}

static DOC: OnceLock<dom::Document> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 4096 {
        return;
    }
    let mut parts = s.split('\0');
    let source = parts.next().unwrap_or_default();
    let tokens = css::tokenize(source);
    math_functions(&tokens);
    counter_styles(&tokens);

    let Ok(sheet) = css::parse_stylesheet(source) else {
        return;
    };
    let mut document = DOC
        .get_or_init(|| {
            let page = html::parse_document(PAGE).expect("the fixed page parses");
            dom::Document::from_node(&page)
        })
        .clone();
    let elements: Vec<(dom::NodeId, String)> = document
        .subtree(document.root())
        .filter_map(|n| Some((n, document.tag(n)?.to_string())))
        .collect();
    for ((name, on), value) in ATTRIBUTES.iter().zip(parts) {
        for (element, tag) in &elements {
            if on.is_none_or(|on| on == tag) {
                document
                    .set_attribute(*element, name, value)
                    .expect("an element");
            }
        }
    }
    let sheets = Stylesheets::new(vec![sheet]);
    let context = CascadeContext {
        viewport_width: 800.0,
        viewport_height: 600.0,
        media: Media::screen(800.0, 600.0),
        ..CascadeContext::default()
    };
    let styles = sheets.cascade(&document, &context);
    for (element, _) in &elements {
        if let Some(style) = styles.get(*element) {
            check(style);
        }
        if let Some(pseudo) = styles.pseudo_styles(*element) {
            for which in [
                PseudoElement::Before,
                PseudoElement::After,
                PseudoElement::Marker,
                PseudoElement::FirstLetter,
                PseudoElement::FirstLine,
            ] {
                if let Some(style) = pseudo.get(which) {
                    check(style);
                }
            }
        }
    }
    let recascade = sheets.recascader(&document, &styles, &context);
    check_generated(&layout::build_box_tree(&document, &styles, &recascade));
});
